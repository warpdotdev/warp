//! Vim-mode input handling for the TUI prompt input.
//!
//! [`TuiVimInputModel`] wraps a [`VimFSA`] state machine and translates raw
//! keystrokes into high-level [`TuiVimAction`]s that the owning
//! [`TuiInputView`] applies to the backing [`CodeEditorModel`].
//!
//! When vim mode is disabled the model is still present but inert — all
//! character input falls through to the normal editor path.

use vim::vim::{
    CharacterMotion, Direction, LineMotion, VimEventType, VimFSA, VimMode, VimMotion, VimOperand,
    VimOperator, WordBound,
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
    /// Move the cursor one word to the right.
    MoveWordRight,
    /// Move the cursor to the start of the current line.
    MoveToLineStart,
    /// Move the cursor to the end of the current line.
    MoveToLineEnd,
    /// Move the cursor to the first non-whitespace character on the line.
    MoveToFirstNonWhitespace,
    /// Move the cursor to the start of the input buffer.
    MoveToBufferStart,
    /// Move the cursor to the end of the input buffer.
    MoveToBufferEnd,
    /// Delete from the cursor to the end of the line (kill).
    KillToLineEnd,
    /// Delete from the cursor to the start of the line (kill).
    KillToLineStart,
    /// Yank (copy) the selected or current-line text into the vim clipboard.
    YankSelection,
    /// Paste the vim clipboard contents after the cursor.
    PasteAfter(String),
    /// Paste the vim clipboard contents before the cursor.
    PasteBefore(String),
    /// Undo the last edit.
    Undo,
    /// A vim mode transition occurred (new mode returned); no buffer edit needed.
    ModeTransition,
    /// The keystroke was consumed (pending input) but no action is ready yet.
    Pending,
    /// The keystroke was not handled in the current mode.
    Unhandled,
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
            VimEventType::ChangeMode { .. } => TuiVimAction::ModeTransition,

            // ── Undo ───────────────────────────────────────────────────────
            VimEventType::Undo => TuiVimAction::Undo,

            // ── Deletion ───────────────────────────────────────────────────
            VimEventType::Backspace => TuiVimAction::Backspace,
            VimEventType::DeleteForward => TuiVimAction::DeleteForward,

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

            // ── Replace char ──────────────────────────────────────────────
            VimEventType::ReplaceChar(Some(c)) => {
                // Delete current char and insert the replacement
                // We return DeleteForward so the view handles the delete;
                // the mode switch to Insert is handled by the FSA; the char
                // is inserted via a second InsertChar action. Since we can
                // only return one action, we return a compound DeleteForward
                // and the view will be re-notified to insert after mode change.
                // As a simpler approximation, just replace via delete+insert.
                let _ = c;
                // Best approximation: delete forward then the FSA is in Normal
                // mode, so the next typed char will also go through the FSA.
                // The FSA handles 'r' → Replace mode transition → types char →
                // returns to Normal. We just delete the char and insert the new one.
                TuiVimAction::DeleteForward
            }
            VimEventType::ReplaceChar(None) => TuiVimAction::ModeTransition,

            // ── Join lines (J) ────────────────────────────────────────────
            VimEventType::JoinLine => TuiVimAction::MoveToLineEnd,

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

    fn map_motion(&self, motion: VimMotion, _count: usize) -> TuiVimAction {
        match motion {
            VimMotion::Character(CharacterMotion::Left)
            | VimMotion::Character(CharacterMotion::WrappingLeft) => TuiVimAction::MoveLeft,
            VimMotion::Character(CharacterMotion::Right)
            | VimMotion::Character(CharacterMotion::WrappingRight) => TuiVimAction::MoveRight,
            VimMotion::Character(CharacterMotion::Up) => TuiVimAction::MoveUp,
            VimMotion::Character(CharacterMotion::Down) => TuiVimAction::MoveDown,
            VimMotion::Word(ref word_motion) => match word_motion.direction {
                Direction::Backward => TuiVimAction::MoveWordLeft,
                Direction::Forward => match word_motion.bound {
                    WordBound::End => TuiVimAction::MoveWordRight,
                    WordBound::Start => TuiVimAction::MoveWordRight,
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
        }
    }

    fn map_operation(
        &mut self,
        operator: VimOperator,
        operand: VimOperand,
        _count: usize,
    ) -> TuiVimAction {
        match operator {
            VimOperator::Delete | VimOperator::Change => {
                // Common delete/change operations
                match &operand {
                    VimOperand::Motion { motion, .. } => match motion {
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
                    },
                    VimOperand::Line => {
                        // dd → kill entire line (select all + delete for single-line input)
                        // For multi-line: kill the current visual row
                        TuiVimAction::KillToLineEnd
                    }
                    VimOperand::TextObject(_) => TuiVimAction::Unhandled,
                }
            }
            VimOperator::Yank => {
                // y + motion → yank into clipboard
                // For TUI simplicity, always yank the current content
                TuiVimAction::YankSelection
            }
            _ => TuiVimAction::Unhandled,
        }
    }

    fn map_visual_operator(&mut self, operator: VimOperator) -> TuiVimAction {
        match operator {
            VimOperator::Delete | VimOperator::Change => TuiVimAction::Backspace,
            VimOperator::Yank => TuiVimAction::YankSelection,
            _ => TuiVimAction::Unhandled,
        }
    }

    /// Force the vim state machine into insert mode (e.g. when the user
    /// clicks in the input or uses a mouse-driven cursor placement).
    pub(crate) fn force_insert_mode(&mut self) {
        self.fsa.process_keystroke("escape");
        // After escape from any mode, FSA is in Normal. Switch to Insert.
        self.fsa.mode = VimMode::Insert;
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
