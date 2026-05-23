use super::*;

/*
0 1 2 3
w a r p
-------
0     4  << the span for the string "black" is (0, 4)

Spanned {
    item: String::new("black"),  << black string
    span: Span::new(0, 4)       << span
}

or >> String::new("black").spanned(Span::new(0, 4))        */
fn black_word() -> Spanned<String> {
    String::from("black").spanned(Span::new(0, 4))
}

fn empty() -> Spanned<String> {
    String::new().spanned_unknown()
}

#[test]
fn knows_distances() {
    assert!(black_word().span.distance() == 4);
    assert!(empty().span.distance() == 0);
}
