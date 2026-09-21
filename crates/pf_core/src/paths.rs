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
    /// `None` when detection comes up empty — no Documents dir, or no LMU
    /// install in any Steam library.
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
fn lmu_install_dir() -> Option<PathBuf> {
    lmu_install_dir_in(&steam_libraries())
}

/// [`lmu_install_dir`] against an explicit library list — the seam the
/// fixture tests drive, because a dev box has no Steam install to detect
/// against for real. Production has nothing to inject but the live list.
fn lmu_install_dir_in(libraries: &[PathBuf]) -> Option<PathBuf> {
    libraries
        .iter()
        .flat_map(|lib| lmu_candidates(lib))
        .find(|dir| dir.is_dir())
}

/// The candidate game-install dirs inside one Steam library, in probe order:
/// the Proton prefix views first (non-Windows), then the library's own
/// `steamapps/common`.
///
/// Steam stores a game's files under `<library>/steamapps/common/…` on every
/// OS — Proton included: the prefix's `drive_c/…/Steam/steamapps` is a link
/// back into the real `steamapps` tree, so on Linux the candidates usually
/// resolve to the same directory and first-hit wins either way. Where they
/// can disagree, no bet is made — every candidate is `is_dir`-probed and
/// the first one actually on disk wins (issue #35: detection was written
/// against fixture trees, not a live Linux Steam). The prefix views go
/// first because a prefix can only exist if the game actually ran through
/// Proton — the only way LMU, a Windows-only title, runs on Linux at all;
/// the library's own `common` dir is the fallback, and the whole story if a
/// native build ever appears.
fn lmu_candidates(lib: &Path) -> Vec<PathBuf> {
    let mut candidates = Vec::with_capacity(3);
    #[cfg(not(windows))]
    candidates.extend(lmu_proton_dirs(lib));
    candidates.extend(lmu_native_dir(lib));
    candidates
}

/// The game dir under the library's own `steamapps/common`. On Windows the
/// file API itself is case-insensitive, so the canonical name joins
/// directly; elsewhere the name is probed by case-insensitive scan like the
/// prefix segments are — a Proton-written directory's casing is not
/// something to bet an exact match on.
#[cfg(windows)]
fn lmu_native_dir(lib: &Path) -> Option<PathBuf> {
    Some(lib.join("steamapps").join("common").join(LMU_FOLDER))
}

/// [`lmu_native_dir`], probed rather than joined: case-sensitive filesystem.
#[cfg(not(windows))]
fn lmu_native_dir(lib: &Path) -> Option<PathBuf> {
    find_dir_ci(&lib.join("steamapps").join("common"), LMU_FOLDER)
}

/// Resolve the game's directory inside a library's Proton prefix:
/// `steamapps/compatdata/<LMU app id>/pfx/drive_c/…/steamapps/common/<LMU>`.
///
/// Two spellings of the `drive_c` side are seen in the wild — the classic
/// layout keeps the client's `Steam` symlink in the chain
/// (`…/Program Files (x86)/Steam/steamapps/…`), some Proton versions write
/// the same chain without it (`…/Program Files (x86)/steamapps/…`) — and
/// without a live Linux Steam install to check against, both are probed and
/// whichever exists on disk wins.
///
/// Every segment is matched by case-insensitive scan ([`find_dir_ci`]): the
/// prefix mimics a Windows `C:` drive but is written by Linux Steam, whose
/// segment casing has drifted between Proton versions (`Program Files (x86)`
/// vs `program files (x86)`), while the Windows file API the game itself
/// sees is case-insensitive anyway.
#[cfg(not(windows))]
fn lmu_proton_dirs(lib: &Path) -> Vec<PathBuf> {
    let drive_c = lib
        .join("steamapps")
        .join("compatdata")
        .join(LMU_APP_ID)
        .join("pfx")
        .join("drive_c");
    ["program files (x86)", "program files"]
        .into_iter()
        .filter_map(|files| find_dir_ci(&drive_c, files))
        .flat_map(|files| {
            [
                find_dir_ci(&files, "steamapps"),
                find_dir_ci(&files, "Steam").map(|steam| steam.join("steamapps")),
            ]
        })
        .flatten()
        .filter_map(|apps| {
            find_dir_ci(&apps, "common").and_then(|common| find_dir_ci(&common, LMU_FOLDER))
        })
        .collect()
}

/// The first direct child of `dir` whose name matches `name` ignoring ASCII
/// case, provided it is a directory. Symlinks are followed (`is_dir`
/// resolves them) and a dangling one fails the check — an uninstall must not
/// leave detection pointing at a ghost.
#[cfg(not(windows))]
fn find_dir_ci(dir: &Path, name: &str) -> Option<PathBuf> {
    std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .find(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .eq_ignore_ascii_case(name)
        })
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
}

/// Every Steam library root: the client's own directory plus each `path`
/// listed in `libraryfolders.vdf` — games routinely live on a different
/// drive from Steam itself, and the VDF's format is identical on every OS
/// Steam runs on.
fn steam_libraries() -> Vec<PathBuf> {
    let Some(steam) = steam_root() else {
        return Vec::new();
    };
    steam_libraries_in(&steam)
}

/// [`steam_libraries`] against an explicit Steam root — the fixture-test
/// seam, mirroring [`lmu_install_dir_in`].
fn steam_libraries_in(steam: &Path) -> Vec<PathBuf> {
    let mut libs = vec![steam.to_path_buf()];
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
fn parse_library_paths(vdf: &str) -> Vec<PathBuf> {
    vdf.lines()
        .filter_map(|line| line.trim().strip_prefix("\"path\""))
        .filter_map(|rest| {
            let quoted = rest.trim().strip_prefix('"')?.strip_suffix('"')?;
            // VDF escapes backslashes; `C:\\Games\\Steam` is really `C:\Games\Steam`.
            // (On a Unix Steam the values hold forward slashes and this is a no-op.)
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

/// Where the Steam client is installed on a Unix host. The candidates, in
/// order of authority:
///
/// 1. `~/.steam/steam` — the symlink the client maintains, which survives
///    the user relocating the real install (`~/.steam` is a symlink farm),
/// 2. `~/.local/share/Steam` — the default install location,
/// 3. `~/.steam/root` — an older alias some distro packages still create.
///
/// Each candidate must look like a real Steam root before it counts: a
/// `~/.steam` entry left behind by an uninstall would otherwise send the
/// whole library walk into a dead directory.
#[cfg(not(windows))]
fn steam_root() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from)?;
    steam_root_in(&home)
}

/// [`steam_root`]'s probe order against an explicit home directory — the
/// fixture-test seam, mirroring [`lmu_install_dir_in`].
#[cfg(not(windows))]
fn steam_root_in(home: &Path) -> Option<PathBuf> {
    [".steam/steam", ".local/share/Steam", ".steam/root"]
        .into_iter()
        .map(|rel| home.join(rel))
        .find(|candidate| looks_like_steam_root(candidate))
}

/// Whether a directory plausibly *is* a Steam root: the library index file
/// or the shared game-files dir exists beneath its `steamapps`.
#[cfg(not(windows))]
fn looks_like_steam_root(root: &Path) -> bool {
    root.join("steamapps").join("libraryfolders.vdf").is_file()
        || root.join("steamapps").join("common").is_dir()
}

/// Le Mans Ultimate's Steam app id — the `compatdata` directory its Proton
/// prefix lives under on a Linux install (store page 1636160).
#[cfg(not(windows))]
const LMU_APP_ID: &str = "1636160";

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

    /// The fixture half of a Steam install, shared by the detection tests:
    /// a root holding `steamapps/libraryfolders.vdf` (so it counts as a real
    /// Steam root), pointing at one extra library, with LMU installed in
    /// whatever layout a test asks for. Built under a fresh temp dir each
    /// call, because this box has no Steam install to detect against for
    /// real — every assertion below runs against a tree this test created.
    #[cfg(not(windows))]
    fn steam_fixture(name: &str) -> std::path::PathBuf {
        let root = std::env::temp_dir().join(format!("pf-paths-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("steamapps/common")).expect("create steam root");
        std::fs::write(
            root.join("steamapps/libraryfolders.vdf"),
            "\"libraryfolders\"\n{\n\t\"0\"\n\t{\n\t\t\"path\"\t\t\"/nowhere\"\n\t}\n}\n",
        )
        .expect("write vdf");
        root
    }

    /// `mkdir -p` for fixture trees: every argument is created under `root`.
    #[cfg(not(windows))]
    fn mkdirs(root: &Path, rel: &str) -> PathBuf {
        let dir = root.join(rel);
        std::fs::create_dir_all(&dir).expect("create fixture dir");
        dir
    }

    /// A Steam root is accepted in all three spellings the client/distros
    /// produce, and rejected when nothing Steam-like lives in it.
    #[cfg(not(windows))]
    #[test]
    fn steam_root_probe_order_matches_the_real_layouts() {
        let home = std::env::temp_dir().join(format!("pf-paths-home-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);

        // No Steam at all → no root, even with the symlink farm present.
        mkdirs(&home, ".steam");
        assert_eq!(steam_root_in(&home), None);

        // The default install location counts on its own, once it holds the
        // library index.
        let real = mkdirs(&home, ".local/share/Steam/steamapps/common");
        std::fs::write(real.join("libraryfolders.vdf"), "").unwrap();
        assert_eq!(steam_root_in(&home), Some(home.join(".local/share/Steam")));

        // The client's own symlink outranks the default, wherever it points
        // (the `~/.steam` symlink farm survives a relocated install). The
        // candidate is returned as the symlink path itself — `is_dir`
        // resolves through it, and production keeps using `~/.steam/steam`.
        let moved = mkdirs(&home, "elsewhere/Steam/steamapps/common");
        std::fs::write(moved.parent().unwrap().join("libraryfolders.vdf"), "").unwrap();
        std::os::unix::fs::symlink(
            moved.parent().unwrap().parent().unwrap(),
            home.join(".steam/steam"),
        )
        .unwrap();
        assert_eq!(steam_root_in(&home), Some(home.join(".steam/steam")));

        let _ = std::fs::remove_dir_all(&home);
    }

    /// `libraryfolders.vdf` is read the same way on every OS, so a second
    /// library on another "drive" is walked and the game found there.
    #[cfg(not(windows))]
    #[test]
    fn libraries_are_enumerated_from_the_vdf_like_on_windows() {
        let root = steam_fixture("libs");
        let other = mkdirs(&root, "mnt/games");
        std::fs::write(
            root.join("steamapps/libraryfolders.vdf"),
            format!(
                "\"libraryfolders\"\n{{\n\t\"1\"\n\t{{\n\t\t\"path\"\t\t\"{}\"\n\t}}\n}}\n",
                other.display()
            ),
        )
        .unwrap();

        let libs = steam_libraries_in(&root);
        assert_eq!(libs, vec![root.clone(), other.clone()]);

        // And the LMU walk finds an install sitting in that second library.
        let game = mkdirs(
            &other,
            "steamapps/common/Le Mans Ultimate/UserData/player/Settings",
        );
        let install = lmu_install_dir_in(&libs).expect("install found in second library");
        assert_eq!(install, other.join("steamapps/common/Le Mans Ultimate"));
        assert_eq!(Sim::Lmu.setups_root(&install), game);

        let _ = std::fs::remove_dir_all(&root);
    }

    /// The Proton prefix view: a compatdata prefix for LMU's app id is
    /// resolved through case-varying `drive_c` segments to the game dir,
    /// exactly the chain a Linux Steam writes for a Windows-only title. The
    /// two `drive_c` spellings seen in the wild are probed as separate
    /// fixtures — a shared `drive_c` would make the winner depend on
    /// `read_dir` order.
    #[cfg(not(windows))]
    #[test]
    fn proton_prefix_resolves_lmu_regardless_of_segment_casing() {
        for layout in [
            "steamapps/compatdata/1636160/pfx/drive_c/program files (x86)/steamapps/common/le mans ultimate",
            "steamapps/compatdata/1636160/pfx/drive_c/Program Files (x86)/Steam/steamapps/common/Le Mans Ultimate",
        ] {
            let root = steam_fixture("proton");
            let settings = mkdirs(&root, &format!("{layout}/UserData/player/Settings"));
            let libs = steam_libraries_in(&root);
            let install =
                lmu_install_dir_in(&libs).unwrap_or_else(|| panic!("layout {layout:?} resolves"));
            assert_eq!(
                install,
                root.join(layout),
                "prefix layout {layout:?} must resolve"
            );
            assert_eq!(Sim::Lmu.setups_root(&install), settings);
            let _ = std::fs::remove_dir_all(&root);
        }
    }

    /// The prefix candidate only counts when the game dir really is in it:
    /// an empty compatdata for the right app id (game uninstalled, prefix
    /// left behind) must not shadow a healthy install elsewhere.
    #[cfg(not(windows))]
    #[test]
    fn an_empty_prefix_does_not_shadow_a_real_install() {
        let root = steam_fixture("shadow");
        mkdirs(
            &root,
            "steamapps/compatdata/1636160/pfx/drive_c/program files (x86)",
        );
        let other = mkdirs(&root, "mnt/big");
        std::fs::write(
            root.join("steamapps/libraryfolders.vdf"),
            format!(
                "\"libraryfolders\"\n{{\n\t\"1\"\n\t{{\n\t\t\"path\"\t\t\"{}\"\n\t}}\n}}\n",
                other.display()
            ),
        )
        .unwrap();
        let game = mkdirs(&other, "steamapps/common/Le Mans Ultimate");

        let libs = steam_libraries_in(&root);
        assert_eq!(lmu_install_dir_in(&libs), Some(game));

        let _ = std::fs::remove_dir_all(&root);
    }

    /// The library's own `common` dir is the fallback a native install (or a
    /// Proton one whose prefix view is missing) is found through — found
    /// case-insensitively, since the walk also runs on case-sensitive
    /// filesystems where the exact folder name is Steam's choice, not ours.
    #[cfg(not(windows))]
    #[test]
    fn native_common_dir_is_found_case_insensitively() {
        let root = steam_fixture("native");
        let settings = mkdirs(
            &root,
            "steamapps/common/le mans ultimate/UserData/player/Settings",
        );
        let libs = steam_libraries_in(&root);
        let install = lmu_install_dir_in(&libs).expect("native install found");
        assert_eq!(install, root.join("steamapps/common/le mans ultimate"));
        assert_eq!(Sim::Lmu.setups_root(&install), settings);

        let _ = std::fs::remove_dir_all(&root);
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
