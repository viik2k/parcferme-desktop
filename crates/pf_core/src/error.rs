//! Shared error type for `pf_core`.

use thiserror::Error;

/// Convenience alias used throughout the crate.
pub type Result<T> = std::result::Result<T, Error>;

/// Everything that can go wrong inside `pf_core`.
///
/// The Tauri layer converts these into a structured `{ kind, message }` before
/// they cross the command boundary, so the UI can attach a per-kind hint (M4
/// unhappy paths) without parsing display strings. `kind` comes from
/// [`Error::kind`]; the message is the `Display` text.
#[derive(Debug, Error)]
pub enum Error {
    /// A code path that is scaffolded but not yet implemented for this
    /// milestone. The string names the function so logs are actionable.
    #[error("not yet implemented: {0}")]
    NotImplemented(&'static str),

    /// This device holds no stored token — the user must connect it first.
    #[error("not connected — link this device to your Parc Fermé account first")]
    NotLinked,

    /// The server rejected our device token (HTTP 401): revoked under
    /// Account → Devices, or expired. Reconnecting mints a fresh token.
    #[error("this device is no longer authorized — sign out and connect again")]
    DeviceRevoked,

    /// The linked user may not access this setup (HTTP 403) — private and not
    /// shared with them. Same `setupShares` check the website runs (audit #2).
    #[error("you don't have access to this setup")]
    AccessDenied,

    /// No setup with that id exists (HTTP 404).
    #[error("setup not found — it may have been deleted")]
    SetupNotFound,

    /// Transport-level HTTP failure (DNS, TLS, timeout, connection reset).
    #[error("network error: {0}")]
    Http(String),

    /// The server returned an error response we don't otherwise model.
    #[error("api error: {0}")]
    Api(String),

    /// OS keychain (Windows Credential Manager) failure.
    #[error("keychain error: {0}")]
    Keychain(String),

    /// The sim setups directory doesn't exist at the detected or override
    /// location. The string is the path we looked for, so Settings can guide
    /// the user to set an override.
    #[error("setups folder not found: {0}")]
    SetupsDirNotFound(String),

    /// Filesystem I/O failure (path detection, atomic write, …).
    #[error("i/o error: {0}")]
    Io(#[from] std::io::Error),

    /// (De)serialization failure crossing the API or command boundary.
    #[error("serialization error: {0}")]
    Serde(#[from] serde_json::Error),
}

impl Error {
    /// Stable, machine-readable kind for the UI to branch on (error hints, an
    /// "Open Settings" shortcut, …). Part of the IPC contract with `pf_desk` —
    /// renaming a kind means updating the frontend's hint map too.
    pub fn kind(&self) -> &'static str {
        match self {
            Error::NotImplemented(_) => "not_implemented",
            Error::NotLinked => "not_linked",
            Error::DeviceRevoked => "device_revoked",
            Error::AccessDenied => "access_denied",
            Error::SetupNotFound => "setup_not_found",
            Error::Http(_) => "network",
            Error::Api(_) => "api",
            Error::Keychain(_) => "keychain",
            Error::SetupsDirNotFound(_) => "setups_dir_not_found",
            Error::Io(_) => "io",
            Error::Serde(_) => "serde",
        }
    }
}
