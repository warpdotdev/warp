use vim::vim::{InsertPosition, VimMode};

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
    // `i` at cursor start: no cursor movement, just switch to Insert mode.
    if let TuiVimAction::ChangeModeToInsert(position) = action {
        assert_eq!(position, InsertPosition::AtCursor);
    } else {
        panic!("expected ChangeModeToInsert(AtCursor), got {action:?}");
    }
    assert_eq!(model.mode(), VimMode::Insert);
}

#[test]
fn v_in_normal_mode_enters_visual() {
    let mut model = TuiVimInputModel::new();
    model.process_special_key("escape"); // → Normal
    let action = model.process_char('v');
    // Visual mode is not Insert mode, so the transition is a plain ModeTransition.
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
    // `w` moves to the start of the next word
    assert_action!(action, TuiVimAction::MoveWordRightStart);
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
fn dd_kills_whole_line() {
    // `dd` must delete the whole line (move to start + kill to end),
    // not just from the cursor to end-of-line. This is the regression
    // test that validates the fix for the dd-only-kills-to-EOL bug.
    let mut model = TuiVimInputModel::new();
    model.process_special_key("escape"); // → Normal
    let _pending = model.process_char('d');
    let action = model.process_char('d');
    assert_action!(action, TuiVimAction::KillLine);
}

// ── Replace char ─────────────────────────────────────────────────

/// Regression test: `r<char>` must produce `ReplaceChar(c)`, not `DeleteForward`.
/// Before the fix, the replacement character was silently discarded.
///
/// Implementation note: pressing `r` in Normal mode transitions the FSA to
/// Replace mode (emitting `ModeTransition`). The NEXT character typed while in
/// Replace mode produces `ReplaceChar(Some(c))` which maps to
/// `TuiVimAction::ReplaceChar(c)` carrying the replacement character.
#[test]
fn r_char_produces_replace_char_action() {
    let mut model = TuiVimInputModel::new();
    model.process_special_key("escape"); // → Normal
    // Pressing `r` alone transitions to Replace mode (plain mode switch).
    let action = model.process_char('r');
    assert_action!(action, TuiVimAction::ModeTransition);
    assert!(matches!(model.mode(), vim::vim::VimMode::Replace));
    // The next character typed is the replacement char.
    let action = model.process_char('x');
    // The replacement action must carry the character.
    if let TuiVimAction::ReplaceChar(c) = action {
        assert_eq!(c, 'x', "replacement char must be preserved");
    } else {
        panic!("expected ReplaceChar('x'), got {action:?}");
    }
}

// ── Join line (J) ────────────────────────────────────────────────

/// Regression test: `J` (JoinLine) must map to `Unhandled` in the TUI, not
/// silently perform an unrelated operation like moving to end-of-line.
#[test]
fn j_capital_maps_to_unhandled() {
    let mut model = TuiVimInputModel::new();
    model.process_special_key("escape"); // → Normal
    let action = model.process_char('J');
    assert_action!(action, TuiVimAction::Unhandled);
}

// ── Mode change positions (a, A, I) ─────────────────────────────────

/// `a` must produce `ChangeModeToInsert(AfterCursor)` so the view moves the
/// cursor one position to the right before entering Insert mode.
#[test]
fn a_produces_change_mode_after_cursor() {
    let mut model = TuiVimInputModel::new();
    model.process_special_key("escape"); // → Normal
    let action = model.process_char('a');
    if let TuiVimAction::ChangeModeToInsert(position) = action {
        assert_eq!(position, InsertPosition::AfterCursor);
    } else {
        panic!("expected ChangeModeToInsert(AfterCursor), got {action:?}");
    }
    assert_eq!(model.mode(), VimMode::Insert);
}

/// `A` must produce `ChangeModeToInsert(LineEnd)` to move the cursor to end-of-line.
#[test]
fn capital_a_produces_change_mode_line_end() {
    let mut model = TuiVimInputModel::new();
    model.process_special_key("escape"); // → Normal
    let action = model.process_char('A');
    if let TuiVimAction::ChangeModeToInsert(position) = action {
        assert_eq!(position, InsertPosition::LineEnd);
    } else {
        panic!("expected ChangeModeToInsert(LineEnd), got {action:?}");
    }
    assert_eq!(model.mode(), VimMode::Insert);
}

/// `I` must produce `ChangeModeToInsert(LineFirstNonWhitespace)` to move to first
/// non-whitespace before entering Insert mode.
#[test]
fn capital_i_produces_change_mode_first_nonwhitespace() {
    let mut model = TuiVimInputModel::new();
    model.process_special_key("escape"); // → Normal
    let action = model.process_char('I');
    if let TuiVimAction::ChangeModeToInsert(position) = action {
        assert_eq!(position, InsertPosition::LineFirstNonWhitespace);
    } else {
        panic!("expected ChangeModeToInsert(LineFirstNonWhitespace), got {action:?}");
    }
    assert_eq!(model.mode(), VimMode::Insert);
}

// ── Count propagation ────────────────────────────────────────────────

/// `3h` must produce a `RepeatCount` wrapping `MoveLeft` with count=3, not a
/// bare `MoveLeft` that discards the count prefix.
#[test]
fn count_prefix_wraps_motion_in_repeat() {
    let mut model = TuiVimInputModel::new();
    model.process_special_key("escape"); // → Normal
    // Type '3' then 'h'
    let _pending = model.process_char('3');
    let action = model.process_char('h');
    if let TuiVimAction::RepeatCount { inner, count } = action {
        assert_eq!(count, 3);
        assert_action!(*inner, TuiVimAction::MoveLeft);
    } else {
        panic!("expected RepeatCount {{ MoveLeft, 3 }}, got {action:?}");
    }
}

// ── Yank buffer ─────────────────────────────────────────────────────

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
