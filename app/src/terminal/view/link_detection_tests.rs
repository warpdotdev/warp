use super::*;

#[test]
fn strip_suffix_from_possible_path_removes_sentence_period_from_range() {
    let possible_path = grid_handler::PossiblePath {
        path: CleanPathResult {
            path: "C:/Users/chris/warp-md-test.md.".to_string(),
            line_and_column_num: None,
        },
        range: Point::new(0, 11)..=Point::new(0, 42),
    };

    let (clean_path, range) =
        super::super::TerminalView::strip_suffix_from_possible_path(&possible_path, 80, ".")
            .expect("trailing period should produce a stripped candidate");

    assert_eq!(clean_path.path, "C:/Users/chris/warp-md-test.md");
    assert_eq!(range, Point::new(0, 11)..=Point::new(0, 41));
}
