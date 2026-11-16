use super::*;
use crate::vim::QuoteType;

#[test]
fn test_a_quote() {
    assert_eq!(vim_a_quote("", 0, QuoteType::Single), None);
    assert_eq!(
        vim_a_quote("'foo'", 1, QuoteType::Single).unwrap(),
        0.into()..5.into()
    );
    assert_eq!(
        vim_a_quote("'foo'", 0, QuoteType::Single).unwrap(),
        0.into()..5.into()
    );
    assert_eq!(
        vim_a_quote("'foo'", 4, QuoteType::Single).unwrap(),
        0.into()..5.into()
    );
    assert_eq!(vim_a_quote("'foo'", 1, QuoteType::Double), None);
    assert_eq!(vim_a_quote("'foo'  ", 5, QuoteType::Single), None);
    assert_eq!(
        vim_a_quote("foo  'foo' ", 0, QuoteType::Single).unwrap(),
        5.into()..10.into()
    );
    assert_eq!(
        vim_a_quote(r#"foo  "" "#, 0, QuoteType::Double).unwrap(),
        5.into()..7.into()
    );

    //          0000000000111111111
    //          0123456789012345678
    let line = "  'foo'  'bar' 'baz";
    for i in 0..=6 {
        assert_eq!(
            vim_a_quote(line, i, QuoteType::Single).unwrap(),
            2.into()..7.into()
        )
    }
    for i in 7..=8 {
        assert_eq!(
            vim_a_quote(line, i, QuoteType::Single).unwrap(),
            6.into()..10.into()
        )
    }
    for i in 9..=13 {
        assert_eq!(
            vim_a_quote(line, i, QuoteType::Single).unwrap(),
            9.into()..14.into()
        )
    }
    assert_eq!(
        vim_a_quote(line, 14, QuoteType::Single).unwrap(),
        13.into()..16.into()
    );
    for i in 15..=18 {
        assert_eq!(vim_a_quote(line, i, QuoteType::Single), None)
    }
}

#[test]
fn test_inner_quote() {
    assert_eq!(vim_inner_quote("", 0, QuoteType::Single), None);
    assert_eq!(
        vim_inner_quote("`foo`", 1, QuoteType::Backtick).unwrap(),
        1.into()..4.into()
    );
    assert_eq!(
        vim_inner_quote("`foo`", 0, QuoteType::Backtick).unwrap(),
        1.into()..4.into()
    );
    assert_eq!(
        vim_inner_quote("'foo'", 4, QuoteType::Single).unwrap(),
        1.into()..4.into()
    );
    assert_eq!(vim_inner_quote("'foo'", 1, QuoteType::Double), None);
    assert_eq!(vim_inner_quote("'foo'  ", 5, QuoteType::Single), None);
    assert_eq!(
        vim_inner_quote("foo  'foo' ", 0, QuoteType::Single).unwrap(),
        6.into()..9.into()
    );
    assert_eq!(
        vim_inner_quote(r#"foo  "" "#, 0, QuoteType::Double).unwrap(),
        6.into()..6.into()
    );

    //          0000000000111111111
    //          0123456789012345678
    let line = "  'foo'  'bar' 'baz";
    for i in 0..=6 {
        assert_eq!(
            vim_inner_quote(line, i, QuoteType::Single).unwrap(),
            3.into()..6.into()
        )
    }
    for i in 7..=8 {
        assert_eq!(
            vim_inner_quote(line, i, QuoteType::Single).unwrap(),
            7.into()..9.into()
        )
    }
    for i in 9..=13 {
        assert_eq!(
            vim_inner_quote(line, i, QuoteType::Single).unwrap(),
            10.into()..13.into()
        )
    }
    assert_eq!(
        vim_inner_quote(line, 14, QuoteType::Single).unwrap(),
        14.into()..15.into()
    );
    for i in 15..=18 {
        assert_eq!(vim_inner_quote(line, i, QuoteType::Single), None)
    }
}
