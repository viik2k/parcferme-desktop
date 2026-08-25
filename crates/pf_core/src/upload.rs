//! Identify a local setup file and push it to parcferme.cc. **[M5: push]**
//!
//! The mirror image of [`crate::download`]: instead of routing a server file
//! into the right sim folder, we read the sim's folder layout back off the
//! picked file's path — extension names the sim (same ground truth the
//! download path trusts), and if the file sits under that sim's setups root
//! the `<car>[\<track>]` components name the car and track. Everything
//! inferred is a *suggestion* the UI lets the user correct before uploading;
//! the server is the final authority on metadata.

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::api::{ApiClient, UploadMeta, UploadResult};
use crate::settings::Settings;
use crate::sim::{Folder, Sim};
use crate::{auth, car_aliases, car_match, paths, Error, Result};

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
    /// The car for the form: a folder id, or the site's own name for it when
    /// one could be resolved (see [`car_source`](Self::car_source)). `None`
    /// for LMU, which files by track alone — the user names the car there.
    pub car: Option<String>,
    /// Track folder id, for sims whose layout has a track level (ACC, LMU).
    pub track: Option<String>,
    /// How [`car`](Self::car) was arrived at, so the form can mark a guess as
    /// one. `Folder` whenever there is no car at all.
    pub car_source: CarSource,
    /// Path to an iRacing garage export (`.htm`/`.html`) found beside the
    /// picked `.sto`, if any — see [`find_garage_export`]. `None` for every
    /// other sim, and whenever no sibling export exists. A suggestion like the
    /// rest of this struct: the form shows it and lets the user drop or
    /// replace it.
    pub garage_export: Option<String>,
}

/// Where a pre-filled car name came from. Only [`CarSource::Matched`] is a
/// guess — the UI says so, because a wrong guess must never ride along on an
/// upload unnoticed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CarSource {
    /// Straight off disk: nothing resolved it, so it's the folder id verbatim
    /// (or there is no car to resolve).
    Folder,
    /// A curated [`car_aliases`] row — a decision someone made by hand.
    Alias,
    /// The site's list holds this exact car under [`car_match::exact_match`];
    /// only the spelling changed, which is what the server does anyway.
    Exact,
    /// [`car_match::best_match`] picked the closest name on the site. A guess,
    /// and the only variant the form flags.
    Matched,
}

/// Infer sim/car/track for a setup file the user picked.
///
/// Sim comes from the extension. Car/track come from the file's position under
/// that sim's setups root (override-aware, same resolution as downloads), read
/// against the sim's own [`Sim::layout`]: `<root>\<car>\file` for iRacing,
/// `<root>\<car>\<track>\file` for ACC, `<root>\<track>\file` for LMU.
/// A file anywhere else simply yields `None`s — the UI asks the user instead.
///
/// The inferred car is resolved to the site's own name where possible (see
/// [`resolve_car`]), so a folder id that abbreviates its car (`mercedesw13`)
/// pre-fills the form with "Mercedes-AMG W13 E Performance" instead of a value
/// the server would reject.
///
/// Fetches the sim's car list to do it, which fails soft to no list at all —
/// offline or unpaired, this behaves exactly as it did before.
pub fn identify(path: &Path, settings: &Settings) -> SetupIdentity {
    // Sim comes off the extension alone, so the list can be fetched before any
    // of the path work — and it's cached per run (see `crate::options`).
    let known_cars = path
        .file_name()
        .and_then(|f| Sim::from_filename(&f.to_string_lossy()))
        .map(|sim| crate::options::options_for(sim).cars)
        .unwrap_or_default();
    identify_with_cars(path, settings, &known_cars)
}

/// [`identify`] with the site's car list supplied by the caller — the pure
/// form, for tests and for any caller that already has the list. An empty
/// `known_cars` means "no list available", not "the site has no cars".
pub fn identify_with_cars(
    path: &Path,
    settings: &Settings,
    known_cars: &[String],
) -> SetupIdentity {
    let filename = path
        .file_name()
        .map(|f| f.to_string_lossy().into_owned())
        .unwrap_or_default();
    let sim = Sim::from_filename(&filename);
    let (car, track) = match sim {
        Some(sim) => locate_in_sim_tree(path, sim, settings),
        None => (None, None),
    };
    let (car, car_source) = match (sim, car) {
        (Some(sim), Some(car)) => {
            let (name, source) = resolve_car(sim, &car, known_cars);
            (Some(name), source)
        }
        (_, car) => (car, CarSource::Folder),
    };
    let garage_export = sim
        .and_then(|sim| find_garage_export(path, sim))
        .map(|p| p.to_string_lossy().into_owned());
    SetupIdentity {
        filename,
        sim,
        car,
        track,
        car_source,
        garage_export,
    }
}

/// The iRacing garage export sitting beside `path`, if there is one.
///
/// `.sto` is binary, so the site can only read an iRacing setup's values from
/// the `.htm` garage export saved alongside it (SERVER_CONTRACT §7b). iRacing
/// writes that export into the same folder under the same name, so a sibling
/// with a matching stem and an `.htm`/`.html` extension is the export for this
/// setup.
///
/// Matching is **case-insensitive on both stem and extension** and goes through
/// `read_dir` rather than probing `<stem>.htm` directly: exports come back from
/// the sim with whatever casing the user typed into the garage, and Windows
/// would resolve a probe case-insensitively while the tests (and any Linux CI
/// run) would not.
///
/// Returns `None` for anything but iRacing, and for an unreadable folder — a
/// missing export is the normal case, never an error, since the upload must
/// still go through without one.
pub fn find_garage_export(path: &Path, sim: Sim) -> Option<PathBuf> {
    if sim != Sim::IRacing {
        return None;
    }
    let stem = path.file_stem()?.to_string_lossy().into_owned();
    let dir = path.parent()?;

    let mut found: Option<PathBuf> = None;
    for entry in std::fs::read_dir(dir).ok()?.flatten() {
        // A *folder* named `quali.htm` would otherwise match and then fail to
        // read at upload time, reporting a failure where there was no export.
        if !entry.file_type().is_ok_and(|t| t.is_file()) {
            continue;
        }
        let candidate = entry.path();
        let matches_stem = candidate
            .file_stem()
            .is_some_and(|s| s.to_string_lossy().eq_ignore_ascii_case(&stem));
        let ext = candidate
            .extension()
            .map(|e| e.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_default();
        if !matches_stem || !matches!(ext.as_str(), "htm" | "html") {
            continue;
        }
        // `.htm` is what iRacing itself writes; prefer it when a hand-saved
        // `.html` sits next to one, and keep the result independent of the
        // order `read_dir` happens to hand entries back in.
        let beats_current = found
            .as_ref()
            .and_then(|f| {
                f.extension()
                    .map(|e| e.to_string_lossy().to_ascii_lowercase())
            })
            .is_none_or(|current| ext == "htm" && current != "htm");
        if found.is_none() || beats_current {
            found = Some(candidate);
        }
    }
    if let Some(export) = &found {
        log::info!(
            "found garage export {:?} beside {stem}.sto",
            export.file_name()
        );
    }
    found
}

/// The site's name for an on-disk car folder id, and how confident we are.
///
/// Order matters. The curated [`car_aliases`] row wins wherever there is one —
/// it is a decision someone made deliberately, and it exists precisely for the
/// ids [`car_match`] gets wrong. It loses only when the list *proves* the site
/// no longer carries that name, which is the rename case the table can't see;
/// then the matcher gets its turn, and a stale alias is still preferred to the
/// bare folder id.
fn resolve_car(sim: Sim, folder: &str, known: &[String]) -> (String, CarSource) {
    let alias = car_aliases::resolve(sim, folder);
    // No list means no evidence of a rename — trust the table, as before.
    if let Some(name) =
        alias.filter(|a| known.is_empty() || car_match::exact_match(a, known).is_some())
    {
        return (name.to_string(), CarSource::Alias);
    }
    // The folder id already names a car the site knows; adopting its spelling
    // is a formality, not a guess.
    if let Some(name) = car_match::exact_match(folder, known) {
        return (name.to_string(), CarSource::Exact);
    }
    if let Some(name) = car_match::best_match(folder, known) {
        log::info!("matched car folder {folder:?} to {name:?} on the site's list");
        return (name.to_string(), CarSource::Matched);
    }
    match alias {
        // Renamed server-side and nothing matched: still closer than the id.
        Some(name) => (name.to_string(), CarSource::Alias),
        None => (folder.to_string(), CarSource::Folder),
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

/// What became of the garage export that rode along with an upload.
///
/// Its own field rather than an error, because attaching the export is
/// **never** allowed to fail the upload: the setup is on the site either way
/// (SERVER_CONTRACT §7b), and the user only needs to know whether the site got
/// the values it needs for the viewer and the diff.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "status", content = "message", rename_all = "snake_case")]
pub enum ExportStatus {
    /// No export was sent — none was found, or the user dropped it. The
    /// ordinary case for ACC and LMU, whose files the site parses directly.
    NotSent,
    /// The site accepted the export; the setup has parsed data.
    Attached,
    /// The upload succeeded and the export did not. `message` is user-facing;
    /// re-uploading through the website is the recovery.
    Failed(String),
}

/// A completed upload: the setup the server created, plus whether its garage
/// export made it across.
#[derive(Debug, Clone)]
pub struct UploadOutcome {
    pub result: UploadResult,
    pub export: ExportStatus,
}

/// Push one setup file to parcferme.cc as this device's linked user.
///
/// `car`/`track` should be the sim's internal folder ids (exactly what
/// [`identify`] reads off disk); the server maps them to its car/track records
/// — see SERVER_CONTRACT §7. `car` runs through [`car_aliases`] one last time
/// here, so a folder id typed by hand is aliased just like an inferred one.
///
/// Deliberately *not* [`car_match`]: a fuzzy match is a guess, and a guess may
/// only be made where the user can see and correct it — [`identify`], which
/// feeds the form. Whatever the form shows at submit time is what ships.
///
/// `types` are the site's setup-type values (see
/// [`crate::api::SetupOptions::setup_types`]); empty means "let the server
/// decide", not "no types". The server validates them, so an unknown value
/// fails the upload rather than being silently dropped.
///
/// `garage_export` is the optional iRacing `.htm` export whose values the site
/// parses into `setupVersions.setupData` — a `.sto` is binary and yields none
/// on its own. It is pushed **after** the setup, in a second request, and a
/// failure there is reported in [`UploadOutcome::export`] rather than raised:
/// an upload that already succeeded must not surface as an error, and a setup
/// with no export must still upload exactly as it did before (issue #3).
#[allow(clippy::too_many_arguments)]
pub fn upload_setup(
    path: &Path,
    sim: Sim,
    car: &str,
    track: Option<&str>,
    name: Option<&str>,
    types: &[String],
    notes: Option<&str>,
    private: bool,
    garage_export: Option<&Path>,
) -> Result<UploadOutcome> {
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

    // Last stop before the wire: a folder id the server can't resolve becomes
    // the site's own car name. A value that isn't a known exception — including
    // one the user typed deliberately — passes through untouched.
    let car = car_aliases::apply(sim, car);

    let token = auth::current_token()?.ok_or(Error::NotLinked)?;

    log::info!(
        "uploading {filename} ({} bytes, sim {}, car {car:?})",
        bytes.len(),
        sim.id()
    );
    let client = ApiClient::from_env();
    let result = client.upload_setup(
        token.as_str(),
        &UploadMeta {
            filename: &filename,
            sim,
            car,
            track,
            name,
            types,
            notes,
            private,
        },
        &bytes,
    )?;
    log::info!("uploaded {filename} as setup {}", result.id);

    let export = match garage_export {
        Some(export) => attach_garage_export(&client, token.as_str(), &result.id, export),
        None => ExportStatus::NotSent,
    };
    Ok(UploadOutcome { result, export })
}

/// Push the garage export for a setup that is already on the site.
///
/// Every failure — unreadable file, oversized, server refusal, an older server
/// with no §7b route at all — comes back as [`ExportStatus::Failed`] carrying a
/// message the user can act on. Nothing here may return `Err`: the setup it
/// belongs to has already uploaded.
fn attach_garage_export(
    client: &ApiClient,
    token: &str,
    setup_id: &str,
    export: &Path,
) -> ExportStatus {
    let filename = match export.file_name() {
        Some(f) => f.to_string_lossy().into_owned(),
        None => return ExportStatus::Failed(format!("not a file: {}", export.display())),
    };
    // Size-check *before* reading, exactly like the §7 path (upload_setup):
    // a huge file is rejected on its metadata alone instead of being read
    // fully into memory just to be thrown away.
    let size = match std::fs::metadata(export) {
        Ok(m) => m.len(),
        Err(e) => return ExportStatus::Failed(format!("could not read {filename}: {e}")),
    };
    if size > MAX_UPLOAD_BYTES {
        return ExportStatus::Failed(format!(
            "{filename} is {size} bytes — too large for a garage export (limit {MAX_UPLOAD_BYTES})"
        ));
    }
    let bytes = match std::fs::read(export) {
        Ok(bytes) => bytes,
        Err(e) => return ExportStatus::Failed(format!("could not read {filename}: {e}")),
    };
    match client.upload_garage_export(token, setup_id, &filename, &bytes) {
        Ok(()) => {
            log::info!("attached garage export {filename} to setup {setup_id}");
            ExportStatus::Attached
        }
        Err(e) => {
            // Warn, don't error: the setup is on the site, only its parsed
            // values are missing, and the message says how to recover.
            log::warn!("garage export {filename} not attached to setup {setup_id}: {e}");
            ExportStatus::Failed(e.to_string())
        }
    }
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

    fn cars(names: &[&str]) -> Vec<String> {
        names.iter().map(|n| (*n).to_string()).collect()
    }

    /// [`identify`] as it behaves with no car list — offline, unpaired, or
    /// before the fetch lands. Layout inference is the same either way, and
    /// these tests are about the layout, not the matcher.
    fn identify_offline(path: &Path, settings: &Settings) -> SetupIdentity {
        identify_with_cars(path, settings, &[])
    }

    #[test]
    fn identify_reads_car_and_track_from_sim_layout() {
        let root = scratch("layout");
        let settings = scratch_settings(&root);

        // ACC: <root>\<car>\<track>\file.json → sim + car + track.
        let acc = identify_offline(
            &root.join("ferrari_488_gt3_evo").join("spa").join("q.json"),
            &settings,
        );
        assert_eq!(acc.sim, Some(Sim::Acc));
        assert_eq!(acc.car.as_deref(), Some("ferrari_488_gt3_evo"));
        assert_eq!(acc.track.as_deref(), Some("spa"));

        // iRacing: <root>\<car>\file.sto → car, never a track.
        let ir = identify_offline(&root.join("ferrari296gt3").join("r.sto"), &settings);
        assert_eq!(ir.sim, Some(Sim::IRacing));
        assert_eq!(ir.car.as_deref(), Some("ferrari296gt3"));
        assert_eq!(ir.track, None);

        // LMU: <root>\<track>\file.svm → track, never a car. Reading the folder
        // as a car is the 2026-07-25 upload bug.
        let lmu = identify_offline(&root.join("Fuji").join("GT3_Balanced.svm"), &settings);
        assert_eq!(lmu.sim, Some(Sim::Lmu));
        assert_eq!(lmu.track.as_deref(), Some("Fuji"));
        assert_eq!(lmu.car, None);

        // An abbreviated iRacing folder id pre-fills the site's car name, not
        // the folder id the server would 422 on.
        let aliased = identify_offline(&root.join("mercedesw13").join("q.sto"), &settings);
        assert_eq!(
            aliased.car.as_deref(),
            Some("Mercedes-AMG W13 E Performance")
        );

        // iRacing file nested deeper still reads the first component as the car
        // (people keep subfolders per season) and no track.
        let deep = identify_offline(
            &root.join("ferrari296gt3").join("2026s3").join("r.sto"),
            &settings,
        );
        assert_eq!(deep.car.as_deref(), Some("ferrari296gt3"));
        assert_eq!(deep.track, None);
    }

    #[test]
    fn identify_resolves_the_car_against_the_sites_list() {
        let root = scratch("known-cars");
        let settings = scratch_settings(&root);
        let sto = |car: &str| root.join(car).join("q.sto");

        // The point of the whole change: a car the site seeded after this
        // binary shipped. No alias row exists for `mercedesw14` — and none
        // needs to, because the name is right there in the list.
        let known = cars(&["Mercedes-AMG W14 E Performance", "Ferrari 296 GT3"]);
        let id = identify_with_cars(&sto("mercedesw14"), &settings, &known);
        assert_eq!(id.car.as_deref(), Some("Mercedes-AMG W14 E Performance"));
        assert_eq!(id.car_source, CarSource::Matched);

        // A folder id that already normalize-matches gets the site's spelling,
        // and isn't reported as a guess — the server would have resolved it.
        let id = identify_with_cars(&sto("ferrari296gt3"), &settings, &known);
        assert_eq!(id.car.as_deref(), Some("Ferrari 296 GT3"));
        assert_eq!(id.car_source, CarSource::Exact);

        // Nothing plausible on the list: the folder id stands, exactly as
        // before, and the server's 422 remains the user's guide.
        let id = identify_with_cars(&sto("somenewcar2027"), &settings, &known);
        assert_eq!(id.car.as_deref(), Some("somenewcar2027"));
        assert_eq!(id.car_source, CarSource::Folder);
    }

    #[test]
    fn curated_aliases_outrank_the_matcher() {
        let root = scratch("alias-wins");
        let settings = scratch_settings(&root);
        let path = root.join("porsche911cup").join("q.sto");

        // Both names are on the site and the textually closer one is the wrong
        // car — `porsche911cup` is the 991 GT3 Cup. This is precisely the kind
        // of row the curated table still earns its place with…
        let known = cars(&["Porsche 911 GT3 Cup (991)", "Porsche 911 Cup (992.2)"]);
        assert_eq!(
            car_match::best_match("porsche911cup", &known),
            Some("Porsche 911 Cup (992.2)"),
            "matcher would take the wrong one — hence the alias row"
        );
        let id = identify_with_cars(&path, &settings, &known);
        assert_eq!(id.car.as_deref(), Some("Porsche 911 GT3 Cup (991)"));
        assert_eq!(id.car_source, CarSource::Alias);
    }

    #[test]
    fn a_renamed_car_falls_back_to_the_matcher() {
        let root = scratch("renamed");
        let settings = scratch_settings(&root);
        let path = root.join("mercedesw13").join("q.sto");

        // The site renamed the car out from under the alias row. Left alone
        // the alias would now *cause* the 422 it was added to prevent, so the
        // matcher gets its turn and finds the new name.
        let renamed = cars(&["Mercedes-AMG W13 E Performance (2023)"]);
        let id = identify_with_cars(&path, &settings, &renamed);
        assert_eq!(
            id.car.as_deref(),
            Some("Mercedes-AMG W13 E Performance (2023)")
        );
        assert_eq!(id.car_source, CarSource::Matched);

        // Renamed beyond recognition: the alias is stale but still a better
        // guess than the folder id, and the user can see and fix it.
        let gone = cars(&["Ferrari 296 GT3"]);
        let id = identify_with_cars(&path, &settings, &gone);
        assert_eq!(id.car.as_deref(), Some("Mercedes-AMG W13 E Performance"));
        assert_eq!(id.car_source, CarSource::Alias);

        // No list at all (offline, unpaired) is not evidence of a rename.
        let id = identify_with_cars(&path, &settings, &[]);
        assert_eq!(id.car.as_deref(), Some("Mercedes-AMG W13 E Performance"));
        assert_eq!(id.car_source, CarSource::Alias);
    }

    #[test]
    fn identify_degrades_to_none_outside_the_sim_tree() {
        let root = scratch("outside");
        let settings = scratch_settings(&root);

        // Recognized format but foreign location: sim yes, car/track no.
        let desktop = identify_offline(Path::new(r"C:\Users\x\Desktop\q.json"), &settings);
        assert_eq!(desktop.sim, Some(Sim::Acc));
        assert_eq!(desktop.car, None);
        assert_eq!(desktop.track, None);

        // File directly in the setups root: no car folder to read.
        let bare = identify_offline(&root.join("loose.sto"), &settings);
        assert_eq!(bare.sim, Some(Sim::IRacing));
        assert_eq!(bare.car, None);

        // Unrecognized extension: nothing inferred, filename still reported.
        let zip = identify_offline(&root.join("car").join("pack.zip"), &settings);
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
    fn identify_finds_the_garage_export_beside_an_iracing_setup() {
        let root = scratch("export");
        let car = root.join("ferrari296gt3");
        std::fs::create_dir_all(&car).unwrap();
        let settings = scratch_settings(&root);

        let sto = car.join("quali.sto");
        std::fs::write(&sto, b"\0binary").unwrap();

        // No export yet: the field is empty and the upload is unaffected.
        assert_eq!(identify_offline(&sto, &settings).garage_export, None);

        // iRacing writes the export next to the setup under the same name.
        let htm = car.join("quali.htm");
        std::fs::write(&htm, b"<html>").unwrap();
        assert_eq!(
            identify_offline(&sto, &settings).garage_export,
            Some(htm.to_string_lossy().into_owned())
        );

        // A same-stem file that isn't an export is not one…
        std::fs::write(car.join("quali.txt"), b"notes").unwrap();
        // …and neither is an export belonging to a different setup.
        std::fs::write(car.join("race.htm"), b"<html>").unwrap();
        assert_eq!(
            identify_offline(&sto, &settings).garage_export,
            Some(htm.to_string_lossy().into_owned())
        );

        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn garage_export_matching_ignores_case_and_prefers_htm() {
        let dir = scratch("export-case");
        std::fs::create_dir_all(&dir).unwrap();
        let sto = dir.join("Quali_Spa.sto");
        std::fs::write(&sto, b"\0binary").unwrap();

        // The garage names the export; the file picker names the setup. The
        // two casings need not agree, and on Windows they routinely don't.
        let html = dir.join("quali_spa.HTML");
        std::fs::write(&html, b"<html>").unwrap();
        assert_eq!(find_garage_export(&sto, Sim::IRacing), Some(html.clone()));

        // `.htm` is what iRacing itself writes, so it wins over a hand-saved
        // `.html` regardless of the order read_dir returns them in.
        let htm = dir.join("QUALI_SPA.htm");
        std::fs::write(&htm, b"<html>").unwrap();
        assert_eq!(find_garage_export(&sto, Sim::IRacing), Some(htm));

        // Only iRacing has this problem — ACC and LMU files parse server-side.
        assert_eq!(find_garage_export(&sto, Sim::Acc), None);
        assert_eq!(find_garage_export(&sto, Sim::Lmu), None);

        // A folder that isn't there is a missing export, not an error.
        assert_eq!(
            find_garage_export(&scratch("gone").join("q.sto"), Sim::IRacing),
            None
        );

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn upload_rejects_oversized_and_missing_files() {
        let dir = scratch("reject");
        std::fs::create_dir_all(&dir).unwrap();

        // Missing file surfaces as Io, before any network call.
        let missing = upload_setup(
            &dir.join("nope.sto"),
            Sim::IRacing,
            "car",
            None,
            None,
            &[],
            None,
            false,
            None,
        );
        assert!(matches!(missing, Err(Error::Io(_))));

        // Oversized file is refused client-side with a clear Api error.
        let big = dir.join("big.sto");
        std::fs::write(&big, vec![0u8; (MAX_UPLOAD_BYTES + 1) as usize]).unwrap();
        let too_big = upload_setup(
            &big,
            Sim::IRacing,
            "car",
            None,
            None,
            &[],
            None,
            false,
            None,
        );
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
            let err = upload_setup(
                &file,
                Sim::Lmu,
                "ferrari_499p",
                track,
                None,
                &[],
                None,
                false,
                None,
            );
            assert!(matches!(err, Err(Error::Api(m)) if m.contains("track")));
        }

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn oversized_garage_export_is_rejected_before_any_network_call() {
        let dir = scratch("big-export");
        std::fs::create_dir_all(&dir).unwrap();

        let big = dir.join("big.htm");
        std::fs::write(&big, vec![0u8; (MAX_UPLOAD_BYTES + 1) as usize]).unwrap();

        // A client aimed at a non-routable port proves the size cap fires on the
        // file's metadata alone — read fully into memory or sent over the wire,
        // this would hang/fail as a transport error instead of the clean
        // "too large" refusal.
        let client = ApiClient::new("http://127.0.0.1:1/");
        let status = attach_garage_export(&client, "token", "setup-1", &big);
        assert!(matches!(
            status,
            ExportStatus::Failed(m)
                if m.contains("too large")
                    && m.contains(&format!("limit {MAX_UPLOAD_BYTES}"))
        ));

        std::fs::remove_dir_all(&dir).unwrap();
    }
}
