//! Fetch a presigned URL and write it into the sim setup directory. **[M2]**
//!
//! Security-critical: the server-supplied filename is **sanitized** so it can
//! never escape the `setups\` directory (path traversal), and the write is
//! **atomic** (temp file + rename) so a half-downloaded file can't be loaded
//! in-sim. See Build Plan §6.

use std::path::{Path, PathBuf};

use crate::{Error, Result};

/// Download `url` and write it atomically into `setups_dir`, returning the
/// final path.
///
/// `filename` is the server-supplied name; it is sanitized to a single path
/// component before being joined to `setups_dir`.
pub fn download_into(_url: &str, _setups_dir: &Path, _filename: &str) -> Result<PathBuf> {
    Err(Error::NotImplemented("download::download_into"))
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
}
