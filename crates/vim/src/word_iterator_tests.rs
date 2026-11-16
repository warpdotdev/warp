use super::*;

//                  0000000000111111111122222222223333333333444444444455555555556666666666
//                  0123456789012345678901234567890123456789012345678901234567890123456789
const LINE: &str = "impl<'a, T: TextBuffer + ?Sized + 'a>   Iterator for WordBoundariesVim";

/// The behavior for pressing "w".
#[test]
fn test_word_forward_heads() {
    let mut iter1 = vim_word_iterator_from_offset(
        0,
        LINE,
        Direction::Forward,
        WordBound::Start,
        WordType::Default,
    )
    .unwrap();

    assert_eq!(iter1.next(), Some(4.into()));
    assert_eq!(iter1.next(), Some(6.into()));
    assert_eq!(iter1.next(), Some(7.into()));
    assert_eq!(iter1.next(), Some(9.into()));
    assert_eq!(iter1.next(), Some(10.into()));
    assert_eq!(iter1.next(), Some(12.into()));
    assert_eq!(iter1.next(), Some(23.into()));
    assert_eq!(iter1.next(), Some(25.into()));
    assert_eq!(iter1.next(), Some(26.into()));
    assert_eq!(iter1.next(), Some(32.into()));
    assert_eq!(iter1.next(), Some(34.into()));
    assert_eq!(iter1.next(), Some(35.into()));
    assert_eq!(iter1.next(), Some(36.into()));
    assert_eq!(iter1.next(), Some(40.into()));
    assert_eq!(iter1.next(), Some(49.into()));
    assert_eq!(iter1.next(), Some(53.into()));
    assert_eq!(iter1.next(), Some(70.into()));
    assert_eq!(iter1.next(), None);

    let mut iter2 = vim_word_iterator_from_offset(
        15,
        LINE,
        Direction::Forward,
        WordBound::Start,
        WordType::Default,
    )
    .unwrap();
    assert_eq!(iter2.next(), Some(23.into()));

    let mut iter3 = vim_word_iterator_from_offset(
        43,
        LINE,
        Direction::Forward,
        WordBound::Start,
        WordType::Default,
    )
    .unwrap();
    assert_eq!(iter3.next(), Some(49.into()));

    let mut iter4 = vim_word_iterator_from_offset(
        38,
        LINE,
        Direction::Forward,
        WordBound::Start,
        WordType::Default,
    )
    .unwrap();
    assert_eq!(iter4.next(), Some(40.into()));

    let mut iter5 = vim_word_iterator_from_offset(
        69,
        LINE,
        Direction::Forward,
        WordBound::Start,
        WordType::Default,
    )
    .unwrap();
    assert_eq!(iter5.next(), Some(70.into()));
    assert_eq!(iter5.next(), None);
}

/// The behavior for pressing "W".
#[test]
fn test_word_forward_heads_including_symbols() {
    let mut iter1 = vim_word_iterator_from_offset(
        0,
        LINE,
        Direction::Forward,
        WordBound::Start,
        WordType::BigWord,
    )
    .unwrap();

    assert_eq!(iter1.next(), Some(9.into()));
    assert_eq!(iter1.next(), Some(12.into()));
    assert_eq!(iter1.next(), Some(23.into()));
    assert_eq!(iter1.next(), Some(25.into()));
    assert_eq!(iter1.next(), Some(32.into()));
    assert_eq!(iter1.next(), Some(34.into()));
    assert_eq!(iter1.next(), Some(40.into()));
    assert_eq!(iter1.next(), Some(49.into()));
    assert_eq!(iter1.next(), Some(53.into()));
    assert_eq!(iter1.next(), Some(70.into()));
    assert_eq!(iter1.next(), None);

    let mut iter2 = vim_word_iterator_from_offset(
        15,
        LINE,
        Direction::Forward,
        WordBound::Start,
        WordType::Default,
    )
    .unwrap();
    assert_eq!(iter2.next(), Some(23.into()));

    let mut iter3 = vim_word_iterator_from_offset(
        43,
        LINE,
        Direction::Forward,
        WordBound::Start,
        WordType::Default,
    )
    .unwrap();
    assert_eq!(iter3.next(), Some(49.into()));

    let mut iter4 = vim_word_iterator_from_offset(
        38,
        LINE,
        Direction::Forward,
        WordBound::Start,
        WordType::Default,
    )
    .unwrap();
    assert_eq!(iter4.next(), Some(40.into()));

    let mut iter5 = vim_word_iterator_from_offset(
        69,
        LINE,
        Direction::Forward,
        WordBound::Start,
        WordType::Default,
    )
    .unwrap();
    assert_eq!(iter5.next(), Some(70.into()));
    assert_eq!(iter5.next(), None);
}

/// The behavior for pressing "ge"
#[test]
fn test_word_backward_heads() {
    let mut iter1 = vim_word_iterator_from_offset(
        69,
        LINE,
        Direction::Backward,
        WordBound::End,
        WordType::Default,
    )
    .unwrap();

    assert_eq!(iter1.next(), Some(51.into()));
    assert_eq!(iter1.next(), Some(47.into()));
    assert_eq!(iter1.next(), Some(36.into()));
    assert_eq!(iter1.next(), Some(35.into()));
    assert_eq!(iter1.next(), Some(34.into()));
    assert_eq!(iter1.next(), Some(32.into()));
    assert_eq!(iter1.next(), Some(30.into()));
    assert_eq!(iter1.next(), Some(25.into()));
    assert_eq!(iter1.next(), Some(23.into()));
    assert_eq!(iter1.next(), Some(21.into()));
    assert_eq!(iter1.next(), Some(10.into()));
    assert_eq!(iter1.next(), Some(9.into()));
    assert_eq!(iter1.next(), Some(7.into()));
    assert_eq!(iter1.next(), Some(6.into()));
    assert_eq!(iter1.next(), Some(5.into()));
    assert_eq!(iter1.next(), Some(3.into()));
    assert_eq!(iter1.next(), Some(0.into()));
    assert_eq!(iter1.next(), None);

    let mut iter2 = vim_word_iterator_from_offset(
        15,
        LINE,
        Direction::Backward,
        WordBound::End,
        WordType::Default,
    )
    .unwrap();
    assert_eq!(iter2.next(), Some(10.into()));

    let mut iter3 = vim_word_iterator_from_offset(
        43,
        LINE,
        Direction::Backward,
        WordBound::End,
        WordType::Default,
    )
    .unwrap();
    assert_eq!(iter3.next(), Some(36.into()));

    let mut iter4 = vim_word_iterator_from_offset(
        38,
        LINE,
        Direction::Backward,
        WordBound::End,
        WordType::Default,
    )
    .unwrap();
    assert_eq!(iter4.next(), Some(36.into()));

    let mut iter5 = vim_word_iterator_from_offset(
        0,
        LINE,
        Direction::Backward,
        WordBound::End,
        WordType::Default,
    )
    .unwrap();
    assert_eq!(iter5.next(), Some(0.into()));
    assert_eq!(iter5.next(), None);
}

/// The behavior for pressing "gE"
#[test]
fn test_word_backward_heads_including_symbols() {
    let mut iter1 = vim_word_iterator_from_offset(
        69,
        LINE,
        Direction::Backward,
        WordBound::End,
        WordType::BigWord,
    )
    .unwrap();

    assert_eq!(iter1.next(), Some(51.into()));
    assert_eq!(iter1.next(), Some(47.into()));
    assert_eq!(iter1.next(), Some(36.into()));
    assert_eq!(iter1.next(), Some(32.into()));
    assert_eq!(iter1.next(), Some(30.into()));
    assert_eq!(iter1.next(), Some(23.into()));
    assert_eq!(iter1.next(), Some(21.into()));
    assert_eq!(iter1.next(), Some(10.into()));
    assert_eq!(iter1.next(), Some(7.into()));
    assert_eq!(iter1.next(), Some(0.into()));
    assert_eq!(iter1.next(), None);

    let mut iter2 = vim_word_iterator_from_offset(
        15,
        LINE,
        Direction::Backward,
        WordBound::End,
        WordType::Default,
    )
    .unwrap();
    assert_eq!(iter2.next(), Some(10.into()));

    let mut iter3 = vim_word_iterator_from_offset(
        43,
        LINE,
        Direction::Backward,
        WordBound::End,
        WordType::Default,
    )
    .unwrap();
    assert_eq!(iter3.next(), Some(36.into()));

    let mut iter4 = vim_word_iterator_from_offset(
        38,
        LINE,
        Direction::Backward,
        WordBound::End,
        WordType::Default,
    )
    .unwrap();
    assert_eq!(iter4.next(), Some(36.into()));

    let mut iter5 = vim_word_iterator_from_offset(
        0,
        LINE,
        Direction::Backward,
        WordBound::End,
        WordType::Default,
    )
    .unwrap();
    assert_eq!(iter5.next(), Some(0.into()));
    assert_eq!(iter5.next(), None);
}

/// The behavior for pressing "e"
#[test]
fn test_word_forward_tails() {
    let mut iter1 = vim_word_iterator_from_offset(
        0,
        LINE,
        Direction::Forward,
        WordBound::End,
        WordType::Default,
    )
    .unwrap();

    assert_eq!(iter1.next(), Some(3.into()));
    assert_eq!(iter1.next(), Some(5.into()));
    assert_eq!(iter1.next(), Some(6.into()));
    assert_eq!(iter1.next(), Some(7.into()));
    assert_eq!(iter1.next(), Some(9.into()));
    assert_eq!(iter1.next(), Some(10.into()));
    assert_eq!(iter1.next(), Some(21.into()));
    assert_eq!(iter1.next(), Some(23.into()));
    assert_eq!(iter1.next(), Some(25.into()));
    assert_eq!(iter1.next(), Some(30.into()));
    assert_eq!(iter1.next(), Some(32.into()));
    assert_eq!(iter1.next(), Some(34.into()));
    assert_eq!(iter1.next(), Some(35.into()));
    assert_eq!(iter1.next(), Some(36.into()));
    assert_eq!(iter1.next(), Some(47.into()));
    assert_eq!(iter1.next(), Some(51.into()));
    assert_eq!(iter1.next(), Some(69.into()));
    assert_eq!(iter1.next(), None);

    let mut iter2 = vim_word_iterator_from_offset(
        15,
        LINE,
        Direction::Forward,
        WordBound::End,
        WordType::Default,
    )
    .unwrap();
    assert_eq!(iter2.next(), Some(21.into()));

    let mut iter3 = vim_word_iterator_from_offset(
        43,
        LINE,
        Direction::Forward,
        WordBound::End,
        WordType::Default,
    )
    .unwrap();
    assert_eq!(iter3.next(), Some(47.into()));

    let mut iter4 = vim_word_iterator_from_offset(
        38,
        LINE,
        Direction::Forward,
        WordBound::End,
        WordType::Default,
    )
    .unwrap();
    assert_eq!(iter4.next(), Some(47.into()));

    let mut iter5 = vim_word_iterator_from_offset(
        69,
        LINE,
        Direction::Forward,
        WordBound::End,
        WordType::Default,
    )
    .unwrap();
    assert_eq!(iter5.next(), Some(69.into()));
    assert_eq!(iter5.next(), None);
}

/// The behavior for pressing "E"
#[test]
fn test_word_forward_tails_including_symbols() {
    let mut iter1 = vim_word_iterator_from_offset(
        0,
        LINE,
        Direction::Forward,
        WordBound::End,
        WordType::BigWord,
    )
    .unwrap();

    assert_eq!(iter1.next(), Some(7.into()));
    assert_eq!(iter1.next(), Some(10.into()));
    assert_eq!(iter1.next(), Some(21.into()));
    assert_eq!(iter1.next(), Some(23.into()));
    assert_eq!(iter1.next(), Some(30.into()));
    assert_eq!(iter1.next(), Some(32.into()));
    assert_eq!(iter1.next(), Some(36.into()));
    assert_eq!(iter1.next(), Some(47.into()));
    assert_eq!(iter1.next(), Some(51.into()));
    assert_eq!(iter1.next(), Some(69.into()));
    assert_eq!(iter1.next(), None);

    let mut iter2 = vim_word_iterator_from_offset(
        15,
        LINE,
        Direction::Forward,
        WordBound::End,
        WordType::Default,
    )
    .unwrap();
    assert_eq!(iter2.next(), Some(21.into()));

    let mut iter3 = vim_word_iterator_from_offset(
        43,
        LINE,
        Direction::Forward,
        WordBound::End,
        WordType::Default,
    )
    .unwrap();
    assert_eq!(iter3.next(), Some(47.into()));

    let mut iter4 = vim_word_iterator_from_offset(
        38,
        LINE,
        Direction::Forward,
        WordBound::End,
        WordType::Default,
    )
    .unwrap();
    assert_eq!(iter4.next(), Some(47.into()));

    let mut iter5 = vim_word_iterator_from_offset(
        69,
        LINE,
        Direction::Forward,
        WordBound::End,
        WordType::Default,
    )
    .unwrap();
    assert_eq!(iter5.next(), Some(69.into()));
    assert_eq!(iter5.next(), None);
}

/// The behavior for pressing "b"
#[test]
fn test_word_backward_tails() {
    let mut iter1 = vim_word_iterator_from_offset(
        69,
        LINE,
        Direction::Backward,
        WordBound::Start,
        WordType::Default,
    )
    .unwrap();

    assert_eq!(iter1.next(), Some(53.into()));
    assert_eq!(iter1.next(), Some(49.into()));
    assert_eq!(iter1.next(), Some(40.into()));
    assert_eq!(iter1.next(), Some(36.into()));
    assert_eq!(iter1.next(), Some(35.into()));
    assert_eq!(iter1.next(), Some(34.into()));
    assert_eq!(iter1.next(), Some(32.into()));
    assert_eq!(iter1.next(), Some(26.into()));
    assert_eq!(iter1.next(), Some(25.into()));
    assert_eq!(iter1.next(), Some(23.into()));
    assert_eq!(iter1.next(), Some(12.into()));
    assert_eq!(iter1.next(), Some(10.into()));
    assert_eq!(iter1.next(), Some(9.into()));
    assert_eq!(iter1.next(), Some(7.into()));
    assert_eq!(iter1.next(), Some(6.into()));
    assert_eq!(iter1.next(), Some(4.into()));
    assert_eq!(iter1.next(), Some(0.into()));
    assert_eq!(iter1.next(), None);

    let mut iter2 = vim_word_iterator_from_offset(
        15,
        LINE,
        Direction::Backward,
        WordBound::Start,
        WordType::Default,
    )
    .unwrap();
    assert_eq!(iter2.next(), Some(12.into()));

    let mut iter3 = vim_word_iterator_from_offset(
        43,
        LINE,
        Direction::Backward,
        WordBound::Start,
        WordType::Default,
    )
    .unwrap();
    assert_eq!(iter3.next(), Some(40.into()));

    let mut iter4 = vim_word_iterator_from_offset(
        38,
        LINE,
        Direction::Backward,
        WordBound::Start,
        WordType::Default,
    )
    .unwrap();
    assert_eq!(iter4.next(), Some(36.into()));

    let mut iter5 = vim_word_iterator_from_offset(
        0,
        LINE,
        Direction::Backward,
        WordBound::Start,
        WordType::Default,
    )
    .unwrap();
    assert_eq!(iter5.next(), Some(0.into()));
    assert_eq!(iter5.next(), None);
}

/// The behavior for pressing "B"
#[test]
fn test_word_backward_tails_including_symbols() {
    let mut iter1 = vim_word_iterator_from_offset(
        69,
        LINE,
        Direction::Backward,
        WordBound::Start,
        WordType::BigWord,
    )
    .unwrap();

    assert_eq!(iter1.next(), Some(53.into()));
    assert_eq!(iter1.next(), Some(49.into()));
    assert_eq!(iter1.next(), Some(40.into()));
    assert_eq!(iter1.next(), Some(34.into()));
    assert_eq!(iter1.next(), Some(32.into()));
    assert_eq!(iter1.next(), Some(25.into()));
    assert_eq!(iter1.next(), Some(23.into()));
    assert_eq!(iter1.next(), Some(12.into()));
    assert_eq!(iter1.next(), Some(9.into()));
    assert_eq!(iter1.next(), Some(0.into()));
    assert_eq!(iter1.next(), None);

    let mut iter2 = vim_word_iterator_from_offset(
        15,
        LINE,
        Direction::Backward,
        WordBound::Start,
        WordType::Default,
    )
    .unwrap();
    assert_eq!(iter2.next(), Some(12.into()));

    let mut iter3 = vim_word_iterator_from_offset(
        43,
        LINE,
        Direction::Backward,
        WordBound::Start,
        WordType::Default,
    )
    .unwrap();
    assert_eq!(iter3.next(), Some(40.into()));

    let mut iter4 = vim_word_iterator_from_offset(
        38,
        LINE,
        Direction::Backward,
        WordBound::Start,
        WordType::Default,
    )
    .unwrap();
    assert_eq!(iter4.next(), Some(36.into()));

    let mut iter5 = vim_word_iterator_from_offset(
        0,
        LINE,
        Direction::Backward,
        WordBound::Start,
        WordType::Default,
    )
    .unwrap();
    assert_eq!(iter5.next(), Some(0.into()));
    assert_eq!(iter5.next(), None);
}

#[test]
fn test_out_of_bounds_is_error() {
    assert!(
        vim_word_iterator_from_offset(
            70,
            LINE,
            Direction::Backward,
            WordBound::Start,
            WordType::BigWord
        )
        .is_err()
    );
}
