//! Locate sim setup directories. iRacing first; ACC/LMU later. **[M2]**
//!
//! Pull is sim-agnostic at the byte level, but folder layout differs per sim,
//! so this module is designed to extend. iRacing folders can also be in
//! non-default / OneDrive-redirected / multi-drive locations, so M2 adds a
//! Settings override on top of this best-effort detection.

use std::path::{Path, PathBuf};

use crate::{Error, Result};

/// Resolve the iRacing setups directory to write into, validating it exists.
///
/// An explicit Settings `override_dir` wins (covers the non-default Documents /
/// OneDrive-redirected / multi-drive cases in the Build Plan's risk table);
/// otherwise we fall back to best-effort detection. Either way the directory
/// must exist — a setups folder is created by iRacing, not by us, so its
/// absence almost always means a wrong path rather than a first run.
pub fn resolve_setups_dir(override_dir: Option<PathBuf>) -> Result<PathBuf> {
    let dir = match override_dir {
        Some(d) => d,
        None => iracing_setups_dir()?,
    };
    if dir.is_dir() {
        Ok(dir)
    } else {
        Err(Error::SetupsDirNotFound(dir.display().to_string()))
    }
}

/// Best-effort location of the user's iRacing setups directory:
/// `…/Documents/iRacing/setups/`. Path only — see [`resolve_setups_dir`] for
/// the validated, override-aware version used at download time.
///
/// Designed to extend per sim (ACC/LMU use different layouts); iRacing first.
pub fn iracing_setups_dir() -> Result<PathBuf> {
    let documents = documents_dir().ok_or(Error::NotImplemented(
        "paths::documents_dir (unsupported platform)",
    ))?;
    Ok(documents.join("iRacing").join("setups"))
}

/// The per-car subfolder a setup belongs in, e.g. `setups\<car>\`. The car name
/// is server-supplied, so it is sanitized to a single, safe path component
/// before being joined — it can never escape `setups_dir`.
pub fn car_subdir(setups_dir: &Path, car: &str) -> PathBuf {
    let safe = crate::download::sanitize_filename(car);
    if safe.is_empty() {
        setups_dir.to_path_buf()
    } else {
        setups_dir.join(safe)
    }
}

#[cfg(windows)]
fn documents_dir() -> Option<PathBuf> {
    // M2 will use the Known Folder API for OneDrive-redirected Documents;
    // the USERPROFILE join is a correct default for the common case.
    std::env::var_os("USERPROFILE").map(|p| PathBuf::from(p).join("Documents"))
}

#[cfg(not(windows))]
fn documents_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(|p| PathBuf::from(p).join("Documents"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iracing_dir_ends_with_setups() {
        let dir = iracing_setups_dir().expect("a home/profile dir exists in test env");
        assert!(dir.ends_with("setups"));
        assert!(dir.to_string_lossy().contains("iRacing"));
    }

    #[test]
    fn resolve_uses_existing_override_and_rejects_missing() {
        // An existing override directory is returned as-is.
        let existing = std::env::temp_dir();
        assert_eq!(
            resolve_setups_dir(Some(existing.clone())).unwrap(),
            existing
        );
        // A non-existent path is a clear, named error (drives the Settings hint).
        let missing = existing.join("pf-core-definitely-missing-xyz");
        assert!(matches!(
            resolve_setups_dir(Some(missing)),
            Err(Error::SetupsDirNotFound(_))
        ));
    }

    #[test]
    fn car_subdir_sanitizes_and_handles_empty() {
        let base = PathBuf::from("/setups");
        assert_eq!(car_subdir(&base, "Ferrari 296 GT3"), base.join("Ferrari 296 GT3"));
        // A traversal attempt collapses to a single safe component.
        assert_eq!(car_subdir(&base, "../../etc"), base.join("etc"));
        // No car → write straight into the setups dir, never above it.
        assert_eq!(car_subdir(&base, ""), base);
    }
}
