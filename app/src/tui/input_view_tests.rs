use super::TuiInputView;

fn view(text: &str, cursor: usize) -> TuiInputView {
    TuiInputView {
        draft: text.to_owned(),
        cursor,
    }
}

#[test]
fn inserts_at_the_cursor() {
    let mut v = view("ac", 1);
    v.insert("b");
    assert_eq!(v.draft, "abc");
    assert_eq!(v.cursor, 2);
}

#[test]
fn inserts_wide_characters_by_scalar() {
    let mut v = view("", 0);
    v.insert("世界");
    assert_eq!(v.draft, "世界");
    // The cursor advances by Unicode scalars, not display columns.
    assert_eq!(v.cursor, 2);
}

#[test]
fn backspace_removes_the_char_before_the_cursor() {
    let mut v = view("abc", 2);
    v.backspace();
    assert_eq!(v.draft, "ac");
    assert_eq!(v.cursor, 1);

    // At the start it is a no-op.
    let mut at_start = view("abc", 0);
    at_start.backspace();
    assert_eq!(at_start.draft, "abc");
    assert_eq!(at_start.cursor, 0);
}

#[test]
fn delete_removes_the_char_at_the_cursor() {
    let mut v = view("abc", 1);
    v.delete();
    assert_eq!(v.draft, "ac");
    assert_eq!(v.cursor, 1);

    // At the end it is a no-op.
    let mut at_end = view("abc", 3);
    at_end.delete();
    assert_eq!(at_end.draft, "abc");
}

#[test]
fn navigation_clamps_to_bounds() {
    let mut v = view("ab", 1);
    v.move_left();
    assert_eq!(v.cursor, 0);
    v.move_left();
    assert_eq!(v.cursor, 0);
    v.move_right();
    assert_eq!(v.cursor, 1);
    v.move_end();
    assert_eq!(v.cursor, 2);
    v.move_right();
    assert_eq!(v.cursor, 2);
    v.move_home();
    assert_eq!(v.cursor, 0);
}

#[test]
fn submission_returns_text_and_resets_for_the_next_draft() {
    let mut v = view("hello", 5);
    assert_eq!(v.take_submission(), Some("hello".to_owned()));
    assert_eq!(v.draft, "");
    assert_eq!(v.cursor, 0);
}

#[test]
fn whitespace_only_submission_is_suppressed() {
    let mut v = view("   ", 3);
    assert_eq!(v.take_submission(), None);
    // The draft is left untouched so the user can keep editing.
    assert_eq!(v.draft, "   ");
    assert_eq!(v.cursor, 3);
}
