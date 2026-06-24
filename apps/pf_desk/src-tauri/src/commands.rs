//! Tauri commands — the thin bridge from the UI to [`pf_core`].
//!
//! Each command delegates to `pf_core`; no product logic lives here (Build Plan
//! §3). Auth/network commands are `async` and run the blocking `pf_core` work on
//! a worker thread via `spawn_blocking`, so the UI thread never stalls.

use std::path::PathBuf;

use serde::Serialize;
use tauri::async_runtime::spawn_blocking;

use pf_core::api::{ApiClient, DeviceUser};
use pf_core::auth::{self, DeviceFlow, FlowOutcome};
use pf_core::download;

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
// Downloads (M2)
// ---------------------------------------------------------------------------

/// Coerce an optional, possibly-blank Settings override into a real path.
fn override_path(override_dir: Option<String>) -> Option<PathBuf> {
    override_dir
        .filter(|s| !s.trim().is_empty())
        .map(PathBuf::from)
}

/// The resolved, validated setups directory (for the UI to show / confirm).
#[tauri::command]
pub async fn setups_dir(override_dir: Option<String>) -> Result<String, String> {
    blocking(move || {
        pf_core::paths::resolve_setups_dir(override_path(override_dir))
            .map(|p| p.display().to_string())
            .map_err(|e| e.to_string())
    })
    .await
}

/// A setup file written into the sim folder; mirrors [`download::InstalledSetup`]
/// with `path` stringified for the UI.
#[derive(Serialize)]
pub struct InstalledSetupDto {
    pub path: String,
    pub car: String,
    pub name: Option<String>,
}

/// Download and install a setup from a pasted parcferme.cc URL (or bare UUID).
#[tauri::command]
pub async fn download_setup(
    input: String,
    override_dir: Option<String>,
) -> Result<InstalledSetupDto, String> {
    blocking(move || {
        let uuid = download::extract_setup_uuid(&input)
            .ok_or_else(|| "That doesn't look like a Parc Fermé setup link.".to_string())?;
        download::download_setup(&uuid, override_path(override_dir))
            .map(|s| InstalledSetupDto {
                path: s.path.display().to_string(),
                car: s.car,
                name: s.name,
            })
            .map_err(|e| e.to_string())
    })
    .await
}
