//! Tauri commands — the thin bridge from the UI to [`pf_core`].
//!
//! Each command delegates to `pf_core`; no product logic lives here (Build Plan
//! §3). Auth/network commands are `async` and run the blocking `pf_core` work on
//! a worker thread via `spawn_blocking`, so the UI thread never stalls.

use std::collections::HashMap;
use std::path::PathBuf;

use serde::Serialize;
use tauri::async_runtime::spawn_blocking;

use pf_core::api::{ApiClient, DeviceUser};
use pf_core::auth::{self, DeviceFlow, FlowOutcome};
use pf_core::download;
use pf_core::sim::Sim;

/// M0 smoke test: round-trip a message through `pf_core` and back to the UI.
#[tauri::command]
pub fn ping(message: String) -> pf_core::Pong {
    pf_core::ping(&message)
}

/// Run a blocking closure on a worker thread and surface errors as `String`
/// (Tauri serializes the `Err` arm to the UI's `catch`).
async fn blocking<T, F>(f: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, String> + Send + 'static,
{
    spawn_blocking(f).await.map_err(|e| e.to_string())?
}

// ---------------------------------------------------------------------------
// Auth (M1)
// ---------------------------------------------------------------------------

/// Whether this device currently holds a stored token, plus the cached profile
/// of the linked user (so the UI can show "Signed in as @user" on startup).
#[derive(Serialize)]
pub struct AuthStatus {
    pub linked: bool,
    pub user: Option<DeviceUser>,
}

#[tauri::command]
pub async fn auth_status() -> Result<AuthStatus, String> {
    blocking(|| {
        let linked = auth::is_linked().map_err(|e| e.to_string())?;
        // Only surface a cached profile while actually linked; a stale profile
        // without a token must not read as signed in.
        let user = if linked {
            auth::cached_user().map_err(|e| e.to_string())?
        } else {
            None
        };
        Ok(AuthStatus { linked, user })
    })
    .await
}

/// UI-facing shape of [`DeviceFlow`] (omits nothing; `device_code` is needed by
/// the frontend only to drive polling and is never displayed).
#[derive(Serialize)]
pub struct DeviceFlowDto {
    pub user_code: String,
    pub verification_uri: String,
    pub verification_uri_complete: Option<String>,
    pub device_code: String,
    pub interval_secs: u64,
    pub expires_in_secs: u64,
}

impl From<DeviceFlow> for DeviceFlowDto {
    fn from(f: DeviceFlow) -> Self {
        Self {
            user_code: f.user_code,
            verification_uri: f.verification_uri,
            verification_uri_complete: f.verification_uri_complete,
            device_code: f.device_code,
            interval_secs: f.interval_secs,
            expires_in_secs: f.expires_in_secs,
        }
    }
}

#[tauri::command]
pub async fn connect_begin() -> Result<DeviceFlowDto, String> {
    blocking(|| {
        auth::begin_device_flow(&ApiClient::from_env())
            .map(DeviceFlowDto::from)
            .map_err(|e| e.to_string())
    })
    .await
}

/// Tagged result of one poll, serialized as `{ "status": "...", ... }`.
#[derive(Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum PollDto {
    Linked { user: Option<DeviceUser> },
    Pending,
    SlowDown,
    Denied,
    Expired,
}

impl From<FlowOutcome> for PollDto {
    fn from(o: FlowOutcome) -> Self {
        match o {
            FlowOutcome::Linked { user } => PollDto::Linked { user },
            FlowOutcome::Pending => PollDto::Pending,
            FlowOutcome::SlowDown => PollDto::SlowDown,
            FlowOutcome::Denied => PollDto::Denied,
            FlowOutcome::Expired => PollDto::Expired,
        }
    }
}

#[tauri::command]
pub async fn connect_poll(device_code: String) -> Result<PollDto, String> {
    blocking(move || {
        auth::poll_device_flow(&ApiClient::from_env(), &device_code)
            .map(PollDto::from)
            .map_err(|e| e.to_string())
    })
    .await
}

#[tauri::command]
pub async fn sign_out() -> Result<(), String> {
    blocking(|| auth::sign_out().map_err(|e| e.to_string())).await
}

// ---------------------------------------------------------------------------
// Downloads (M2 iRacing · M3 multi-sim)
// ---------------------------------------------------------------------------

/// Turn the UI's `{ "<sim id>": "<path>" }` override map into the typed,
/// blank-filtered form `pf_core` expects.
fn parse_overrides(overrides: Option<HashMap<String, String>>) -> HashMap<Sim, PathBuf> {
    overrides
        .unwrap_or_default()
        .into_iter()
        .filter_map(|(id, path)| {
            let path = path.trim();
            match Sim::from_id(&id) {
                Some(sim) if !path.is_empty() => Some((sim, PathBuf::from(path))),
                _ => None,
            }
        })
        .collect()
}

/// One sim's setups folder + whether it was found, for the UI's folder list.
#[derive(Serialize)]
pub struct SimFolderDto {
    pub id: String,
    pub name: String,
    pub dir: Option<String>,
    pub found: bool,
    pub overridden: bool,
}

/// Detect every sim's setups folder, applying any per-sim Settings overrides, so
/// the UI can show what's installed and where setups will land.
#[tauri::command]
pub async fn detect_sims(
    overrides: Option<HashMap<String, String>>,
) -> Result<Vec<SimFolderDto>, String> {
    blocking(move || {
        let overrides = parse_overrides(overrides);
        Ok(Sim::ALL
            .into_iter()
            .map(|sim| {
                let status = pf_core::paths::sim_folder_status(sim, overrides.get(&sim).cloned());
                SimFolderDto {
                    id: sim.id().to_string(),
                    name: sim.display_name().to_string(),
                    dir: status.dir.map(|d| d.display().to_string()),
                    found: status.found,
                    overridden: status.overridden,
                }
            })
            .collect())
    })
    .await
}

/// A setup file written into a sim folder; mirrors [`download::InstalledSetup`]
/// with `path` stringified and `sim` as its display name for the UI.
#[derive(Serialize, Clone)]
pub struct InstalledSetupDto {
    pub path: String,
    pub sim: String,
    pub car: String,
    pub track: Option<String>,
    pub name: Option<String>,
}

impl From<download::InstalledSetup> for InstalledSetupDto {
    fn from(s: download::InstalledSetup) -> Self {
        Self {
            path: s.path.display().to_string(),
            sim: s.sim.display_name().to_string(),
            car: s.car,
            track: s.track,
            name: s.name,
        }
    }
}

/// Download and install a setup from a pasted parcferme.cc URL (or bare UUID).
/// `overrides` maps each sim id to a folder override (blank/unknown ignored).
#[tauri::command]
pub async fn download_setup(
    input: String,
    overrides: Option<HashMap<String, String>>,
) -> Result<InstalledSetupDto, String> {
    blocking(move || {
        let uuid = download::extract_setup_uuid(&input)
            .ok_or_else(|| "That doesn't look like a Parc Fermé setup link.".to_string())?;
        download::download_setup(&uuid, &parse_overrides(overrides))
            .map(InstalledSetupDto::from)
            .map_err(|e| e.to_string())
    })
    .await
}

// ---------------------------------------------------------------------------
// Equip deep link (M3)
// ---------------------------------------------------------------------------

/// Outcome of an equip deep link, emitted to the frontend as the `equip-result`
/// event. Internally tagged on `status` so the UI can branch on success/failure.
#[derive(Serialize, Clone)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum EquipOutcome {
    /// The setup was fetched and written into its sim folder.
    Installed(InstalledSetupDto),
    /// Parsing the link or downloading failed; `message` is user-facing.
    Error { message: String },
}

/// Run a `parcferme://equip?…` deep link to completion (parse + download).
///
/// Blocking — the deep-link handler in `lib.rs` calls this on a worker thread.
/// Never panics; any failure (bad link, not linked, no access, network) becomes
/// [`EquipOutcome::Error`]. Folder overrides aren't available on this path yet
/// (persisted Settings are M4), so detection-only folders are used.
pub fn run_equip(url: &str) -> EquipOutcome {
    match download::install_from_equip_link(url, &HashMap::new()) {
        Ok(s) => EquipOutcome::Installed(s.into()),
        Err(e) => EquipOutcome::Error {
            message: e.to_string(),
        },
    }
}
