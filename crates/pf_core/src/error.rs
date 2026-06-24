//! Shared error type for `pf_core`.

use thiserror::Error;

/// Convenience alias used throughout the crate.
pub type Result<T> = std::result::Result<T, Error>;

/// Everything that can go wrong inside `pf_core`.
///
/// The Tauri layer converts these into a `String` before they cross the command
/// boundary to the UI (see `pf_desk`).
#[derive(Debug, Error)]
pub enum Error {
    /// A code path that is scaffolded but not yet implemented for this
    /// milestone. The string names the function so logs are actionable.
    #[error("not yet implemented: {0}")]
    NotImplemented(&'static str),

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
