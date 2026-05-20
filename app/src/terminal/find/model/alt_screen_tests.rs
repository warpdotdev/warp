use crate::terminal::{
    find::model::{alt_screen::run_find_on_alt_screen, FindOptions},
    model::index::Point,
    TerminalModel,
};

#[test]
fn test_run_find_on_alt_screen() {
    let mut mock_terminal_model = TerminalModel::mock(None, None);
    mock_terminal_model.set_altscreen_active();
    mock_terminal_model.process_bytes("foo\r\nbar foo\r\nfoo");

    let run = run_find_on_alt_screen(
        FindOptions {
            query: Some("foo".to_owned().into()),
            is_regex_enabled: false,
            is_case_sensitive: false,
            ..Default::default()
        },
        mock_terminal_model.alt_screen(),
    );

    assert_eq!(
        run.matches(),
        &[
            Point { row: 2, col: 0 }..=Point { row: 2, col: 2 },
            Point { row: 1, col: 4 }..=Point { row: 1, col: 6 },
            Point { row: 0, col: 0 }..=Point { row: 0, col: 2 },
        ]
    );
    assert_eq!(run.focused_match_index(), Some(0));
}

#[test]
fn test_run_find_on_alt_screen_case_sensitive() {
    let mut mock_terminal_model = TerminalModel::mock(None, None);
    mock_terminal_model.set_altscreen_active();
    mock_terminal_model.process_bytes("foo\r\nbar foo\r\nFoo");

    let run = run_find_on_alt_screen(
        FindOptions {
            query: Some("Foo".to_owned().into()),
            is_regex_enabled: false,
            is_case_sensitive: true,
            ..Default::default()
        },
        mock_terminal_model.alt_screen(),
    );

    assert_eq!(
        run.matches(),
        &[Point { row: 2, col: 0 }..=Point { row: 2, col: 2 },]
    );
    assert_eq!(run.focused_match_index(), Some(0));
}

#[test]
fn test_run_find_on_alt_screen_regex() {
    let mut mock_terminal_model = TerminalModel::mock(None, None);
    mock_terminal_model.set_altscreen_active();
    mock_terminal_model.process_bytes("aoo\r\nbar foo\r\nboo");

    let run = run_find_on_alt_screen(
        FindOptions {
            query: Some("[ab]oo".to_owned().into()),
            is_regex_enabled: true,
            is_case_sensitive: false,
            ..Default::default()
        },
        mock_terminal_model.alt_screen(),
    );

    assert_eq!(
        run.matches(),
        &[
            Point { row: 2, col: 0 }..=Point { row: 2, col: 2 },
            Point { row: 0, col: 0 }..=Point { row: 0, col: 2 },
        ]
    );
    assert_eq!(run.focused_match_index(), Some(0));
}
