//! Fetch a presigned URL and write it into the sim setup directory. **[M2]**
//!
//! Security-critical: the server-supplied filename is **sanitized** so it can
//! never escape the `setups\` directory (path traversal), and the write is
//! **atomic** (temp file + rename) so a half-downloaded file can't be loaded
//! in-sim. See Build Plan §6.

use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};

use crate::api::ApiClient;
use crate::sim::Sim;
use crate::{auth, paths, Error, Result};

/// A setup file successfully written into a sim folder. Drives the UI toast.
#[derive(Debug, Clone)]
pub struct InstalledSetup {
    /// Absolute path of the written setup file.
    pub path: PathBuf,
    /// Which sim it was installed for.
    pub sim: Sim,
    /// Car the setup is for (its `<car>\` subfolder).
    pub car: String,
    /// Track subfolder, for sims that nest by track (ACC).
    pub track: Option<String>,
    /// Setup display name, if the server provided one.
    pub name: Option<String>,
}

/// The manual-pull path end to end: authorize this device, ask the server to
/// presign the setup identified by `setup_uuid`, then atomically write the bytes
/// into the destination sim's folder (`<car>` or `<car>\<track>`).
///
/// The sim comes from the server response, so `overrides` is a per-sim map of
/// Settings folder overrides; the entry for the resolved sim (if any) wins over
/// detection.
pub fn download_setup(
    setup_uuid: &str,
    overrides: &HashMap<Sim, PathBuf>,
) -> Result<InstalledSetup> {
    let token = auth::current_token()?
        .ok_or_else(|| Error::Api("not connected — link this device first".into()))?;

    let info = ApiClient::from_env().get_download(setup_uuid, token.as_str())?;

    let setups_dir = paths::resolve_setups_dir(info.sim, overrides.get(&info.sim).cloned())?;
    let target_dir =
        paths::setup_target_dir(&setups_dir, info.sim, &info.car, info.track.as_deref());
    let path = download_into(&info.url, &target_dir, &info.filename)?;

    Ok(InstalledSetup {
        path,
        sim: info.sim,
        car: info.car,
        track: info.track,
        name: info.name,
    })
}

/// Parse a `parcferme://equip?…` deep link and run the full install for the
/// setup it names (the M3 handshake). The link only *names* a setup; the download
/// is authorized exactly like a manual pull — this device's stored token plus the
/// server's access check — so an unlinked device or an inaccessible setup fails
/// cleanly. `overrides` are the per-sim folder overrides (empty for the deep-link
/// path until persisted Settings land in M4).
pub fn install_from_equip_link(
    url: &str,
    overrides: &HashMap<Sim, PathBuf>,
) -> Result<InstalledSetup> {
    let req = crate::deeplink::parse(url)?;
    download_setup(&req.setup_id, overrides)
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

/// Download `url` and write it atomically into `dest_dir`, returning the final
/// path.
///
/// `filename` is the server-supplied name; it is sanitized to a single path
/// component before being joined to `dest_dir`. The fetch streams straight to a
/// temp file that is then renamed into place, so iRacing never sees a partial
/// `.sto`.
pub fn download_into(url: &str, dest_dir: &Path, filename: &str) -> Result<PathBuf> {
    let resp = ureq::get(url)
        .call()
        .map_err(|e| Error::Http(e.to_string()))?;
    write_atomic(&mut resp.into_reader(), dest_dir, filename)
}

/// Stream `reader` into `dest_dir/<sanitized filename>` atomically: write to a
/// sibling temp file on the same volume, flush, then rename over the target.
///
/// Split out from the HTTP fetch so the path-safety and atomicity guarantees are
/// unit-testable without a network.
fn write_atomic(reader: &mut impl Read, dest_dir: &Path, filename: &str) -> Result<PathBuf> {
    let safe = sanitize_filename(filename);
    if safe.is_empty() {
        return Err(Error::Api(format!(
            "server supplied an unusable filename: {filename:?}"
        )));
    }

    std::fs::create_dir_all(dest_dir)?;
    let final_path = dest_dir.join(&safe);
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

    // Best-effort: clean up the temp file if the rename fails midway.
    if let Err(e) = std::fs::rename(&tmp_path, &final_path) {
        let _ = std::fs::remove_file(&tmp_path);
        return Err(Error::Io(e));
    }
    Ok(final_path)
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

    #[test]
    fn sanitize_strips_path_traversal() {
        assert_eq!(sanitize_filename("../../evil.sto"), "evil.sto");
        assert_eq!(sanitize_filename(r"C:\Windows\system32\x.sto"), "x.sto");
        assert_eq!(sanitize_filename("setup.sto"), "setup.sto");
    }

    #[test]
    fn write_atomic_lands_sanitized_file_with_contents() {
        // Unique scratch dir under the system temp dir; cleaned up at the end.
        let dir = std::env::temp_dir().join(format!("pf-dl-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);

        // A traversal-laden name must collapse to a single component inside dir.
        let mut bytes = &b"[setup]\nrearwing=3\n"[..];
        let path = write_atomic(&mut bytes, &dir, "../../pwned.sto").unwrap();

        assert_eq!(path, dir.join("pwned.sto"));
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
            write_atomic(&mut bytes, &dir, "///"),
            Err(Error::Api(_))
        ));
    }
}
