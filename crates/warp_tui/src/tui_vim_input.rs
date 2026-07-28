//! Vim-mode input handling for the TUI prompt input.
//!
//! [`TuiVimInputModel`] wraps a [`VimFSA`] state machine and translates raw
//! keystrokes into high-level [`TuiVimAction`]s that the owning
//! [`TuiInputView`] applies to the backing [`CodeEditorModel`].
//!
//! When vim mode is disabled the model is still present but inert — all
//! character input falls through to the normal editor path.

use vim::vim::{
    CharacterMotion, Direction, InsertPosition, LineMotion, VimEventType, VimFSA, VimMode,
    VimMotion, VimOperand, VimOperator, WordBound,
};

/// A high-level action for the TUI to apply after processing a keystroke in
/// vim mode. Returned by [`TuiVimInputModel::process_char`] and
/// [`TuiVimInputModel::process_special_key`].
#[derive(Debug, Clone)]
pub(crate) enum TuiVimAction {
    /// Insert a printable character at the current cursor (insert mode only).
    InsertChar(char),
    /// Insert a string of text at the current cursor.
    InsertText(String),
    /// Delete the character before the cursor.
    Backspace,
    /// Delete the character at the cursor (forward).
    DeleteForward,
    /// Delete a word backward.
    DeleteWordBackward,
    /// Delete a word forward.
    DeleteWordForward,
    /// Move the cursor one character to the left.
    MoveLeft,
    /// Move the cursor one character to the right.
    MoveRight,
    /// Move the cursor one visual row up.
    MoveUp,
    /// Move the cursor one visual row down.
    MoveDown,
    /// Move the cursor one word to the left.
    MoveWordLeft,
    /// Move the cursor one word to the right (to start of next word; `w`).
    MoveWordRightStart,
    /// Move the cursor one word to the right to end of current word (`e`).
    MoveWordRightEnd,
    /// Move the cursor to the start of the current line.
    MoveToLineStart,
    /// Move the cursor to the end of the current line.
    MoveToLineEnd,
    /// Move the cursor to the first non-whitespace character on the line.
    MoveToFirstNonWhitespace,
    /// Move the cursor to the start of the input buffer (`gg`).
    MoveToBufferStart,
    /// Move the cursor to the end of the input buffer (`G`).
    MoveToBufferEnd,
    /// Delete from the cursor to the end of the line (kill).
    KillToLineEnd,
    /// Delete from the cursor to the start of the line (kill).
    KillToLineStart,
    /// Delete the entire current line: move to line start, then kill to end.
    /// Used for `dd` to match vim semantics regardless of cursor column.
    KillLine,
    /// Replace the character at the cursor with `c` (`r<char>`).
    ReplaceChar(char),
    /// Yank (copy) from the cursor to the end of the line.
    YankToLineEnd,
    /// Yank (copy) one word forward.
    YankWordForward,
    /// Yank (copy) the full buffer content into the vim clipboard (`yy`).
    YankBuffer,
    /// Paste the vim clipboard contents after the cursor.
    PasteAfter(String),
    /// Paste the vim clipboard contents before the cursor.
    PasteBefore(String),
    /// Undo the last edit.
    Undo,
    /// Switch to Insert mode with a cursor movement appropriate for the entry
    /// command (`i`, `a`, `A`, `I`, `o`, `O`).
    ChangeModeToInsert(InsertPosition),
    /// A vim mode transition occurred (new mode returned); no buffer edit needed.
    ModeTransition,
    /// The keystroke was consumed (pending input) but no action is ready yet.
    Pending,
    /// The keystroke was not handled in the current mode.
    Unhandled,
    /// Delete the visual selection (for `d` and `c` in visual mode).
    /// The view tracks the visual anchor; this action signals that the
    /// range [anchor, cursor] (or [cursor, anchor]) should be deleted.
    DeleteVisualSelection,
    /// Yank (copy) the visual selection without deleting it (for `y` in visual mode).
    YankVisualSelection,
    /// Repeat an inner action `count` times (for count-prefixed commands).
    RepeatCount {
        inner: Box<TuiVimAction>,
        count: usize,
    },
}

/// Vim-mode state machine for the TUI prompt.
///
/// Wraps a [`VimFSA`] and a small yank buffer. The caller drives it by passing
/// characters and special key names; it returns [`TuiVimAction`]s for the
/// caller to apply.
#[derive(Debug, Default)]
pub(crate) struct TuiVimInputModel {
    /// The underlying finite-state automaton.
    fsa: VimFSA,
    /// Internal yank / delete clipboard (separate from the OS clipboard so
    /// that `p`/`P` work even without clipboard access).
    yank_buffer: String,
}

impl TuiVimInputModel {
    pub(crate) fn new() -> Self {
        Self {
            fsa: VimFSA::new(),
            yank_buffer: String::new(),
        }
    }

    /// The current vim mode.
    pub(crate) fn mode(&self) -> VimMode {
        self.fsa.mode
    }

    /// Whether the FSA has pending input (an incomplete multi-key command).
    #[allow(dead_code)]
    pub(crate) fn has_pending(&self) -> bool {
        !self.fsa.state().showcmd.is_empty()
    }

    /// Return a borrowed view of the showcmd string (pending command buffer).
    #[allow(dead_code)]
    pub(crate) fn showcmd(&self) -> &str {
        self.fsa.state().showcmd
    }

    /// Reset the vim state machine to insert mode (used when vim mode is
    /// toggled off and back on, or when the input buffer is cleared).
    pub(crate) fn reset_to_insert(&mut self) {
        self.fsa = VimFSA::new();
        // VimFSA starts in Insert mode by default (see VimMode::default).
    }

    /// Process a printable character. Returns the action the caller should
    /// apply to the editor.
    ///
    /// In Insert mode the character is passed through verbatim. In Normal /
    /// Visual / Replace mode it goes through the VimFSA.
    pub(crate) fn process_char(&mut self, c: char) -> TuiVimAction {
        match self.fsa.mode {
            VimMode::Insert => TuiVimAction::InsertChar(c),
            _ => {
                let event = self.fsa.process_char(c);
                match event {
                    None => TuiVimAction::Pending,
                    Some(event) => self.map_event(event),
                }
            }
        }
    }

    /// Process a special key name (as used by the TUI keymap, e.g.
    /// `"escape"`, `"backspace"`, `"enter"`). Returns the action to apply.
    pub(crate) fn process_special_key(&mut self, key: &str) -> TuiVimAction {
        let event = self.fsa.process_keystroke(key);
        match event {
            None => {
                // escape with no pending command in normal mode → no-op
                TuiVimAction::ModeTransition
            }
            Some(event) => self.map_event(event),
        }
    }

    /// Map a `VimEventType` (wrapped in `VimEvent`) to a [`TuiVimAction`].
    fn map_event(&mut self, event: vim::vim::VimEvent) -> TuiVimAction {
        let count = event.count.max(1) as usize;
        match event.event_type {
            // ── Insert mode pass-throughs ──────────────────────────────────
            VimEventType::InsertChar(c) => TuiVimAction::InsertChar(c),

            // ── Escape / mode transitions ──────────────────────────────────
            VimEventType::Escape => TuiVimAction::ModeTransition,
            // `ChangeMode` is emitted for `i`, `a`, `A`, `I`, `o`, `O` (entering
            // Insert mode with a position) as well as `v`/`V` (Visual), `R`
            // (Replace), etc. Only Insert mode transitions carry a meaningful
            // cursor-movement position; all others are plain mode switches.
            VimEventType::ChangeMode { new, .. } => {
                if matches!(new.mode, VimMode::Insert) {
                    TuiVimAction::ChangeModeToInsert(new.position)
                } else {
                    TuiVimAction::ModeTransition
                }
            }

            // ── Undo ───────────────────────────────────────────────────────
            VimEventType::Undo => TuiVimAction::Undo,

            // ── Deletion ───────────────────────────────────────────────────
            VimEventType::Backspace => TuiVimAction::Backspace,
            VimEventType::DeleteForward => Self::maybe_repeat(TuiVimAction::DeleteForward, count),

            // ── Navigation ────────────────────────────────────────────────
            VimEventType::Navigate(motion) => self.map_motion(motion, count),

            // ── Operators (d, c, y) with operands ─────────────────────────
            VimEventType::Operation {
                operator, operand, ..
            } => self.map_operation(operator, operand, count),

            // ── Visual mode operators ──────────────────────────────────────
            VimEventType::VisualOperator { operator, .. } => {
                // Visual selection → apply operator to selection
                self.map_visual_operator(operator)
            }

            // ── Paste ─────────────────────────────────────────────────────
            VimEventType::Paste {
                direction: Direction::Forward,
                ..
            } => {
                if self.yank_buffer.is_empty() {
                    TuiVimAction::Unhandled
                } else {
                    TuiVimAction::PasteAfter(self.yank_buffer.clone())
                }
            }
            VimEventType::Paste {
                direction: Direction::Backward,
                ..
            } => {
                if self.yank_buffer.is_empty() {
                    TuiVimAction::Unhandled
                } else {
                    TuiVimAction::PasteBefore(self.yank_buffer.clone())
                }
            }

            // ── Text insertion (e.g. dot-repeat) ──────────────────────────
            VimEventType::InsertText { text, .. } => {
                if text.is_empty() {
                    TuiVimAction::ModeTransition
                } else {
                    TuiVimAction::InsertText(text)
                }
            }

            // ── Replace char (`r<char>`) ───────────────────────────────────
            // Delete the character at the cursor and insert the replacement.
            VimEventType::ReplaceChar(Some(c)) => TuiVimAction::ReplaceChar(c),
            VimEventType::ReplaceChar(None) => TuiVimAction::ModeTransition,

            // ── Join lines (J) ────────────────────────────────────────────
            // Joining lines is not meaningful in a single- or few-line TUI
            // prompt. Emit Unhandled rather than doing something visually
            // indistinguishable from an unrelated command.
            VimEventType::JoinLine => TuiVimAction::Unhandled,

            // ── Misc events we don't support ──────────────────────────────
            VimEventType::Search(_)
            | VimEventType::CycleSearch(_)
            | VimEventType::SearchWordAtCursor(_)
            | VimEventType::KeywordPrg
            | VimEventType::ExCommand
            | VimEventType::VisualPaste { .. }
            | VimEventType::VisualTextObject(_)
            | VimEventType::GotoDefinition
            | VimEventType::FindReferences
            | VimEventType::ShowHover
            | VimEventType::CenterCursorVertically
            | VimEventType::ScrollHalfPageDown
            | VimEventType::ScrollHalfPageUp
            | VimEventType::ToggleCase => TuiVimAction::Unhandled,
        }
    }

    /// Wrap `action` in a [`TuiVimAction::RepeatCount`] when `count > 1`.
    fn maybe_repeat(action: TuiVimAction, count: usize) -> TuiVimAction {
        if count <= 1 {
            action
        } else {
            TuiVimAction::RepeatCount {
                inner: Box::new(action),
                count,
            }
        }
    }

    fn map_motion(&self, motion: VimMotion, count: usize) -> TuiVimAction {
        let base = match motion {
            VimMotion::Character(CharacterMotion::Left)
            | VimMotion::Character(CharacterMotion::WrappingLeft) => TuiVimAction::MoveLeft,
            VimMotion::Character(CharacterMotion::Right)
            | VimMotion::Character(CharacterMotion::WrappingRight) => TuiVimAction::MoveRight,
            VimMotion::Character(CharacterMotion::Up) => TuiVimAction::MoveUp,
            VimMotion::Character(CharacterMotion::Down) => TuiVimAction::MoveDown,
            VimMotion::Word(ref word_motion) => match word_motion.direction {
                Direction::Backward => TuiVimAction::MoveWordLeft,
                // `e` moves to end of current word; `w` moves to start of next
                Direction::Forward => match word_motion.bound {
                    WordBound::End => TuiVimAction::MoveWordRightEnd,
                    WordBound::Start => TuiVimAction::MoveWordRightStart,
                },
            },
            VimMotion::Line(LineMotion::Start) => TuiVimAction::MoveToLineStart,
            VimMotion::Line(LineMotion::End) => TuiVimAction::MoveToLineEnd,
            VimMotion::Line(LineMotion::FirstNonWhitespace) => {
                TuiVimAction::MoveToFirstNonWhitespace
            }
            VimMotion::FirstNonWhitespace(_) => TuiVimAction::MoveToFirstNonWhitespace,
            VimMotion::JumpToFirstLine => TuiVimAction::MoveToBufferStart,
            VimMotion::JumpToLastLine => TuiVimAction::MoveToBufferEnd,
            // Other motions are complex (find-char, paragraph, etc.) — no-op
            _ => TuiVimAction::Unhandled,
        };
        Self::maybe_repeat(base, count)
    }

    fn map_operation(
        &mut self,
        operator: VimOperator,
        operand: VimOperand,
        count: usize,
    ) -> TuiVimAction {
        match operator {
            VimOperator::Delete | VimOperator::Change => {
                // Common delete/change operations
                match &operand {
                    VimOperand::Motion { motion, .. } => {
                        let base = match motion {
                            VimMotion::Character(CharacterMotion::Right)
                            | VimMotion::Character(CharacterMotion::WrappingRight) => {
                                TuiVimAction::DeleteForward
                            }
                            VimMotion::Character(CharacterMotion::Left)
                            | VimMotion::Character(CharacterMotion::WrappingLeft) => {
                                TuiVimAction::Backspace
                            }
                            VimMotion::Word(word_motion) => match word_motion.direction {
                                Direction::Forward => TuiVimAction::DeleteWordForward,
                                Direction::Backward => TuiVimAction::DeleteWordBackward,
                            },
                            VimMotion::Line(LineMotion::End) => TuiVimAction::KillToLineEnd,
                            VimMotion::Line(LineMotion::Start) => TuiVimAction::KillToLineStart,
                            _ => TuiVimAction::Unhandled,
                        };
                        Self::maybe_repeat(base, count)
                    }
                    VimOperand::Line => {
                        // `dd` — delete the whole current line regardless of cursor column.
                        // Move to start of the visual row first, then kill to end, so the
                        // entire row is removed even when the cursor is mid-line.
                        TuiVimAction::KillLine
                    }
                    VimOperand::TextObject(_) => TuiVimAction::Unhandled,
                }
            }
            VimOperator::Yank => {
                // `y` + motion → yank into clipboard
                match &operand {
                    VimOperand::Line => TuiVimAction::YankBuffer,
                    VimOperand::Motion { motion, .. } => match motion {
                        VimMotion::Line(LineMotion::End) => TuiVimAction::YankToLineEnd,
                        VimMotion::Word(_) => TuiVimAction::YankWordForward,
                        // All other motions fall back to yanking the full buffer.
                        _ => TuiVimAction::YankBuffer,
                    },
                    VimOperand::TextObject(_) => TuiVimAction::YankBuffer,
                }
            }
            _ => TuiVimAction::Unhandled,
        }
    }

    fn map_visual_operator(&mut self, operator: VimOperator) -> TuiVimAction {
        match operator {
            // In visual mode, d/c operate on the visual selection from anchor to
            // cursor. The view tracks the anchor and deletes/yanks accordingly.
            VimOperator::Delete | VimOperator::Change => TuiVimAction::DeleteVisualSelection,
            VimOperator::Yank => TuiVimAction::YankVisualSelection,
            _ => TuiVimAction::Unhandled,
        }
    }

    /// Force the vim state machine into insert mode (e.g. when the user
    /// clicks in the input or uses a mouse-driven cursor placement).
    pub(crate) fn force_insert_mode(&mut self) {
        // Drive the FSA to Normal mode first via Escape (clears pending state),
        // then to Insert via `i` so the state machine's internal invariants
        // stay intact. Direct field assignment bypasses those invariants.
        self.fsa.process_keystroke("escape");
        self.fsa.process_char('i');
    }

    /// Store text in the internal yank buffer (called by the view after
    /// performing a yank operation).
    pub(crate) fn set_yank_buffer(&mut self, text: String) {
        self.yank_buffer = text;
    }

    /// Read the current yank buffer.
    #[allow(dead_code)]
    pub(crate) fn yank_buffer(&self) -> &str {
        &self.yank_buffer
    }
}

#[cfg(test)]
#[path = "tui_vim_input_tests.rs"]
mod tests;
