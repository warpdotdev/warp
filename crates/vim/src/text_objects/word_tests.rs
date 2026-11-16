use super::*;

#[test]
fn test_vim_inner_word() {
    assert_eq!(vim_inner_word("", 0, WordType::Default), None);
    assert_eq!(
        vim_inner_word("foo", 0, WordType::Default).unwrap(),
        0.into()..3.into()
    );
    assert_eq!(
        vim_inner_word("foo<<?>>", 4, WordType::Default).unwrap(),
        3.into()..8.into()
    );

    //          000000000011111111112222222222333333333344444444445555555555666666666677
    //          012345678901234567890123456789012345678901234567890123456789012345678901
    let line = "impl<'a, T: TextBuffer + ?Sized + 'a>   Iterator for WordBoundariesVim {";
    for i in 0..=3 {
        assert_eq!(
            vim_inner_word(line, i, WordType::Default).unwrap(),
            0.into()..4.into()
        );
    }
    for i in 4..=5 {
        assert_eq!(
            vim_inner_word(line, i, WordType::Default).unwrap(),
            4.into()..6.into()
        );
    }
    assert_eq!(
        vim_inner_word(line, 6, WordType::Default).unwrap(),
        6.into()..7.into()
    );
    assert_eq!(
        vim_inner_word(line, 7, WordType::Default).unwrap(),
        7.into()..8.into()
    );
    assert_eq!(
        vim_inner_word(line, 8, WordType::Default).unwrap(),
        8.into()..9.into()
    );
    assert_eq!(
        vim_inner_word(line, 9, WordType::Default).unwrap(),
        9.into()..10.into()
    );
    assert_eq!(
        vim_inner_word(line, 10, WordType::Default).unwrap(),
        10.into()..11.into()
    );
    for i in 12..=21 {
        assert_eq!(
            vim_inner_word(line, i, WordType::Default).unwrap(),
            12.into()..22.into()
        );
    }
    assert_eq!(
        vim_inner_word(line, 22, WordType::Default).unwrap(),
        22.into()..23.into()
    );
    assert_eq!(
        vim_inner_word(line, 23, WordType::Default).unwrap(),
        23.into()..24.into()
    );
    for i in 26..=30 {
        assert_eq!(
            vim_inner_word(line, i, WordType::Default).unwrap(),
            26.into()..31.into()
        );
    }
    for i in 37..=39 {
        assert_eq!(
            vim_inner_word(line, i, WordType::Default).unwrap(),
            37.into()..40.into()
        );
    }
    for i in 40..=47 {
        assert_eq!(
            vim_inner_word(line, i, WordType::Default).unwrap(),
            40.into()..48.into()
        );
    }
    assert_eq!(
        vim_inner_word(line, 48, WordType::Default).unwrap(),
        48.into()..49.into()
    );
    assert_eq!(
        vim_inner_word(line, 71, WordType::Default).unwrap(),
        71.into()..72.into()
    );
}

#[test]
fn test_vim_a_word() {
    assert_eq!(vim_a_word("", 0, WordType::Default), None);
    assert_eq!(
        vim_a_word("foo", 0, WordType::Default).unwrap(),
        0.into()..3.into()
    );
    assert_eq!(
        vim_a_word("  foo", 0, WordType::Default).unwrap(),
        0.into()..5.into()
    );
    assert_eq!(
        vim_a_word("  foo", 1, WordType::Default).unwrap(),
        0.into()..5.into()
    );
    assert_eq!(
        vim_a_word("  foo", 3, WordType::Default).unwrap(),
        0.into()..5.into()
    );
    assert_eq!(
        vim_a_word("foo ", 1, WordType::Default).unwrap(),
        0.into()..4.into()
    );
    assert_eq!(
        vim_a_word("foo  ", 1, WordType::Default).unwrap(),
        0.into()..5.into()
    );
    assert_eq!(
        vim_a_word("foo  ", 3, WordType::Default).unwrap(),
        3.into()..5.into()
    );

    //          00000000001111111111222222222233333333334444444444555555
    //          01234567890123456789012345678901234567890123456789012345
    let line = "impl<T>  Thing<T> { fn foo(&self,foo   :&Foo, a:i32) {{{";

    for i in 0..=3 {
        assert_eq!(
            vim_a_word(line, i, WordType::Default).unwrap(),
            0.into()..4.into()
        );
    }
    assert_eq!(
        vim_a_word(line, 4, WordType::Default).unwrap(),
        4.into()..5.into()
    );
    assert_eq!(
        vim_a_word(line, 5, WordType::Default).unwrap(),
        5.into()..6.into()
    );
    assert_eq!(
        vim_a_word(line, 6, WordType::Default).unwrap(),
        6.into()..9.into()
    );
    for i in 7..=13 {
        assert_eq!(
            vim_a_word(line, i, WordType::Default).unwrap(),
            7.into()..14.into()
        );
    }
    assert_eq!(
        vim_a_word(line, 16, WordType::Default).unwrap(),
        16.into()..18.into()
    );
    assert_eq!(
        vim_a_word(line, 17, WordType::Default).unwrap(),
        17.into()..19.into()
    );
    assert_eq!(
        vim_a_word(line, 18, WordType::Default).unwrap(),
        18.into()..20.into()
    );
    assert_eq!(
        vim_a_word(line, 19, WordType::Default).unwrap(),
        19.into()..22.into()
    );
    for i in 20..=21 {
        assert_eq!(
            vim_a_word(line, i, WordType::Default).unwrap(),
            20.into()..23.into()
        );
    }
    assert_eq!(
        vim_a_word(line, 22, WordType::Default).unwrap(),
        22.into()..26.into()
    );
    for i in 23..=25 {
        assert_eq!(
            vim_a_word(line, i, WordType::Default).unwrap(),
            22.into()..26.into()
        );
    }
    for i in 26..=27 {
        assert_eq!(
            vim_a_word(line, i, WordType::Default).unwrap(),
            26.into()..28.into()
        );
    }
    for i in 28..=31 {
        assert_eq!(
            vim_a_word(line, i, WordType::Default).unwrap(),
            28.into()..32.into()
        );
    }
    assert_eq!(
        vim_a_word(line, 32, WordType::Default).unwrap(),
        32.into()..33.into()
    );
    for i in 33..=35 {
        assert_eq!(
            vim_a_word(line, i, WordType::Default).unwrap(),
            33.into()..39.into()
        );
    }
    for i in 36..=40 {
        assert_eq!(
            vim_a_word(line, i, WordType::Default).unwrap(),
            36.into()..41.into()
        );
    }
    for i in 41..=43 {
        assert_eq!(
            vim_a_word(line, i, WordType::Default).unwrap(),
            41.into()..44.into()
        );
    }
    assert_eq!(
        vim_a_word(line, 44, WordType::Default).unwrap(),
        44.into()..46.into()
    );
    for i in 45..=46 {
        assert_eq!(
            vim_a_word(line, i, WordType::Default).unwrap(),
            45.into()..47.into()
        );
    }
    assert_eq!(
        vim_a_word(line, 47, WordType::Default).unwrap(),
        47.into()..48.into()
    );
    for i in 48..=50 {
        assert_eq!(
            vim_a_word(line, i, WordType::Default).unwrap(),
            48.into()..51.into()
        );
    }
    assert_eq!(
        vim_a_word(line, 51, WordType::Default).unwrap(),
        51.into()..53.into()
    );
    assert_eq!(
        vim_a_word(line, 52, WordType::Default).unwrap(),
        52.into()..56.into()
    );
    for i in 53..=55 {
        assert_eq!(
            vim_a_word(line, i, WordType::Default).unwrap(),
            52.into()..56.into()
        );
    }
}

#[test]
fn test_vim_inner_bigword() {
    assert_eq!(vim_inner_word("", 0, WordType::BigWord), None);
    assert_eq!(
        vim_inner_word("foo", 0, WordType::BigWord).unwrap(),
        0.into()..3.into()
    );
    assert_eq!(
        vim_inner_word("foo<<?>>", 4, WordType::BigWord).unwrap(),
        0.into()..8.into()
    );

    //          000000000011111111112222222222333333333344444444445555555555666666666677
    //          012345678901234567890123456789012345678901234567890123456789012345678901
    let line = "impl<'a, T: TextBuffer + ?Sized + 'a>   Iterator for WordBoundariesVim {";
    for i in 0..=7 {
        assert_eq!(
            vim_inner_word(line, i, WordType::BigWord).unwrap(),
            0.into()..8.into()
        );
    }
    assert_eq!(
        vim_inner_word(line, 8, WordType::BigWord).unwrap(),
        8.into()..9.into()
    );
    for i in 9..=10 {
        assert_eq!(
            vim_inner_word(line, i, WordType::BigWord).unwrap(),
            9.into()..11.into()
        );
    }
    assert_eq!(
        vim_inner_word(line, 11, WordType::BigWord).unwrap(),
        11.into()..12.into()
    );
    for i in 12..=21 {
        assert_eq!(
            vim_inner_word(line, i, WordType::BigWord).unwrap(),
            12.into()..22.into()
        );
    }
    assert_eq!(
        vim_inner_word(line, 22, WordType::BigWord).unwrap(),
        22.into()..23.into()
    );
    assert_eq!(
        vim_inner_word(line, 23, WordType::BigWord).unwrap(),
        23.into()..24.into()
    );
    assert_eq!(
        vim_inner_word(line, 24, WordType::BigWord).unwrap(),
        24.into()..25.into()
    );
    for i in 25..=30 {
        assert_eq!(
            vim_inner_word(line, i, WordType::BigWord).unwrap(),
            25.into()..31.into()
        );
    }
    for i in 34..=36 {
        assert_eq!(
            vim_inner_word(line, i, WordType::BigWord).unwrap(),
            34.into()..37.into()
        );
    }
    for i in 37..=39 {
        assert_eq!(
            vim_inner_word(line, i, WordType::BigWord).unwrap(),
            37.into()..40.into()
        );
    }
    for i in 40..=47 {
        assert_eq!(
            vim_inner_word(line, i, WordType::BigWord).unwrap(),
            40.into()..48.into()
        );
    }
    assert_eq!(
        vim_inner_word(line, 48, WordType::BigWord).unwrap(),
        48.into()..49.into()
    );
    assert_eq!(
        vim_inner_word(line, 71, WordType::BigWord).unwrap(),
        71.into()..72.into()
    );
}

#[test]
fn test_vim_a_bigword() {
    assert_eq!(vim_a_word("", 0, WordType::BigWord), None);
    assert_eq!(
        vim_a_word("foo.bar", 0, WordType::BigWord).unwrap(),
        0.into()..7.into()
    );
    assert_eq!(
        vim_a_word("  foo.bar", 0, WordType::BigWord).unwrap(),
        0.into()..9.into()
    );
    assert_eq!(
        vim_a_word("  foo.bar", 1, WordType::BigWord).unwrap(),
        0.into()..9.into()
    );
    assert_eq!(
        vim_a_word("  foo.bar", 3, WordType::BigWord).unwrap(),
        0.into()..9.into()
    );
    assert_eq!(
        vim_a_word("foo.bar ", 1, WordType::BigWord).unwrap(),
        0.into()..8.into()
    );
    assert_eq!(
        vim_a_word("foo.bar  ", 1, WordType::BigWord).unwrap(),
        0.into()..9.into()
    );
    assert_eq!(
        vim_a_word("foo.bar  ", 7, WordType::BigWord).unwrap(),
        7.into()..9.into()
    );

    //          00000000001111111111222222222233333333334444444444555555
    //          01234567890123456789012345678901234567890123456789012345
    let line = "impl<T>  Thing<T> { fn foo(&self,foo   :&Foo, a:i32) {{{";

    for i in 0..=6 {
        assert_eq!(
            vim_a_word(line, i, WordType::BigWord).unwrap(),
            0.into()..9.into()
        );
    }
    for i in 7..=8 {
        assert_eq!(
            vim_a_word(line, i, WordType::BigWord).unwrap(),
            7.into()..17.into()
        );
    }
    for i in 9..=16 {
        assert_eq!(
            vim_a_word(line, i, WordType::BigWord).unwrap(),
            9.into()..18.into()
        );
    }
    assert_eq!(
        vim_a_word(line, 17, WordType::BigWord).unwrap(),
        17.into()..19.into()
    );
    assert_eq!(
        vim_a_word(line, 18, WordType::BigWord).unwrap(),
        18.into()..20.into()
    );
    assert_eq!(
        vim_a_word(line, 19, WordType::BigWord).unwrap(),
        19.into()..22.into()
    );
    for i in 20..=21 {
        assert_eq!(
            vim_a_word(line, i, WordType::BigWord).unwrap(),
            20.into()..23.into()
        );
    }
    assert_eq!(
        vim_a_word(line, 22, WordType::BigWord).unwrap(),
        22.into()..36.into()
    );
    for i in 23..=35 {
        assert_eq!(
            vim_a_word(line, i, WordType::BigWord).unwrap(),
            23.into()..39.into()
        );
    }
    for i in 36..=38 {
        assert_eq!(
            vim_a_word(line, i, WordType::BigWord).unwrap(),
            36.into()..45.into()
        );
    }
    for i in 39..=44 {
        assert_eq!(
            vim_a_word(line, i, WordType::BigWord).unwrap(),
            39.into()..46.into()
        );
    }
    assert_eq!(
        vim_a_word(line, 45, WordType::BigWord).unwrap(),
        45.into()..52.into()
    );
    for i in 46..=51 {
        assert_eq!(
            vim_a_word(line, i, WordType::BigWord).unwrap(),
            46.into()..53.into()
        );
    }
    for i in 52..=55 {
        assert_eq!(
            vim_a_word(line, i, WordType::BigWord).unwrap(),
            52.into()..56.into()
        );
    }
}
