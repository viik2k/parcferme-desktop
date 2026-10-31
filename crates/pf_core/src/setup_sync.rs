//! Setup Auto-Sync: a setup saved in the garage reaches the site with no
//! button to press (#32, parc-ferme#131, SERVER_CONTRACT §7c).
//!
//! Same shape as [`crate::sync`] (one daemon thread, [`Settings`] read fresh
//! each cycle, off until the user opts in), but the source is the sims' setups
//! folders, swept by a plain mtime+size poll. No OS watch API: that would be
//! the first callback-shaped dependency in a synchronous crate, and a poll
//! every few seconds over a few hundred small files costs nothing.
//!
//! - **Baseline first.** Turning it on records every file already there and
//!   pushes none of them. Turning it off forgets the baseline, so switching it
//!   back on never dumps what changed in between.
//! - **Settle, then push.** A file is pushed only once its mtime and size have
//!   held still for a whole cycle, and only if its bytes actually changed:
//!   sims write in small pieces, and a half-written setup on the site is worse
//!   than a missing one.
//! - **New setup, then versions.** The first push of a file creates a private
//!   setup; every later save of it is a new version of that setup (`setup=`).
//!   A manual upload from the Push tab counts as the first push too.
//! - **No echo.** A file the app installed from the site (equip, team
//!   auto-install) is marked foreign and skipped until the user changes it.
//! - **Not other apps' setups.** Folders that setup apps install into
//!   ([`VENDOR_DIRS`]) are never walked, and more than [`MAX_NEW_PER_CYCLE`]
//!   new files settling at once is treated as an installer drop: marked
//!   foreign, reported once on the Sync tab, not pushed.
//! - **Skip and continue.** A file the site refuses (no car, no track, 422) is
//!   reported and left alone until it changes again; it never blocks the rest.
//!   Offline or signed out ends the cycle and waits, like [`crate::sync`].
//!
//! The state file (`setup-sync.json`, next to `settings.json`) maps each file
//! to what was last seen and the setup it became. It is its own file, not part
//! of the hand-editable settings.

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::settings::Settings;
use crate::sim::Sim;
use crate::{paths, sync, upload, Error, Result};

/// How often to sweep the setups folders. A save shows up on the site within
/// two of these (one to see it, one to see it has settled).
const POLL: Duration = Duration::from_secs(10);

/// How long to leave the folders alone after a cycle failed offline or signed out.
const BACKOFF: Duration = Duration::from_secs(60);

/// Most pushes per cycle. The upload route allows 20 a minute per user; the
/// rest stay unsynced and go on the next cycle.
const MAX_PUSHES_PER_CYCLE: usize = 4;

/// Deepest folder level searched below a sim's setups root. The layouts are
/// one or two levels; people keep per-season subfolders under those.
const MAX_DEPTH: usize = 4;

/// How many recent pushes [`status`] keeps. Not persisted.
const HISTORY_LEN: usize = 20;

/// Folders that setup apps (Coach Dave Delta, Garage 61's agent, ...) install
/// their files into. Those are someone else's setups, often paid ones, so the
/// engine never looks inside. Compared lowercase with only letters and digits
/// kept, so "Garage 61" and "Garage61" both match. Add a name when a user
/// reports another app's folder.
const VENDOR_DIRS: &[&str] = &[
    "coachdaveacademy",
    "garage61",
    "gofast",
    "vrs",
    "tracktitan",
    "hymo",
];

/// More new files than this settling in one sweep is an installer drop, not
/// garage saves (a save writes one file): they're recorded as someone else's
/// and not pushed. Catches the apps [`VENDOR_DIRS`] doesn't name.
const MAX_NEW_PER_CYCLE: usize = 2;

/// When a file was last written and how big it was: the cheap change check.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Stamp {
    mtime_ms: u64,
    size: u64,
}

impl Stamp {
    fn of(meta: &std::fs::Metadata) -> Stamp {
        let mtime_ms = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map_or(0, |d| d.as_millis() as u64);
        Stamp {
            mtime_ms,
            size: meta.len(),
        }
    }
}

/// What the engine last knew about one file.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct Entry {
    #[serde(flatten)]
    stamp: Stamp,
    /// [`fingerprint`] of the bytes last seen (pushed, baselined or installed).
    hash: u64,
    /// The site setup this file became; later saves are versions of it.
    setup: Option<String>,
    /// Installed from the site by this app, not written by the user.
    foreign: bool,
}

#[derive(Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct State {
    /// Whether the files below are a baseline to diff against. False until
    /// the first cycle after the toggle goes on.
    seeded: bool,
    files: HashMap<PathBuf, Entry>,
}

/// One push the engine attempted, for the Sync tab and the toast poller.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Pushed {
    pub file: String,
    pub sim: Sim,
    pub at_unix: u64,
    /// The setup's page, when it went up.
    pub url: Option<String>,
    /// The version it is on the site: 1 for a new setup.
    pub version: Option<u32>,
    /// Why it didn't go up. Permanent: the file waits until it changes again.
    pub error: Option<String>,
}

/// Everything the Sync tab renders for setups.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub enabled: bool,
    /// Newest first.
    pub recent: Vec<Pushed>,
}

static RECENT: Mutex<VecDeque<Pushed>> = Mutex::new(VecDeque::new());

/// Serialises every load-modify-save of the state file: the engine, a manual
/// upload and a download each write to it from their own thread.
static STATE: Mutex<()> = Mutex::new(());

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// FNV-1a over the bytes: stable across Rust releases, unlike `DefaultHasher`,
/// which matters because it's persisted. A change check, not a security one;
/// the server keeps its own sha256 (§7c).
fn fingerprint(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |h, &b| {
        (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
    })
}

/// The state file's key for `path`. Windows paths are case-insensitive and
/// reach us from different places (the file picker for a manual upload, the
/// detected setups root for a sweep), so the key is lowercased with one kind of
/// separator, or a manual upload would never be matched to its later saves.
/// The real path is still what gets read and uploaded.
fn key(path: &Path) -> PathBuf {
    if cfg!(windows) {
        PathBuf::from(path.to_string_lossy().replace('/', "\\").to_lowercase())
    } else {
        path.to_path_buf()
    }
}

fn state_path() -> Result<PathBuf> {
    Ok(crate::settings::default_path()?.with_file_name("setup-sync.json"))
}

/// Missing or corrupt reads as empty and unseeded: the next cycle baselines
/// again, and the server's dedupe (§7c) answers any re-push with the setup it
/// already has.
fn load(path: &Path) -> State {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

fn save(state: &State, path: &Path) {
    let write = || -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec(state)?)?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    };
    if let Err(e) = write() {
        log::warn!("setup sync: couldn't save {}: {e}", path.display());
    }
}

/// Load, change and save the state file under [`STATE`]. Only writes when
/// something changed, so an idle sweep costs a read and no write.
fn with_state<R>(f: impl FnOnce(&mut State) -> R) -> Option<R> {
    let path = state_path().ok()?;
    let _guard = lock(&STATE);
    let before = load(&path);
    let mut state = State {
        seeded: before.seeded,
        files: before.files.clone(),
    };
    let out = f(&mut state);
    if state != before {
        save(&state, &path);
    }
    Some(out)
}

/// A file this app just installed from the site. Skipped by the engine until
/// its bytes change, so someone else's setup is never pushed as the user's.
pub(crate) fn note_installed(path: &Path) {
    let (Ok(meta), Ok(bytes)) = (std::fs::metadata(path), std::fs::read(path)) else {
        return;
    };
    with_state(|s| {
        s.files.insert(
            key(path),
            Entry {
                stamp: Stamp::of(&meta),
                hash: fingerprint(&bytes),
                setup: None,
                foreign: true,
            },
        );
    });
}

/// A file that just went up as `setup_id` (manual or automatic): later saves
/// of it are versions of that setup. `meta` is from before the read, so a write
/// racing the upload shows up as a change on the next sweep.
pub(crate) fn note_uploaded(path: &Path, meta: &std::fs::Metadata, bytes: &[u8], setup_id: &str) {
    with_state(|s| {
        s.files.insert(
            key(path),
            Entry {
                stamp: Stamp::of(meta),
                hash: fingerprint(bytes),
                setup: Some(setup_id.to_string()),
                foreign: false,
            },
        );
    });
}

/// What the engine has pushed recently, and whether it's on.
pub fn status() -> Status {
    Status {
        enabled: Settings::load_default().setup_sync_enabled,
        recent: lock(&RECENT).iter().cloned().collect(),
    }
}

/// Start the engine on its own thread. Call once, at startup.
pub fn spawn() {
    if let Err(e) = std::thread::Builder::new()
        .name("pf-setup-sync".into())
        .spawn(run)
    {
        log::error!("setup sync engine didn't start: {e}");
    }
}

fn run() {
    // Stamps seen last sweep that differ from the state file: a file is only
    // pushed once it shows the same stamp twice.
    let mut settling: HashMap<PathBuf, Stamp> = HashMap::new();
    let mut unseeded = false;
    let mut resume = Instant::now();
    loop {
        let settings = Settings::load_default();
        if !settings.setup_sync_enabled {
            if !unseeded {
                with_state(|s| s.seeded = false);
                settling.clear();
                unseeded = true;
            }
        } else if Instant::now() >= resume {
            unseeded = false;
            if !cycle(&settings, &mut settling) {
                resume = Instant::now() + BACKOFF;
            }
        }
        std::thread::sleep(POLL);
    }
}

/// Every setup file under each sim's setups root, with its stamp.
fn scan(settings: &Settings) -> HashMap<PathBuf, (Sim, Stamp)> {
    let mut found = HashMap::new();
    for sim in Sim::ALL {
        if let Ok(root) = paths::resolve_setups_dir(sim, settings.sim_folders.get(&sim).cloned()) {
            walk(&root, sim, 0, &mut found);
        }
    }
    found
}

fn walk(dir: &Path, sim: Sim, depth: usize, found: &mut HashMap<PathBuf, (Sim, Stamp)>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(meta) = entry.metadata() else { continue };
        let name = entry.file_name().to_string_lossy().into_owned();
        if meta.is_dir() {
            if depth < MAX_DEPTH && !is_vendor_dir(&name) {
                walk(&path, sim, depth + 1, found);
            }
        // Dot-files are our own `.name.part` downloads in flight.
        } else if !name.starts_with('.') && Sim::from_filename(&name) == Some(sim) {
            found.insert(path, (sim, Stamp::of(&meta)));
        }
    }
}

/// Files to push, and an installer drop held back: its size and sim.
type Plan = (Vec<(PathBuf, Sim)>, Option<(usize, Sim)>);

fn is_vendor_dir(name: &str) -> bool {
    let norm: String = name
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|c| c.to_ascii_lowercase())
        .collect();
    VENDOR_DIRS.contains(&norm.as_str())
}

/// Compare a sweep with the state: baseline when unseeded, otherwise return
/// the settled files whose bytes changed, oldest first. An installer drop
/// ([`MAX_NEW_PER_CYCLE`]) is recorded as foreign instead, and comes back as
/// its size and sim. Forgets files that are gone. Pure apart from reading the
/// changed files' bytes.
fn plan(
    state: &mut State,
    scanned: &HashMap<PathBuf, (Sim, Stamp)>,
    settling: &mut HashMap<PathBuf, Stamp>,
) -> Plan {
    let keys: std::collections::HashSet<PathBuf> = scanned.keys().map(|p| key(p)).collect();
    state.files.retain(|k, _| keys.contains(k));
    settling.retain(|p, _| scanned.contains_key(p));

    if !state.seeded {
        for (path, &(_, stamp)) in scanned {
            if let Ok(bytes) = std::fs::read(path) {
                let e = state.files.entry(key(path)).or_default();
                e.stamp = stamp;
                e.hash = fingerprint(&bytes);
            }
        }
        state.seeded = true;
        settling.clear();
        log::info!(
            "setup sync: baseline of {} setup(s), none pushed",
            scanned.len()
        );
        return (Vec::new(), None);
    }

    let mut push = Vec::new();
    let mut new = Vec::new();
    for (path, &(sim, stamp)) in scanned {
        let k = key(path);
        let known = state.files.get(&k);
        if known.is_some_and(|e| e.stamp == stamp) {
            settling.remove(path);
            continue;
        }
        if settling.insert(path.clone(), stamp) != Some(stamp) {
            continue; // still being written, or just noticed
        }
        settling.remove(path);
        let Ok(bytes) = std::fs::read(path) else {
            continue;
        };
        let hash = fingerprint(&bytes);
        if known.is_some_and(|e| e.hash == hash) {
            // Touched, not changed (or a foreign file re-saved as-is).
            if let Some(e) = state.files.get_mut(&k) {
                e.stamp = stamp;
            }
            continue;
        }
        if known.is_none() {
            new.push((k, sim, stamp, hash));
        }
        push.push((path.clone(), sim, stamp.mtime_ms));
    }
    // ponytail: a drop the installer spreads over several sweeps gets past
    // this; VENDOR_DIRS is the backstop for the apps we know.
    let ignored = if new.len() > MAX_NEW_PER_CYCLE {
        push.retain(|(p, _, _)| state.files.contains_key(&key(p)));
        for (k, _, stamp, hash) in &new {
            state.files.insert(
                k.clone(),
                Entry {
                    stamp: *stamp,
                    hash: *hash,
                    setup: None,
                    foreign: true,
                },
            );
        }
        log::info!("setup sync: {} new setups at once, not pushed", new.len());
        Some((new.len(), new[0].1))
    } else {
        None
    };
    push.sort_by_key(|&(_, _, mtime)| mtime);
    (
        push.into_iter().map(|(p, sim, _)| (p, sim)).collect(),
        ignored,
    )
}

/// One sweep and its pushes. False when it stopped on something that affects
/// every file (offline, signed out), so the caller backs off.
fn cycle(settings: &Settings, settling: &mut HashMap<PathBuf, Stamp>) -> bool {
    let scanned = scan(settings);
    let Some((changed, ignored)) = with_state(|s| plan(s, &scanned, settling)) else {
        return true;
    };
    if let Some((ignored, sim)) = ignored {
        record(
            Path::new(&format!("{ignored} setups")),
            sim,
            None,
            Some(
                "appeared together, so another app probably installed them. Not uploaded; edit one in the garage and it will be"
                    .into(),
            ),
        );
    }
    for (path, sim) in changed.into_iter().take(MAX_PUSHES_PER_CYCLE) {
        match push(&path, sim, settings) {
            Ok(()) => {}
            Err(e @ (Error::NotLinked | Error::DeviceRevoked | Error::Http(_))) => {
                log::debug!("setup sync: {} waits: {e}", path.display());
                return false;
            }
            Err(e) => {
                log::warn!("setup sync: {} not pushed: {e}", path.display());
                skip(&path, &scanned);
                record(&path, sim, None, Some(e.to_string()));
            }
        }
    }
    true
}

/// Mark a refused file as seen at its current bytes, so it waits for the user's
/// next change instead of being retried every cycle.
fn skip(path: &Path, scanned: &HashMap<PathBuf, (Sim, Stamp)>) {
    let (Some(&(_, stamp)), Ok(bytes)) = (scanned.get(path), std::fs::read(path)) else {
        return;
    };
    with_state(|s| {
        let e = s.files.entry(key(path)).or_default();
        e.stamp = stamp;
        e.hash = fingerprint(&bytes);
    });
}

/// Push one changed file: a version of the setup it already became, or a new
/// private setup. A successful upload updates the state itself
/// ([`note_uploaded`]).
fn push(path: &Path, sim: Sim, settings: &Settings) -> Result<()> {
    let known = with_state(|s| s.files.get(&key(path)).cloned()).flatten();
    // A recording of this game running right now: link the setup to it (§7c).
    let live = if sim == Sim::Lmu { sync::live() } else { None };
    let session = live.as_ref().map(|l| l.started_unix);
    // Folder ids only, never a fuzzy match: nobody is looking at this upload
    // to correct a guess. `upload_setup` still applies the curated aliases.
    let id = upload::identify_with_cars(path, settings, &[]);
    let export = id.garage_export.as_deref().map(Path::new);

    if let Some(setup) = known
        .as_ref()
        .filter(|e| !e.foreign)
        .and_then(|e| e.setup.clone())
    {
        let car = id.car.as_deref().unwrap_or("");
        match upload::upload_setup(
            path,
            sim,
            car,
            id.track.as_deref(),
            None,
            &[],
            None,
            true,
            export,
            Some(&setup),
            session,
        ) {
            Ok(out) => {
                record(path, sim, Some(&out.result), None);
                return Ok(());
            }
            // Deleted on the site: start over as a new setup.
            Err(Error::SetupNotFound) => {}
            Err(e) => return Err(e),
        }
    }

    // LMU files setups by track alone; the car is the one in the garage.
    let car = id
        .car
        .or_else(|| live.map(|l| l.car).filter(|c| !c.is_empty()))
        .ok_or_else(|| {
            Error::Api(format!(
                "couldn't tell which car this {} setup is for: upload it once from the Push tab, and later saves follow it",
                sim.display_name()
            ))
        })?;
    let out = upload::upload_setup(
        path,
        sim,
        &car,
        id.track.as_deref(),
        None,
        &[],
        None,
        true,
        export,
        None,
        session,
    )?;
    record(path, sim, Some(&out.result), None);
    Ok(())
}

fn record(path: &Path, sim: Sim, result: Option<&crate::api::UploadResult>, error: Option<String>) {
    let file = path
        .file_name()
        .map(|f| f.to_string_lossy().into_owned())
        .unwrap_or_default();
    // The server already had these exact bytes: nothing new to announce.
    if result.is_some_and(|r| r.unchanged || r.duplicate) {
        log::info!("setup sync: {file} already on the site");
        return;
    }
    if let Some(r) = result {
        log::info!("setup sync: pushed {file} as {} (v{:?})", r.id, r.version);
    }
    let mut recent = lock(&RECENT);
    recent.push_front(Pushed {
        file,
        sim,
        at_unix: now_unix(),
        url: result.map(|r| r.url.clone()),
        version: result.and_then(|r| r.version),
        error,
    });
    recent.truncate(HISTORY_LEN);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("pf-setup-sync-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn sweep(dir: &Path) -> HashMap<PathBuf, (Sim, Stamp)> {
        let mut found = HashMap::new();
        walk(dir, Sim::IRacing, 0, &mut found);
        found
    }

    /// Baseline pushes nothing; a change is pushed only once it has held still
    /// for a sweep; a touch with the same bytes is not a change; a foreign file
    /// stays quiet until the user edits it; a deleted file is forgotten.
    #[test]
    fn baseline_settle_echo_and_forget() {
        let dir = scratch("plan");
        let car = dir.join("mx5");
        std::fs::create_dir_all(&car).unwrap();
        let a = car.join("a.sto");
        std::fs::write(&a, b"one").unwrap();
        std::fs::write(car.join(".b.sto.part"), b"half").unwrap();
        std::fs::write(car.join("a.htm"), b"export").unwrap();

        let mut state = State::default();
        let mut settling = HashMap::new();
        let scanned = sweep(&dir);
        assert_eq!(scanned.len(), 1, "only real setup files: {scanned:?}");
        assert!(plan(&mut state, &scanned, &mut settling).0.is_empty());
        assert!(state.seeded);

        // Unchanged: nothing.
        assert!(plan(&mut state, &sweep(&dir), &mut settling).0.is_empty());

        // Changed: seen once, then pushed when it holds still.
        std::fs::write(&a, b"two!").unwrap();
        assert!(plan(&mut state, &sweep(&dir), &mut settling).0.is_empty());
        let out = plan(&mut state, &sweep(&dir), &mut settling).0;
        assert_eq!(out, vec![(a.clone(), Sim::IRacing)]);

        // Same bytes rewritten (new size forces a new stamp): no push.
        state.files.get_mut(&key(&a)).unwrap().hash = fingerprint(b"two!");
        state.files.get_mut(&key(&a)).unwrap().stamp.size = 0;
        assert!(plan(&mut state, &sweep(&dir), &mut settling).0.is_empty());
        assert!(plan(&mut state, &sweep(&dir), &mut settling).0.is_empty());

        // A foreign install with the bytes we recorded stays quiet...
        let b = car.join("b.sto");
        std::fs::write(&b, b"theirs").unwrap();
        state.files.insert(
            key(&b),
            Entry {
                stamp: Stamp::of(&std::fs::metadata(&b).unwrap()),
                hash: fingerprint(b"theirs"),
                setup: None,
                foreign: true,
            },
        );
        assert!(plan(&mut state, &sweep(&dir), &mut settling).0.is_empty());
        assert!(plan(&mut state, &sweep(&dir), &mut settling).0.is_empty());
        // ...until the user edits it.
        std::fs::write(&b, b"theirs, tuned").unwrap();
        plan(&mut state, &sweep(&dir), &mut settling);
        assert_eq!(
            plan(&mut state, &sweep(&dir), &mut settling).0,
            vec![(b.clone(), Sim::IRacing)]
        );

        std::fs::remove_file(&a).unwrap();
        plan(&mut state, &sweep(&dir), &mut settling);
        assert!(!state.files.contains_key(&key(&a)));

        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Another app's files are never pushed: not under a vendor folder, and not
    /// when many new ones land at once. A single garage save still is.
    #[test]
    fn vendor_folders_and_installer_drops_stay_put() {
        let dir = scratch("vendor");
        let car = dir.join("mx5");
        let delta = car.join("Coach Dave Academy").join("26S4");
        std::fs::create_dir_all(&delta).unwrap();
        std::fs::write(delta.join("x.sto"), b"paid").unwrap();
        assert!(sweep(&dir).is_empty(), "vendor folder is not walked");
        assert!(is_vendor_dir("Garage 61") && is_vendor_dir("GO-Fast"));
        assert!(!is_vendor_dir("mx5"));

        let mut state = State::default();
        let mut settling = HashMap::new();
        plan(&mut state, &sweep(&dir), &mut settling);
        assert!(state.seeded);

        // Five files at once: recorded as foreign, none pushed.
        for i in 0..5 {
            std::fs::write(car.join(format!("drop{i}.sto")), b"pack").unwrap();
        }
        assert_eq!(
            plan(&mut state, &sweep(&dir), &mut settling),
            (vec![], None)
        );
        assert_eq!(
            plan(&mut state, &sweep(&dir), &mut settling),
            (vec![], Some((5, Sim::IRacing)))
        );
        assert!(plan(&mut state, &sweep(&dir), &mut settling).0.is_empty());
        assert!(state.files[&key(&car.join("drop0.sto"))].foreign);

        // One garage save: pushed as usual.
        let mine = car.join("mine.sto");
        std::fs::write(&mine, b"mine").unwrap();
        plan(&mut state, &sweep(&dir), &mut settling);
        assert_eq!(
            plan(&mut state, &sweep(&dir), &mut settling),
            (vec![(mine, Sim::IRacing)], None)
        );

        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A manual upload (file picker path) and a sweep (detected root) must land
    /// on the same entry on Windows.
    #[cfg(windows)]
    #[test]
    fn keys_ignore_case_and_separators() {
        assert_eq!(
            key(Path::new("c:/program files (x86)/Steam/Fuji/Q.svm")),
            key(Path::new(r"C:\Program Files (x86)\steam\fuji\q.SVM"))
        );
    }

    /// The state file is persisted, so its hash must not change between builds.
    #[test]
    fn fingerprint_is_fnv1a() {
        assert_eq!(fingerprint(b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fingerprint(b"a"), 0xaf63_dc4c_8601_ec8c);
    }

    #[test]
    fn state_round_trips_and_corrupt_is_empty() {
        let dir = scratch("state");
        let path = dir.join("setup-sync.json");
        let mut state = State {
            seeded: true,
            ..State::default()
        };
        state.files.insert(
            dir.join("x.sto"),
            Entry {
                stamp: Stamp {
                    mtime_ms: 5,
                    size: 3,
                },
                hash: 7,
                setup: Some("u".into()),
                foreign: false,
            },
        );
        save(&state, &path);
        assert_eq!(load(&path), state);
        std::fs::write(&path, "{ nope").unwrap();
        assert_eq!(load(&path), State::default());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
