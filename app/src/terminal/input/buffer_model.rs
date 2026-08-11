use warpui::{Entity, ModelContext, ViewHandle};

use crate::editor::{self, EditorView, Point};

/// A 'replica' of the input buffer's contents that may be subscribed to without
/// a strict dependency on the Input view itself.
pub struct InputBufferModel {
    buffer_value: String,
    cursor_point: Option<Point>,
}

impl InputBufferModel {
    pub fn new(editor: &ViewHandle<EditorView>, ctx: &mut ModelContext<Self>) -> Self {
        let editor_clone = editor.downgrade();
        ctx.subscribe_to_view(editor, move |me, _, event, ctx| match event {
            // This is intended to be the set of Editor view events that exhaustively
            // capture any changes to editor contents or cursor position.
            editor::Event::Edited(..)
            | editor::Event::CtrlC { .. }
            | editor::Event::BufferReinitialized
            | editor::Event::SelectionChanged
            | editor::Event::BufferReplaced => {
                if let Some(editor) = editor_clone.upgrade(ctx) {
                    let editor_ref = editor.as_ref(ctx);
                    let mut value = editor_ref.buffer_text(ctx);
                    let cursor_point = editor_ref.single_cursor_to_point(ctx);

                    me.cursor_point = cursor_point;
                    if value != me.buffer_value {
                        std::mem::swap(&mut value, &mut me.buffer_value);
                        ctx.emit(InputBufferUpdateEvent {
                            old_content: value,
                            new_content: me.buffer_value.clone(),
                        });
                    }
                }
            }
            _ => (),
        });
        let editor_ref = editor.as_ref(ctx);
        Self {
            buffer_value: editor_ref.buffer_text(ctx),
            cursor_point: editor_ref.single_cursor_to_point(ctx),
        }
    }

    pub fn current_value(&self) -> &str {
        self.buffer_value.as_str()
    }

    pub fn cursor_point(&self) -> Option<Point> {
        self.cursor_point
    }
}

pub struct InputBufferUpdateEvent {
    pub old_content: String,
    pub new_content: String,
}

impl Entity for InputBufferModel {
    type Event = InputBufferUpdateEvent;
}
