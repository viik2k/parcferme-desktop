//! Fetch a presigned URL and write it into the sim setup directory. **[M2 · M4]**
//!
//! Security-critical: the server-supplied filename is **sanitized** so it can
//! never escape the `setups\` directory (path traversal), and the write is
//! **atomic** (temp file + rename) so a half-downloaded file can't be loaded
//! in-sim. See Build Plan §6.
//!
//! M4 adds the conflict policy: an existing same-named file is never silently
//! clobbered. Byte-identical content short-circuits to "already installed"
//! (re-equipping is idempotent); different content is either replaced or kept
//! alongside as `name (2).ext`, per [`ConflictPolicy`].

use std::io::Read;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::api::ApiClient;
use crate::settings::{ConflictPolicy, Settings};
use crate::sim::Sim;
use crate::{auth, paths, Error, Result};

/// How a downloaded file landed on disk relative to what was already there.
/// Serialized snake_case across IPC so the UI can word the toast precisely.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InstallAction {
    /// No file with that name existed; written fresh.
    Installed,
    /// A different file existed and was atomically replaced
    /// ([`ConflictPolicy::Overwrite`]).
    Replaced,
    /// A different file existed; this one was written under a numbered sibling
    /// name ([`ConflictPolicy::KeepBoth`]) — the path holds the actual name.
    KeptBoth,
    /// A byte-identical copy was already on disk; nothing was written.
    AlreadyInstalled,
}

/// A setup file successfully written into a sim folder. Drives the UI toast.
#[derive(Debug, Clone)]
pub struct InstalledSetup {
    /// Absolute path of the setup file on disk (for [`InstallAction::KeptBoth`]
    /// this is the numbered sibling; for
    /// [`InstallAction::AlreadyInstalled`] the pre-existing identical file).
    pub path: PathBuf,
    /// How the file landed relative to what was already there.
    pub action: InstallAction,
    /// Which sim it was installed for.
    pub sim: Sim,
    /// Car the setup is for (its `<car>\` subfolder).
    pub car: String,
    /// Track subfolder, for sims that nest by track (ACC).
    pub track: Option<String>,
    /// Setup display name, if the server provided one.
    pub name: Option<String>,
}

/// The pull path end to end: authorize this device, ask the server to presign
/// the setup identified by `setup_uuid`, then atomically write the bytes into
/// the destination sim's folder (`<car>` or `<car>\<track>`).
///
/// `settings` supplies both the per-sim folder overrides and the conflict
/// policy — manual pulls and equip deep links read the same persisted file, so
/// the two paths can't drift (the M3 detection-only gap is closed).
pub fn download_setup(setup_uuid: &str, settings: &Settings) -> Result<InstalledSetup> {
    let token = auth::current_token()?.ok_or(Error::NotLinked)?;

    log::info!("download requested for setup {setup_uuid}");
    let info = ApiClient::from_env().get_download(setup_uuid, token.as_str())?;
    // The one line that answers "why did it land there?" in a support log.
    // Everything but the presigned URL, which stays out of logs.
    log::debug!(
        "server response: filename={:?} sim_tag={:?} car={:?} track={:?} name={:?}",
        info.filename,
        info.sim,
        info.car,
        info.track,
        info.name
    );
    let sim = info.resolved_sim();

    let setups_dir = paths::resolve_setups_dir(sim, settings.sim_folders.get(&sim).cloned())?;
    let target_dir = paths::setup_target_dir(&setups_dir, sim, &info.car, info.track.as_deref());
    let (path, action) = download_into(
        &info.url,
        &target_dir,
        &info.filename,
        settings.conflict_policy,
    )?;
    log::info!(
        "setup {setup_uuid} → {} ({:?}, {})",
        path.display(),
        action,
        sim.id()
    );

    Ok(InstalledSetup {
        path,
        action,
        sim,
        car: info.car,
        track: info.track,
        name: info.name,
    })
}

/// Parse a `parcferme://equip?…` deep link and run the full install for the
/// setup it names (the M3 handshake). The link only *names* a setup; the download
/// is authorized exactly like a manual pull — this device's stored token plus the
/// server's access check — so an unlinked device or an inaccessible setup fails
/// cleanly.
pub fn install_from_equip_link(url: &str, settings: &Settings) -> Result<InstalledSetup> {
    let req = crate::deeplink::parse(url)?;
    download_setup(&req.setup_id, settings)
}

/// Extract a setup UUID from a pasted parcferme.cc setup URL, or accept a bare
/// UUID. Returns `None` if neither is present so the UI can show a clear hint
/// rather than firing a doomed request.
pub fn extract_setup_uuid(input: &str) -> Option<String> {
    let trimmed = input.trim();
    let candidate = match trimmed.find("/setups/") {
        Some(idx) => trimmed[idx + "/setups/".len()..]
            .split(['/', '?', '#'])
            .next()
            .unwrap_or(""),
        None => trimmed,
    };
    is_uuid(candidate).then(|| candidate.to_string())
}

/// Loose RFC-4122 shape check (`8-4-4-4-12` hex). The server is the real
/// authority; this just rejects obvious non-UUIDs before a round-trip.
fn is_uuid(s: &str) -> bool {
    let bytes = s.as_bytes();
    bytes.len() == 36
        && bytes.iter().enumerate().all(|(i, &b)| match i {
            8 | 13 | 18 | 23 => b == b'-',
            _ => b.is_ascii_hexdigit(),
        })
}

/// Download `url` and write it atomically into `dest_dir` under `policy`,
/// returning the final path and how it landed.
///
/// `filename` is the server-supplied name; it is sanitized to a single path
/// component before being joined to `dest_dir`. The fetch streams straight to a
/// temp file that is then placed per the conflict policy, so the sim never sees
/// a partial file.
pub fn download_into(
    url: &str,
    dest_dir: &Path,
    filename: &str,
    policy: ConflictPolicy,
) -> Result<(PathBuf, InstallAction)> {
    let resp = ureq::get(url)
        .call()
        .map_err(|e| Error::Http(e.to_string()))?;
    write_atomic(&mut resp.into_reader(), dest_dir, filename, policy)
}

/// Stream `reader` into `dest_dir/<sanitized filename>` atomically: write to a
/// sibling temp file on the same volume, flush, then place it per `policy`.
///
/// Split out from the HTTP fetch so the path-safety, atomicity, and conflict
/// guarantees are unit-testable without a network.
fn write_atomic(
    reader: &mut impl Read,
    dest_dir: &Path,
    filename: &str,
    policy: ConflictPolicy,
) -> Result<(PathBuf, InstallAction)> {
    let safe = sanitize_filename(filename);
    if safe.is_empty() {
        return Err(Error::Api(format!(
            "server supplied an unusable filename: {filename:?}"
        )));
    }

    std::fs::create_dir_all(dest_dir)?;
    // Temp sibling in the same directory guarantees the rename is atomic (same
    // filesystem) and the partial file lands next to its target, not in /tmp.
    let tmp_path = dest_dir.join(format!(".{safe}.part"));

    // Scope the file handle so it's closed before the rename (required on
    // Windows, which won't rename an open file).
    {
        let mut file = std::fs::File::create(&tmp_path)?;
        std::io::copy(reader, &mut file)?;
        file.sync_all()?;
    }

    // Best-effort: never leave the temp file behind, whatever went wrong.
    let placed = place_download(dest_dir, &safe, &tmp_path, policy);
    if placed.is_err() {
        let _ = std::fs::remove_file(&tmp_path);
    }
    placed
}

/// Move the fully-written temp file to its final name per the conflict policy.
///
/// Identical bytes short-circuit to [`InstallAction::AlreadyInstalled`] under
/// *any* policy — under [`ConflictPolicy::KeepBoth`] the numbered siblings are
/// checked too, so re-equipping an updated setup twice reuses its `(2)` copy
/// instead of minting a `(3)`.
fn place_download(
    dest_dir: &Path,
    name: &str,
    tmp: &Path,
    policy: ConflictPolicy,
) -> Result<(PathBuf, InstallAction)> {
    let target = dest_dir.join(name);
    if !target.exists() {
        std::fs::rename(tmp, &target)?;
        return Ok((target, InstallAction::Installed));
    }
    if files_identical(tmp, &target)? {
        std::fs::remove_file(tmp)?;
        return Ok((target, InstallAction::AlreadyInstalled));
    }

    match policy {
        ConflictPolicy::Overwrite => {
            std::fs::rename(tmp, &target)?;
            Ok((target, InstallAction::Replaced))
        }
        ConflictPolicy::KeepBoth => {
            // Walk `name (2)`, `name (3)`, … dedupe against each existing
            // sibling; land in the first free slot. Bounded so a pathological
            // folder errors instead of spinning.
            for n in 2..=999u32 {
                let candidate = dest_dir.join(numbered_name(name, n));
                if !candidate.exists() {
                    std::fs::rename(tmp, &candidate)?;
                    return Ok((candidate, InstallAction::KeptBoth));
                }
                if files_identical(tmp, &candidate)? {
                    std::fs::remove_file(tmp)?;
                    return Ok((candidate, InstallAction::AlreadyInstalled));
                }
            }
            Err(Error::Api(format!(
                "too many copies of {name:?} in {} — clean up the folder or switch the conflict policy to overwrite",
                dest_dir.display()
            )))
        }
    }
}

/// `quali.sto` → `quali (2).sto`; extensionless names get the suffix at the end.
fn numbered_name(name: &str, n: u32) -> String {
    match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => format!("{stem} ({n}).{ext}"),
        _ => format!("{name} ({n})"),
    }
}

/// Whether two files hold identical bytes. Setup files are a few KB, so a
/// length check followed by a full read is simpler than chunked comparison and
/// just as safe.
fn files_identical(a: &Path, b: &Path) -> Result<bool> {
    if std::fs::metadata(a)?.len() != std::fs::metadata(b)?.len() {
        return Ok(false);
    }
    Ok(std::fs::read(a)? == std::fs::read(b)?)
}

/// Reduce a server-supplied name to a single, safe filename component.
///
/// Strips any directory separators and parent (`..`) segments so the result
/// can only ever land directly inside the setups directory.
pub fn sanitize_filename(name: &str) -> String {
    name.rsplit(['/', '\\'])
        .next()
        .unwrap_or("")
        .replace("..", "")
        .trim()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("pf-dl-{name}-{}", std::process::id()))
    }

    fn write(
        dir: &Path,
        bytes: &[u8],
        name: &str,
        policy: ConflictPolicy,
    ) -> (PathBuf, InstallAction) {
        let mut reader = bytes;
        write_atomic(&mut reader, dir, name, policy).unwrap()
    }

    #[test]
    fn sanitize_strips_path_traversal() {
        assert_eq!(sanitize_filename("../../evil.sto"), "evil.sto");
        assert_eq!(sanitize_filename(r"C:\Windows\system32\x.sto"), "x.sto");
        assert_eq!(sanitize_filename("setup.sto"), "setup.sto");
    }

    #[test]
    fn write_atomic_lands_sanitized_file_with_contents() {
        // Unique scratch dir under the system temp dir; cleaned up at the end.
        let dir = scratch("fresh");
        let _ = std::fs::remove_dir_all(&dir);

        // A traversal-laden name must collapse to a single component inside dir.
        let (path, action) = write(
            &dir,
            b"[setup]\nrearwing=3\n",
            "../../pwned.sto",
            ConflictPolicy::KeepBoth,
        );

        assert_eq!(path, dir.join("pwned.sto"));
        assert_eq!(action, InstallAction::Installed);
        assert!(path.starts_with(&dir), "must not escape the dest dir");
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "[setup]\nrearwing=3\n"
        );
        // No leftover temp artifact.
        assert!(!dir.join(".pwned.sto.part").exists());

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn identical_redownload_is_idempotent_under_any_policy() {
        for policy in [ConflictPolicy::KeepBoth, ConflictPolicy::Overwrite] {
            let dir = scratch(&format!("idem-{policy:?}"));
            let _ = std::fs::remove_dir_all(&dir);

            write(&dir, b"v1", "q.sto", policy);
            let (path, action) = write(&dir, b"v1", "q.sto", policy);

            assert_eq!(action, InstallAction::AlreadyInstalled);
            assert_eq!(path, dir.join("q.sto"));
            // Exactly one file — no `(2)` litter, no temp leftovers.
            assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);

            std::fs::remove_dir_all(&dir).unwrap();
        }
    }

    #[test]
    fn keep_both_numbers_conflicts_and_dedupes_against_siblings() {
        let dir = scratch("keepboth");
        let _ = std::fs::remove_dir_all(&dir);
        let p = ConflictPolicy::KeepBoth;

        write(&dir, b"v1", "q.sto", p);

        // Different bytes, same name → numbered sibling; original untouched.
        let (path, action) = write(&dir, b"v2", "q.sto", p);
        assert_eq!(action, InstallAction::KeptBoth);
        assert_eq!(path, dir.join("q (2).sto"));
        assert_eq!(std::fs::read(dir.join("q.sto")).unwrap(), b"v1");

        // Re-equipping v2 reuses its (2) copy instead of minting a (3).
        let (path, action) = write(&dir, b"v2", "q.sto", p);
        assert_eq!(action, InstallAction::AlreadyInstalled);
        assert_eq!(path, dir.join("q (2).sto"));

        // A third distinct version takes the next slot.
        let (path, action) = write(&dir, b"v3", "q.sto", p);
        assert_eq!(action, InstallAction::KeptBoth);
        assert_eq!(path, dir.join("q (3).sto"));

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn overwrite_replaces_contents_atomically() {
        let dir = scratch("overwrite");
        let _ = std::fs::remove_dir_all(&dir);
        let p = ConflictPolicy::Overwrite;

        write(&dir, b"old bytes", "q.sto", p);
        let (path, action) = write(&dir, b"new", "q.sto", p);

        assert_eq!(action, InstallAction::Replaced);
        assert_eq!(std::fs::read(&path).unwrap(), b"new");
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn numbered_name_handles_extensions() {
        assert_eq!(numbered_name("q.sto", 2), "q (2).sto");
        assert_eq!(numbered_name("no_ext", 3), "no_ext (3)");
        // Leading-dot names don't lose the dot.
        assert_eq!(numbered_name(".hidden", 2), ".hidden (2)");
    }

    #[test]
    fn extract_setup_uuid_from_url_or_bare() {
        let uuid = "3f8a1c2d-1234-4abc-89ef-0123456789ab";
        assert_eq!(
            extract_setup_uuid(&format!("https://www.parcferme.cc/setups/{uuid}")),
            Some(uuid.to_string())
        );
        // Trailing path/query/fragment are stripped.
        assert_eq!(
            extract_setup_uuid(&format!("https://www.parcferme.cc/setups/{uuid}?v=2#notes")),
            Some(uuid.to_string())
        );
        // A bare UUID is accepted.
        assert_eq!(
            extract_setup_uuid(&format!("  {uuid}  ")),
            Some(uuid.to_string())
        );
        // Non-UUIDs are rejected so we never fire a doomed request.
        assert_eq!(extract_setup_uuid("https://www.parcferme.cc/setups/"), None);
        assert_eq!(extract_setup_uuid("not a setup link"), None);
    }

    #[test]
    fn write_atomic_rejects_empty_filename() {
        let dir = std::env::temp_dir();
        let mut bytes = &b"x"[..];
        assert!(matches!(
            write_atomic(&mut bytes, &dir, "///", ConflictPolicy::KeepBoth),
            Err(Error::Api(_))
        ));
    }
}
