//! Tauri commands — the thin bridge from the UI to [`pf_core`].
//!
//! Each command delegates to `pf_core`; no product logic lives here (Build Plan
//! §3). Auth/network commands are `async` and run the blocking `pf_core` work on
//! a worker thread via `spawn_blocking`, so the UI thread never stalls.
//!
//! Errors cross the boundary as [`CmdError`] `{ kind, message }` so the UI can
//! attach a per-kind hint (M4 unhappy paths) instead of parsing strings.

use serde::Serialize;
use tauri::async_runtime::spawn_blocking;
use tauri::Manager;

use pf_core::api::{ApiClient, DeviceUser};
use pf_core::auth::{self, DeviceFlow, FlowOutcome};
use pf_core::download::{self, InstallAction};
use pf_core::settings::Settings;
use pf_core::sim::Sim;

/// M0 smoke test: round-trip a message through `pf_core` and back to the UI.
#[tauri::command]
pub fn ping(message: String) -> pf_core::Pong {
    pf_core::ping(&message)
}

// ---------------------------------------------------------------------------
// Errors across the IPC boundary (M4)
// ---------------------------------------------------------------------------

/// Structured command error: `kind` is [`pf_core::Error::kind`] (or a
/// shell-local kind like `"invalid_link"`), `message` is the user-facing text.
/// The frontend's `lib/errors.ts` mirrors the kinds.
#[derive(Serialize, Clone)]
pub struct CmdError {
    pub kind: String,
    pub message: String,
}

impl CmdError {
    fn new(kind: &str, message: impl Into<String>) -> Self {
        Self {
            kind: kind.into(),
            message: message.into(),
        }
    }
}

impl From<pf_core::Error> for CmdError {
    fn from(e: pf_core::Error) -> Self {
        Self::new(e.kind(), e.to_string())
    }
}

/// Run a blocking closure on a worker thread and surface failures as
/// [`CmdError`] (Tauri serializes the `Err` arm to the UI's `catch`).
async fn blocking<T, F>(f: F) -> Result<T, CmdError>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, CmdError> + Send + 'static,
{
    spawn_blocking(f)
        .await
        .map_err(|e| CmdError::new("internal", e.to_string()))?
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
pub async fn auth_status() -> Result<AuthStatus, CmdError> {
    blocking(|| {
        let linked = auth::is_linked()?;
        // Only surface a cached profile while actually linked; a stale profile
        // without a token must not read as signed in.
        let user = if linked { auth::cached_user()? } else { None };
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
pub async fn connect_begin() -> Result<DeviceFlowDto, CmdError> {
    blocking(|| Ok(auth::begin_device_flow(&ApiClient::from_env()).map(DeviceFlowDto::from)?)).await
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
pub async fn connect_poll(device_code: String) -> Result<PollDto, CmdError> {
    blocking(move || {
        Ok(auth::poll_device_flow(&ApiClient::from_env(), &device_code).map(PollDto::from)?)
    })
    .await
}

#[tauri::command]
pub async fn sign_out() -> Result<(), CmdError> {
    blocking(|| Ok(auth::sign_out()?)).await
}

// ---------------------------------------------------------------------------
// Settings (M4)
// ---------------------------------------------------------------------------

/// The persisted settings, straight from disk. [`Settings`] serializes
/// camelCase (`simFolders`, `conflictPolicy`) — mirrored in `lib/settings.ts`.
#[tauri::command]
pub async fn get_settings() -> Result<Settings, CmdError> {
    blocking(|| Ok(Settings::load_default())).await
}

/// Persist the whole settings object (the UI auto-saves on each change).
#[tauri::command]
pub async fn save_settings(settings: Settings) -> Result<(), CmdError> {
    blocking(move || Ok(settings.save_default()?)).await
}

/// Whether the app is registered to launch at startup (Windows Run key).
#[tauri::command]
pub fn get_autostart(app: tauri::AppHandle) -> Result<bool, CmdError> {
    use tauri_plugin_autostart::ManagerExt;
    app.autolaunch()
        .is_enabled()
        .map_err(|e| CmdError::new("autostart", e.to_string()))
}

/// Register/unregister launch-at-startup. Registered runs pass `--hidden` so a
/// login launch goes straight to the tray without flashing the window.
#[tauri::command]
pub fn set_autostart(app: tauri::AppHandle, enabled: bool) -> Result<(), CmdError> {
    use tauri_plugin_autostart::ManagerExt;
    let autolaunch = app.autolaunch();
    let result = if enabled {
        autolaunch.enable()
    } else {
        autolaunch.disable()
    };
    log::info!("autostart set to {enabled}");
    result.map_err(|e| CmdError::new("autostart", e.to_string()))
}

/// Reveal the app's log folder in Explorer (Settings → "Open logs folder"),
/// so a support request can start with "send me the log file".
#[tauri::command]
pub fn open_logs_dir(app: tauri::AppHandle) -> Result<(), CmdError> {
    use tauri_plugin_opener::OpenerExt;
    let dir = app
        .path()
        .app_log_dir()
        .map_err(|e| CmdError::new("io", e.to_string()))?;
    // A fresh install may not have logged yet; an empty folder beats an error.
    std::fs::create_dir_all(&dir).map_err(|e| CmdError::new("io", e.to_string()))?;
    app.opener()
        .open_path(dir.to_string_lossy(), None::<&str>)
        .map_err(|e| CmdError::new("io", e.to_string()))
}

// ---------------------------------------------------------------------------
// Downloads (M2 iRacing · M3 multi-sim · M4 conflict policy)
// ---------------------------------------------------------------------------

/// One sim's setups folder + whether it was found, for the Settings folder list.
#[derive(Serialize)]
pub struct SimFolderDto {
    pub id: String,
    pub name: String,
    pub dir: Option<String>,
    pub found: bool,
    pub overridden: bool,
}

/// Detect every sim's setups folder, applying the persisted per-sim overrides,
/// so the UI can show what's installed and where setups will land.
#[tauri::command]
pub async fn detect_sims() -> Result<Vec<SimFolderDto>, CmdError> {
    blocking(move || {
        let settings = Settings::load_default();
        Ok(Sim::ALL
            .into_iter()
            .map(|sim| {
                let status =
                    pf_core::paths::sim_folder_status(sim, settings.sim_folders.get(&sim).cloned());
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
    /// How the file landed: `installed` | `replaced` | `kept_both` |
    /// `already_installed`. Words the toast.
    pub action: InstallAction,
    pub sim: String,
    /// Stable sim id ("iracing" | "acc" | "lmu") for UI logic (e.g. the ACC
    /// missing-track heads-up); `sim` above is the display name.
    pub sim_id: String,
    pub car: String,
    pub track: Option<String>,
    pub name: Option<String>,
}

impl From<download::InstalledSetup> for InstalledSetupDto {
    fn from(s: download::InstalledSetup) -> Self {
        Self {
            path: s.path.display().to_string(),
            action: s.action,
            sim: s.sim.display_name().to_string(),
            sim_id: s.sim.id().to_string(),
            car: s.car,
            track: s.track,
            name: s.name,
        }
    }
}

impl InstalledSetupDto {
    /// `(title, body)` for the native equip notification. Keeps the wording in
    /// one place so the tray toast and any future UI copy agree.
    pub fn toast(&self) -> (String, String) {
        let what = self
            .name
            .clone()
            .or_else(|| {
                std::path::Path::new(&self.path)
                    .file_name()
                    .map(|f| f.to_string_lossy().into_owned())
            })
            .unwrap_or_else(|| "Setup".to_string());
        let mut place = vec![self.sim.clone()];
        if !self.car.is_empty() {
            place.push(self.car.clone());
        }
        if let Some(track) = &self.track {
            place.push(track.clone());
        }
        let place = place.join(" · ");
        match self.action {
            InstallAction::AlreadyInstalled => (
                "Already in your garage ✓".to_string(),
                format!("“{what}” is already installed — {place}."),
            ),
            InstallAction::KeptBoth => (
                "Equipped ✓ (kept your existing file)".to_string(),
                format!("“{what}” → {place}"),
            ),
            InstallAction::Installed | InstallAction::Replaced => {
                ("Equipped ✓".to_string(), format!("“{what}” → {place}"))
            }
        }
    }
}

/// Download and install a setup from a pasted parcferme.cc URL (or bare UUID),
/// using the persisted Settings (folder overrides + conflict policy).
#[tauri::command]
pub async fn download_setup(input: String) -> Result<InstalledSetupDto, CmdError> {
    blocking(move || {
        let uuid = download::extract_setup_uuid(&input).ok_or_else(|| {
            CmdError::new(
                "invalid_link",
                "That doesn't look like a Parc Fermé setup link.",
            )
        })?;
        let settings = Settings::load_default();
        Ok(download::download_setup(&uuid, &settings).map(InstalledSetupDto::from)?)
    })
    .await
}

/// The setups the linked account can install, for the browse list —
/// `scope` is `"mine"` (their own) or `"team"` (their teams' vaults).
/// Returns ids the UI hands straight to [`download_setup`].
#[tauri::command]
pub async fn list_setups(scope: String) -> Result<Vec<pf_core::api::SetupSummary>, CmdError> {
    blocking(move || Ok(download::list_setups(&scope)?)).await
}

// ---------------------------------------------------------------------------
// Push a setup (M5)
// ---------------------------------------------------------------------------

/// Inspect a picked setup file: infer sim from the extension and car/track
/// from its position under that sim's setups folder (override-aware). Pure
/// inference — nothing leaves the machine. Pre-fills the upload form.
#[tauri::command]
pub async fn identify_setup(path: String) -> Result<pf_core::upload::SetupIdentity, CmdError> {
    blocking(move || {
        let settings = Settings::load_default();
        Ok(pf_core::upload::identify(
            std::path::Path::new(&path),
            &settings,
        ))
    })
    .await
}

/// Car/track name lists the site knows for `sim`, for the upload form's
/// picker suggestions.
///
/// Fails soft by design: an unreachable site or an unpaired device yields
/// empty lists, never an error, so the fields stay usable as free text (see
/// [`pf_core::options`]).
#[tauri::command]
pub async fn setup_options(sim: String) -> Result<pf_core::api::SetupOptions, CmdError> {
    blocking(move || {
        Ok(Sim::from_id(&sim)
            .map(pf_core::options::options_for)
            .unwrap_or_default())
    })
    .await
}

/// The uploaded setup as shown in the success card: its id and page URL.
#[derive(Serialize)]
pub struct UploadedSetupDto {
    pub id: String,
    pub url: String,
}

/// Push a local setup file to parcferme.cc as the linked user. `sim` is the
/// short id ("iracing" | "acc" | "lmu"); `car`/`track` are the sim's internal
/// folder ids (pre-filled by [`identify_setup`], editable by the user).
/// `types` come from [`setup_options`]; an empty list lets the server default.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn upload_setup(
    path: String,
    sim: String,
    car: String,
    track: Option<String>,
    name: Option<String>,
    types: Option<Vec<String>>,
    notes: Option<String>,
    private: Option<bool>,
) -> Result<UploadedSetupDto, CmdError> {
    blocking(move || {
        let sim = Sim::from_id(&sim)
            .ok_or_else(|| CmdError::new("api", format!("unknown sim: {sim:?}")))?;
        let types: Vec<String> = types
            .unwrap_or_default()
            .into_iter()
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty())
            .collect();
        let result = pf_core::upload::upload_setup(
            std::path::Path::new(&path),
            sim,
            car.trim(),
            track.as_deref().map(str::trim).filter(|t| !t.is_empty()),
            name.as_deref().map(str::trim).filter(|n| !n.is_empty()),
            &types,
            notes.as_deref().map(str::trim).filter(|n| !n.is_empty()),
            private.unwrap_or(false),
        )?;
        Ok(UploadedSetupDto {
            id: result.id,
            url: result.url,
        })
    })
    .await
}

// ---------------------------------------------------------------------------
// Equip deep link (M3 · M4 persisted settings + typed errors)
// ---------------------------------------------------------------------------

/// Outcome of an equip deep link, emitted to the frontend as the `equip-result`
/// event. Internally tagged on `status` so the UI can branch on success/failure.
#[derive(Serialize, Clone)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum EquipOutcome {
    /// The setup was fetched and written into its sim folder.
    Installed(InstalledSetupDto),
    /// Parsing the link or downloading failed; `message` is user-facing and
    /// `kind` picks the recovery hint (see `lib/errors.ts`).
    Error { kind: String, message: String },
}

/// Run a `parcferme://equip?…` deep link to completion (parse + download).
///
/// Blocking — the deep-link handler in `lib.rs` calls this on a worker thread.
/// Never panics; any failure (bad link, not linked, no access, network) becomes
/// [`EquipOutcome::Error`]. Reads the same persisted Settings as a manual pull,
/// so folder overrides and the conflict policy apply to equips too.
pub fn run_equip(url: &str) -> EquipOutcome {
    let settings = Settings::load_default();
    match download::install_from_equip_link(url, &settings) {
        Ok(s) => EquipOutcome::Installed(s.into()),
        Err(e) => EquipOutcome::Error {
            kind: e.kind().to_string(),
            message: e.to_string(),
        },
    }
}
