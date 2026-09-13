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
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::Serialize;

use crate::lmu::{Config as LmuConfig, Lmu};
use crate::session::{self, Session};
use crate::settings::Settings;
use crate::{Error, Result};

/// How long to wait before looking for the game again.
const IDLE_POLL: Duration = Duration::from_secs(5);

/// A recording smaller than this never saw a lap — a couple of frames from a
/// game that opened and closed. The server has nothing to do with it.
///
/// ponytail: a size check, not a frame count, so the sweep doesn't read every
/// file every cycle. One JSON line is ~600 bytes.
const MIN_SESSION_BYTES: u64 = 4096;

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
}

static RECORDING: AtomicBool = AtomicBool::new(false);
static LIVE: Mutex<Option<Live>> = Mutex::new(None);
static LAST: Mutex<Option<LastPush>> = Mutex::new(None);

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

    loop {
        let settings = Settings::load_default();
        if settings.sync_enabled {
            push_pending();
            match Lmu::start(LmuConfig::default()) {
                Ok(source) => {
                    last_err.clear();
                    record(&source);
                    continue; // straight back round to push what was just recorded
                }
                Err(e) => {
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
        }
        std::thread::sleep(IDLE_POLL);
    }
}

/// Record `source` until the game exits.
fn record(source: &Lmu) {
    let mut rec = match session::Recorder::create() {
        Ok(rec) => rec,
        Err(e) => {
            log::warn!("sync: couldn't open a recording: {e}");
            return;
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
    RECORDING.store(true, Ordering::Relaxed);

    loop {
        match source.next_frame_timeout(Duration::from_millis(250)) {
            Some(frame) => {
                if let Err(e) = rec.write(&frame) {
                    log::warn!("sync: recording stopped: {e}");
                    break;
                }
                // Car and track arrive when the driver loads a session, not
                // when the game opens — take the first non-empty pair and
                // leave it alone after that.
                if let Some(live) = lock(&LIVE).as_mut() {
                    live.frames = rec.frames();
                    if live.car.is_empty() {
                        live.car.clone_from(&frame.car_name);
                    }
                    if live.track.is_empty() {
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
/// has accepted.
///
/// The first failure ends the sweep: they share one cause (offline, signed
/// out, server down), and hammering the API once per file helps nobody. The
/// files stay put and the next cycle tries again.
fn push_pending() {
    let Ok(dir) = session::dir() else { return };
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
                return;
            }
            Err(e) => {
                log::warn!("sync: couldn't push {}: {e}", s.file);
                *lock(&LAST) = Some(LastPush {
                    file: s.file,
                    at_unix: now_unix(),
                    url: None,
                    error: Some(e.to_string()),
                });
                return;
            }
        }
    }
}

/// One upload. Private: the site has no public telemetry feed yet.
///
/// ponytail: hard-coded visibility. Add a Settings toggle when there's
/// somewhere public for a session to land.
fn push(file: &str) -> Result<()> {
    let shared = session::share(file, true)?;
    log::info!("sync: pushed {file} -> {}", shared.url);
    *lock(&LAST) = Some(LastPush {
        file: file.to_string(),
        at_unix: now_unix(),
        url: Some(shared.url),
        error: None,
    });
    Ok(())
}
