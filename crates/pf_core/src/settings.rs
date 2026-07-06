//! Persisted desktop settings. **[M4]**
//!
//! One small JSON file under the app's config dir (Windows:
//! `%APPDATA%\cc.parcferme.desktop\settings.json` — the same directory Tauri's
//! `app_config_dir` resolves for our identifier). `pf_core` owns the format and
//! the I/O so *every* download path — the manual pull and the `parcferme://`
//! equip deep link alike — reads the same overrides; the M3 gap where deep
//! links ignored folder overrides closes here.
//!
//! The file is read fresh at each use (it's tiny and downloads are rare), so
//! there is no cache to invalidate across the UI, tray, and deep-link threads.
//! Loads are tolerant: a missing file is defaults, a corrupt file is defaults
//! plus a warning in the log — a broken settings file must never brick an
//! equip. Saves are atomic (temp + rename), same discipline as setup writes.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::sim::Sim;
use crate::{Error, Result};

/// What a download does when the target file already exists with *different*
/// contents. Byte-identical contents always short-circuit to
/// [`crate::download::InstallAction::AlreadyInstalled`] regardless of policy,
/// so re-equipping the same setup is idempotent and never litters the folder.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConflictPolicy {
    /// Keep the existing file and write the new one as `name (2).ext`
    /// (then `(3)`, …). The default: a racer's locally tweaked setup is
    /// irreplaceable, a re-download is not.
    #[default]
    KeepBoth,
    /// Atomically replace the existing file with the downloaded one.
    Overwrite,
}

/// The user's persisted settings. `#[serde(default)]` on the struct keeps old
/// files loadable as fields are added in later milestones.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Settings {
    /// Per-sim setups-folder overrides, keyed by [`Sim`] (serialized as its
    /// short id). A sim absent from the map uses best-effort detection.
    pub sim_folders: HashMap<Sim, PathBuf>,
    /// What to do when a downloaded file collides with an existing name.
    pub conflict_policy: ConflictPolicy,
}

impl Settings {
    /// Load from `path`. Missing file → defaults; unreadable or corrupt file →
    /// defaults with a `warn!` (the next save rewrites it cleanly).
    pub fn load(path: &Path) -> Settings {
        let raw = match std::fs::read_to_string(path) {
            Ok(raw) => raw,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Settings::default(),
            Err(e) => {
                log::warn!(
                    "settings unreadable at {}: {e} — using defaults",
                    path.display()
                );
                return Settings::default();
            }
        };
        match serde_json::from_str(&raw) {
            Ok(settings) => settings,
            Err(e) => {
                log::warn!(
                    "settings corrupt at {}: {e} — using defaults",
                    path.display()
                );
                Settings::default()
            }
        }
    }

    /// Load from the default location (see [`default_path`]).
    pub fn load_default() -> Settings {
        match default_path() {
            Ok(path) => Settings::load(&path),
            Err(e) => {
                log::warn!("no settings path on this platform ({e}) — using defaults");
                Settings::default()
            }
        }
    }

    /// Persist to `path` atomically, creating parent directories as needed.
    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(self)?;
        // Same-directory temp + rename, so a crash mid-write can't leave a
        // half-written settings.json to be read as corrupt next launch.
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, json)?;
        if let Err(e) = std::fs::rename(&tmp, path) {
            let _ = std::fs::remove_file(&tmp);
            return Err(Error::Io(e));
        }
        log::debug!("settings saved to {}", path.display());
        Ok(())
    }

    /// Persist to the default location (see [`default_path`]).
    pub fn save_default(&self) -> Result<()> {
        self.save(&default_path()?)
    }
}

/// Default settings file location: `<config dir>/cc.parcferme.desktop/settings.json`.
pub fn default_path() -> Result<PathBuf> {
    config_dir()
        .map(|d| d.join(crate::APP_ID).join("settings.json"))
        .ok_or(Error::NotImplemented(
            "settings::config_dir (unsupported platform)",
        ))
}

/// Windows: `%APPDATA%` — matches Tauri's `app_config_dir` root for our
/// identifier, so the shell and the core agree on where config lives.
#[cfg(windows)]
fn config_dir() -> Option<PathBuf> {
    std::env::var_os("APPDATA").map(PathBuf::from)
}

/// Non-Windows (dev/test only; v1 ships Windows-only): XDG-ish `~/.config`.
#[cfg(not(windows))]
fn config_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("pf-settings-{name}-{}", std::process::id()))
    }

    #[test]
    fn save_load_round_trips_with_sim_keyed_map() {
        let dir = scratch("roundtrip");
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("settings.json");

        let mut settings = Settings::default();
        settings
            .sim_folders
            .insert(Sim::Acc, PathBuf::from(r"D:\sims\acc\Setups"));
        settings.conflict_policy = ConflictPolicy::Overwrite;

        settings.save(&path).unwrap();
        assert_eq!(Settings::load(&path), settings);
        // No temp artifact left behind by the atomic save.
        assert!(!path.with_extension("json.tmp").exists());

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn wire_shape_is_stable() {
        // The JSON shape is IPC + on-disk contract: camelCase fields, sim short
        // ids as map keys, snake_case policy values. The frontend mirrors this
        // in lib/settings.ts — a failure here means updating both sides.
        let mut settings = Settings::default();
        settings
            .sim_folders
            .insert(Sim::IRacing, PathBuf::from("C:/x"));
        let json = serde_json::to_string(&settings).unwrap();
        assert!(json.contains("\"simFolders\""), "{json}");
        assert!(json.contains("\"iracing\""), "{json}");
        assert!(json.contains("\"conflictPolicy\":\"keep_both\""), "{json}");
    }

    #[test]
    fn missing_file_is_defaults() {
        let path = scratch("missing").join("settings.json");
        assert_eq!(Settings::load(&path), Settings::default());
    }

    #[test]
    fn corrupt_file_degrades_to_defaults() {
        let dir = scratch("corrupt");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("settings.json");
        std::fs::write(&path, "{ not json").unwrap();
        assert_eq!(Settings::load(&path), Settings::default());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn old_files_stay_loadable_as_fields_are_added() {
        // A file written before a field existed must still load (serde default).
        let v: Settings = serde_json::from_str("{}").unwrap();
        assert_eq!(v, Settings::default());
        let v: Settings = serde_json::from_str(r#"{"conflictPolicy":"overwrite"}"#).unwrap();
        assert_eq!(v.conflict_policy, ConflictPolicy::Overwrite);
    }
}
