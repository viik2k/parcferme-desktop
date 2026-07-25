//! Curated folder-id → Parc Fermé car-name aliases. **[M5: push]**
//!
//! The server resolves an uploaded setup's `car` by *normalized equality*: both
//! sides are case-folded and stripped to `[a-z0-9]`, so the on-disk folder
//! `ferrari296gt3` already matches the seeded name "Ferrari 296 GT3" with no
//! help from us. What normalization cannot bridge is a folder id that
//! **abbreviates** its car — iRacing's `mercedesw13` will never equal
//! `mercedesamgw13eperformance` — and those uploads came back as `422 unknown
//! car`, leaving the user to guess the site's exact wording by hand.
//!
//! This table closes exactly that gap. It is **exceptions-only**: a car whose
//! folder id already normalizes to its display name must *not* appear here, so
//! the list stays short enough to review by eye. Anything absent passes through
//! untouched and the server's 422 still guides the user.
//!
//! **Provenance.** The iRacing rows pair the folder ids published in iRacing's
//! own "Filepath for active iRacing cars" support article with the exact car
//! names seeded on parcferme.cc (the public `cars.getAll` endpoint), keeping
//! only the pairs that genuinely disagree under normalization. Rows whose
//! folder id that article shows as a *sub-variant* of a parent car directory
//! (`\cars\rufrt12r\awd`) are parked in the `// verify` list below rather than
//! guessed at — a wrong alias silently files a setup under the wrong car, which
//! is worse than the 422 it replaces. Confirm those against a live
//! `Documents\iRacing\setups` before promoting them.

use crate::sim::Sim;

/// One `(normalized folder id, exact Parc Fermé car name)` pair.
type Alias = (&'static str, &'static str);

/// iRacing folder ids that do **not** normalize to their site car name. Keys
/// are pre-normalized (lowercase, `[a-z0-9]` only), so a lookup is a plain
/// comparison against [`normalize`].
const IRACING: &[Alias] = &[
    ("acuransxevo22gt3", "Acura NSX GT3 EVO 22"),
    ("amvantageevogt3", "Aston Martin Vantage GT3 EVO"),
    ("amvantagegt4", "Aston Martin Vantage GT4"),
    ("arcachevy25", "ARCA Chevrolet SS"),
    ("arcaford25", "ARCA Ford Mustang"),
    ("arcatoyota25", "ARCA Toyota Camry"),
    ("audir8gt3", "Audi R8 LMS GT3"),
    ("audirs3lms", "Audi RS3 LMS TCR"),
    ("bigblock", "Dirt Big Block Modified"),
    ("bmwlmdh", "BMW M Hybrid V8"),
    ("bmwm4evogt4", "BMW M4 G82 GT4"),
    ("bmwrn2csr", "BMW M2 CSR"),
    ("buicklesabre87", "NASCAR Legends Buick LeSabre - 1987"),
    ("c6r", "Chevrolet Corvette C6.R GT1"),
    ("c7vettedp", "Chevrolet Corvette C7 Daytona Prototype"),
    ("c8rvettegte", "Chevrolet Corvette C8.R GTE"),
    ("cadillacctsvr", "Cadillac CTS-V Racecar"),
    ("cadillacvseriesgtp", "Cadillac V-Series.R GTP"),
    ("camaro2019", "NASCAR Xfinity Chevrolet Camaro"),
    (
        "chevymontecarlo03",
        "NASCAR Gen 4 Chevrolet Monte Carlo - 2003",
    ),
    (
        "chevymontecarlo87",
        "NASCAR Legends Chevrolet Monte Carlo - 1987",
    ),
    ("chevyvettez06rgt3", "Chevrolet Corvette Z06 GT3.R"),
    ("crosscartn11", "FIA Cross Car"),
    ("dallarail15", "Dallara IL-15 (INDY NXT)"),
    ("dbr9", "Aston Martin DBR9 GT1"),
    ("dirtumpmod", "Dirt UMP Modified"),
    ("ferrarievogt3", "Ferrari 488 GT3 Evo 2020"),
    ("ford34c", "Legends Ford '34 Coupe"),
    ("fordf150", "NASCAR Truck Ford F150"),
    ("fordgt2017", "Ford GTE"),
    ("fordmustanggen3", "Supercars Ford Mustang Gen 3"),
    ("fordmustanggt", "Supercars Ford Mustang GT"),
    ("fordtaurus03", "NASCAR Gen 4 Ford Taurus - 2003"),
    (
        "fordthunderbird87",
        "NASCAR Legends Ford Thunderbird - 1987",
    ),
    ("formulair04", "FIA F4"),
    ("formulamazda", "Pro Mazda"),
    ("fr500s", "Ford Mustang FR500S"),
    ("hondacivictyper", "Honda Civic Type R TCR"),
    ("hyundaielantracn7", "Hyundai Elantra N TCR"),
    ("hyundaivelostern", "Hyundai Veloster N TCR"),
    ("indypropm18", "Indy Pro 2000 PM-18"),
    ("jettatdi", "VW Jetta TDI Cup"),
    ("lamborghinievogt3", "Lamborghini Huracán GT3 EVO"),
    ("latemodel2023", "Late Model Stock"),
    ("mclaren720sgt3", "McLaren 720S GT3 EVO"),
    ("mercedesamgevogt3", "Mercedes-AMG GT3 2020"),
    ("mercedesw12", "Mercedes-AMG W12 E Performance"),
    ("mercedesw13", "Mercedes-AMG W13 E Performance"),
    ("mustang2019", "NASCAR Xfinity Ford Mustang"),
    ("mx52016", "Global Mazda MX-5 Cup"),
    (
        "pontiacgrandprix87",
        "NASCAR Legends Pontiac Grand Prix - 1987",
    ),
    ("porsche718gt4", "Porsche 718 Cayman GT4 Clubsport MR"),
    ("porsche911cup", "Porsche 911 GT3 Cup (991)"),
    ("porsche991rsr", "Porsche 911 RSR"),
    ("porsche9922cup", "Porsche 911 Cup (992.2)"),
    ("porsche992rgt3", "Porsche 911 GT3 R (992)"),
    ("pro2lite", "Lucas Oil Off Road Pro 2 Lite"),
    ("pro2truck", "Lucas Oil Off Road Pro 2 Truck"),
    ("pro4truck", "Lucas Oil Off Road Pro 4 Truck"),
    ("raygr22", "Ray FF1600"),
    ("renaultcliocup", "Renault Clio"),
    ("rt2000", "Skip Barber Formula 2000"),
    ("silverado2019", "NASCAR Truck Chevrolet Silverado"),
    ("skmodified", "Modified - SK"),
    ("solstice", "Pontiac Solstice"),
    ("specracer", "SCCA Spec Racer Ford"),
    ("sr8", "Radical SR8"),
    ("superformulalights324", "Super Formula Lights"),
    ("supra2019", "NASCAR Xfinity Toyota Supra"),
    ("toyotatundra2022", "NASCAR Truck Toyota Tundra TRD Pro"),
    ("usf17", "USF 2000"),
];

// Unconfirmed folder ids: iRacing's article lists each of these as a sub-variant
// folder under a parent car directory, so the flat setups-folder name is not
// proven. Verify against a real install, then move the row into IRACING above.
//   305                    -> Dirt Sprint Car - 305
//   358                    -> Dirt 358 Modified
//   360                    -> Dirt Sprint Car - 360
//   410                    -> Dirt Sprint Car - 410
//   awd                    -> Ruf RT 12R AWD
//   cspec                  -> Ruf RT 12R C-Spec
//   dallara                -> Dallara IR-05
//   fordgt                 -> Ford GT GT2
//   gt3                    -> Ford GT GT3
//   mclarenmp4             -> McLaren MP4-12C GT3
//   nonwinged              -> Dirt Micro Sprint Car - Non-Winged
//   porsche992cup          -> Porsche 911 Cup (992)
//   rufrt12r               -> Ruf RT 12R Track
//   rwd                    -> Ruf RT 12R RWD
//   sprint                 -> Sprint Car
//   tour                   -> NASCAR Whelen Tour Modified
//   winged                 -> Dirt Micro Sprint Car - Winged

/// ACC and LMU have no confirmed exceptions yet. ACC's folder ids
/// (`ferrari_488_gt3_evo`) normalize cleanly and the server keeps its own ACC
/// map; LMU files by track, so its car is typed by the user rather than read off
/// disk. Both are declared so a future exception has an obvious home.
const ACC: &[Alias] = &[];
const LMU: &[Alias] = &[];

/// The alias table for `sim`.
fn table(sim: Sim) -> &'static [Alias] {
    match sim {
        Sim::IRacing => IRACING,
        Sim::Acc => ACC,
        Sim::Lmu => LMU,
    }
}

/// Case-fold and strip to `[a-z0-9]`, mirroring the server's comparison.
///
/// ponytail: no NFKD pass — every folder id a sim writes is ASCII, and the
/// server folds both sides anyway, so an accent can only appear in the name we
/// *emit*, never in the key we match on.
fn normalize(value: &str) -> String {
    value
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

/// The site's car name for an on-disk folder id, or `None` when the value isn't
/// a known exception (including when it already normalize-matches, which needs
/// no alias).
pub fn resolve(sim: Sim, car: &str) -> Option<&'static str> {
    let key = normalize(car);
    // ponytail: linear scan of a ~70-entry table, run once per upload. A real
    // map would need a dependency or a lazy static to beat it on a cold path.
    table(sim)
        .iter()
        .find(|(folder, _)| *folder == key)
        .map(|(_, name)| *name)
}

/// [`resolve`], falling back to the caller's value untouched — the form stays
/// editable and the server remains the authority on what a car is called.
pub fn apply(sim: Sim, car: &str) -> &str {
    resolve(sim, car).unwrap_or(car)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn abbreviated_folder_ids_alias_to_the_site_name() {
        // The reported failure: `mercedesw13` normalizes to `mercedesw13`, the
        // site name to `mercedesamgw13eperformance` — never equal, so the
        // server 422'd until this table existed.
        assert_eq!(
            apply(Sim::IRacing, "mercedesw13"),
            "Mercedes-AMG W13 E Performance"
        );
        // iRacing's folder for the Skip Barber is `rt2000`, not `skipbarber`.
        assert_eq!(apply(Sim::IRacing, "rt2000"), "Skip Barber Formula 2000");
        // Folder ids come off disk, so tolerate whatever casing/punctuation the
        // user or the sim produced.
        assert_eq!(
            apply(Sim::IRacing, "MercedesW13"),
            apply(Sim::IRacing, "mercedesw13")
        );
    }

    #[test]
    fn normalize_matching_cars_are_left_alone() {
        // These already match server-side; aliasing them would be dead weight,
        // so they must be absent from the table entirely.
        for folder in ["ferrari296gt3", "bmwm4gt3", "porsche963gtp", "lotus79"] {
            assert_eq!(
                resolve(Sim::IRacing, folder),
                None,
                "{folder} needs no alias"
            );
            assert_eq!(apply(Sim::IRacing, folder), folder);
        }
    }

    #[test]
    fn unknown_values_pass_through_untouched() {
        // A car we've never seen, a hand-typed site name, and empty input all
        // reach the server verbatim — its 422 is what guides the user.
        for value in ["somenewcar2027", "Ferrari 296 GT3", ""] {
            assert_eq!(apply(Sim::IRacing, value), value);
        }
        // Aliases are per-sim: an iRacing folder id must not leak into ACC/LMU.
        assert_eq!(apply(Sim::Acc, "mercedesw13"), "mercedesw13");
        assert_eq!(apply(Sim::Lmu, "mercedesw13"), "mercedesw13");
    }

    /// Hits the live site, so ignored by default: run
    /// `cargo test -p pf_core aliases_match_the_live_site -- --ignored --nocapture`.
    ///
    /// Every alias target must still be a car the site knows — if one is renamed
    /// server-side the alias starts *causing* the 422 it was added to prevent,
    /// and nothing else would catch that.
    #[test]
    #[ignore]
    fn aliases_match_the_live_site() {
        for sim in Sim::ALL {
            let known = crate::cars::names_for(sim);
            assert!(!known.is_empty(), "no car list for {}", sim.id());
            let stale: Vec<_> = table(sim)
                .iter()
                .filter(|(_, name)| !known.iter().any(|k| k == name))
                .collect();
            assert!(
                stale.is_empty(),
                "{} aliases no longer on the site: {stale:?}",
                sim.id()
            );
            println!("{}: {} aliases all resolve", sim.id(), table(sim).len());
        }
    }

    #[test]
    fn table_is_exceptions_only_and_unambiguous() {
        for sim in Sim::ALL {
            let rows = table(sim);
            for (folder, name) in rows {
                // A key that isn't already normalized would be unreachable.
                assert_eq!(&normalize(folder), folder, "{folder} is not normalized");
                // An entry whose sides already agree is by definition redundant.
                assert_ne!(
                    normalize(folder),
                    normalize(name),
                    "{folder} normalize-matches {name} — drop it"
                );
            }
            // Two rows sharing a key would make the winner scan-order dependent.
            let mut keys: Vec<_> = rows.iter().map(|(f, _)| *f).collect();
            keys.sort_unstable();
            let count = keys.len();
            keys.dedup();
            assert_eq!(keys.len(), count, "duplicate folder id in {:?}", sim.id());
        }
    }
}
