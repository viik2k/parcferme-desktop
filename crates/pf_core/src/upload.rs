//! Identify a local setup file and push it to parcferme.cc. **[M5: push]**
//!
//! The mirror image of [`crate::download`]: instead of routing a server file
//! into the right sim folder, we read the sim's folder layout back off the
//! picked file's path — extension names the sim (same ground truth the
//! download path trusts), and if the file sits under that sim's setups root
//! the `<car>[\<track>]` components name the car and track. Everything
//! inferred is a *suggestion* the UI lets the user correct before uploading;
//! the server is the final authority on metadata.

use std::path::Path;

use serde::Serialize;

use crate::api::{ApiClient, UploadMeta, UploadResult};
use crate::settings::Settings;
use crate::sim::{Folder, Sim};
use crate::{auth, paths, Error, Result};

/// Largest file the client will push. Real setup files are a few KB — this
/// only guards against picking the wrong file entirely (the server enforces
/// its own limit too).
pub const MAX_UPLOAD_BYTES: u64 = 2 * 1024 * 1024;

/// What could be inferred about a picked setup file before uploading.
/// Serialized straight across IPC (`sim` as its short id) for the upload form.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SetupIdentity {
    /// The file's own name, e.g. `quali_spa.json`.
    pub filename: String,
    /// Sim inferred from the extension; `None` for an unrecognized format.
    pub sim: Option<Sim>,
    /// Car folder id, for sims whose layout has a car level (iRacing, ACC).
    /// Always `None` for LMU, which files by track alone — the user names the
    /// car in the form.
    pub car: Option<String>,
    /// Track folder id, for sims whose layout has a track level (ACC, LMU).
    pub track: Option<String>,
}

/// Infer sim/car/track for a setup file the user picked.
///
/// Sim comes from the extension. Car/track come from the file's position under
/// that sim's setups root (override-aware, same resolution as downloads), read
/// against the sim's own [`Sim::layout`]: `<root>\<car>\file` for iRacing,
/// `<root>\<car>\<track>\file` for ACC, `<root>\<track>\file` for LMU.
/// A file anywhere else simply yields `None`s — the UI asks the user instead.
pub fn identify(path: &Path, settings: &Settings) -> SetupIdentity {
    let filename = path
        .file_name()
        .map(|f| f.to_string_lossy().into_owned())
        .unwrap_or_default();
    let sim = Sim::from_filename(&filename);
    let (car, track) = match sim {
        Some(sim) => locate_in_sim_tree(path, sim, settings),
        None => (None, None),
    };
    SetupIdentity {
        filename,
        sim,
        car,
        track,
    }
}

/// Read the sim's folder levels off `path`'s position under its setups root,
/// mapping them onto [`Sim::layout`] in order. Any mismatch (different root,
/// file directly in the root) is `(None, None)` — inference must never block an
/// upload.
///
/// LMU caveat: a setup saved from the garage rather than at a track sits in a
/// *car*-named folder, which we'd report as its track. Nothing on disk
/// distinguishes the two, and every field stays editable, so the guess stands.
fn locate_in_sim_tree(
    path: &Path,
    sim: Sim,
    settings: &Settings,
) -> (Option<String>, Option<String>) {
    let status = paths::sim_folder_status(sim, settings.sim_folders.get(&sim).cloned());
    let Some(root) = status.dir else {
        return (None, None);
    };
    let Some(parts) = relative_components(path, &root) else {
        return (None, None);
    };
    // Drop the file itself; whatever folders precede it line up with the layout.
    // Extra nesting past the layout (people keep per-season subfolders) is
    // ignored by the zip.
    let folders = &parts[..parts.len().saturating_sub(1)];

    let mut car = None;
    let mut track = None;
    for (level, value) in sim.layout().iter().zip(folders) {
        match level {
            Folder::Car => car = Some(value.clone()),
            Folder::Track => track = Some(value.clone()),
        }
    }
    (car, track)
}

/// The path components of `path` below `root`, or `None` if it isn't under it.
///
/// Compares component-wise and **case-insensitively**, unlike
/// `Path::strip_prefix`. Windows paths are case-insensitive and the two sides
/// reach us from different places — the file picker gives
/// `C:\Program Files (x86)\Steam\…` while LMU's root is detected from Steam's
/// registry value, which reads `c:/program files (x86)/steam`. A literal
/// comparison never matches those, silently dropping every LMU inference.
/// Comparing components also makes the `/` vs `\` difference a non-issue.
fn relative_components(path: &Path, root: &Path) -> Option<Vec<String>> {
    let split = |p: &Path| -> Vec<String> {
        p.components()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect()
    };
    let parts = split(path);
    let root_parts = split(root);
    // ponytail: ASCII-only folding — a non-ASCII folder name still compares
    // exactly, which is the old behaviour and degrades to manual entry.
    let under = parts.len() > root_parts.len()
        && root_parts
            .iter()
            .zip(&parts)
            .all(|(r, p)| r.eq_ignore_ascii_case(p));
    under.then(|| parts[root_parts.len()..].to_vec())
}

/// Push one setup file to parcferme.cc as this device's linked user.
///
/// `car`/`track` should be the sim's internal folder ids (exactly what
/// [`identify`] reads off disk); the server maps them to its car/track records
/// — see SERVER_CONTRACT §7.
pub fn upload_setup(
    path: &Path,
    sim: Sim,
    car: &str,
    track: Option<&str>,
    name: Option<&str>,
) -> Result<UploadResult> {
    // File checks first — cheap, pure, and they keep obviously-bad input from
    // ever touching the keychain or the network.
    let filename = path
        .file_name()
        .map(|f| f.to_string_lossy().into_owned())
        .filter(|f| !f.is_empty())
        .ok_or_else(|| Error::Api(format!("not a file: {}", path.display())))?;

    // Sims that file by track can't place the setup without one, and the server
    // rejects the metadata outright — say so plainly instead of surfacing an HTTP
    // error the user can't act on.
    if sim.needs_track_folder() && track.is_none_or(|t| t.trim().is_empty()) {
        return Err(Error::Api(format!(
            "{} files setups by track, so this upload needs a track before it can be shared",
            sim.display_name()
        )));
    }

    let size = std::fs::metadata(path)?.len();
    if size > MAX_UPLOAD_BYTES {
        return Err(Error::Api(format!(
            "{filename} is {size} bytes — setup files are a few KB, so this doesn't look like one (limit {MAX_UPLOAD_BYTES})"
        )));
    }
    let bytes = std::fs::read(path)?;

    let token = auth::current_token()?.ok_or(Error::NotLinked)?;

    log::info!(
        "uploading {filename} ({} bytes, sim {})",
        bytes.len(),
        sim.id()
    );
    let result = ApiClient::from_env().upload_setup(
        token.as_str(),
        &UploadMeta {
            filename: &filename,
            sim,
            car,
            track,
            name,
        },
        &bytes,
    )?;
    log::info!("uploaded {filename} as setup {}", result.id);
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// Settings whose ACC/iRacing roots point at a scratch dir we control.
    fn scratch_settings(root: &Path) -> Settings {
        let mut s = Settings::default();
        for sim in Sim::ALL {
            s.sim_folders.insert(sim, root.to_path_buf());
        }
        s
    }

    fn scratch(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("pf-upload-{name}-{}", std::process::id()))
    }

    #[test]
    fn identify_reads_car_and_track_from_sim_layout() {
        let root = scratch("layout");
        let settings = scratch_settings(&root);

        // ACC: <root>\<car>\<track>\file.json → sim + car + track.
        let acc = identify(
            &root.join("ferrari_488_gt3_evo").join("spa").join("q.json"),
            &settings,
        );
        assert_eq!(acc.sim, Some(Sim::Acc));
        assert_eq!(acc.car.as_deref(), Some("ferrari_488_gt3_evo"));
        assert_eq!(acc.track.as_deref(), Some("spa"));

        // iRacing: <root>\<car>\file.sto → car, never a track.
        let ir = identify(&root.join("ferrari296gt3").join("r.sto"), &settings);
        assert_eq!(ir.sim, Some(Sim::IRacing));
        assert_eq!(ir.car.as_deref(), Some("ferrari296gt3"));
        assert_eq!(ir.track, None);

        // LMU: <root>\<track>\file.svm → track, never a car. Reading the folder
        // as a car is the 2026-07-25 upload bug.
        let lmu = identify(&root.join("Fuji").join("GT3_Balanced.svm"), &settings);
        assert_eq!(lmu.sim, Some(Sim::Lmu));
        assert_eq!(lmu.track.as_deref(), Some("Fuji"));
        assert_eq!(lmu.car, None);

        // iRacing file nested deeper still reads the first component as the car
        // (people keep subfolders per season) and no track.
        let deep = identify(
            &root.join("ferrari296gt3").join("2026s3").join("r.sto"),
            &settings,
        );
        assert_eq!(deep.car.as_deref(), Some("ferrari296gt3"));
        assert_eq!(deep.track, None);
    }

    #[test]
    fn identify_degrades_to_none_outside_the_sim_tree() {
        let root = scratch("outside");
        let settings = scratch_settings(&root);

        // Recognized format but foreign location: sim yes, car/track no.
        let desktop = identify(Path::new(r"C:\Users\x\Desktop\q.json"), &settings);
        assert_eq!(desktop.sim, Some(Sim::Acc));
        assert_eq!(desktop.car, None);
        assert_eq!(desktop.track, None);

        // File directly in the setups root: no car folder to read.
        let bare = identify(&root.join("loose.sto"), &settings);
        assert_eq!(bare.sim, Some(Sim::IRacing));
        assert_eq!(bare.car, None);

        // Unrecognized extension: nothing inferred, filename still reported.
        let zip = identify(&root.join("car").join("pack.zip"), &settings);
        assert_eq!(zip.sim, None);
        assert_eq!(zip.filename, "pack.zip");
    }

    #[test]
    fn root_matching_ignores_case_and_separators() {
        // LMU's root comes from Steam's registry value (`c:/program files…`)
        // while the file picker returns `C:\Program Files…` — a literal
        // strip_prefix matches neither, and every inference silently dies.
        let root = Path::new(r"c:/steam/common/Le Mans Ultimate/UserData");
        let picked = Path::new(r"C:\Steam\Common\Le Mans Ultimate\USERDATA\Fuji\q.svm");
        assert_eq!(
            relative_components(picked, root),
            Some(vec!["Fuji".to_string(), "q.svm".to_string()])
        );

        // A genuinely different tree still doesn't match…
        assert_eq!(
            relative_components(Path::new(r"C:\Elsewhere\q.svm"), root),
            None
        );
        // …and the root itself has no components below it.
        assert_eq!(relative_components(root, root), None);
    }

    #[test]
    fn upload_rejects_oversized_and_missing_files() {
        let dir = scratch("reject");
        std::fs::create_dir_all(&dir).unwrap();

        // Missing file surfaces as Io, before any network call.
        let missing = upload_setup(&dir.join("nope.sto"), Sim::IRacing, "car", None, None);
        assert!(matches!(missing, Err(Error::Io(_))));

        // Oversized file is refused client-side with a clear Api error.
        let big = dir.join("big.sto");
        std::fs::write(&big, vec![0u8; (MAX_UPLOAD_BYTES + 1) as usize]).unwrap();
        let too_big = upload_setup(&big, Sim::IRacing, "car", None, None);
        assert!(matches!(too_big, Err(Error::Api(_))));

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn upload_refuses_a_track_sim_with_no_track() {
        let dir = scratch("no-track");
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("q.svm");
        std::fs::write(&file, b"[setup]").unwrap();

        // Missing and blank both fail before any keychain or network access —
        // the server would 422 with nothing the user could act on.
        for track in [None, Some("  ")] {
            let err = upload_setup(&file, Sim::Lmu, "ferrari_499p", track, None);
            assert!(matches!(err, Err(Error::Api(m)) if m.contains("track")));
        }

        std::fs::remove_dir_all(&dir).unwrap();
    }
}
