//! Match an on-disk folder id against the site's live car list. **[M5: push]**
//!
//! [`crate::car_aliases`] is a table compiled into the binary, so every car the
//! site adds whose folder id *abbreviates* it — or every car it renames — costs
//! a client release before uploads stop coming back `422 unknown car`. The
//! authoritative name list is already a fetch away ([`crate::options`]), so
//! match against it instead and let the table shrink to the cases this gets
//! wrong.
//!
//! **Never guesses when it isn't sure.** Every tier below requires a *single*
//! best candidate; a tie returns `None` and the caller falls back to the raw
//! folder id, which is what the user saw before. A wrong pre-fill files a setup
//! under the wrong car — worse than the 422 it replaces — so the whole design
//! is biased towards declining:
//!
//! 1. **Exact** — normalized equality, the same comparison the server makes.
//! 2. **Containment** — either side contains the other, e.g. `hondacivictyper`
//!    in "Honda Civic Type R TCR", or "Super Formula Lights" in
//!    `superformulalights324`.
//! 3. **Subsequence** — the folder id's letters appear in order within the
//!    name: `mercedesw13` inside "Mercedes-AMG **W13** E Performance". This is
//!    the abbreviation case the alias table exists for, so it is fenced in by a
//!    length floor and a shared prefix — without the anchor, three-letter
//!    folder ids match half the list.
//! 4. **Edit distance** — a tight, length-relative bound, for a name that has
//!    drifted rather than been rewritten.
//!
//! Purely computational: the caller supplies the name list, so this stays
//! testable offline and callable from anywhere.

/// Below this many normalized characters a folder id carries too little signal
/// for the subsequence and edit-distance tiers — `sr8` is a subsequence of
/// almost anything. Such ids stay with the curated table.
const MIN_FUZZY_LEN: usize = 6;

/// Leading characters a name must share with the folder id before the
/// subsequence tier will consider it. In practice this is the make
/// (`mercedesw13` → "Mercedes-…"), which is the part folder ids never
/// abbreviate.
const ANCHOR_LEN: usize = 3;

/// Shortest overlap the containment tier accepts, so a one- or two-character
/// id can't latch onto an unrelated name.
const MIN_CONTAIN_LEN: usize = 3;

/// Case-fold and strip to `[a-z0-9]`, mirroring the server's comparison.
///
/// ponytail: no NFKD pass — every folder id a sim writes is ASCII, and both
/// sides are folded the same way, so an accent ("Huracán") drops out of the
/// site name and the folder id alike and the two still meet.
pub(crate) fn normalize(value: &str) -> String {
    value
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

/// The entry of `known` that equals `candidate` under [`normalize`] — the same
/// comparison the server makes, so a hit is a certainty rather than a guess.
///
/// Doubles as "does the site still call a car this?": a curated alias whose
/// target has stopped matching has been renamed server-side, and is about to
/// start *causing* the 422 it was added to prevent.
pub fn exact_match<'a>(candidate: &str, known: &'a [String]) -> Option<&'a str> {
    let target = normalize(candidate);
    if target.is_empty() {
        return None;
    }
    known
        .iter()
        .find(|k| normalize(k) == target)
        .map(String::as_str)
}

/// The site's name for `candidate` (a folder id, or anything else the user
/// typed), or `None` when no single name stands out.
///
/// `known` is the site's car list for the sim — an empty list always yields
/// `None`, so an offline or unpaired client behaves exactly as it did before.
pub fn best_match<'a>(candidate: &str, known: &'a [String]) -> Option<&'a str> {
    let query = normalize(candidate);
    if query.is_empty() {
        return None;
    }
    // Normalize the list once; every tier compares against it.
    let names: Vec<(String, &'a str)> = known
        .iter()
        .map(|name| (normalize(name), name.as_str()))
        .filter(|(norm, _)| !norm.is_empty())
        .collect();

    // Tier 1 — exact. Two names that normalize alike are the same car to the
    // server, so the first is as good as any.
    if let Some(name) = exact_match(candidate, known) {
        return Some(name);
    }

    // Tier 2 — containment, either direction: the folder id may be the shorter
    // side ("Honda Civic Type R TCR") or the longer one
    // (`superformulalights324`). Closest in length wins.
    let contained = |norm: &str| {
        let overlap = norm.len().min(query.len());
        (overlap >= MIN_CONTAIN_LEN && (norm.contains(&query) || query.contains(norm)))
            .then(|| norm.len().abs_diff(query.len()))
    };
    if let Some(hit) = pick(&names, contained) {
        return Some(hit);
    }

    // Below the floor, the remaining tiers are guesswork rather than matching.
    if query.len() < MIN_FUZZY_LEN {
        return None;
    }
    let anchor = &query[..ANCHOR_LEN];

    // Tier 3 — subsequence, anchored on the make. Prefer the name that adds
    // fewest characters to the id, so "Mercedes-AMG W13 E Performance" beats a
    // longer name the id also threads through.
    let subsequence = |norm: &str| {
        (norm.len() > query.len() && norm.starts_with(anchor) && is_subsequence(&query, norm))
            .then(|| norm.len() - query.len())
    };
    if let Some(hit) = pick(&names, subsequence) {
        return Some(hit);
    }

    // Tier 4 — edit distance, bounded relative to the id's length so long
    // names can't drift far and short ones barely at all.
    let budget = (query.len() / 5).max(1);
    let close = |norm: &str| {
        let distance = edit_distance(&query, norm);
        (distance <= budget).then_some(distance)
    };
    pick(&names, close)
}

/// The single lowest-scoring name under `score`, or `None` when nothing scores
/// or two names tie for best. Ties are the ambiguous case the whole module
/// refuses to resolve — falling through to a later tier would only re-find the
/// same pair.
fn pick<'a>(names: &[(String, &'a str)], score: impl Fn(&str) -> Option<usize>) -> Option<&'a str> {
    let mut best: Option<(usize, &'a str)> = None;
    let mut tied = false;
    for (norm, name) in names {
        let Some(points) = score(norm) else { continue };
        match best {
            Some((bestpoints, _)) if points > bestpoints => {}
            Some((bestpoints, _)) if points == bestpoints => tied = true,
            _ => {
                best = Some((points, name));
                tied = false;
            }
        }
    }
    best.filter(|_| !tied).map(|(_, name)| name)
}

/// Whether every character of `needle` appears in `haystack` in order.
fn is_subsequence(needle: &str, haystack: &str) -> bool {
    let mut chars = haystack.chars();
    needle.chars().all(|c| chars.any(|h| h == c))
}

/// Levenshtein distance, two-row DP. Inputs are normalized car names — tens of
/// ASCII characters — so the allocation-free row reuse is the only tuning this
/// needs.
fn edit_distance(a: &str, b: &str) -> usize {
    let b_chars: Vec<char> = b.chars().collect();
    let mut previous: Vec<usize> = (0..=b_chars.len()).collect();
    let mut current = vec![0usize; b_chars.len() + 1];
    for (i, ac) in a.chars().enumerate() {
        current[0] = i + 1;
        for (j, bc) in b_chars.iter().enumerate() {
            let substitute = previous[j] + usize::from(ac != *bc);
            current[j + 1] = substitute.min(previous[j + 1] + 1).min(current[j] + 1);
        }
        std::mem::swap(&mut previous, &mut current);
    }
    previous[b_chars.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn known(names: &[&str]) -> Vec<String> {
        names.iter().map(|n| (*n).to_string()).collect()
    }

    #[test]
    fn abbreviated_folder_ids_find_their_car() {
        // The case the alias table was created for — and the one that costs a
        // release every time the site seeds another of them.
        let cars = known(&[
            "Mercedes-AMG W13 E Performance",
            "Mercedes-AMG W14 E Performance",
            "Ferrari 296 GT3",
        ]);
        assert_eq!(
            best_match("mercedesw13", &cars),
            Some("Mercedes-AMG W13 E Performance")
        );
        // A car added after this binary shipped resolves just the same, which
        // is the entire point: no alias row, no release.
        assert_eq!(
            best_match("mercedesw14", &cars),
            Some("Mercedes-AMG W14 E Performance")
        );

        // Suffixed names (containment) and suffixed folder ids (the reverse).
        let cars = known(&["Honda Civic Type R TCR", "Super Formula Lights"]);
        assert_eq!(
            best_match("hondacivictyper", &cars),
            Some("Honda Civic Type R TCR")
        );
        assert_eq!(
            best_match("superformulalights324", &cars),
            Some("Super Formula Lights")
        );

        // Letters interleaved with the site's extra words still thread through.
        let cars = known(&["Audi R8 LMS GT3", "Audi RS3 LMS TCR"]);
        assert_eq!(best_match("audir8gt3", &cars), Some("Audi R8 LMS GT3"));

        // Two names contain the id; the one adding least wins. This is one of
        // the unconfirmed `// verify` rows in `car_aliases`, resolved without
        // anyone having to confirm it against an install.
        let cars = known(&["Dallara IR-05", "Dallara IL-15 (INDY NXT)"]);
        assert_eq!(best_match("dallara", &cars), Some("Dallara IR-05"));
    }

    #[test]
    fn exact_and_normalized_matches_return_the_sites_spelling() {
        let cars = known(&["Ferrari 296 GT3", "Porsche 963 GTP"]);
        // A folder id that already normalize-matches is upgraded to the site's
        // punctuation — the server accepts both, the user sees the real name.
        assert_eq!(best_match("ferrari296gt3", &cars), Some("Ferrari 296 GT3"));
        // Idempotent: feeding a site name back in returns it unchanged.
        assert_eq!(
            best_match("Ferrari 296 GT3", &cars),
            Some("Ferrari 296 GT3")
        );
    }

    #[test]
    fn ambiguity_declines_rather_than_guessing() {
        // The Ruf variants: four names contain `rufrt12r` and two of them are
        // equally close, so there is no honest answer. Filing a setup under a
        // coin-flip is worse than leaving the folder id showing.
        let cars = known(&[
            "Ruf RT 12R Track",
            "Ruf RT 12R AWD",
            "Ruf RT 12R RWD",
            "Ruf RT 12R C-Spec",
        ]);
        assert_eq!(best_match("rufrt12r", &cars), None);
        // Equal-scoring subsequence hits are the same story.
        assert_eq!(best_match("rufrt12rwd", &cars), None);
    }

    #[test]
    fn short_and_unrelated_ids_are_left_alone() {
        let cars = known(&[
            "Chevrolet Corvette C6.R GT1",
            "NASCAR Xfinity Ford Mustang",
            "Global Mazda MX-5 Cup",
            "BMW M Hybrid V8",
        ]);
        // Too short for the fuzzy tiers and not contained anywhere.
        assert_eq!(best_match("usf17", &cars), None);
        // Digits the site name doesn't carry: no tier can bridge these, and
        // that's correct — they belong in the curated table.
        assert_eq!(best_match("mustang2019", &cars), None);
        assert_eq!(best_match("bmwlmdh", &cars), None);
        // The anchor keeps `mx52016` off "Global Mazda MX-5 Cup": the id's
        // letters are all in there, but the name doesn't start with them.
        assert_eq!(best_match("mx52016", &cars), None);
        // Empty input and an empty list are both non-events.
        assert_eq!(best_match("", &cars), None);
        assert_eq!(best_match("ferrari296gt3", &[]), None);
    }

    #[test]
    fn exact_match_tracks_the_live_list() {
        let cars = known(&["Ferrari 296 GT3"]);
        assert_eq!(
            exact_match("Ferrari 296 GT3", &cars),
            Some("Ferrari 296 GT3")
        );
        // Normalized, so punctuation drift doesn't read as a rename.
        assert_eq!(exact_match("ferrari296gt3", &cars), Some("Ferrari 296 GT3"));
        assert_eq!(exact_match("Ferrari 488 GT3 Evo 2020", &cars), None);
        assert_eq!(exact_match("", &cars), None);
    }

    #[test]
    fn edit_distance_is_symmetric_and_bounded() {
        assert_eq!(edit_distance("", "abc"), 3);
        assert_eq!(edit_distance("abc", "abc"), 0);
        assert_eq!(edit_distance("kitten", "sitting"), 3);
        assert_eq!(edit_distance("sitting", "kitten"), 3);
    }

    #[test]
    fn subsequence_respects_order() {
        assert!(is_subsequence("abc", "axbxc"));
        assert!(!is_subsequence("cba", "axbxc"));
        assert!(is_subsequence("", "anything"));
        assert!(!is_subsequence("abcd", "abc"));
    }
}
