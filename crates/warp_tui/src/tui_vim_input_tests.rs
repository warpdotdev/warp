use vim::vim::VimMode;

use super::{TuiVimAction, TuiVimInputModel};

// ── Helper assertions ─────────────────────────────────────────────────────────

/// Assert that a `TuiVimAction` is the expected variant (by discriminant name).
macro_rules! assert_action {
    ($action:expr, $variant:pat) => {
        assert!(
            matches!($action, $variant),
            "expected action {:?}, got {:?}",
            stringify!($variant),
            $action,
        );
    };
}

// ── Mode tracking ─────────────────────────────────────────────────────────────

#[test]
fn new_model_starts_in_insert_mode() {
    let model = TuiVimInputModel::new();
    assert_eq!(model.mode(), VimMode::Insert);
}

#[test]
fn escape_from_insert_transitions_to_normal() {
    let mut model = TuiVimInputModel::new();
    assert_eq!(model.mode(), VimMode::Insert);
    model.process_special_key("escape");
    assert_eq!(model.mode(), VimMode::Normal);
}

#[test]
fn escape_from_normal_stays_normal() {
    let mut model = TuiVimInputModel::new();
    model.process_special_key("escape"); // Insert → Normal
    assert_eq!(model.mode(), VimMode::Normal);
    model.process_special_key("escape"); // Normal → Normal (clears pending)
    assert_eq!(model.mode(), VimMode::Normal);
}

#[test]
fn i_in_normal_mode_enters_insert() {
    let mut model = TuiVimInputModel::new();
    model.process_special_key("escape"); // → Normal
    let action = model.process_char('i');
    assert_action!(action, TuiVimAction::ModeTransition);
    assert_eq!(model.mode(), VimMode::Insert);
}

#[test]
fn v_in_normal_mode_enters_visual() {
    let mut model = TuiVimInputModel::new();
    model.process_special_key("escape"); // → Normal
    let action = model.process_char('v');
    assert_action!(action, TuiVimAction::ModeTransition);
    assert!(matches!(model.mode(), VimMode::Visual(_)));
}

#[test]
fn reset_to_insert_returns_insert_mode() {
    let mut model = TuiVimInputModel::new();
    model.process_special_key("escape"); // → Normal
    assert_eq!(model.mode(), VimMode::Normal);
    model.reset_to_insert();
    assert_eq!(model.mode(), VimMode::Insert);
}

// ── Insert mode pass-through ──────────────────────────────────────────────────

#[test]
fn insert_mode_chars_pass_through() {
    let mut model = TuiVimInputModel::new();
    // In insert mode, every printable char should become InsertChar.
    for c in "hello world".chars() {
        let action = model.process_char(c);
        assert_action!(action, TuiVimAction::InsertChar(_));
        if let TuiVimAction::InsertChar(got) = action {
            assert_eq!(got, c);
        }
    }
}

// ── Navigation in normal mode ─────────────────────────────────────────────────

#[test]
fn h_in_normal_mode_moves_left() {
    let mut model = TuiVimInputModel::new();
    model.process_special_key("escape"); // → Normal
    let action = model.process_char('h');
    assert_action!(action, TuiVimAction::MoveLeft);
}

#[test]
fn l_in_normal_mode_moves_right() {
    let mut model = TuiVimInputModel::new();
    model.process_special_key("escape"); // → Normal
    let action = model.process_char('l');
    assert_action!(action, TuiVimAction::MoveRight);
}

#[test]
fn k_in_normal_mode_moves_up() {
    let mut model = TuiVimInputModel::new();
    model.process_special_key("escape"); // → Normal
    let action = model.process_char('k');
    assert_action!(action, TuiVimAction::MoveUp);
}

#[test]
fn j_in_normal_mode_moves_down() {
    let mut model = TuiVimInputModel::new();
    model.process_special_key("escape"); // → Normal
    let action = model.process_char('j');
    assert_action!(action, TuiVimAction::MoveDown);
}

#[test]
fn w_in_normal_mode_moves_word_right() {
    let mut model = TuiVimInputModel::new();
    model.process_special_key("escape"); // → Normal
    let action = model.process_char('w');
    assert_action!(action, TuiVimAction::MoveWordRight);
}

#[test]
fn b_in_normal_mode_moves_word_left() {
    let mut model = TuiVimInputModel::new();
    model.process_special_key("escape"); // → Normal
    let action = model.process_char('b');
    assert_action!(action, TuiVimAction::MoveWordLeft);
}

#[test]
fn dollar_in_normal_mode_moves_to_line_end() {
    let mut model = TuiVimInputModel::new();
    model.process_special_key("escape"); // → Normal
    let action = model.process_char('$');
    assert_action!(action, TuiVimAction::MoveToLineEnd);
}

#[test]
fn caret_in_normal_mode_moves_to_first_non_whitespace() {
    let mut model = TuiVimInputModel::new();
    model.process_special_key("escape"); // → Normal
    let action = model.process_char('^');
    assert_action!(action, TuiVimAction::MoveToFirstNonWhitespace);
}

// ── Deletion in normal mode ───────────────────────────────────────────────────

#[test]
fn x_in_normal_mode_deletes_forward() {
    let mut model = TuiVimInputModel::new();
    model.process_special_key("escape"); // → Normal
    let action = model.process_char('x');
    assert_action!(action, TuiVimAction::DeleteForward);
}

#[test]
fn backspace_in_normal_mode_moves_left() {
    let mut model = TuiVimInputModel::new();
    model.process_special_key("escape"); // → Normal
    // In normal mode, backspace is a leftward character motion (like 'h').
    let action = model.process_special_key("backspace");
    assert_action!(action, TuiVimAction::MoveLeft);
}

// ── Undo ──────────────────────────────────────────────────────────────────────

#[test]
fn u_in_normal_mode_undoes() {
    let mut model = TuiVimInputModel::new();
    model.process_special_key("escape"); // → Normal
    let action = model.process_char('u');
    assert_action!(action, TuiVimAction::Undo);
}

// ── Pending commands ──────────────────────────────────────────────────────────

#[test]
fn d_alone_is_pending() {
    let mut model = TuiVimInputModel::new();
    model.process_special_key("escape"); // → Normal
    let action = model.process_char('d');
    assert_action!(action, TuiVimAction::Pending);
    assert!(model.has_pending());
}

#[test]
fn dd_kills_to_line_end() {
    let mut model = TuiVimInputModel::new();
    model.process_special_key("escape"); // → Normal
    let _pending = model.process_char('d');
    let action = model.process_char('d');
    assert_action!(action, TuiVimAction::KillToLineEnd);
}

// ── Yank buffer ───────────────────────────────────────────────────────────────

#[test]
fn yank_buffer_round_trip() {
    let mut model = TuiVimInputModel::new();
    assert_eq!(model.yank_buffer(), "");
    model.set_yank_buffer("hello".to_owned());
    assert_eq!(model.yank_buffer(), "hello");
}

#[test]
fn paste_after_returns_yank_buffer_content() {
    let mut model = TuiVimInputModel::new();
    model.set_yank_buffer("text".to_owned());
    model.process_special_key("escape"); // → Normal
    let action = model.process_char('p');
    if let TuiVimAction::PasteAfter(text) = action {
        assert_eq!(text, "text");
    } else {
        panic!("expected PasteAfter, got {action:?}");
    }
}

#[test]
fn paste_before_returns_yank_buffer_content() {
    let mut model = TuiVimInputModel::new();
    model.set_yank_buffer("text".to_owned());
    model.process_special_key("escape"); // → Normal
    let action = model.process_char('P');
    if let TuiVimAction::PasteBefore(text) = action {
        assert_eq!(text, "text");
    } else {
        panic!("expected PasteBefore, got {action:?}");
    }
}

#[test]
fn paste_with_empty_yank_buffer_is_unhandled() {
    let mut model = TuiVimInputModel::new();
    // yank buffer empty → paste is a no-op
    model.process_special_key("escape"); // → Normal
    let action = model.process_char('p');
    assert_action!(action, TuiVimAction::Unhandled);
}
