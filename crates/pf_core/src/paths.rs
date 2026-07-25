//! Locate sim setup directories and lay out one setup file. **[M2 iRacing · M3 multi-sim]**
//!
//! Pull is sim-agnostic at the byte level, but folder layout differs per sim, so
//! the *where* lives in [`crate::sim::Sim`] and this module turns that into a
//! validated, override-aware directory. Sim folders can also be in non-default /
//! OneDrive-redirected / multi-drive locations, so a per-sim Settings override
//! sits on top of best-effort detection.

use std::path::{Path, PathBuf};

use crate::sim::{Folder, Sim};
use crate::{Error, Result};

/// Le Mans Ultimate's folder name inside a Steam library's `steamapps\common`.
const LMU_FOLDER: &str = "Le Mans Ultimate";

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

/// Best-effort default location of `sim`'s setups directory. Path only — see
/// [`resolve_setups_dir`] for the validated, override-aware version used at
/// download time.
///
/// Most sims keep setups under Documents. LMU (rFactor 2 heritage) keeps
/// `UserData` inside the game install instead, so its base has to be located in
/// the Steam libraries; if that fails the user sets the folder in Settings.
pub fn default_setups_dir(sim: Sim) -> Result<PathBuf> {
    let base = match sim {
        Sim::Lmu => lmu_install_dir().ok_or_else(|| {
            Error::SetupsDirNotFound(format!(
                "no {LMU_FOLDER} install in any of your Steam libraries"
            ))
        })?,
        _ => documents_dir().ok_or(Error::NotImplemented(
            "paths::documents_dir (unsupported platform)",
        ))?,
    };
    Ok(sim.setups_root(&base))
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

/// The folder a single setup is written into, beneath `setups_dir`, following
/// the sim's [`Sim::layout`]: `<car>` for iRacing, `<car>\<track>` for ACC,
/// `<track>` for LMU. Levels the sim doesn't use are ignored, so a `car` sent
/// for an LMU setup never becomes a stray folder.
///
/// Both the server-supplied `car` and `track` are sanitized to a single safe path
/// component each, so neither can ever escape `setups_dir` (path-traversal
/// defence, Build Plan §6). A missing/empty component is simply skipped — for a
/// track-nesting sim that means the setup lands one level up and may not show
/// in-game, which is better than refusing to write it.
pub fn setup_target_dir(setups_dir: &Path, sim: Sim, car: &str, track: Option<&str>) -> PathBuf {
    let mut dir = setups_dir.to_path_buf();
    for level in sim.layout() {
        let value = match level {
            Folder::Car => Some(car),
            Folder::Track => track,
        };
        if let Some(component) = value.and_then(safe_component) {
            dir.push(component);
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

/// Locate the Le Mans Ultimate install by walking every Steam library on this
/// machine. Returns the game directory (the one holding `UserData`).
#[cfg(windows)]
fn lmu_install_dir() -> Option<PathBuf> {
    steam_libraries()
        .into_iter()
        .map(|lib| lib.join("steamapps").join("common").join(LMU_FOLDER))
        .find(|dir| dir.is_dir())
}

#[cfg(not(windows))]
fn lmu_install_dir() -> Option<PathBuf> {
    None
}

/// Every Steam library root: the client's own directory plus each `path` listed
/// in `libraryfolders.vdf` — games routinely live on a different drive from
/// Steam itself.
#[cfg(windows)]
fn steam_libraries() -> Vec<PathBuf> {
    let Some(steam) = steam_root() else {
        return Vec::new();
    };
    let mut libs = vec![steam.clone()];
    if let Ok(vdf) = std::fs::read_to_string(steam.join("steamapps").join("libraryfolders.vdf")) {
        libs.extend(parse_library_paths(&vdf));
    }
    libs
}

/// Scrape the quoted `"path"` values out of `libraryfolders.vdf`.
///
/// ponytail: the file is a tiny key/value tree and we want exactly one key out
/// of it — a real VDF parser would be a dependency for two lines of work. If we
/// ever need more of the file (app ids, sizes), swap in `keyvalues-parser`.
#[cfg(windows)]
fn parse_library_paths(vdf: &str) -> Vec<PathBuf> {
    vdf.lines()
        .filter_map(|line| line.trim().strip_prefix("\"path\""))
        .filter_map(|rest| {
            let quoted = rest.trim().strip_prefix('"')?.strip_suffix('"')?;
            // VDF escapes backslashes; `C:\\Games\\Steam` is really `C:\Games\Steam`.
            Some(PathBuf::from(quoted.replace("\\\\", "\\")))
        })
        .collect()
}

/// Where the Steam client is installed. The registry value is authoritative
/// (Steam can be installed anywhere); the Program Files defaults are the
/// fallback for a machine where the key is missing.
#[cfg(windows)]
fn steam_root() -> Option<PathBuf> {
    reg_read(r"HKCU\Software\Valve\Steam", "SteamPath")
        .map(PathBuf::from)
        .filter(|p| p.is_dir())
        .or_else(|| {
            ["ProgramFiles(x86)", "ProgramFiles"]
                .iter()
                .filter_map(std::env::var_os)
                .map(|p| PathBuf::from(p).join("Steam"))
                .find(|p| p.is_dir())
        })
}

/// Read a single registry string value.
///
/// ponytail: shells out to `reg.exe` rather than adding a registry crate to
/// `pf_core` for one cold-path lookup. `CREATE_NO_WINDOW` keeps a console from
/// flashing in front of the tray app.
#[cfg(windows)]
fn reg_read(key: &str, value: &str) -> Option<String> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    let out = std::process::Command::new("reg")
        .args(["query", key, "/v", value])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .ok()?;
    // `    SteamPath    REG_SZ    C:/Program Files (x86)/Steam`
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .find_map(|line| line.split("REG_SZ").nth(1))
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
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

        // iRacing: car only, track ignored even if present.
        assert_eq!(
            setup_target_dir(&base, Sim::IRacing, "ferrari296gt3", Some("spa")),
            base.join("ferrari296gt3")
        );

        // LMU: track only — the car is metadata, never a folder.
        assert_eq!(
            setup_target_dir(&base, Sim::Lmu, "ferrari_499p", Some("Fuji")),
            base.join("Fuji")
        );
        // …and with no track it lands in the root rather than a bogus car folder.
        assert_eq!(
            setup_target_dir(&base, Sim::Lmu, "ferrari_499p", None),
            base
        );

        // ACC: car + track, in that order.
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

    /// The whole LMU chain against a live install — Steam detection, the setups
    /// root, and reading a track back off a real setup file. Machine-dependent,
    /// so ignored by default: run
    /// `cargo test -p pf_core lmu_on_this_machine -- --ignored --nocapture`
    /// on a box with Le Mans Ultimate installed.
    #[cfg(windows)]
    #[test]
    #[ignore]
    fn lmu_on_this_machine() {
        println!("steam libraries: {:#?}", steam_libraries());
        let root = default_setups_dir(Sim::Lmu).expect("LMU install not found");
        println!("lmu setups dir: {}", root.display());
        assert!(root.is_dir(), "{} should exist", root.display());

        // Any `.svm` under a track folder must identify as LMU + that track.
        let setup = std::fs::read_dir(&root)
            .unwrap()
            .filter_map(|e| e.ok())
            .flat_map(|track| std::fs::read_dir(track.path()).into_iter().flatten())
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .find(|p| p.extension().is_some_and(|e| e == "svm"));
        let Some(setup) = setup else {
            println!("no .svm setups saved yet — detection half verified only");
            return;
        };
        let id = crate::upload::identify(&setup, &crate::settings::Settings::default());
        println!("{} → {id:?}", setup.display());
        assert_eq!(id.sim, Some(Sim::Lmu));
        assert!(id.track.is_some(), "track must be inferred");
        assert_eq!(id.car, None, "LMU has no car folder");
    }

    #[cfg(windows)]
    #[test]
    fn library_paths_are_scraped_and_unescaped() {
        let vdf = r#"
"libraryfolders"
{
	"0"
	{
		"path"		"C:\\Program Files (x86)\\Steam"
		"totalsize"		"0"
	}
	"1"
	{
		"path"		"F:\\SteamLibrary"
	}
}"#;
        assert_eq!(
            parse_library_paths(vdf),
            vec![
                PathBuf::from(r"C:\Program Files (x86)\Steam"),
                PathBuf::from(r"F:\SteamLibrary"),
            ]
        );
        // Nothing usable in the file must not panic or invent a library.
        assert!(parse_library_paths("junk").is_empty());
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
