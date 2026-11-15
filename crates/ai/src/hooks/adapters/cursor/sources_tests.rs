use std::sync::Arc;

use super::*;
use crate::hooks::adapters::cursor::config::parse_cursor_file;
use crate::hooks::test_util::test_source_id;

const THREE_MATCHERS: &[u8] = br#"{ "version": 1, "hooks": { "beforeShellExecution": [
    { "command": "a", "matcher": "^git " },
    { "command": "b", "matcher": "^rm " },
    { "command": "c", "matcher": "*" }
] } }"#;

#[test]
fn a_remembered_parse_is_returned_only_for_the_document_it_came_from() {
    let sources = SourceCache::default();
    let source = test_source_id("hooks.json");
    let (document, parsed) = parse_cursor_file(THREE_MATCHERS).expect("parses");
    sources.remember(&source, &document, &parsed.definitions);

    assert_eq!(
        sources.definitions(&source, &document),
        Some(parsed.definitions.clone())
    );
    assert_eq!(
        sources.pattern_count(&source),
        2,
        "match-all matchers are not regexes"
    );
    let kept = match &parsed.definitions[0].matcher {
        Some(ParsedMatcher::Pattern(regex)) => regex.clone(),
        other => panic!("expected a compiled pattern, got {other:?}"),
    };
    let looked_up = sources.regex(&source, "^git ").expect("kept");
    assert!(
        Arc::ptr_eq(&kept, &looked_up),
        "the regex compiled while parsing is the one matching uses"
    );

    let other_document = serde_json::json!({ "version": 1, "hooks": {} });
    assert_eq!(
        sources.definitions(&source, &other_document),
        None,
        "a different document is not answered from the cache"
    );
    assert_eq!(
        sources.definitions(&test_source_id("other.json"), &document),
        None
    );
}

#[test]
fn a_reparse_replaces_the_sources_entry() {
    let sources = SourceCache::default();
    let source = test_source_id("hooks.json");
    let (document, parsed) = parse_cursor_file(THREE_MATCHERS).expect("parses");
    sources.remember(&source, &document, &parsed.definitions);

    let (reloaded_document, reloaded) = parse_cursor_file(
        br#"{ "version": 1, "hooks": { "beforeShellExecution": [
            { "command": "a", "matcher": "^cargo " }
        ] } }"#,
    )
    .expect("parses");
    sources.remember(&source, &reloaded_document, &reloaded.definitions);
    assert_eq!(sources.pattern_count(&source), 1);
    assert_eq!(sources.definitions(&source, &document), None);
    assert_eq!(
        sources.definitions(&source, &reloaded_document),
        Some(reloaded.definitions)
    );
}

#[test]
fn a_pattern_that_was_not_kept_compiles_on_first_use() {
    let sources = SourceCache::default();
    let source = test_source_id("direct.json");
    assert_eq!(sources.pattern_count(&source), 0);
    let on_demand = sources.regex(&source, "^git ").expect("compiles");
    assert!(Arc::ptr_eq(
        &on_demand,
        &sources.regex(&source, "^git ").expect("cached")
    ));
    assert_eq!(sources.pattern_count(&source), 1);
    assert!(sources.regex(&source, "(").is_none());
    assert_eq!(
        sources.definitions(&source, &serde_json::Value::Null),
        None,
        "a source that was never parsed has no definitions, whatever document is asked about"
    );
}
