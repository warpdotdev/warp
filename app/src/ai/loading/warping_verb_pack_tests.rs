use super::super::warping_verb::{normalize_warping_verbs, MAX_WARPING_VERB_CHARS};
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
