use super::*;

#[test]
fn test_single_paragraph_returns_none_both_directions() {
    // Single paragraph: no double-newline separator anywhere
    let text = "p1 line1\np1 line2\np1 line3";
    let pos = text.find("line2").unwrap();

    assert_eq!(find_previous_paragraph_start(text, pos), None);
    assert_eq!(find_next_paragraph_end(text, pos), None);
}

#[test]
fn test_single_paragraph_surrounded_returns_prev_and_next() {
    // Single paragraph: no double-newline separator anywhere
    let text = "\n\n\np1 line1\np1 line2\np1 line3\n\n\n";
    let pos = text.find("line2").unwrap();

    // The first empty line after the end of the paragraph (the second newline character)
    let goal_end = text.find("line3").unwrap() + "line3".len() + 1;

    assert_eq!(find_previous_paragraph_start(text, pos), Some(2.into()));
    assert_eq!(find_next_paragraph_end(text, pos), Some(goal_end.into()));
}

#[test]
fn test_next_none_in_last_paragraph_previous_goes_up() {
    // Two paragraphs separated by one blank line (double newline)
    let text = "p1\n\np2";
    let pos_in_p2 = text.find("p2").unwrap();

    // previous should find the boundary before p2: index of the second newline in the pair
    // for both cursor positioning on the `p` and `2`
    assert_eq!(
        find_previous_paragraph_start(text, pos_in_p2),
        Some((pos_in_p2 - 1).into())
    );
    assert_eq!(
        find_previous_paragraph_start(text, pos_in_p2 + 1),
        Some((pos_in_p2 - 1).into())
    );
    // next should find nothing after the last paragraph
    assert_eq!(find_next_paragraph_end(text, pos_in_p2), None);
}

#[test]
fn test_previous_none_in_first_paragraph_next_goes_down() {
    let text = "p1\n\np2";
    let pos_in_p1 = text.find("p1").unwrap();

    // previous should find nothing before the first paragraph
    assert_eq!(find_previous_paragraph_start(text, pos_in_p1), None);
    // next should find the boundary after p1: index of the second newline in the pair
    // for both cursor positioning on the `p` and `1`
    assert_eq!(
        find_next_paragraph_end(text, pos_in_p1),
        Some((pos_in_p1 + "p1".len() + 1).into())
    );
    assert_eq!(
        find_next_paragraph_end(text, pos_in_p1 + 1),
        Some((pos_in_p1 + "p1".len() + 1).into())
    );
}

#[test]
fn test_three_paragraphs_lots_of_newlines_middle_para() {
    // Three paragraphs, separated by four newlines each
    let text = "p1\n\n\n\n".to_string() + "p2\n\n\n\n" + "p3";
    let pos_in_p2 = text.find("p2").unwrap();

    assert_eq!(
        find_previous_paragraph_start(text.as_str(), pos_in_p2),
        Some((pos_in_p2 - 1).into())
    );
    assert_eq!(
        find_next_paragraph_end(text.as_str(), pos_in_p2),
        Some((pos_in_p2 + "p2".len() + 1).into())
    );
}

#[test]
fn test_cursor_in_middle_of_newline_patch_quad_runs() {
    // Lots of newlines between paras, but cursor starts in the middle of the newlines
    let text = "p1\n\n\n\n\n\n".to_string() + "p2\n\n\n\n\n\n" + "p3";
    let pos_between_p1_p2 = "p1".len() + 3;
    let pos_p2 = text.find("p2").unwrap();
    let pos_between_p2_p3 = pos_p2 + "p2".len() + 3;

    // Place cursor amongst newlines between p1 and p2
    assert_eq!(
        find_previous_paragraph_start(text.as_str(), pos_between_p1_p2),
        None
    );
    assert_eq!(
        find_next_paragraph_end(text.as_str(), pos_between_p1_p2),
        Some((pos_p2 + "p2".len() + 1).into())
    );

    // Place cursor amongst newlines between p2 and p3
    assert_eq!(
        find_previous_paragraph_start(text.as_str(), pos_between_p2_p3),
        Some((pos_p2 - 1).into())
    );
    assert_eq!(
        find_next_paragraph_end(text.as_str(), pos_between_p2_p3),
        None
    );
}
