use super::super::{normalize_warping_verbs, MAX_WARPING_VERB_CHARS};
use super::*;

#[test]
fn all_packs_have_non_empty_verbs() {
    for pack in WarpingVerbPack::all() {
        assert!(
            !pack.verbs().is_empty(),
            "pack {:?} should have at least one verb",
            pack
        );
    }
}

#[test]
fn all_pack_verbs_fit_display_length() {
    for pack in WarpingVerbPack::all() {
        for verb in pack.verbs() {
            assert!(
                verb.chars().count() <= MAX_WARPING_VERB_CHARS,
                "verb {:?} in pack {:?} exceeds max display length",
                verb,
                pack,
            );
        }
    }
}

#[test]
fn pack_verbs_survive_normalization_unchanged() {
    // Every pack verb is already trimmed, non-empty, and fits the length cap,
    // so normalization should yield the same list.
    for pack in WarpingVerbPack::all() {
        let original = pack.verbs_as_vec();
        let normalized = normalize_warping_verbs(original.clone());
        assert_eq!(
            original, normalized,
            "pack {:?} verbs changed after normalization",
            pack
        );
    }
}

#[test]
fn from_identifier_round_trips() {
    for pack in WarpingVerbPack::all() {
        assert_eq!(
            WarpingVerbPack::from_identifier(pack.identifier()),
            Some(*pack)
        );
    }
}

#[test]
fn from_identifier_is_case_insensitive_and_trims() {
    assert_eq!(
        WarpingVerbPack::from_identifier("MEDIEVAL"),
        Some(WarpingVerbPack::Medieval)
    );
    assert_eq!(
        WarpingVerbPack::from_identifier("  cooking  "),
        Some(WarpingVerbPack::Cooking)
    );
    assert_eq!(
        WarpingVerbPack::from_identifier("  CONSPIRACY  "),
        Some(WarpingVerbPack::ConspiracyTheorist)
    );
    assert_eq!(
        WarpingVerbPack::from_identifier("  CONPSIRACY  "),
        Some(WarpingVerbPack::ConspiracyTheorist)
    );
    assert_eq!(
        WarpingVerbPack::from_identifier("WaRpY"),
        Some(WarpingVerbPack::Warpy)
    );
}

#[test]
fn from_identifier_returns_none_for_unknown() {
    assert!(WarpingVerbPack::from_identifier("not-a-pack").is_none());
    assert!(WarpingVerbPack::from_identifier("").is_none());
}

#[test]
fn pack_identifiers_are_unique() {
    let mut seen = std::collections::HashSet::new();
    for pack in WarpingVerbPack::all() {
        assert!(
            seen.insert(pack.identifier()),
            "duplicate identifier for {:?}",
            pack
        );
    }
}

#[test]
fn pack_display_names_are_unique() {
    let mut seen = std::collections::HashSet::new();
    for pack in WarpingVerbPack::all() {
        assert!(
            seen.insert(pack.display_name()),
            "duplicate display name for {:?}",
            pack
        );
    }
}
