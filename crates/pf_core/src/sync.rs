//! The Sync Engine: record every LMU session and push it to parcferme.cc,
//! with no button to press.
//!
//! One daemon thread, started at app launch and never stopped (the process
//! exit is the stop — [`session::Recorder`] flushes per frame, so a kill mid
//! session loses at most the last line, which `summarize` already tolerates).
//!
//! The cycle is deliberately dumb:
//!
//! 1. read [`Settings`] fresh — the toggle takes effect within one nap, with
//!    no restart and nothing to invalidate;
//! 2. push whatever recordings are sitting in the sessions folder (leftovers
//!    from a crash, from an earlier stint, or from time spent signed out);
//! 3. try to open the game. If it's up, record until it goes away.
//!
//! Pushing before recording is what keeps the two safe on one thread: the file
//! being written is never a file being uploaded, because this loop can only do
//! one of them at a time.
//!
//! [`status`] is the whole of what the UI sees — the engine reports, the UI
//! never drives it.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::Serialize;

use crate::lmu::{Config as LmuConfig, Lmu};
use crate::session::{self, Session};
use crate::settings::Settings;
use crate::{Error, Result};

/// How long to wait before looking for the game again.
const IDLE_POLL: Duration = Duration::from_secs(5);

/// How long to leave the queue alone after a sweep failed offline or signed out.
const PUSH_BACKOFF: Duration = Duration::from_secs(60);

/// A recording smaller than this never saw a lap — a couple of frames from a
/// game that opened and closed. The server has nothing to do with it.
///
/// ponytail: a size check, not a frame count, so the sweep doesn't read every
/// file every cycle. One JSON line is ~600 bytes.
const MIN_SESSION_BYTES: u64 = 4096;

/// How many `.rejected` recordings to keep. They are only kept so a driver
/// can dig one out by hand; older ones are deleted so a season of over-size
/// or malformed recordings can't grow the sessions folder without bound.
/// Engineering default, not a product decision — Finn to review.
const MAX_REJECTED: usize = 20;

/// The session being recorded right now.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Live {
    /// File name in the sessions folder, so the UI can tie it to the queue.
    pub file: String,
    pub started_unix: u64,
    pub frames: u64,
    /// The game's own names, filled in once a session loads — both empty while
    /// the driver sits in the menus.
    pub car: String,
    pub track: String,
}

/// The last push the engine attempted, so the tab can show where a session
/// went (or why it didn't).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LastPush {
    pub file: String,
    pub at_unix: u64,
    /// The session's page on the site, when it went up.
    pub url: Option<String>,
    /// Why it didn't, when it didn't.
    pub error: Option<String>,
    /// The server refused this recording outright, so it was set aside as
    /// `.rejected` rather than left in the queue to retry.
    pub rejected: bool,
}

/// Why the engine can't record although the game is running: a reason the
/// driver has to act on, so the Sync tab shows it instead of "waiting".
///
/// `kind` is [`Error::kind`], the same key the IPC error contract uses, so the
/// UI picks its recovery hint off the kind and never matches on prose.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Blocked {
    pub kind: String,
    pub message: String,
}

impl Blocked {
    /// The blocker a failed start reports, or `None` for the ordinary states.
    /// The game not running is the normal idle state, not a blocker (#36).
    fn from_start_error(e: &Error) -> Option<Self> {
        match e {
            Error::LmuNotRunning => None,
            e => Some(Self {
                kind: e.kind().to_string(),
                message: e.to_string(),
            }),
        }
    }
}

/// Everything the Sync tab renders.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    /// The Settings toggle, read fresh — the engine is the authority on
    /// whether it is actually going to do anything.
    pub enabled: bool,
    pub recording: Option<Live>,
    /// Finished recordings waiting to be pushed, newest first. The one being
    /// written is *not* in here — it isn't waiting, it's still happening.
    pub pending: Vec<Session>,
    pub last_push: Option<LastPush>,
    /// Set while the game is up but can't be recorded (plugins disabled).
    pub blocked: Option<Blocked>,
}

static RECORDING: AtomicBool = AtomicBool::new(false);
static LIVE: Mutex<Option<Live>> = Mutex::new(None);
static LAST: Mutex<Option<LastPush>> = Mutex::new(None);
static BLOCKED: Mutex<Option<Blocked>> = Mutex::new(None);

/// A poisoned lock here means a thread panicked mid-update, which costs the UI
/// one stale status line and nothing more — never a crash.
fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// True while a session is being written to disk.
pub fn is_recording() -> bool {
    RECORDING.load(Ordering::Relaxed)
}

/// Why the engine can't record right now, if it's something the driver must
/// fix. A mutex read, cheap enough for the tray to poll.
pub fn blocked() -> Option<Blocked> {
    lock(&BLOCKED).clone()
}

/// What the engine is doing. Cheap enough to poll on a timer.
pub fn status() -> Status {
    let recording = lock(&LIVE).clone();
    let active = recording.as_ref().map(|l| l.file.clone());
    Status {
        enabled: Settings::load_default().sync_enabled,
        pending: session::list()
            .into_iter()
            .filter(|s| Some(&s.file) != active.as_ref())
            .collect(),
        recording,
        last_push: lock(&LAST).clone(),
        blocked: blocked(),
    }
}

/// Start the engine on its own thread. Call once, at startup.
pub fn spawn() {
    if let Err(e) = std::thread::Builder::new()
        .name("pf-sync".into())
        .spawn(run)
    {
        log::error!("sync engine didn't start: {e}");
    }
}

/// The engine loop. Runs until the process exits.
fn run() {
    // The game being down is the normal state, so start failures are logged
    // only when the reason changes — otherwise a driver who left plugins
    // disabled gets a warning every five seconds for the rest of the session.
    let mut last_err = String::new();

    // Offline or the server down: every retry reads and gzips whole recordings,
    // so a failed sweep waits PUSH_BACKOFF rather than one IDLE_POLL.
    let mut push_after = Instant::now();

    loop {
        let settings = Settings::load_default();
        if settings.sync_enabled {
            if Instant::now() >= push_after && !push_pending() {
                push_after = Instant::now() + PUSH_BACKOFF;
            }
            match Lmu::start(LmuConfig::default()) {
                Ok(source) => {
                    last_err.clear();
                    *lock(&BLOCKED) = None;
                    record(&source);
                    // Push what was just recorded on the next pass. Still
                    // nap first: `record` returns at once when the file
                    // can't be opened, and looping straight back would spin.
                    push_after = Instant::now();
                }
                Err(e) => {
                    *lock(&BLOCKED) = Blocked::from_start_error(&e);
                    let msg = e.to_string();
                    if msg != last_err {
                        match e {
                            Error::LmuNotRunning => log::debug!("sync: {msg}"),
                            _ => log::warn!("sync: {msg}"),
                        }
                        last_err = msg;
                    }
                }
            }
        } else {
            // Off means off: no stale blocker left behind for the tab.
            *lock(&BLOCKED) = None;
        }
        std::thread::sleep(IDLE_POLL);
    }
}

/// Open a recording and publish it as the live one.
fn open_recording() -> Option<session::Recorder> {
    let rec = match session::Recorder::create() {
        Ok(rec) => rec,
        Err(e) => {
            log::warn!("sync: couldn't open a recording: {e}");
            return None;
        }
    };
    log::info!("sync: recording to {}", rec.path().display());
    *lock(&LIVE) = Some(Live {
        file: rec
            .path()
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned(),
        started_unix: now_unix(),
        ..Live::default()
    });
    Some(rec)
}

/// Record `source` until the game exits or sync is switched off.
///
/// One game run can hold several sessions, so a new file starts whenever the
/// car is driven in a different car/track combo from the one this file was
/// driven in. Otherwise a Spa stint and a Le Mans race land in one upload,
/// labelled with whichever came first.
fn record(source: &Lmu) {
    let Some(mut rec) = open_recording() else {
        return;
    };
    RECORDING.store(true, Ordering::Relaxed);
    // The combo this file has been driven in; `None` while it's all menus.
    let mut driven: Option<(String, String)> = None;
    let mut checked = Instant::now();

    loop {
        // Switching sync off means stop recording, not "stop after the game
        // closes". Settings are a file read, so only every few seconds.
        if checked.elapsed() >= IDLE_POLL {
            checked = Instant::now();
            if !Settings::load_default().sync_enabled {
                log::info!("sync: switched off, recording stopped");
                break;
            }
        }
        match source.next_frame_timeout(Duration::from_millis(250)) {
            Some(frame) => {
                let moving = session::is_moving(frame.speed_kph) && !frame.car_name.is_empty();
                let combo = || (frame.car_name.clone(), frame.track_name.clone());
                if moving {
                    match &driven {
                        None => driven = Some(combo()),
                        Some(d) if *d != combo() => {
                            log::info!("sync: new car/track, starting a new recording");
                            let Some(next) = open_recording() else { break };
                            rec = next; // closes the finished one
                            driven = Some(combo());
                        }
                        Some(_) => {}
                    }
                }
                if let Err(e) = rec.write(&frame) {
                    log::warn!("sync: recording stopped: {e}");
                    break;
                }
                // The menus show the last combo loaded, possibly weeks old, so
                // the car on track overrides whatever the menu said.
                if let Some(live) = lock(&LIVE).as_mut() {
                    live.frames = rec.frames();
                    if moving || live.car.is_empty() {
                        live.car.clone_from(&frame.car_name);
                        live.track.clone_from(&frame.track_name);
                    }
                }
            }
            // A quiet session and a dead source both read as `None`, but a dead
            // one returns instantly — without this the loop would spin.
            None if !source.is_running() => break,
            None => {}
        }
    }

    RECORDING.store(false, Ordering::Relaxed);
    *lock(&LIVE) = None;
    log::info!("sync: recorded {} frames", rec.frames());
}

/// Push every recording on disk, oldest first, deleting each one the server
/// has accepted. Returns false when the sweep stopped early on a transient
/// failure, so the caller can back off.
///
/// `NotLinked` and `Http` end the sweep: they share one cause (offline,
/// signed out, server down), and hammering the API once per file helps
/// nobody. The files stay put and a later cycle tries again. `Api` is a
/// per-file rejection (413, 403, 422, ...) that will not resolve by retrying
/// — the file is renamed out of the queue (kept on disk as `.rejected`, so
/// nothing is lost) instead of being re-uploaded every cycle forever.
fn push_pending() -> bool {
    let Ok(dir) = session::dir() else { return true };
    let mut sessions = session::list();
    sessions.reverse();

    for s in sessions {
        if s.bytes < MIN_SESSION_BYTES {
            log::debug!(
                "sync: dropping {} ({} bytes, nothing in it)",
                s.file,
                s.bytes
            );
            let _ = std::fs::remove_file(dir.join(&s.file));
            continue;
        }
        match push(&s.file) {
            Ok(()) => {
                let _ = std::fs::remove_file(dir.join(&s.file));
            }
            // Signed out isn't a failure worth showing as one — the recording
            // waits, and signing in drains the queue.
            Err(Error::NotLinked) => {
                log::debug!("sync: signed out — {} waits", s.file);
                return false;
            }
            // A permanent per-file rejection: retrying it won't help, but it
            // also isn't evidence anything is wrong with the other files in
            // the queue, so the sweep keeps going.
            Err(e @ Error::Api(_)) => {
                log::warn!("sync: {} permanently rejected, skipping: {e}", s.file);
                let from = dir.join(&s.file);
                let _ = std::fs::rename(&from, from.with_extension("jsonl.rejected"));
                prune_rejected(&dir, MAX_REJECTED);
                *lock(&LAST) = Some(LastPush {
                    file: s.file,
                    at_unix: now_unix(),
                    url: None,
                    error: Some(e.to_string()),
                    rejected: true,
                });
            }
            Err(e) => {
                log::warn!("sync: couldn't push {}: {e}", s.file);
                *lock(&LAST) = Some(LastPush {
                    file: s.file,
                    at_unix: now_unix(),
                    url: None,
                    error: Some(e.to_string()),
                    rejected: false,
                });
                return false;
            }
        }
    }
    true
}

/// Delete the oldest `.rejected` files in `dir` beyond the newest `max`
/// (by modified time, name as tie-break). Anything else is left alone.
fn prune_rejected(dir: &std::path::Path, max: usize) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    let mut rejected: Vec<_> = rd
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|x| x == "rejected"))
        .map(|e| {
            let at = e
                .metadata()
                .and_then(|m| m.modified())
                .unwrap_or(UNIX_EPOCH);
            (at, e.path())
        })
        .collect();
    rejected.sort();
    let excess = rejected.len().saturating_sub(max);
    for (_, path) in rejected.into_iter().take(excess) {
        let _ = std::fs::remove_file(path);
    }
}

/// One upload. Private: the site has no public telemetry feed yet.
///
/// ponytail: hard-coded visibility. Add a Settings toggle when there's
/// somewhere public for a session to land.
fn push(file: &str) -> Result<()> {
    let Some(shared) = session::share(file, true)? else {
        // The car never moved: a game that sat in the menus. Not a push,
        // so the last-push line keeps showing the real last one.
        log::debug!("sync: {file} never left the menus, dropping it");
        return Ok(());
    };
    log::info!("sync: pushed {file} -> {}", shared.url);
    *lock(&LAST) = Some(LastPush {
        file: file.to_string(),
        at_unix: now_unix(),
        url: Some(shared.url),
        error: None,
        rejected: false,
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_fixable_start_failure_blocks() {
        // The game being closed is the normal idle state.
        assert_eq!(Blocked::from_start_error(&Error::LmuNotRunning), None);
        // Plugins off is the one the driver must act on, carried by kind.
        let b = Blocked::from_start_error(&Error::LmuPluginsDisabled).unwrap();
        assert_eq!(b.kind, "lmu_plugins_disabled");
        assert!(b.message.contains("Enable Plugins"));
    }

    #[test]
    fn prune_rejected_keeps_newest_and_only_touches_rejected() {
        let dir = std::env::temp_dir().join(format!("pf-sync-prune-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let make = |name: &str, age: u64| {
            let f = std::fs::File::create(dir.join(name)).unwrap();
            f.set_modified(UNIX_EPOCH + Duration::from_secs(1_000_000 - age))
                .unwrap();
        };
        for i in 0..5 {
            make(&format!("r{i}.jsonl.rejected"), 100 - i); // r4 newest
        }
        make("live.jsonl", 500);
        make("notes.txt", 500);

        prune_rejected(&dir, 10); // under the cap: nothing goes
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 7);

        prune_rejected(&dir, 2);
        let mut left: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        left.sort();
        assert_eq!(
            left,
            [
                "live.jsonl",
                "notes.txt",
                "r3.jsonl.rejected",
                "r4.jsonl.rejected"
            ]
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
