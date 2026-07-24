use super::*;

fn link(uri: &str) -> Hyperlink {
    Hyperlink {
        id: None,
        uri: uri.to_owned(),
    }
}

#[test]
fn intern_dedupes_same_hyperlink() {
    let mut reg = HyperlinkRegistry::new();
    let a = reg.intern(link("https://example.com")).unwrap();
    let b = reg.intern(link("https://example.com")).unwrap();
    assert_eq!(a, b);
    assert_eq!(reg.len_for_test(), 1);
}

#[test]
fn intern_returns_distinct_ids_for_distinct_uris() {
    let mut reg = HyperlinkRegistry::new();
    let a = reg.intern(link("https://a")).unwrap();
    let b = reg.intern(link("https://b")).unwrap();
    assert_ne!(a, b);
}

#[test]
fn get_resolves_interned_link() {
    let mut reg = HyperlinkRegistry::new();
    let id = reg.intern(link("https://example.com")).unwrap();
    assert_eq!(reg.get(id).unwrap().uri, "https://example.com");
}

#[test]
fn intern_returns_none_past_distinct_entries_cap() {
    // Override the cap for the test by exhausting it directly. The cap
    // is `pub const`, but we can simulate by interning the cap itself.
    let mut reg = HyperlinkRegistry::new();
    for i in 0..MAX_DISTINCT_ENTRIES {
        assert!(
            reg.intern(link(&format!("https://example.com/{i}")))
                .is_some()
        );
    }
    // Past the cap → None.
    assert!(reg.intern(link("https://overflow.example")).is_none());
    // Existing entries still resolve.
    let first = reg.intern(link("https://example.com/0")).unwrap();
    assert_eq!(reg.get(first).unwrap().uri, "https://example.com/0");
    // `len_for_test` does NOT shrink on cap-hit; the failed intern is a
    // no-op rather than an eviction.
    assert_eq!(reg.len_for_test(), MAX_DISTINCT_ENTRIES);
}

#[test]
fn intern_rejects_uri_above_max_bytes() {
    let mut reg = HyperlinkRegistry::new();
    let big = "x".repeat(MAX_URI_BYTES + 1);
    // Defensive backstop: even if parser was bypassed, the registry won't
    // accept an over-length URI.
    assert!(reg.intern(link(&big)).is_none());
}

#[test]
fn no_reclaim_overwrite_does_not_shrink_registry() {
    // The registry has no API to "remove" or "decrement" an entry; this
    // test pins down that contract by interning, then re-interning the
    // same value (which must reuse the slot, not grow the registry).
    let mut reg = HyperlinkRegistry::new();
    let id = reg.intern(link("https://example.com")).unwrap();
    assert_eq!(reg.len_for_test(), 1);
    let id2 = reg.intern(link("https://example.com")).unwrap();
    assert_eq!(id, id2);
    assert_eq!(reg.len_for_test(), 1);
}
