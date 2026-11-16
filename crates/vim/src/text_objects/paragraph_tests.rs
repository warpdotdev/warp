use super::*;

#[test]
fn vim_inner_paragraph_empty_buffer() {
    assert_eq!(vim_inner_paragraph("", 0), Some(0.into()..0.into()));
    assert_eq!(vim_inner_paragraph("", 1), None);
}

#[test]
fn vim_a_paragraph_empty_buffer() {
    assert_eq!(vim_a_paragraph("", 0), Some(0.into()..0.into()));
    assert_eq!(vim_a_paragraph("", 1), None);
}

#[test]
fn vim_inner_paragraph_single_paragraph() {
    let text = "foo bar\nnext line\n";

    for (i, ch) in text.chars().enumerate() {
        if ch != '\n' {
            let range = vim_inner_paragraph(text, i).unwrap();
            assert_eq!(range, 0.into()..text.len().into());
        }
    }
}

#[test]
fn vim_a_paragraph_single_paragraph() {
    let text = "foo bar\nnext line\n";

    for (i, ch) in text.chars().enumerate() {
        if ch != '\n' {
            let range = vim_a_paragraph(text, i).unwrap();
            assert_eq!(range, 0.into()..text.len().into());
        }
    }
}

#[test]
fn vim_inner_paragraph_two_paragraphs() {
    let text = "first line\nof first para\n\nsecond para line\n";
    let blank_index = text.find("\n\n").unwrap();
    let second_start = blank_index + 2;

    for (i, ch) in text.chars().take(blank_index).enumerate() {
        if ch != '\n' {
            let range = vim_inner_paragraph(text, i).unwrap();
            assert_eq!(range, 0.into()..blank_index.into());
        }
    }

    for (i, ch) in text.chars().enumerate().skip(second_start) {
        if ch != '\n' {
            let range = vim_inner_paragraph(text, i).unwrap();
            assert_eq!(range, second_start.into()..text.len().into());
        }
    }
}

#[test]
fn vim_inner_paragraph_three_paragraphs() {
    let text = "first\n\nsecond\n\nthird\n";
    let (first_blank, second_blank) = {
        let mut it = text.match_indices("\n\n");
        (it.next().unwrap().0, it.next().unwrap().0)
    };
    let second_start = first_blank + 2;
    let third_start = second_blank + 2;

    for (i, ch) in text.chars().take(first_blank).enumerate() {
        if ch != '\n' {
            let range = vim_inner_paragraph(text, i).unwrap();
            assert_eq!(range, 0.into()..first_blank.into());
        }
    }

    for (i, ch) in text
        .chars()
        .enumerate()
        .skip(second_start)
        .take(second_blank - second_start)
    {
        if ch != '\n' {
            let range = vim_inner_paragraph(text, i).unwrap();
            assert_eq!(range, second_start.into()..second_blank.into());
        }
    }

    for (i, ch) in text.chars().enumerate().skip(third_start) {
        if ch != '\n' {
            let range = vim_inner_paragraph(text, i).unwrap();
            assert_eq!(range, third_start.into()..text.len().into());
        }
    }
}

#[test]
fn vim_a_paragraph_two_paragraphs() {
    let text = "first line\nof first para\n\nsecond para line\n";
    let blank_index = text.find("\n\n").unwrap();

    for (i, ch) in text.chars().take(blank_index).enumerate() {
        if ch != '\n' {
            let range = vim_a_paragraph(text, i).unwrap();
            assert_eq!(range, 0.into()..(blank_index + 1).into());
        }
    }

    let second_range_start = (blank_index + 1).into();
    let second_range_end = text.len().into();
    let second_start = blank_index + 2;

    for (i, ch) in text.chars().enumerate().skip(second_start) {
        if ch != '\n' {
            let range = vim_a_paragraph(text, i).unwrap();
            assert_eq!(range, second_range_start..second_range_end);
        }
    }
}

#[test]
fn vim_a_paragraph_three_paragraphs() {
    let text = "first\n\nsecond\n\nthird\n";
    let (first_blank, second_blank) = {
        let mut it = text.match_indices("\n\n");
        (it.next().unwrap().0, it.next().unwrap().0)
    };
    let second_start = first_blank + 2;
    let third_start = second_blank + 2;

    for (i, ch) in text.chars().take(first_blank).enumerate() {
        if ch != '\n' {
            let range = vim_a_paragraph(text, i).unwrap();
            assert_eq!(range, 0.into()..(first_blank + 1).into());
        }
    }

    for (i, ch) in text
        .chars()
        .enumerate()
        .skip(second_start)
        .take(second_blank - second_start)
    {
        if ch != '\n' {
            let range = vim_a_paragraph(text, i).unwrap();
            assert_eq!(range, second_start.into()..(second_blank + 1).into());
        }
    }

    let last_range = (second_blank + 1).into()..text.len().into();
    for (i, ch) in text.chars().enumerate().skip(third_start) {
        if ch != '\n' {
            let range = vim_a_paragraph(text, i).unwrap();
            assert_eq!(range, last_range);
        }
    }
}

#[test]
fn vim_inner_paragraph_blank_lines() {
    let text = "first\n\n\nsecond\n";

    for offset in 5..=7 {
        let range = vim_inner_paragraph(text, offset).unwrap();
        assert_eq!(range, 6.into()..7.into());
    }
}

#[test]
fn vim_a_paragraph_blank_lines() {
    let text = "first\n\n\nsecond\n";

    for offset in 5..=7 {
        let range = vim_a_paragraph(text, offset).unwrap();
        assert_eq!(range, 6.into()..text.len().into());
    }
}

#[test]
fn vim_a_paragraph_many_trailing_blank_lines() {
    let text = "first\n\n\nsecond\n\n\n\n\nthird";

    for offset in 0..=4 {
        let range = vim_a_paragraph(text, offset).unwrap();
        assert_eq!(range, 0.into()..7.into());
    }

    for offset in 8..=13 {
        let range = vim_a_paragraph(text, offset).unwrap();
        assert_eq!(range, 8.into()..18.into());
    }

    for offset in 19..=23 {
        let range = vim_a_paragraph(text, offset).unwrap();
        assert_eq!(range, 15.into()..24.into());
    }
}

#[test]
fn vim_paragraph_lines_with_spaces_included() {
    // Despite being invisible, a line containing spaces still counts as "content".
    let text = "first\n\n    \nsecond";

    let range = vim_a_paragraph(text, 3).unwrap();
    assert_eq!(range, 0.into()..6.into());
    let range = vim_a_paragraph(text, 14).unwrap();
    assert_eq!(range, 6.into()..18.into());
}
