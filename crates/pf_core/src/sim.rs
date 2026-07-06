//! The sims ParcFerme can install setups for. **[M3: multi-sim]**
//!
//! Pull is sim-agnostic at the byte level, but each sim keeps its setups in a
//! different place and lays a single setup file out differently underneath it
//! (Build Plan §3 + risk table). This module is the single source of truth for
//! *where* a sim's setups live and *how* one setup is placed. iRacing shipped
//! first (M2); ACC and LMU are added here. To support another sim, add a variant
//! and fill in [`Sim::setups_root`] + [`Sim::needs_track_subfolder`].

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// A racing sim ParcFerme can install setups into.
///
/// Serializes to a short lowercase id (`"iracing"`, `"acc"`, `"lmu"`) — that is
/// both the wire form in the download API and the IPC override-map key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Sim {
    /// iRacing — `Documents\iRacing\setups\<car>\*.sto`.
    #[default]
    IRacing,
    /// Assetto Corsa Competizione — setups under `…\Setups\<car>\<track>\*.json`.
    /// The `<track>` subfolder is **required**; without it ACC won't list the
    /// setup in-game.
    Acc,
    /// Le Mans Ultimate — `…\UserData\player\Settings\<car>\*.svm` (rFactor 2
    /// heritage). Path unverified against a live install — see [`Sim::setups_root`].
    Lmu,
}

impl Sim {
    /// Every sim, for folder detection and enumeration in the UI.
    pub const ALL: [Sim; 3] = [Sim::IRacing, Sim::Acc, Sim::Lmu];

    /// Stable short id used on the wire and as the IPC override-map key. Kept in
    /// lockstep with the `serde(rename_all = "lowercase")` representation.
    pub fn id(self) -> &'static str {
        match self {
            Sim::IRacing => "iracing",
            Sim::Acc => "acc",
            Sim::Lmu => "lmu",
        }
    }

    /// Parse the short id back into a [`Sim`]; `None` for an unknown id.
    /// Tolerant of casing and surrounding whitespace — this also parses the
    /// server's `sim` tag, and a cosmetic server-side change ("ACC", "iRacing")
    /// must not reroute files.
    pub fn from_id(id: &str) -> Option<Sim> {
        let id = id.trim().to_ascii_lowercase();
        Sim::ALL.into_iter().find(|s| s.id() == id)
    }

    /// Infer the sim from a setup file's extension. Each sim has a distinct
    /// format — `.sto` iRacing, `.json` ACC, `.svm` LMU (rFactor 2 heritage) —
    /// so the extension is ground truth for *which game can load the file*,
    /// independent of how the setup is tagged server-side.
    pub fn from_extension(ext: &str) -> Option<Sim> {
        match ext.trim_start_matches('.').to_ascii_lowercase().as_str() {
            "sto" => Some(Sim::IRacing),
            "json" => Some(Sim::Acc),
            "svm" => Some(Sim::Lmu),
            _ => None,
        }
    }

    /// [`Sim::from_extension`] applied to a full filename.
    pub fn from_filename(filename: &str) -> Option<Sim> {
        let ext = Path::new(filename).extension()?.to_str()?;
        Sim::from_extension(ext)
    }

    /// Human-facing name for toasts and the folder list.
    pub fn display_name(self) -> &'static str {
        match self {
            Sim::IRacing => "iRacing",
            Sim::Acc => "Assetto Corsa Competizione",
            Sim::Lmu => "Le Mans Ultimate",
        }
    }

    /// The setups root for this sim under the user's `documents` directory. The
    /// per-setup `<car>[\<track>]` subfolders are added by
    /// [`crate::paths::setup_target_dir`], not here.
    ///
    /// **LMU caveat:** the rFactor 2 layout (`UserData\player\Settings`) is our
    /// best understanding but has not been confirmed against a live Le Mans
    /// Ultimate install. Verify before relying on the LMU flow; iRacing and ACC
    /// are the confirmed targets.
    pub fn setups_root(self, documents: &Path) -> PathBuf {
        match self {
            Sim::IRacing => documents.join("iRacing").join("setups"),
            Sim::Acc => documents.join("Assetto Corsa Competizione").join("Setups"),
            Sim::Lmu => documents
                .join("Le Mans Ultimate")
                .join("UserData")
                .join("player")
                .join("Settings"),
        }
    }

    /// Whether a setup needs a `<track>` subfolder under its `<car>` folder to be
    /// visible in-game. Only ACC does.
    pub fn needs_track_subfolder(self) -> bool {
        matches!(self, Sim::Acc)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn id_round_trips_and_matches_all() {
        for sim in Sim::ALL {
            assert_eq!(Sim::from_id(sim.id()), Some(sim));
        }
        assert_eq!(Sim::from_id("nope"), None);
        // Tolerant of server-side cosmetic variations.
        assert_eq!(Sim::from_id(" ACC "), Some(Sim::Acc));
        assert_eq!(Sim::from_id("iRacing"), Some(Sim::IRacing));
    }

    #[test]
    fn extension_maps_to_the_sim_that_loads_it() {
        assert_eq!(Sim::from_extension("sto"), Some(Sim::IRacing));
        assert_eq!(Sim::from_extension(".JSON"), Some(Sim::Acc));
        assert_eq!(Sim::from_extension("svm"), Some(Sim::Lmu));
        assert_eq!(Sim::from_extension("zip"), None);

        assert_eq!(Sim::from_filename("quali_spa.json"), Some(Sim::Acc));
        assert_eq!(Sim::from_filename("baseline.sto"), Some(Sim::IRacing));
        assert_eq!(Sim::from_filename("no_extension"), None);
    }

    #[test]
    fn id_matches_serde_wire_form() {
        // The IPC key (`id`) and the JSON wire form must stay identical, else the
        // override map and the download payload would disagree on a sim's name.
        for sim in Sim::ALL {
            let json = serde_json::to_string(&sim).unwrap();
            assert_eq!(json, format!("\"{}\"", sim.id()));
            assert_eq!(serde_json::from_str::<Sim>(&json).unwrap(), sim);
        }
    }

    #[test]
    fn setups_root_is_sim_specific_and_under_documents() {
        let docs = Path::new("/home/u/Documents");
        let iracing = Sim::IRacing.setups_root(docs);
        assert!(iracing.ends_with("setups"));
        assert!(iracing.starts_with(docs));

        let acc = Sim::Acc.setups_root(docs);
        assert!(acc.ends_with("Setups"));
        assert!(acc.to_string_lossy().contains("Assetto Corsa Competizione"));

        let lmu = Sim::Lmu.setups_root(docs);
        assert!(lmu.ends_with("Settings"));
        assert!(lmu.to_string_lossy().contains("Le Mans Ultimate"));
    }

    #[test]
    fn only_acc_needs_a_track_subfolder() {
        assert!(Sim::Acc.needs_track_subfolder());
        assert!(!Sim::IRacing.needs_track_subfolder());
        assert!(!Sim::Lmu.needs_track_subfolder());
    }

    #[test]
    fn default_is_iracing() {
        // Backward-compat: a download payload with no `sim` field deserializes to
        // iRacing, matching the single-sim M2 server.
        assert_eq!(Sim::default(), Sim::IRacing);
    }
}
