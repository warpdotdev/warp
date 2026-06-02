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
