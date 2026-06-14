use super::*;

fn build(pattern: &str, language_id: Option<&str>) -> LspFiletypePattern {
    let matcher = globset::GlobBuilder::new(pattern)
        .build()
        .expect("test pattern compiles")
        .compile_matcher();
    LspFiletypePattern::from_parts(
        pattern.to_string(),
        language_id.map(str::to_string),
        matcher,
    )
}

#[test]
fn partial_eq_equal_when_inputs_match_despite_separately_compiled_matchers() {
    // Two independently-built patterns with identical user inputs should
    // compare equal. Guards against a future regression where the hand-rolled
    // PartialEq accidentally includes the matcher field — `globset::GlobMatcher`
    // instances aren't reference-equal even when built from the same pattern,
    // so including matcher in eq would make this assertion fail.
    let a = build("*.rb", Some("ruby"));
    let b = build("*.rb", Some("ruby"));
    assert_eq!(a, b);
}

#[test]
fn partial_eq_unequal_when_language_id_differs() {
    let a = build("*.rb", Some("ruby"));
    let b = build("*.rb", None);
    assert_ne!(a, b);
}

#[test]
fn partial_eq_unequal_when_pattern_differs() {
    let a = build("*.rb", Some("ruby"));
    let b = build("*.py", Some("ruby"));
    assert_ne!(a, b);
}

#[test]
fn serde_skip_omits_matcher_on_serialize() {
    // The compiled matcher must not be written to disk. Verifies the
    // #[serde(skip)] attribute on `LspFiletypePattern::matcher`.
    let pattern = build("*.rb", Some("ruby"));
    let json = serde_json::to_value(&pattern).expect("pattern serializes");
    assert_eq!(json["pattern"], "*.rb");
    assert_eq!(json["language_id"], "ruby");
    assert!(
        json.get("matcher").is_none(),
        "matcher leaked into serde output"
    );
}

#[test]
fn serde_deserialize_uses_placeholder_matcher_that_never_matches() {
    // Deserializing an LspFiletypePattern directly via serde (bypassing
    // parse_entries) populates the matcher field via `placeholder_matcher`.
    // The resulting pattern is structurally valid — it remembers what the
    // user typed — but its matcher should never match any real filename.
    // This is the contract production code relies on: only parse_entries
    // produces descriptors with usable matchers.
    let json = serde_json::json!({
        "pattern": "*.rb",
        "language_id": "ruby",
    });
    let pattern: LspFiletypePattern = serde_json::from_value(json).expect("deserializes");
    assert_eq!(pattern.pattern, "*.rb");
    assert_eq!(pattern.language_id.as_deref(), Some("ruby"));
    // The placeholder is the only thing protecting us if a user later
    // adopts a literal filename that happens to match. The literal here is
    // deliberately weird to keep collision probability vanishingly small.
    assert!(
        !pattern.is_match("foo.rb"),
        "placeholder matched a real .rb file"
    );
    assert!(
        !pattern.is_match("Gemfile"),
        "placeholder matched a real basename"
    );
}
