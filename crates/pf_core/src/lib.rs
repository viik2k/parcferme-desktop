//! `pf_core` — the real product.
//!
//! All ParcFerme desktop logic lives here as a plain Rust library with no UI.
//! The Tauri shell (`pf_desk`) only does windowing, the tray, and toasts; it
//! calls into these modules. Keeping everything here means a future CLI or
//! push-daemon is new functions, not a new app (see Build Plan §3, §8).

pub mod api;
pub mod auth;
pub mod car_aliases;
pub mod car_match;
pub mod deeplink;
pub mod download;
mod error;
pub mod options;
pub mod paths;
pub mod settings;
pub mod sim;
pub mod upload;

pub use error::{Error, Result};
pub use sim::Sim;

/// The app's bundle identifier — shared by the keychain service name
/// ([`auth`]) and the settings config dir ([`settings`]), and matching
/// `tauri.conf.json`'s `identifier` so all app data lives under one name.
pub const APP_ID: &str = "cc.parcferme.desktop";

use serde::{Deserialize, Serialize};

/// Result of [`ping`] — proves the React → Tauri → `pf_core` round-trip (M0).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pong {
    /// The original message, echoed back.
    pub message: String,
    /// The `pf_core` version that handled the call.
    pub core_version: String,
}

/// Echo a message back through the core, tagged with the core version.
///
/// This is the M0 smoke test: the UI invokes the `ping` Tauri command, which
/// is a thin wrapper over this function. If a `Pong` comes back, the whole
/// React → Tauri → `pf_core` wiring is sound.
pub fn ping(message: &str) -> Pong {
    Pong {
        message: format!("pong: {message}"),
        core_version: env!("CARGO_PKG_VERSION").to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ping_echoes_message_and_tags_version() {
        let pong = ping("hello from React");
        assert!(pong.message.contains("hello from React"));
        assert_eq!(pong.core_version, env!("CARGO_PKG_VERSION"));
    }
}
