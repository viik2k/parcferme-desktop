//! Recording a live [`crate::lmu`] frame stream to disk.
//!
//! One session is one JSON-Lines file under
//! `%LOCALAPPDATA%\cc.parcferme.desktop\sessions` — the same frame the relay
//! would put on the wire, one object per line, in arrival order. Plain text
//! because every consumer we have (the CLI's `jq`, a future upload to
//! parcferme.cc, a replay) already speaks it, and because a half-written file
//! from a crash mid-session still replays up to the last complete line.
//!
//! A line is flushed per frame: at 10 Hz that is cheap, and the alternative is
//! losing the last few seconds of every session that ends with the game
//! crashing — which is exactly the session worth keeping.

use std::fs::{self, File};
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use flate2::write::GzEncoder;
use flate2::Compression;
use serde::{Deserialize, Serialize};

use crate::api::{ApiClient, TelemetryMeta, UploadResult};
use crate::auth;
use crate::lmu::Frame;
use crate::{Error, Result, APP_ID};

/// Where sessions are kept: `%LOCALAPPDATA%\cc.parcferme.desktop\sessions`.
///
/// Local, not roaming, app data — a long stint is tens of megabytes and has no
/// business syncing to a domain profile (settings.json, which is tiny, lives in
/// `%APPDATA%`).
pub fn dir() -> Result<PathBuf> {
    let root = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .ok_or_else(|| Error::Io(std::io::Error::other("LOCALAPPDATA is not set")))?;
    Ok(root.join(APP_ID).join("sessions"))
}

/// Seconds since the Unix epoch. The file name carries this rather than a
/// formatted date so `pf_core` needs no calendar dependency — the UI already
/// has `Date`, and it knows the user's locale and we don't.
fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// An open recording. Dropping it closes the file; the session is whatever was
/// written by then.
pub struct Recorder {
    path: PathBuf,
    out: BufWriter<File>,
    frames: u64,
}

impl Recorder {
    /// Start a recording in the default [`dir`], creating it if needed.
    pub fn create() -> Result<Recorder> {
        Recorder::create_in(&dir()?)
    }

    /// Start a recording in `dir` (the seam the tests use).
    pub fn create_in(dir: &Path) -> Result<Recorder> {
        fs::create_dir_all(dir)?;
        let path = dir.join(format!("session-{}.jsonl", now_unix()));
        let out = BufWriter::new(File::create(&path)?);
        Ok(Recorder {
            path,
            out,
            frames: 0,
        })
    }

    /// Append one frame. A frame that fails to serialize is dropped with a
    /// warning rather than ending the recording — one bad line must not cost
    /// the rest of the stint.
    pub fn write(&mut self, frame: &Frame) -> Result<()> {
        let line = match serde_json::to_string(frame) {
            Ok(line) => line,
            Err(e) => {
                log::warn!("skipped a frame that wouldn't serialize: {e}");
                return Ok(());
            }
        };
        self.out.write_all(line.as_bytes())?;
        self.out.write_all(b"\n")?;
        self.out.flush()?;
        self.frames += 1;
        Ok(())
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn frames(&self) -> u64 {
        self.frames
    }
}

/// One recorded session on disk.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Session {
    /// File name, e.g. `session-1757203200.jsonl`.
    pub file: String,
    /// When the recording started, seconds since the Unix epoch.
    pub started_unix: u64,
    pub bytes: u64,
}

/// Recorded sessions, newest first. A missing directory is an empty list — a
/// user who has never recorded is not an error.
pub fn list() -> Vec<Session> {
    match dir() {
        Ok(dir) => list_in(&dir),
        Err(e) => {
            log::warn!("no sessions dir: {e}");
            Vec::new()
        }
    }
}

/// [`list`], against an explicit directory.
pub fn list_in(dir: &Path) -> Vec<Session> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut sessions: Vec<Session> = entries
        .flatten()
        .filter_map(|entry| {
            let file = entry.file_name().to_string_lossy().into_owned();
            let started_unix = file
                .strip_prefix("session-")?
                .strip_suffix(".jsonl")?
                .parse()
                .ok()?;
            Some(Session {
                file,
                started_unix,
                bytes: entry.metadata().map_or(0, |m| m.len()),
            })
        })
        .collect();
    sessions.sort_unstable_by_key(|s| std::cmp::Reverse(s.started_unix));
    sessions
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A recording round-trips: every frame written is one readable line, and
    /// the file shows up in the listing with the size it has on disk.
    #[test]
    fn records_frames_as_lines_and_lists_them() {
        let dir = std::env::temp_dir().join(format!("pf-sessions-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);

        let mut rec = Recorder::create_in(&dir).expect("create recorder");
        let frame = crate::lmu::Frame::default();
        rec.write(&frame).unwrap();
        rec.write(&frame).unwrap();
        let path = rec.path().to_path_buf();
        assert_eq!(rec.frames(), 2);
        drop(rec);

        let body = fs::read_to_string(&path).unwrap();
        assert_eq!(body.lines().count(), 2);
        let parsed: serde_json::Value = serde_json::from_str(body.lines().next().unwrap()).unwrap();
        assert_eq!(parsed["seq"], 0);

        let listed = list_in(&dir);
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].bytes, body.len() as u64);

        let _ = fs::remove_dir_all(&dir);
    }

    /// A summary describes the session, not just its first frame: the car and
    /// track come from the first frame that has them (a recording opens in the
    /// menus with both empty), and the best lap is the fastest one seen.
    #[test]
    fn summarize_reads_names_and_the_best_lap() {
        let dir = std::env::temp_dir().join(format!("pf-summary-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);

        let mut rec = Recorder::create_in(&dir).expect("create recorder");
        let mut frame = crate::lmu::Frame::default();
        rec.write(&frame).unwrap(); // still in the menus: no names, no lap
        frame.t_ms = 1_000;
        frame.car_name = "Ferrari 499P".into();
        frame.track_name = "Le Mans 24h".into();
        frame.last_lap_time_s = Some(212.5);
        rec.write(&frame).unwrap();
        frame.t_ms = 2_500;
        frame.last_lap_time_s = Some(209.25);
        rec.write(&frame).unwrap();
        let path = rec.path().to_path_buf();
        drop(rec);

        // A half-written final line — how a session that ended in a crash looks.
        let mut f = fs::OpenOptions::new().append(true).open(&path).unwrap();
        f.write_all(b"{\"t_ms\": 3000, \"spe").unwrap();
        drop(f);

        let s = summarize(&path).expect("summarize");
        assert_eq!(s.car, "Ferrari 499P");
        assert_eq!(s.track, "Le Mans 24h");
        assert_eq!(s.frames, 3, "the torn line is skipped, not counted");
        assert_eq!(s.duration_s, 2.5);
        assert_eq!(s.best_lap_s, Some(209.25));
        assert!(s.started_unix > 0);

        let _ = fs::remove_dir_all(&dir);
    }

    /// The share path takes a file name from the UI, so it must refuse anything
    /// that could walk out of the sessions folder — before it touches the
    /// keychain or the network.
    #[test]
    fn share_refuses_anything_that_is_not_a_bare_file_name() {
        for name in ["", "..", "../secrets.jsonl", "sub/dir.jsonl", r"a\b.jsonl"] {
            let err = share(name, false).expect_err(name);
            assert!(
                matches!(err, Error::Api(_)),
                "{name} should be rejected by name, got {err:?}"
            );
        }
    }
}

// --------------------------------------------------------------- for sharing

/// The fields of a frame a summary needs. Everything else in the line is
/// ignored, which keeps a 200 MB file's scan to the parse of six values a line.
#[derive(Deserialize)]
struct SummaryLine {
    t_ms: u64,
    #[serde(default)]
    car_name: String,
    #[serde(default)]
    track_name: String,
    #[serde(default)]
    last_lap_time_s: Option<f64>,
}

/// What a recorded file says about itself — the metadata SERVER_CONTRACT §10's
/// create call wants.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Summary {
    /// The game's own name for the car, e.g. "Ferrari 499P". Empty when the
    /// recording never saw a session (the game sat in the menus).
    pub car: String,
    /// The game's own name for the track, e.g. "Le Mans 24h".
    pub track: String,
    pub started_unix: u64,
    /// Wall clock from the first frame to the last.
    pub duration_s: f64,
    pub frames: u64,
    /// Fastest completed lap in the recording, if any.
    pub best_lap_s: Option<f64>,
}

/// Read a recorded session end to end and describe it.
///
/// Car and track are taken from the first frame that *has* them rather than the
/// first frame: a recording that starts in the garage opens with several
/// seconds of empty names, and a session labelled "" is worthless to share.
///
/// A malformed line is skipped, not fatal — the last line of a session that
/// ended in a crash is routinely half-written, and that session is exactly the
/// one worth keeping.
pub fn summarize(path: &Path) -> Result<Summary> {
    let mut first_t = None;
    let mut last_t = 0;
    let mut frames = 0;
    let mut car = String::new();
    let mut track = String::new();
    let mut best_lap_s: Option<f64> = None;

    for line in BufReader::new(File::open(path)?).lines() {
        let Ok(line) = line else { break };
        let Ok(frame) = serde_json::from_str::<SummaryLine>(&line) else {
            continue;
        };
        frames += 1;
        first_t.get_or_insert(frame.t_ms);
        last_t = frame.t_ms;
        if car.is_empty() && !frame.car_name.is_empty() {
            car = frame.car_name;
        }
        if track.is_empty() && !frame.track_name.is_empty() {
            track = frame.track_name;
        }
        if let Some(lap) = frame.last_lap_time_s {
            if best_lap_s.is_none_or(|best| lap < best) {
                best_lap_s = Some(lap);
            }
        }
    }

    Ok(Summary {
        car,
        track,
        started_unix: started_unix(path),
        duration_s: (last_t - first_t.unwrap_or(0)) as f64 / 1000.0,
        frames,
        best_lap_s,
    })
}

/// The recording start, off the file name (see [`Recorder::create_in`]), or the
/// file's mtime when someone has renamed it.
fn started_unix(path: &Path) -> u64 {
    let from_name = path
        .file_name()
        .and_then(|f| f.to_str())
        .and_then(|f| f.strip_prefix("session-"))
        .and_then(|f| f.strip_suffix(".jsonl"))
        .and_then(|f| f.parse().ok());
    from_name.unwrap_or_else(|| {
        fs::metadata(path)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map_or(0, |d| d.as_secs())
    })
}

/// Gzip a session for upload, in memory.
///
/// ponytail: the compressed side is what is held, not the raw file — JSON lines
/// of mostly-numbers go ~10:1, so an hour's recording lands around 2 MB. Stream
/// it to a temp file instead if a session ever gets long enough for that to
/// matter.
pub fn gzip(path: &Path) -> Result<Vec<u8>> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    std::io::copy(&mut File::open(path)?, &mut encoder)?;
    Ok(encoder.finish()?)
}

/// Share one recorded session on parcferme.cc (SERVER_CONTRACT §10).
///
/// `file` is a bare file name inside [`dir`], never a path: the UI passes back
/// what [`list`] gave it, and joining a caller-supplied path here would let a
/// `..` walk out of the sessions folder and upload anything on disk.
///
/// Blocking and slow by nature — the scan reads the whole file and the PUT
/// sends every compressed byte — so callers run it off the UI thread.
pub fn share(file: &str, private: bool) -> Result<UploadResult> {
    if file.is_empty() || Path::new(file).file_name().is_none_or(|f| f != file) {
        return Err(Error::Api(format!("{file:?} is not a session file name")));
    }
    let path = dir()?.join(file);
    let summary = summarize(&path)?;
    if summary.frames == 0 {
        return Err(Error::Api(format!(
            "{file} has no frames in it — nothing to share"
        )));
    }

    let token = auth::current_token()?.ok_or(Error::NotLinked)?;
    let body = gzip(&path)?;
    log::info!(
        "sharing {file}: {} frames, {:.0}s, {} -> {} bytes gzipped",
        summary.frames,
        summary.duration_s,
        std::fs::metadata(&path).map_or(0, |m| m.len()),
        body.len(),
    );

    let client = ApiClient::from_env();
    let pending = client.create_telemetry(
        token.as_str(),
        &TelemetryMeta {
            // One sim records today; the field exists so the server never has
            // to guess when iRacing and ACC sources land.
            sim: "lmu",
            car: summary.car,
            track: summary.track,
            started_unix: summary.started_unix,
            duration_s: summary.duration_s,
            frames: summary.frames,
            // The source's default. Recording at another rate means passing
            // the rate through from the running config.
            hz: crate::lmu::Config::default().target_hz,
            best_lap_s: summary.best_lap_s,
            filename: format!("{file}.gz"),
            bytes: body.len() as u64,
            content_encoding: "gzip",
            private,
        },
    )?;
    client.put_presigned(&pending.upload_url, &body)?;
    // Only now is the row worth showing: a session that never finished its PUT
    // must not appear on the site pointing at bytes that aren't there.
    client.complete_telemetry(token.as_str(), &pending.id)
}
