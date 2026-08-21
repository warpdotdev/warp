use super::*;

/*
0 1 2 3
w a r p
-------
0     4  << the span for the string "warp" is (0, 4)

Spanned {
    item: String::new("warp"),  << warp string
    span: Span::new(0, 4)       << span
}

or >> String::new("warp").spanned(Span::new(0, 4))        */
fn warp() -> Spanned<String> {
    String::from("warp").spanned(Span::new(0, 4))
}

fn empty() -> Spanned<String> {
    String::new().spanned_unknown()
}

#[test]
fn knows_distances() {
    assert!(warp().span.distance() == 4);
    assert!(empty().span.distance() == 0);
}

#[test]
fn slice_returns_the_exact_substring_for_a_well_formed_span() {
    assert_eq!(Span::new(0, 4).slice("warp terminal"), "warp");
    assert_eq!(Span::new(5, 13).slice("warp terminal"), "terminal");
}

#[test]
fn slice_does_not_panic_when_offsets_land_inside_a_multi_byte_character() {
    // Reproduces a real crash: a shell reporting offsets in a unit other than UTF-8 bytes
    // (PowerShell's own .NET UTF-16 code-unit ReplacementIndex/ReplacementLength, before the
    // client-side conversion at the OSC boundary) can land mid-character once the buffer
    // contains any multi-byte UTF-8 text before the completed token. "echo 中 Get-Ch" is 13
    // UTF-16 code units but 15 UTF-8 bytes; byte 7 (a UTF-16-unit-based offset used as a byte
    // offset) falls inside "中"'s 3-byte encoding (bytes 5..8).
    let source = "echo \u{4e2d} Get-Ch";
    assert_eq!(
        source.len(),
        15,
        "sanity check on the UTF-8 byte length used above"
    );
    assert!(
        !source.is_char_boundary(7),
        "sanity check: 7 is genuinely mid-character"
    );

    // Must not panic, regardless of what it returns.
    let _ = Span::new(7, 13).slice(source);
}

#[test]
fn slice_clamps_a_char_boundary_violation_to_the_nearest_valid_boundary_at_or_before_it() {
    let source = "echo \u{4e2d} Get-Ch";
    // Byte 7 is inside "中"'s 3-byte encoding (bytes 5..8), so it must clamp down to 5, the
    // start of that character, rather than either panicking or silently including a partial
    // character. Byte 13 already lands on a boundary (the start of "Get-Ch"'s "C", all
    // single-byte ASCII), so it's used unchanged.
    assert_eq!(Span::new(7, 13).slice(source), &source[5..13]);
}

#[test]
fn slice_clamps_out_of_bounds_offsets_to_the_end_of_the_string() {
    assert_eq!(Span::new(0, 1000).slice("warp"), "warp");
    assert_eq!(Span::new(1000, 2000).slice("warp"), "");
}

#[test]
fn slice_clamps_an_end_before_start_to_start_rather_than_underflowing() {
    // Not reachable through `Span::new` (which asserts end >= start), but `slice` itself
    // should not assume that invariant given how it's exercised here: defensively clamping
    // the end down to a char boundary must never leave it before the (already-clamped) start.
    let span = Span { start: 10, end: 2 };
    assert_eq!(span.slice("warp terminal"), "");
}
