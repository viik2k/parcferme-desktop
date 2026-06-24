//! Locate sim setup directories and lay out one setup file. **[M2 iRacing · M3 multi-sim]**
//!
//! Pull is sim-agnostic at the byte level, but folder layout differs per sim, so
//! the *where* lives in [`crate::sim::Sim`] and this module turns that into a
//! validated, override-aware directory. Sim folders can also be in non-default /
//! OneDrive-redirected / multi-drive locations, so a per-sim Settings override
//! sits on top of best-effort detection.

use std::path::{Path, PathBuf};

use crate::sim::Sim;
use crate::{Error, Result};

/// Resolve the setups directory for `sim` to write into, validating it exists.
///
/// An explicit Settings `override_dir` wins (covers the non-default Documents /
/// OneDrive-redirected / multi-drive cases in the Build Plan's risk table);
/// otherwise we fall back to best-effort detection. Either way the directory
/// must exist — a setups folder is created by the sim, not by us, so its absence
/// almost always means a wrong path rather than a first run.
pub fn resolve_setups_dir(sim: Sim, override_dir: Option<PathBuf>) -> Result<PathBuf> {
    let dir = match override_dir {
        Some(d) => d,
        None => default_setups_dir(sim)?,
    };
    if dir.is_dir() {
        Ok(dir)
    } else {
        Err(Error::SetupsDirNotFound(dir.display().to_string()))
    }
}

/// Best-effort default location of `sim`'s setups directory under the user's
/// Documents folder. Path only — see [`resolve_setups_dir`] for the validated,
/// override-aware version used at download time.
pub fn default_setups_dir(sim: Sim) -> Result<PathBuf> {
    let documents = documents_dir().ok_or(Error::NotImplemented(
        "paths::documents_dir (unsupported platform)",
    ))?;
    Ok(sim.setups_root(&documents))
}

/// Read-only view of a sim's setups folder for the UI's "detected folders" list.
/// Never errors: an unresolvable or missing folder is reported as `found = false`
/// so the UI can prompt for an override rather than failing.
#[derive(Debug, Clone)]
pub struct SimFolderStatus {
    pub sim: Sim,
    /// The directory we'd use (override if given, else the detected default).
    /// `None` only if the platform has no Documents dir (not Windows).
    pub dir: Option<PathBuf>,
    /// Whether `dir` exists on disk.
    pub found: bool,
    /// Whether `dir` came from a Settings override rather than detection.
    pub overridden: bool,
}

/// Compute a sim's folder status without erroring (drives the folder list UI).
pub fn sim_folder_status(sim: Sim, override_dir: Option<PathBuf>) -> SimFolderStatus {
    let overridden = override_dir.is_some();
    let dir = override_dir.or_else(|| default_setups_dir(sim).ok());
    let found = dir.as_deref().is_some_and(Path::is_dir);
    SimFolderStatus {
        sim,
        dir,
        found,
        overridden,
    }
}

/// The folder a single setup is written into, beneath `setups_dir`, following the
/// sim's layout: `<car>` for iRacing/LMU, `<car>\<track>` for ACC.
///
/// Both the server-supplied `car` and `track` are sanitized to a single safe path
/// component each, so neither can ever escape `setups_dir` (path-traversal
/// defence, Build Plan §6). A missing/empty component is simply skipped — for ACC
/// that means a setup with no track lands in `<car>\` and may not show in-game,
/// which is better than refusing to write it.
pub fn setup_target_dir(setups_dir: &Path, sim: Sim, car: &str, track: Option<&str>) -> PathBuf {
    let mut dir = setups_dir.to_path_buf();
    if let Some(c) = safe_component(car) {
        dir.push(c);
    }
    if sim.needs_track_subfolder() {
        if let Some(t) = track.and_then(safe_component) {
            dir.push(t);
        }
    }
    dir
}

/// Reduce a server-supplied folder name to a single safe path component, or
/// `None` if nothing usable remains.
fn safe_component(name: &str) -> Option<String> {
    let safe = crate::download::sanitize_filename(name);
    (!safe.is_empty()).then_some(safe)
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
    fn default_dir_is_sim_specific() {
        let iracing =
            default_setups_dir(Sim::IRacing).expect("home/profile dir exists in test env");
        assert!(iracing.ends_with("setups"));
        assert!(iracing.to_string_lossy().contains("iRacing"));

        let acc = default_setups_dir(Sim::Acc).expect("home/profile dir exists in test env");
        assert!(acc.to_string_lossy().contains("Assetto Corsa Competizione"));
    }

    #[test]
    fn resolve_uses_existing_override_and_rejects_missing() {
        // An existing override directory is returned as-is, for any sim.
        let existing = std::env::temp_dir();
        assert_eq!(
            resolve_setups_dir(Sim::Acc, Some(existing.clone())).unwrap(),
            existing
        );
        // A non-existent path is a clear, named error (drives the Settings hint).
        let missing = existing.join("pf-core-definitely-missing-xyz");
        assert!(matches!(
            resolve_setups_dir(Sim::IRacing, Some(missing)),
            Err(Error::SetupsDirNotFound(_))
        ));
    }

    #[test]
    fn target_dir_follows_per_sim_layout() {
        let base = PathBuf::from("/setups");

        // iRacing / LMU: car only, track ignored even if present.
        assert_eq!(
            setup_target_dir(&base, Sim::IRacing, "ferrari296gt3", Some("spa")),
            base.join("ferrari296gt3")
        );
        assert_eq!(
            setup_target_dir(&base, Sim::Lmu, "ferrari_499p", None),
            base.join("ferrari_499p")
        );

        // ACC: car + track.
        assert_eq!(
            setup_target_dir(&base, Sim::Acc, "ferrari_488_gt3_evo", Some("spa")),
            base.join("ferrari_488_gt3_evo").join("spa")
        );
        // ACC with no track still writes under the car folder (won't list in-game,
        // but we don't drop the file).
        assert_eq!(
            setup_target_dir(&base, Sim::Acc, "ferrari_488_gt3_evo", None),
            base.join("ferrari_488_gt3_evo")
        );
    }

    #[test]
    fn target_dir_sanitizes_traversal_in_car_and_track() {
        let base = PathBuf::from("/setups");
        // A traversal attempt in either component collapses to a single safe name.
        assert_eq!(
            setup_target_dir(&base, Sim::Acc, "../../etc", Some("../../passwd")),
            base.join("etc").join("passwd")
        );
        // No car → write straight into the setups dir, never above it.
        assert_eq!(setup_target_dir(&base, Sim::IRacing, "", None), base);
    }

    #[test]
    fn folder_status_reports_existence_and_override() {
        let existing = std::env::temp_dir();
        let s = sim_folder_status(Sim::Acc, Some(existing.clone()));
        assert!(s.found);
        assert!(s.overridden);
        assert_eq!(s.dir.as_deref(), Some(existing.as_path()));

        let missing = existing.join("pf-core-missing-status-xyz");
        let s = sim_folder_status(Sim::Lmu, Some(missing));
        assert!(!s.found);
        assert!(s.overridden);
    }
}
