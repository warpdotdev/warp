use super::{FileOutline, Symbol};

fn symbol(name: &str, comment: Option<Vec<&str>>) -> Symbol {
    Symbol {
        name: name.to_string(),
        type_prefix: None,
        comment: comment.map(|lines| lines.into_iter().map(str::to_string).collect()),
        line_number: 1,
    }
}

#[test]
fn approx_heap_size_is_zero_without_symbols() {
    let outline = FileOutline { symbols: None };
    assert_eq!(outline.approx_heap_size(), 0);

    let outline = FileOutline {
        symbols: Some(Vec::new()),
    };
    assert_eq!(outline.approx_heap_size(), 0);
}

#[test]
fn approx_heap_size_counts_owned_string_bytes() {
    let outline = FileOutline {
        symbols: Some(vec![
            symbol("foo", None),
            symbol("bar", Some(vec!["a", "bc"])),
        ]),
    };

    let size = outline.approx_heap_size();
    let symbols = outline.symbols().expect("symbols");

    // The owned name bytes ("foo" + "bar") and comment bytes ("a" + "bc") are
    // all counted.
    let owned_bytes = 3 + 3 + 1 + 2;
    // Plus the inline `Symbol` array backing the `Vec`.
    let vec_bytes = symbols.capacity() * std::mem::size_of::<Symbol>();
    assert!(size >= owned_bytes + vec_bytes, "size = {size}");
}

#[test]
fn approx_heap_size_grows_with_more_symbols() {
    let small = FileOutline {
        symbols: Some(vec![symbol("a", None)]),
    };
    let large = FileOutline {
        symbols: Some(vec![symbol(&"a".repeat(10_000), None)]),
    };
    assert!(large.approx_heap_size() > small.approx_heap_size());
}
