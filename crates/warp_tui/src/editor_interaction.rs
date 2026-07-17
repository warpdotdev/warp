use std::ops::Range;

use string_offset::CharOffset;
use warp::editor::CodeEditorModel;
use warp_editor::model::{CoreEditorModel, PlainTextEditorModel};
use warp_editor::render::model::CharCellState;
use warp_editor::selection::{TextDirection, TextUnit};
use warpui_core::text::word_boundaries::WordBoundariesPolicy;
use warpui_core::{AppContext, ModelHandle};

use crate::editor_element::TuiEditorAction;

/// Editing commands shared by TUI text fields.
#[derive(Clone, Copy, Debug)]
pub enum TuiEditorCommand {
    InsertNewline,
    Backspace,
    DeleteForward,
    DeleteWordBackward,
    DeleteWordForward,
    MoveLeft,
    MoveRight,
    MoveUp,
    MoveDown,
    MoveWordLeft,
    MoveWordRight,
    MoveToLineStart,
    MoveToLineEnd,
    SelectLeft,
    SelectRight,
    SelectUp,
    SelectDown,
    SelectWordLeft,
    SelectWordRight,
    SelectAll,
    KillToLineEnd,
    KillToLineStart,
    Yank,
    Undo,
    Redo,
}

/// Controls whether an editor accepts hard line breaks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TuiEditorLineMode {
    SingleLine,
    Multiline,
}

/// Editing behavior supplied by each TUI editor consumer.
#[derive(Clone, Copy, Debug)]
pub(crate) struct TuiEditorBehavior {
    line_mode: TuiEditorLineMode,
    viewport_rows: u32,
}

impl TuiEditorBehavior {
    /// Configures a one-row editor that rejects text after the first line.
    pub(crate) const fn single_line() -> Self {
        Self {
            line_mode: TuiEditorLineMode::SingleLine,
            viewport_rows: 1,
        }
    }

    /// Configures a multiline editor with a bounded viewport.
    pub(crate) const fn multiline(viewport_rows: u32) -> Self {
        Self {
            line_mode: TuiEditorLineMode::Multiline,
            viewport_rows,
        }
    }

    /// Returns the number of visible editor rows.
    pub(crate) const fn viewport_rows(self) -> u32 {
        self.viewport_rows
    }

    /// Applies this editor's line policy to inserted or replacement text.
    pub(crate) fn normalize_text(self, text: &str) -> &str {
        match self.line_mode {
            TuiEditorLineMode::SingleLine => text.lines().next().unwrap_or_default(),
            TuiEditorLineMode::Multiline => text,
        }
    }

    /// Returns whether hard newlines are accepted.
    const fn accepts_newlines(self) -> bool {
        matches!(self.line_mode, TuiEditorLineMode::Multiline)
    }
}

/// Viewport work required after applying an editor interaction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TuiEditorInteractionOutcome {
    FollowCursor,
    PreserveViewport,
}

/// Selects stable user-configurable names for each binding consumer.
#[derive(Clone, Copy)]
pub(crate) enum TuiEditorBindingTarget {
    Input,
    Editor,
}

struct EditorBindingSpec {
    command: TuiEditorCommand,
    input_name: Option<&'static str>,
    editor_name: Option<&'static str>,
    description: &'static str,
    keys: &'static [&'static str],
}

/// One target-specific editor binding definition.
#[derive(Clone, Copy)]
pub(crate) struct TuiEditorBindingSpec {
    pub(crate) command: TuiEditorCommand,
    pub(crate) name: &'static str,
    pub(crate) description: &'static str,
    pub(crate) keys: &'static [&'static str],
}

const SHARED_EDITOR_BINDINGS: &[EditorBindingSpec] = &[
    EditorBindingSpec {
        command: TuiEditorCommand::InsertNewline,
        input_name: Some("tui:input:insert_newline"),
        editor_name: None,
        description: "Insert a newline",
        keys: &["shift-enter", "ctrl-j", "alt-enter"],
    },
    EditorBindingSpec {
        command: TuiEditorCommand::Backspace,
        input_name: Some("tui:input:backspace"),
        editor_name: Some("tui:editor:backspace"),
        description: "Delete the previous character",
        keys: &["backspace", "shift-backspace", "ctrl-h"],
    },
    EditorBindingSpec {
        command: TuiEditorCommand::DeleteForward,
        input_name: Some("tui:input:delete_forward"),
        editor_name: Some("tui:editor:delete_forward"),
        description: "Delete the next character",
        keys: &["delete", "ctrl-d"],
    },
    EditorBindingSpec {
        command: TuiEditorCommand::DeleteWordBackward,
        input_name: Some("tui:input:delete_word_backward"),
        editor_name: Some("tui:editor:delete_word_backward"),
        description: "Delete the previous word",
        keys: &["ctrl-w", "ctrl-backspace", "alt-backspace"],
    },
    EditorBindingSpec {
        command: TuiEditorCommand::DeleteWordForward,
        input_name: Some("tui:input:delete_word_forward"),
        editor_name: Some("tui:editor:delete_word_forward"),
        description: "Delete the next word",
        keys: &["alt-d", "alt-delete", "ctrl-delete"],
    },
    EditorBindingSpec {
        command: TuiEditorCommand::MoveLeft,
        input_name: Some("tui:input:move_left"),
        editor_name: Some("tui:editor:move_left"),
        description: "Move cursor left",
        keys: &["left", "ctrl-b"],
    },
    EditorBindingSpec {
        command: TuiEditorCommand::MoveRight,
        input_name: Some("tui:input:move_right"),
        editor_name: Some("tui:editor:move_right"),
        description: "Move cursor right",
        keys: &["right", "ctrl-f"],
    },
    EditorBindingSpec {
        command: TuiEditorCommand::MoveUp,
        input_name: Some("tui:input:move_up"),
        editor_name: None,
        description: "Move cursor up",
        keys: &["up", "ctrl-p"],
    },
    EditorBindingSpec {
        command: TuiEditorCommand::MoveDown,
        input_name: Some("tui:input:move_down"),
        editor_name: None,
        description: "Move cursor down",
        keys: &["down", "ctrl-n"],
    },
    EditorBindingSpec {
        command: TuiEditorCommand::MoveWordLeft,
        input_name: Some("tui:input:move_word_left"),
        editor_name: Some("tui:editor:move_word_left"),
        description: "Move cursor one word left",
        keys: &["alt-left", "alt-b", "ctrl-left"],
    },
    EditorBindingSpec {
        command: TuiEditorCommand::MoveWordRight,
        input_name: Some("tui:input:move_word_right"),
        editor_name: Some("tui:editor:move_word_right"),
        description: "Move cursor one word right",
        keys: &["alt-right", "alt-f", "ctrl-right"],
    },
    EditorBindingSpec {
        command: TuiEditorCommand::MoveToLineStart,
        input_name: Some("tui:input:move_to_line_start"),
        editor_name: Some("tui:editor:move_to_line_start"),
        description: "Move cursor to start of line",
        keys: &["home", "ctrl-a"],
    },
    EditorBindingSpec {
        command: TuiEditorCommand::MoveToLineEnd,
        input_name: Some("tui:input:move_to_line_end"),
        editor_name: Some("tui:editor:move_to_line_end"),
        description: "Move cursor to end of line",
        keys: &["end", "ctrl-e"],
    },
    EditorBindingSpec {
        command: TuiEditorCommand::SelectLeft,
        input_name: Some("tui:input:select_left"),
        editor_name: Some("tui:editor:select_left"),
        description: "Extend selection left",
        keys: &["shift-left"],
    },
    EditorBindingSpec {
        command: TuiEditorCommand::SelectRight,
        input_name: Some("tui:input:select_right"),
        editor_name: Some("tui:editor:select_right"),
        description: "Extend selection right",
        keys: &["shift-right"],
    },
    EditorBindingSpec {
        command: TuiEditorCommand::SelectUp,
        input_name: Some("tui:input:select_up"),
        editor_name: None,
        description: "Extend selection up",
        keys: &["shift-up"],
    },
    EditorBindingSpec {
        command: TuiEditorCommand::SelectDown,
        input_name: Some("tui:input:select_down"),
        editor_name: None,
        description: "Extend selection down",
        keys: &["shift-down"],
    },
    EditorBindingSpec {
        command: TuiEditorCommand::SelectWordLeft,
        input_name: Some("tui:input:select_word_left"),
        editor_name: Some("tui:editor:select_word_left"),
        description: "Extend selection one word left",
        keys: &["ctrl-shift-left", "alt-shift-left"],
    },
    EditorBindingSpec {
        command: TuiEditorCommand::SelectWordRight,
        input_name: Some("tui:input:select_word_right"),
        editor_name: Some("tui:editor:select_word_right"),
        description: "Extend selection one word right",
        keys: &["ctrl-shift-right", "alt-shift-right"],
    },
    EditorBindingSpec {
        command: TuiEditorCommand::SelectAll,
        input_name: Some("tui:input:select_all"),
        editor_name: Some("tui:editor:select_all"),
        description: "Select all text",
        keys: &["ctrl-shift-A"],
    },
    EditorBindingSpec {
        command: TuiEditorCommand::KillToLineEnd,
        input_name: Some("tui:input:kill_to_line_end"),
        editor_name: Some("tui:editor:kill_to_line_end"),
        description: "Delete to end of line",
        keys: &["ctrl-k"],
    },
    EditorBindingSpec {
        command: TuiEditorCommand::KillToLineStart,
        input_name: Some("tui:input:kill_to_line_start"),
        editor_name: Some("tui:editor:kill_to_line_start"),
        description: "Delete to start of line",
        keys: &["ctrl-u"],
    },
    EditorBindingSpec {
        command: TuiEditorCommand::Yank,
        input_name: Some("tui:input:yank"),
        editor_name: Some("tui:editor:yank"),
        description: "Paste the last deleted text",
        keys: &["ctrl-y"],
    },
    EditorBindingSpec {
        command: TuiEditorCommand::Undo,
        input_name: Some("tui:input:undo"),
        editor_name: Some("tui:editor:undo"),
        description: "Undo",
        keys: &["ctrl-z"],
    },
    EditorBindingSpec {
        command: TuiEditorCommand::Redo,
        input_name: Some("tui:input:redo"),
        editor_name: Some("tui:editor:redo"),
        description: "Redo",
        keys: &["ctrl-shift-Z"],
    },
];

/// Returns the editor binding metadata applicable to one consumer.
pub(crate) fn editor_binding_specs(
    target: TuiEditorBindingTarget,
) -> impl Iterator<Item = TuiEditorBindingSpec> {
    SHARED_EDITOR_BINDINGS.iter().filter_map(move |spec| {
        let name = match target {
            TuiEditorBindingTarget::Input => spec.input_name,
            TuiEditorBindingTarget::Editor => spec.editor_name,
        }?;
        Some(TuiEditorBindingSpec {
            command: spec.command,
            name,
            description: spec.description,
            keys: spec.keys,
        })
    })
}

/// Mutable editing state shared by TUI editor views.
#[derive(Debug, Default)]
pub(crate) struct TuiEditorState {
    kill_buffer: String,
}

impl TuiEditorState {
    /// Applies a keybound command to a char-cell editor model.
    pub(crate) fn apply_command(
        &mut self,
        model: &ModelHandle<CodeEditorModel>,
        command: TuiEditorCommand,
        behavior: TuiEditorBehavior,
        ctx: &mut AppContext,
    ) -> TuiEditorInteractionOutcome {
        match command {
            TuiEditorCommand::InsertNewline => {
                if behavior.accepts_newlines() {
                    model.update(ctx, |model, ctx| model.user_insert("\n", ctx));
                }
            }
            TuiEditorCommand::Backspace => {
                model.update(ctx, |model, ctx| model.backspace(ctx));
            }
            TuiEditorCommand::DeleteForward => {
                model.update(ctx, |model, ctx| {
                    model.delete(TextDirection::Forwards, TextUnit::Character, false, ctx);
                });
            }
            TuiEditorCommand::DeleteWordBackward => {
                model.update(ctx, |model, ctx| {
                    model.delete(
                        TextDirection::Backwards,
                        TextUnit::Word(WordBoundariesPolicy::Default),
                        false,
                        ctx,
                    );
                });
            }
            TuiEditorCommand::DeleteWordForward => {
                model.update(ctx, |model, ctx| {
                    model.delete(
                        TextDirection::Forwards,
                        TextUnit::Word(WordBoundariesPolicy::Default),
                        false,
                        ctx,
                    );
                });
            }
            TuiEditorCommand::MoveLeft => {
                model.update(ctx, |model, ctx| model.move_left(ctx));
            }
            TuiEditorCommand::MoveRight => {
                model.update(ctx, |model, ctx| model.move_right(ctx));
            }
            TuiEditorCommand::MoveUp => {
                model.update(ctx, |model, ctx| model.move_up(ctx));
            }
            TuiEditorCommand::MoveDown => {
                model.update(ctx, |model, ctx| model.move_down(ctx));
            }
            TuiEditorCommand::MoveWordLeft => {
                model.update(ctx, |model, ctx| {
                    model.backward_word_with_unit(
                        false,
                        TextUnit::Word(WordBoundariesPolicy::Default),
                        ctx,
                    );
                });
            }
            TuiEditorCommand::MoveWordRight => {
                model.update(ctx, |model, ctx| {
                    model.forward_word_with_unit(
                        false,
                        TextUnit::Word(WordBoundariesPolicy::Default),
                        ctx,
                    );
                });
            }
            TuiEditorCommand::MoveToLineStart => {
                model.update(ctx, |model, ctx| model.move_to_line_start(ctx));
            }
            TuiEditorCommand::MoveToLineEnd => {
                model.update(ctx, |model, ctx| model.move_to_line_end(ctx));
            }
            TuiEditorCommand::SelectLeft => {
                model.update(ctx, |model, ctx| model.select_left(ctx));
            }
            TuiEditorCommand::SelectRight => {
                model.update(ctx, |model, ctx| model.select_right(ctx));
            }
            TuiEditorCommand::SelectUp => {
                model.update(ctx, |model, ctx| model.select_up(ctx));
            }
            TuiEditorCommand::SelectDown => {
                model.update(ctx, |model, ctx| model.select_down(ctx));
            }
            TuiEditorCommand::SelectWordLeft => {
                model.update(ctx, |model, ctx| {
                    model.backward_word_with_unit(
                        true,
                        TextUnit::Word(WordBoundariesPolicy::Default),
                        ctx,
                    );
                });
            }
            TuiEditorCommand::SelectWordRight => {
                model.update(ctx, |model, ctx| {
                    model.forward_word_with_unit(
                        true,
                        TextUnit::Word(WordBoundariesPolicy::Default),
                        ctx,
                    );
                });
            }
            TuiEditorCommand::SelectAll => {
                model.update(ctx, |model, ctx| model.select_all(ctx));
            }
            TuiEditorCommand::KillToLineEnd => {
                if let Some(killed) = model.update(ctx, |model, ctx| {
                    model.kill_to_char_cell_visual_row_end(ctx)
                }) {
                    self.kill_buffer = killed;
                }
            }
            TuiEditorCommand::KillToLineStart => {
                if let Some(killed) = model.update(ctx, |model, ctx| {
                    model.kill_to_char_cell_visual_row_start(ctx)
                }) {
                    self.kill_buffer = killed;
                }
            }
            TuiEditorCommand::Yank => {
                if !self.kill_buffer.is_empty() {
                    model.update(ctx, |model, ctx| {
                        model.user_insert(&self.kill_buffer, ctx);
                    });
                }
            }
            TuiEditorCommand::Undo => {
                model.update(ctx, |model, ctx| model.undo(ctx));
            }
            TuiEditorCommand::Redo => {
                model.update(ctx, |model, ctx| model.redo(ctx));
            }
        }
        TuiEditorInteractionOutcome::FollowCursor
    }
}

/// Applies an element-originated action and reports the required viewport work.
pub(crate) fn apply_editor_action(
    model: &ModelHandle<CodeEditorModel>,
    action: &TuiEditorAction,
    behavior: TuiEditorBehavior,
    ctx: &mut AppContext,
) -> TuiEditorInteractionOutcome {
    match action {
        TuiEditorAction::InsertChar(c) => {
            model.update(ctx, |model, ctx| model.user_insert(&c.to_string(), ctx));
        }
        TuiEditorAction::InsertText(text) => {
            let text = behavior.normalize_text(text);
            model.update(ctx, |model, ctx| model.user_insert(text, ctx));
        }
        TuiEditorAction::SelectionStartAt { offset } => {
            model.update(ctx, |model, ctx| model.select_at(*offset, false, ctx));
        }
        TuiEditorAction::SelectionExtendTo { offset } => {
            model.update(ctx, |model, ctx| {
                model.set_last_selection_head(*offset, ctx)
            });
        }
        TuiEditorAction::SelectWordAt { offset } => {
            model.update(ctx, |model, ctx| model.select_word_at(*offset, false, ctx));
        }
        TuiEditorAction::SelectLineAt { offset } => {
            model.update(ctx, |model, ctx| model.select_line_at(*offset, false, ctx));
        }
        TuiEditorAction::SelectionUpdateTo { offset } => {
            model.update(ctx, |model, ctx| {
                model.update_pending_selection(*offset, ctx)
            });
        }
        TuiEditorAction::SelectionEnd => {
            model.update(ctx, |model, ctx| model.end_selection(ctx));
        }
        TuiEditorAction::Scroll { rows } => {
            scroll_editor_viewport(model, *rows, behavior, ctx);
            return TuiEditorInteractionOutcome::PreserveViewport;
        }
    }
    TuiEditorInteractionOutcome::FollowCursor
}

/// Scrolls a char-cell viewport just enough to keep its primary cursor visible.
pub(crate) fn follow_editor_cursor(
    model: &ModelHandle<CodeEditorModel>,
    behavior: TuiEditorBehavior,
    ctx: &AppContext,
) {
    with_editor_viewport(model, ctx, |char_cell, cursor_offset, hidden| {
        char_cell.follow_cursor(cursor_offset, behavior.viewport_rows(), hidden);
    });
}

/// Scrolls a char-cell viewport without moving its primary cursor.
fn scroll_editor_viewport(
    model: &ModelHandle<CodeEditorModel>,
    rows: isize,
    behavior: TuiEditorBehavior,
    ctx: &AppContext,
) {
    with_editor_viewport(model, ctx, |char_cell, cursor_offset, hidden| {
        char_cell.scroll_by(rows, behavior.viewport_rows(), cursor_offset, hidden);
    });
}

/// Reads the primary cursor and hidden rows for a char-cell viewport operation.
fn with_editor_viewport(
    model: &ModelHandle<CodeEditorModel>,
    ctx: &AppContext,
    f: impl FnOnce(&CharCellState, CharOffset, &[Range<usize>]),
) {
    let model = model.as_ref(ctx);
    let cursor_offset = model
        .selection_model()
        .as_ref(ctx)
        .cursors(ctx)
        .into_iter()
        .next()
        .unwrap_or_default();
    let render = model.render_state().as_ref(ctx);
    let Some(char_cell) = render.char_cell() else {
        return;
    };
    let cursor_offset = CharOffset::from(cursor_offset.as_usize().saturating_sub(1));
    let hidden = char_cell.hidden_line_ranges(ctx);
    f(char_cell, cursor_offset, &hidden);
}
