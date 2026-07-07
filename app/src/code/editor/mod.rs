#![cfg_attr(target_family = "wasm", allow(dead_code, unused_imports))]

pub(crate) mod comment_editor;
mod comments;
pub(super) mod diff;
mod element;
pub mod embedded_comment;
pub mod find;
pub mod goto_line;
pub mod line;
mod line_iterator;
pub mod model;
mod nav_bar;
pub mod scroll;
pub mod view;

pub use comment_editor::{CommentEditor, CommentEditorEvent};
pub use comments::{EditorCommentsModel, EditorReviewComment};
pub(crate) use diff::{add_color, compute_unified_diff, remove_color};
pub use element::GutterHoverTarget;
pub use nav_bar::NavBarBehavior;
