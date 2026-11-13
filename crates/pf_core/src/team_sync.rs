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
use std::sync::{Condvar, Mutex, MutexGuard};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::Serialize;

use crate::download::{self, InstallAction};
use crate::settings::Settings;
use crate::{Error, Result};

/// How often to poll the team vault while setups are arriving. A full HTTP
/// round trip, and setups don't arrive nearly as often as telemetry does — no
/// need for [`crate::sync`]'s 5 s cadence.
const POLL_INTERVAL: Duration = Duration::from_secs(60);

/// The longest the poll backs off to when sweeps keep finding nothing new.
/// Each poll is a server function call for every running tray app, so an idle
/// vault shouldn't cost one a minute all day; [`poke`] brings it straight back.
const MAX_POLL_INTERVAL: Duration = Duration::from_secs(300);

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

/// Set by [`poke`]; the engine's sleep waits on it.
static WAKE: (Mutex<bool>, Condvar) = (Mutex::new(false), Condvar::new());

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

/// Poll now and go back to the short interval: the user is at the app (window
/// focused, an equip just ran), so a teammate's new setup should land soon.
pub fn poke() {
    let (flag, cv) = &WAKE;
    *flag.lock().unwrap_or_else(|e| e.into_inner()) = true;
    cv.notify_one();
}

/// The wait before the next poll: back to [`POLL_INTERVAL`] when the last
/// sweep found something new, otherwise double it, up to [`MAX_POLL_INTERVAL`].
fn next_interval(current: Duration, found_new: bool) -> Duration {
    if found_new {
        POLL_INTERVAL
    } else {
        (current * 2).min(MAX_POLL_INTERVAL)
    }
}

/// The engine loop. Runs until the process exits.
fn run() {
    // Only warn when the reason changes — a driver with no teams shouldn't
    // get a warning every minute for the rest of the session.
    let mut last_err = String::new();
    let mut interval = POLL_INTERVAL;
    loop {
        let mut settings = Settings::load_default();
        if settings.team_auto_install_enabled {
            let found_new = match sweep(&mut settings, download::list_setups("team")) {
                Ok(found_new) => {
                    last_err.clear();
                    found_new
                }
                Err(e) => {
                    let msg = e.to_string();
                    if msg != last_err {
                        log::warn!("team sync: {msg}");
                        last_err = msg;
                    }
                    false
                }
            };
            interval = next_interval(interval, found_new);
        } else {
            // Off costs no request, and turning it on shouldn't wait 5 minutes.
            interval = POLL_INTERVAL;
            // Re-toggling is the user's retry for setups that failed to install.
            if !settings.team_failed_setups.is_empty() {
                settings.team_failed_setups.clear();
                if let Err(e) = settings.save_default() {
                    log::warn!("team sync: couldn't clear the failed-set: {e}");
                }
            }
        }
        let (flag, cv) = &WAKE;
        let guard = flag.lock().unwrap_or_else(|e| e.into_inner());
        let (mut poked, _) = cv
            .wait_timeout_while(guard, interval, |poked| !*poked)
            .unwrap_or_else(|e| e.into_inner());
        if *poked {
            *poked = false;
            interval = POLL_INTERVAL;
        }
    }
}

/// One poll cycle over the team vault's `listing`: seed on first run, install
/// the rest. Says whether the listing held anything not seen before, which is
/// what the poll interval backs off on. The listing is passed in so tests don't
/// reach the keychain.
fn sweep(settings: &mut Settings, listing: Result<Vec<crate::api::SetupSummary>>) -> Result<bool> {
    sweep_with(settings, listing, download::download_setup)
}

/// [`sweep`] with the installer injected, so tests can fail it without a network.
fn sweep_with(
    settings: &mut Settings,
    listing: Result<Vec<crate::api::SetupSummary>>,
    mut install: impl FnMut(&str, &Settings) -> Result<download::InstalledSetup>,
) -> Result<bool> {
    let items = listing?;

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
        return Ok(true);
    }
    // A failed id is retried only once it has left the listing (and so can
    // come back as a fresh upload) or the user re-toggles auto-install; until
    // then it costs no request. Drop ids that have left the listing.
    let before = settings.team_failed_setups.len();
    settings
        .team_failed_setups
        .retain(|id| items.iter().any(|s| &s.id == id));
    let pruned = settings.team_failed_setups.len() != before;

    // Nothing new: nothing to install, and the poll backs off.
    let done = |settings: &Settings, id: &str| {
        settings.team_seen_setups.contains(id) || settings.team_failed_setups.contains(id)
    };
    if items.iter().all(|s| done(settings, &s.id)) {
        if pruned {
            if let Err(e) = settings.save_default() {
                log::warn!("team sync: couldn't persist the failed-set: {e}");
            }
        }
        return Ok(false);
    }

    let mut installed_any = false;
    for item in &items {
        if done(settings, &item.id) {
            continue;
        }
        match install(&item.id, settings) {
            Ok(installed) => {
                installed_any = true;
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
            // Most likely permanent (a sim that isn't installed here): remember
            // it so it isn't retried against the server every poll.
            Err(e) => {
                log::warn!("team sync: couldn't install {}: {e}", item.name);
                settings.team_failed_setups.insert(item.id.clone());
            }
        }
    }

    if let Err(e) = settings.save_default() {
        log::warn!("team sync: couldn't persist the seen-set: {e}");
    }
    // Pure failures don't reset the poll to the short interval.
    Ok(installed_any)
}

fn push_recent(entry: Installed) {
    let mut recent = lock(&RECENT);
    recent.push_front(entry);
    recent.truncate(HISTORY_LEN);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A signed-out listing propagates cleanly and leaves the seen-set
    /// untouched rather than wrongly marking the sweep as seeded. The failure
    /// is injected: reading the real keychain made this pass only on machines
    /// never signed in, and on a signed-in one the seed path would overwrite
    /// the real settings.json.
    #[test]
    fn sweep_propagates_auth_failure_without_touching_state() {
        let mut settings = Settings::default();
        let err = sweep(&mut settings, Err(Error::NotLinked)).unwrap_err();
        assert!(matches!(err, Error::NotLinked));
        assert!(!settings.team_auto_install_seeded);
        assert!(settings.team_seen_setups.is_empty());
    }

    fn summary(id: &str) -> crate::api::SetupSummary {
        serde_json::from_str(&format!(r#"{{"id": "{id}"}}"#)).unwrap()
    }

    /// A setup that can't install is recorded, doesn't reset the poll to the
    /// short interval, and isn't offered to the installer again.
    #[test]
    fn failed_install_is_remembered_not_retried_and_does_not_count_as_new() {
        let mut settings = Settings {
            team_auto_install_seeded: true,
            ..Settings::default()
        };
        let mut calls = 0;
        let mut fail = |_: &str, _: &Settings| {
            calls += 1;
            Err(Error::SetupNotFound)
        };
        assert!(!sweep_with(&mut settings, Ok(vec![summary("a")]), &mut fail).unwrap());
        assert!(settings.team_failed_setups.contains("a"));
        assert!(!settings.team_seen_setups.contains("a"));
        assert!(!sweep_with(&mut settings, Ok(vec![summary("a")]), &mut fail).unwrap());
        assert_eq!(calls, 1);
    }

    /// A failed id that leaves the listing is forgotten, so a re-upload retries.
    #[test]
    fn failed_id_is_pruned_when_it_leaves_the_listing() {
        let mut settings = Settings {
            team_auto_install_seeded: true,
            ..Settings::default()
        };
        settings.team_failed_setups.insert("a".into());
        let mut never =
            |_: &str, _: &Settings| -> Result<download::InstalledSetup> { unreachable!() };
        assert!(!sweep_with(&mut settings, Ok(vec![]), &mut never).unwrap());
        assert!(settings.team_failed_setups.is_empty());
    }

    #[test]
    fn idle_polls_back_off_to_five_minutes_and_reset_on_news() {
        let mut d = POLL_INTERVAL;
        let mut seen = vec![];
        for _ in 0..5 {
            d = next_interval(d, false);
            seen.push(d.as_secs());
        }
        assert_eq!(seen, [120, 240, 300, 300, 300]);
        assert_eq!(next_interval(d, true), POLL_INTERVAL);
    }

    /// An all-seen listing is "nothing new", so the poll backs off. It returns
    /// before any install or save, so this never touches the network, the
    /// keychain or the real settings.json.
    #[test]
    fn sweep_reports_nothing_new_when_everything_is_seen() {
        let mut settings = Settings {
            team_auto_install_seeded: true,
            ..Settings::default()
        };
        settings.team_seen_setups.insert("a".into());
        let item: crate::api::SetupSummary = serde_json::from_str(r#"{"id": "a"}"#).unwrap();
        assert!(!sweep(&mut settings, Ok(vec![item])).unwrap());
    }
}
