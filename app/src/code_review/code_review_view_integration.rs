use std::path::PathBuf;

use warp_editor::model::CoreEditorModel;
use warp_editor::render::model::{
    BlockItem, HitTestOptions, LineCount, Location, RenderLineLocation,
};
use warpui::units::Pixels;
use warpui::{AppContext, ViewContext};

use super::{CodeReviewView, CodeReviewViewState, FILE_HEADER_HEIGHT};
use crate::code::buffer_location::LocalOrRemotePath;
use crate::code::editor::line::EditorLineLocation;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CodeReviewVisibleAnchorForTest {
    pub file_path: String,
    pub line_number: usize,
    pub line_text: String,
}

impl CodeReviewView {
    pub fn visible_anchor_for_test(
        &self,
        ctx: &AppContext,
    ) -> Option<CodeReviewVisibleAnchorForTest> {
        let CodeReviewViewState::Loaded(state) = self.state() else {
            return None;
        };

        let file_index = self.viewported_list_state.get_scroll_index();
        let (_, file_state) = state.file_states.get_index(file_index)?;
        let editor_state = file_state.editor_state.as_ref()?;
        let scroll_offset = self.viewported_list_state.get_scroll_offset();
        let content_y = (scroll_offset - Pixels::new(FILE_HEADER_HEIGHT) + Pixels::new(2.0))
            .max(Pixels::zero());

        let editor = editor_state.editor.as_ref(ctx).editor();
        let render_state_handle = editor.as_ref(ctx).model.as_ref(ctx).render_state().clone();
        let location = render_state_handle
            .as_ref(ctx)
            .render_coordinates_to_location(
                Pixels::new(10.0),
                content_y,
                &HitTestOptions {
                    force_text_selection: true,
                },
            );
        let char_offset = match location {
            Location::Text { char_offset, .. } => char_offset,
            Location::Block { start_offset, .. } => start_offset,
        };
        let render_state = render_state_handle.as_ref(ctx);
        let line_number = render_state.offset_to_softwrap_point(char_offset).row() as usize + 1;
        let (start_offset, end_offset) =
            render_state.line_number_to_offset_range(LineCount::from(line_number));
        let line_text = editor
            .as_ref(ctx)
            .model
            .as_ref(ctx)
            .content()
            .as_ref(ctx)
            .text_in_range(start_offset..end_offset)
            .into_string();

        Some(CodeReviewVisibleAnchorForTest {
            file_path: file_state.file_diff.file_path.clone(),
            line_number,
            line_text: line_text.trim_matches('\n').to_string(),
        })
    }

    pub fn scroll_to_line_for_test(
        &mut self,
        path: &str,
        line_number: usize,
        ctx: &mut ViewContext<Self>,
    ) -> bool {
        let CodeReviewViewState::Loaded(state) = self.state() else {
            return false;
        };

        let Some(editor_index) = state
            .file_states
            .iter()
            .position(|(_, file_state)| file_state.file_diff.file_path == path)
        else {
            return false;
        };
        let Some(editor_state) = state
            .file_states
            .get_index(editor_index)
            .and_then(|(_, file_state)| file_state.editor_state.as_ref())
        else {
            return false;
        };

        let editor = editor_state.editor().clone();
        let line_number = LineCount::from(line_number);
        let line = EditorLineLocation::Current {
            line_number,
            line_range: line_number..line_number + LineCount::from(1),
        };
        let (start_offset, end_offset) = editor
            .as_ref(ctx)
            .editor()
            .read(ctx, |code_editor_view, ctx| {
                code_editor_view.line_location_to_offsets(&line, ctx)
            });

        if let Some((start_top_y, _end_bottom_y)) =
            self.get_match_character_bounds(editor_index, start_offset, end_offset, ctx)
        {
            self.viewported_list_state
                .scroll_to_with_offset(editor_index, Pixels::new(FILE_HEADER_HEIGHT) + start_top_y);
            self.horizontally_scroll_to_match(editor_index, start_offset, end_offset, ctx);

            // Eagerly compute and store scroll context so it is available
            // before the next buffer edit (the debounce may not have fired yet).
            let context = self.compute_scroll_context_for_index(editor_index, &editor, ctx);
            if let Some(context) = context {
                self.viewported_list_state.set_scroll_context(Some(context));
            }

            ctx.notify();
            true
        } else {
            self.scroll_to_position(editor_index, start_offset, end_offset, 0.0, ctx);
            ctx.notify();
            false
        }
    }

    /// Scrolls the code review to the header region of the given file.
    /// The header region is the area above the editor content (< FILE_HEADER_HEIGHT).
    pub fn scroll_to_header_for_test(&mut self, path: &str, ctx: &mut ViewContext<Self>) -> bool {
        let CodeReviewViewState::Loaded(state) = self.state() else {
            return false;
        };

        let Some(editor_index) = state
            .file_states
            .iter()
            .position(|(_, file_state)| file_state.file_diff.file_path == path)
        else {
            return false;
        };
        let Some(editor_state) = state
            .file_states
            .get_index(editor_index)
            .and_then(|(_, file_state)| file_state.editor_state.as_ref())
        else {
            return false;
        };

        let editor = editor_state.editor().clone();

        // Scroll to 10px into the header (FILE_HEADER_HEIGHT is 41px)
        self.viewported_list_state
            .scroll_to_with_offset(editor_index, Pixels::new(10.0));

        let context = self.compute_scroll_context_for_index(editor_index, &editor, ctx);
        if let Some(context) = context {
            self.viewported_list_state.set_scroll_context(Some(context));
        }

        ctx.notify();
        true
    }

    /// Scrolls the code review past the end of editor content into the footer region.
    pub fn scroll_to_footer_for_test(&mut self, path: &str, ctx: &mut ViewContext<Self>) -> bool {
        let CodeReviewViewState::Loaded(state) = self.state() else {
            return false;
        };

        let Some(editor_index) = state
            .file_states
            .iter()
            .position(|(_, file_state)| file_state.file_diff.file_path == path)
        else {
            return false;
        };
        let Some(editor_state) = state
            .file_states
            .get_index(editor_index)
            .and_then(|(_, file_state)| file_state.editor_state.as_ref())
        else {
            return false;
        };

        let editor = editor_state.editor().clone();

        let content_height = editor
            .as_ref(ctx)
            .editor()
            .as_ref(ctx)
            .model
            .as_ref(ctx)
            .render_state()
            .as_ref(ctx)
            .height();

        // Scroll 5px past the editor content into the footer/margin area
        self.viewported_list_state.scroll_to_with_offset(
            editor_index,
            Pixels::new(FILE_HEADER_HEIGHT) + content_height + Pixels::new(5.0),
        );

        let context = self.compute_scroll_context_for_index(editor_index, &editor, ctx);
        if let Some(context) = context {
            self.viewported_list_state.set_scroll_context(Some(context));
        }

        ctx.notify();
        true
    }

    /// Scrolls the code review to a deleted (temporary) block near the given current buffer line.
    /// Scans forward from the y-offset of `near_line` to find the first TemporaryBlock.
    pub fn scroll_to_deleted_range_for_test(
        &mut self,
        path: &str,
        near_line: usize,
        ctx: &mut ViewContext<Self>,
    ) -> bool {
        let CodeReviewViewState::Loaded(state) = self.state() else {
            return false;
        };

        let Some(editor_index) = state
            .file_states
            .iter()
            .position(|(_, file_state)| file_state.file_diff.file_path == path)
        else {
            return false;
        };
        let Some(editor_state) = state
            .file_states
            .get_index(editor_index)
            .and_then(|(_, file_state)| file_state.editor_state.as_ref())
        else {
            return false;
        };

        let editor = editor_state.editor().clone();
        let editor_model_handle = editor.as_ref(ctx).editor().as_ref(ctx).model.clone();

        // Phase 1: Find the y-offset of a temporary block near the given line.
        let found_offset = {
            let editor_model = editor_model_handle.as_ref(ctx);
            let render_state = editor_model.render_state().as_ref(ctx);

            // Get approximate content-relative y position of near_line.
            // vertical_offset_at_render_location internally borrows and releases
            // the content RefCell, so calling content() afterwards is safe.
            let line_offset = render_state
                .vertical_offset_at_render_location(RenderLineLocation::Current(LineCount::from(
                    near_line,
                )))
                .unwrap_or(Pixels::zero());

            let content = render_state.content();
            let mut y = line_offset.as_f32() as f64;
            let scan_limit = y + 2000.0;
            let mut found = None;

            while y < scan_limit {
                let Some(block) = content.block_at_height(y) else {
                    break;
                };
                if matches!(block.item, BlockItem::TemporaryBlock { .. }) {
                    found = Some(block.start_y_offset + Pixels::new(5.0));
                    break;
                }
                // Advance past this block
                let block_end = (block.start_y_offset + block.item.height()).as_f32() as f64;
                y = if block_end <= y {
                    y + 1.0
                } else {
                    block_end + 0.5
                };
            }

            found
        };

        let Some(offset_in_editor) = found_offset else {
            return false;
        };

        self.viewported_list_state.scroll_to_with_offset(
            editor_index,
            Pixels::new(FILE_HEADER_HEIGHT) + offset_in_editor,
        );

        let context = self.compute_scroll_context_for_index(editor_index, &editor, ctx);
        if let Some(context) = context {
            self.viewported_list_state.set_scroll_context(Some(context));
        }

        ctx.notify();
        true
    }

    /// Returns a string describing which scroll region the current scroll position
    /// is in: "header", "current_line", "removed_line", "footer", or "unknown".
    pub fn scroll_region_for_test(&self, ctx: &AppContext) -> String {
        let file_index = self.viewported_list_state.get_scroll_index();
        let scroll_offset = self.viewported_list_state.get_scroll_offset();
        let file_header_height = Pixels::new(FILE_HEADER_HEIGHT);

        if scroll_offset < file_header_height {
            return "header".to_string();
        }

        let CodeReviewViewState::Loaded(state) = self.state() else {
            return "unknown".to_string();
        };

        let Some((_, file_state)) = state.file_states.get_index(file_index) else {
            return "unknown".to_string();
        };

        let Some(editor_state) = &file_state.editor_state else {
            return "unknown".to_string();
        };

        let editor_model = editor_state
            .editor
            .as_ref(ctx)
            .editor()
            .as_ref(ctx)
            .model
            .as_ref(ctx);
        let render_state = editor_model.render_state().as_ref(ctx);
        let content_height = render_state.height();
        let scroll_in_editor = scroll_offset - file_header_height;

        if scroll_in_editor >= content_height {
            return "footer".to_string();
        }

        let content = render_state.content();
        if let Some(block) = content.block_at_height(scroll_in_editor.as_f32() as f64) {
            match block.item {
                BlockItem::TemporaryBlock { .. } => return "removed_line".to_string(),
                _ => return "current_line".to_string(),
            }
        }

        "unknown".to_string()
    }

    pub fn all_editors_loaded_for_test(&self) -> bool {
        self.all_editors_loaded()
    }

    pub fn line_text_for_test(
        &self,
        path: &str,
        line_number: usize,
        ctx: &AppContext,
    ) -> Option<String> {
        // Test helper: probe by both the raw path (wrapped as a local
        // `LocalOrRemotePath`) and by the repo-joined absolute path.
        let local_path = LocalOrRemotePath::Local(PathBuf::from(path));
        let editor = if let Some(editor) = self.editor_for_path(&local_path, ctx) {
            editor
        } else {
            let absolute_path = self.repo_path()?.join(path);
            self.editor_for_path(&absolute_path, ctx)?
        };
        let text = editor
            .as_ref(ctx)
            .editor()
            .as_ref(ctx)
            .text(ctx)
            .into_string();
        let line_index = line_number.checked_sub(1)?;
        text.lines().nth(line_index).map(ToOwned::to_owned)
    }

    /// The number of collapsed hidden sections in the given file's diff editor.
    pub fn hidden_section_count_for_test(&self, path: &str, ctx: &AppContext) -> Option<usize> {
        let CodeReviewViewState::Loaded(state) = self.state() else {
            return None;
        };
        let editor_index = state
            .file_states
            .iter()
            .position(|(_, file_state)| file_state.file_diff.file_path == path)?;
        let editor_state = state
            .file_states
            .get_index(editor_index)
            .and_then(|(_, file_state)| file_state.editor_state.as_ref())?;
        let editor = editor_state.editor().clone();
        Some(
            editor
                .as_ref(ctx)
                .editor()
                .read(ctx, |code_editor_view, ctx| {
                    code_editor_view.hidden_section_count_for_test(ctx)
                }),
        )
    }

    /// Fully expand the first hidden section of the given file's diff editor,
    /// mirroring what a bar double-click does. Returns whether a section was
    /// expanded.
    pub fn fully_expand_first_hidden_section_for_test(
        &mut self,
        path: &str,
        ctx: &mut ViewContext<Self>,
    ) -> bool {
        let CodeReviewViewState::Loaded(state) = self.state() else {
            return false;
        };
        let Some(editor_index) = state
            .file_states
            .iter()
            .position(|(_, file_state)| file_state.file_diff.file_path == path)
        else {
            return false;
        };
        let Some(editor_state) = state
            .file_states
            .get_index(editor_index)
            .and_then(|(_, file_state)| file_state.editor_state.as_ref())
        else {
            return false;
        };
        let editor = editor_state.editor().clone();
        editor
            .as_ref(ctx)
            .editor()
            .clone()
            .update(ctx, |code_editor_view, ctx| {
                code_editor_view.fully_expand_first_hidden_section_for_test(ctx)
            })
    }
}
