use std::collections::HashSet;

use rand::prelude::StdRng;
use rand::SeedableRng;

use super::{generate_unique_name, WORDS};

fn seeded_rng(seed: u64) -> StdRng {
    StdRng::seed_from_u64(seed)
}

#[test]
fn deterministic_output_with_seeded_rng() {
    let existing = HashSet::new();
    let a = generate_unique_name(&existing, &mut seeded_rng(42));
    let b = generate_unique_name(&existing, &mut seeded_rng(42));
    assert_eq!(a, b, "same seed should produce the same name");
}

#[test]
fn format_is_two_hyphenated_words() {
    let existing = HashSet::new();
    let name = generate_unique_name(&existing, &mut seeded_rng(1));
    let parts: Vec<&str> = WORDS
        .iter()
        .copied()
        .filter(|w| name.starts_with(&format!("{w}-")) || name.ends_with(&format!("-{w}")))
        .collect();
    // Instead of splitting on `-` (which would break hyphenated words like
    // `palo-verde`), verify that the name starts with one word, ends with
    // another, and those two words are distinct.
    let word_set: HashSet<&str> = WORDS.iter().copied().collect();
    let found_prefix = word_set
        .iter()
        .find(|w| name.starts_with(*w) && name.len() > w.len() && name.as_bytes()[w.len()] == b'-');
    let found_suffix = word_set.iter().find(|w| {
        name.ends_with(*w)
            && name.len() > w.len()
            && name.as_bytes()[name.len() - w.len() - 1] == b'-'
    });
    assert!(
        found_prefix.is_some(),
        "name should start with a word from WORDS: {name}"
    );
    assert!(
        found_suffix.is_some(),
        "name should end with a word from WORDS: {name}"
    );
    assert_ne!(
        found_prefix.unwrap(),
        found_suffix.unwrap(),
        "the two words should be distinct: {name}"
    );
    assert!(
        !parts.is_empty(),
        "name should contain words from WORDS: {name}"
    );
}

#[test]
fn words_are_distinct() {
    let existing = HashSet::new();
    for seed in 0..100 {
        let name = generate_unique_name(&existing, &mut seeded_rng(seed));
        // For a 2-word name, `choose_multiple` guarantees distinct indices,
        // so the two words will always be different. Verify by checking that
        // the name is not a word repeated (e.g. "mesa-mesa").
        let word_set: HashSet<&str> = WORDS.iter().copied().collect();
        for word in &word_set {
            let repeated = format!("{word}-{word}");
            assert_ne!(name, repeated, "generated name should never repeat a word");
        }
    }
}

#[test]
fn avoids_existing_branches() {
    let mut existing = HashSet::new();
    let mut rng = seeded_rng(7);
    // Generate a name, add it to existing, then generate again —
    // the second name must differ.
    let first = generate_unique_name(&existing, &mut rng);
    existing.insert(first.as_str());
    let mut rng2 = seeded_rng(7);
    let second = generate_unique_name(&existing, &mut rng2);
    assert_ne!(first, second, "second name should avoid the first");
    assert!(
        !existing.contains(second.as_str()),
        "second name should not be in existing set"
    );
}

#[test]
fn escalates_to_three_words_when_two_word_space_exhausted() {
    // Fill existing with all possible 2-word combos (198 * 197 = 39006).
    // This is a large set but the test verifies the escalation logic.
    let word_set: Vec<&str> = WORDS.to_vec();
    let mut existing = HashSet::new();
    for (i, a) in word_set.iter().enumerate() {
        for (j, b) in word_set.iter().enumerate() {
            if i != j {
                existing.insert(format!("{a}-{b}"));
            }
        }
    }
    let existing_refs: HashSet<&str> = existing.iter().map(|s| s.as_str()).collect();
    let name = generate_unique_name(&existing_refs, &mut seeded_rng(99));
    // The name should have 3 words (3+ hyphens when words themselves may
    // contain hyphens, so count words by checking the word list).
    let mut remaining = name.as_str();
    let mut word_count = 0;
    while !remaining.is_empty() {
        // Find the longest matching word at the start of `remaining`.
        let matched = word_set
            .iter()
            .filter(|w| remaining.starts_with(**w))
            .max_by_key(|w| w.len());
        match matched {
            Some(w) => {
                word_count += 1;
                remaining = &remaining[w.len()..];
                if remaining.starts_with('-') {
                    remaining = &remaining[1..];
                }
            }
            None => {
                panic!("name contains a segment not in WORDS: remaining={remaining}, full={name}")
            }
        }
    }
    assert!(
        word_count >= 3,
        "expected >=3 words when 2-word space is exhausted, got {word_count} in {name}"
    );
}

#[test]
fn all_words_are_valid_git_branch_components() {
    for word in WORDS {
        assert!(!word.is_empty(), "word must not be empty");
        assert!(
            !word.starts_with('-'),
            "word must not start with hyphen: {word}"
        );
        assert!(
            !word.ends_with('-'),
            "word must not end with hyphen: {word}"
        );
        assert!(!word.contains(".."), "word must not contain '..': {word}");
        assert!(!word.contains(' '), "word must not contain spaces: {word}");
        assert!(
            !word.contains(|c: char| c.is_ascii_control()),
            "word must not contain control chars: {word}"
        );
        assert!(
            word.chars().all(|c| c.is_ascii_lowercase() || c == '-'),
            "word must be lowercase ascii + hyphens: {word}"
        );
    }
}

#[test]
fn word_list_has_no_duplicates() {
    let mut seen = HashSet::new();
    for word in WORDS {
        assert!(seen.insert(word), "duplicate word in WORDS: {word}");
    }
}
