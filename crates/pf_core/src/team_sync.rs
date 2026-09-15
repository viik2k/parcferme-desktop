//! Team Auto-Install: install new team-vault setups with no button to press,
//! one opt-in toggle away.
//!
//! Same shape as [`crate::sync`] (one daemon thread, [`Settings`] read fresh
//! each cycle) but pull instead of push: poll `GET /api/device/setups?scope=team`
//! (SERVER_CONTRACT §9 — already ships everything a poll needs, entitlement,
//! identity and ordering, so this needs no server changes), diff the returned
//! ids against a persisted seen-set, and route every new one through the
//! existing [`crate::download::download_setup`] install path so the conflict
//! policy is honoured for free.
//!
//! First run seeds the seen-set without installing anything — otherwise
//! flipping the toggle on dumps the entire team vault into the user's sim
//! folders at once (parc-ferme#80 / this repo's #6).

use std::collections::VecDeque;
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::Serialize;

use crate::download::{self, InstallAction};
use crate::settings::Settings;
use crate::{Error, Result};

/// How often to poll the team vault. A full HTTP round trip, and setups don't
/// arrive nearly as often as telemetry does — no need for [`crate::sync`]'s
/// 5 s cadence.
const POLL_INTERVAL: Duration = Duration::from_secs(60);

/// How many recent installs [`status`] keeps for the tray-toast poller. Old
/// entries fall off; nothing here is persisted.
const HISTORY_LEN: usize = 20;

/// One team setup this engine installed, for the toast poller and any future UI.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Installed {
    pub id: String,
    pub name: String,
    pub car: String,
    pub track: Option<String>,
    pub action: InstallAction,
    pub at_unix: u64,
}

/// What the engine has done recently, and whether it's on. Cheap enough to
/// poll on a timer, same contract as [`crate::sync::status`].
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub enabled: bool,
    /// Newest first.
    pub recent: Vec<Installed>,
}

static RECENT: Mutex<VecDeque<Installed>> = Mutex::new(VecDeque::new());

/// A poisoned lock here costs the toast poller one missed batch, never a
/// crash — same discipline as [`crate::sync`]'s `LIVE`/`LAST` locks.
fn lock(m: &Mutex<VecDeque<Installed>>) -> MutexGuard<'_, VecDeque<Installed>> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// What the engine has installed recently, and whether it's on.
pub fn status() -> Status {
    Status {
        enabled: Settings::load_default().team_auto_install_enabled,
        recent: lock(&RECENT).iter().cloned().collect(),
    }
}

/// Start the engine on its own thread. Call once, at startup.
pub fn spawn() {
    if let Err(e) = std::thread::Builder::new()
        .name("pf-team-sync".into())
        .spawn(run)
    {
        log::error!("team sync engine didn't start: {e}");
    }
}

/// The engine loop. Runs until the process exits.
fn run() {
    // Only warn when the reason changes — a driver with no teams shouldn't
    // get a warning every minute for the rest of the session.
    let mut last_err = String::new();
    loop {
        let mut settings = Settings::load_default();
        if settings.team_auto_install_enabled {
            match sweep(&mut settings) {
                Ok(()) => last_err.clear(),
                Err(e) => {
                    let msg = e.to_string();
                    if msg != last_err {
                        log::warn!("team sync: {msg}");
                        last_err = msg;
                    }
                }
            }
        }
        std::thread::sleep(POLL_INTERVAL);
    }
}

/// One poll cycle: list the team vault, seed on first run, install the rest.
fn sweep(settings: &mut Settings) -> Result<()> {
    let items = download::list_setups("team")?;

    if !settings.team_auto_install_seeded {
        settings.team_seen_setups = items.iter().map(|s| s.id.clone()).collect();
        settings.team_auto_install_seeded = true;
        if let Err(e) = settings.save_default() {
            log::warn!("team sync: couldn't persist the seeded seen-set: {e}");
        }
        log::info!(
            "team sync: seeded {} existing team setup(s), none installed",
            settings.team_seen_setups.len()
        );
        return Ok(());
    }

    for item in &items {
        if settings.team_seen_setups.contains(&item.id) {
            continue;
        }
        match download::download_setup(&item.id, settings) {
            Ok(installed) => {
                settings.team_seen_setups.insert(item.id.clone());
                log::info!(
                    "team sync: installed {} ({:?}) -> {}",
                    item.name,
                    installed.action,
                    installed.path.display()
                );
                push_recent(Installed {
                    id: item.id.clone(),
                    name: item.name.clone(),
                    car: item.car.clone(),
                    track: item.track.clone(),
                    action: installed.action,
                    at_unix: now_unix(),
                });
            }
            // Signed out mid-sweep: stop for this cycle. Nothing here is
            // marked seen, so the whole remaining batch waits for the next
            // poll rather than draining partially.
            Err(Error::NotLinked) => {
                log::debug!("team sync: signed out — {} item(s) wait", items.len());
                break;
            }
            // ponytail: logs every retry rather than deduping per id like
            // sync.rs's last_err — fine at a 60s poll with rare failures; add
            // per-id backoff if a permanently un-installable setup (e.g. a
            // sim not on this machine) turns out to be noisy.
            Err(e) => {
                log::warn!("team sync: couldn't install {}: {e}", item.name);
            }
        }
    }

    if let Err(e) = settings.save_default() {
        log::warn!("team sync: couldn't persist the seen-set: {e}");
    }
    Ok(())
}

fn push_recent(entry: Installed) {
    let mut recent = lock(&RECENT);
    recent.push_front(entry);
    recent.truncate(HISTORY_LEN);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// No token is ever stored under this crate's keychain service in a CI
    /// runner, so `list_setups` fails at the auth check before any network
    /// call. Confirms the failure propagates cleanly and leaves the seen-set
    /// untouched rather than wrongly marking the sweep as seeded.
    #[test]
    fn sweep_propagates_auth_failure_without_touching_state() {
        let mut settings = Settings::default();
        let err = sweep(&mut settings).unwrap_err();
        assert!(matches!(err, Error::NotLinked));
        assert!(!settings.team_auto_install_seeded);
        assert!(settings.team_seen_setups.is_empty());
    }
}
