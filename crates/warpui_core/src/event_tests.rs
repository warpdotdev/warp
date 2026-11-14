use super::Event;

/// A replacement commit that no surface handled must still reach that surface as an
/// ordinary append. Dropping it would lose a character the user typed — the macOS
/// accent popup produces these in a focused terminal too, not only in an editor
/// (#13631).
#[test]
fn replace_preceding_characters_falls_back_to_an_append() {
    let event = Event::ReplacePrecedingCharacters {
        chars: "ò".to_owned(),
    };

    match event.unhandled_fallback() {
        Some(Event::TypedCharacters { chars }) => assert_eq!(chars, "ò"),
        other => panic!("expected a TypedCharacters fallback, got {other:?}"),
    }
}

/// Every other event keeps whatever handling it already had; the fallback exists
/// only for the replacement commit.
#[test]
fn other_events_have_no_fallback() {
    let event = Event::TypedCharacters {
        chars: "o".to_owned(),
    };

    assert!(event.unhandled_fallback().is_none());
}
