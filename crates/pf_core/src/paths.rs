//! Locate sim setup directories. iRacing first; ACC/LMU later. **[M2]**
//!
//! Pull is sim-agnostic at the byte level, but folder layout differs per sim,
//! so this module is designed to extend. iRacing folders can also be in
//! non-default / OneDrive-redirected / multi-drive locations, so M2 adds a
//! Settings override on top of this best-effort detection.

use std::path::PathBuf;

use crate::{Error, Result};

/// Best-effort location of the user's iRacing setups directory:
/// `…/Documents/iRacing/setups/`.
///
/// This is the default; M2 layers a user override + validation on top for the
/// non-default-folder cases called out in the Build Plan's risk table.
pub fn iracing_setups_dir() -> Result<PathBuf> {
    let documents = documents_dir().ok_or(Error::NotImplemented(
        "paths::documents_dir (unsupported platform)",
    ))?;
    Ok(documents.join("iRacing").join("setups"))
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
}
