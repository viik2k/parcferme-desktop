//! Tauri commands — the thin bridge from the UI to [`pf_core`].
//!
//! Each command delegates to `pf_core`; no product logic lives here (Build Plan
//! §3). Auth/network commands are `async` and run the blocking `pf_core` work on
//! a worker thread via `spawn_blocking`, so the UI thread never stalls.

use serde::Serialize;
use tauri::async_runtime::spawn_blocking;

use pf_core::api::{ApiClient, DeviceUser};
use pf_core::auth::{self, DeviceFlow, FlowOutcome};

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

/// Whether this device currently holds a stored token.
#[derive(Serialize)]
pub struct AuthStatus {
    pub linked: bool,
}

#[tauri::command]
pub async fn auth_status() -> Result<AuthStatus, String> {
    blocking(|| {
        auth::is_linked()
            .map(|linked| AuthStatus { linked })
            .map_err(|e| e.to_string())
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
