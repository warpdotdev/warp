//! The split (side-by-side) diff layout for the code review pane.
//!
//! Each file keeps its existing editor as the modified (right) pane. In the split layout a second,
//! read-only editor shows the base text on the left. Both panes render at full height inside the
//! same list row, so they scroll together; blank gap rows keep their lines aligned.

use std::ops::Range;
use std::sync::Arc;

use rangemap::RangeSet;
use warp_core::settings::Setting as _;
use warp_editor::content::buffer::InitialBufferState;
use warp_editor::content::text::LineCount;
use warp_editor::multiline::{LF, MultilineString};
use warp_editor::render::element::VerticalExpansionBehavior;
use warp_editor::render::model::LineCount as RenderLineCount;
use warp_util::content_version::ContentVersion;
use warpui::{AppContext, SingletonEntity as _, ViewContext, ViewHandle};

use super::{
    CODE_REVIEW_EDITOR_LINE_HEIGHT_RATIO, CodeReviewEditorState, CodeReviewView,
    CodeReviewViewEvent, CodeReviewViewState, ShowCommentEditor,
};
use crate::code::diff_layout::DiffLayout;
use crate::code::editor::diff_alignment::{ChangeSpan, DiffAlignment};
use crate::code::editor::line::EditorLineLocation;
use crate::code::editor::model::DiffDecorationMode;
use crate::code::editor::view::{CodeEditorEvent, CodeEditorRenderOptions, CodeEditorView};
use crate::code::editor::{EditorReviewComment, GutterHoverTarget};
use crate::code::local_code_editor::LocalCodeEditorView;
use crate::code_review::comments::AttachedReviewCommentTarget;
use crate::editor::InteractionState;
use crate::features::FeatureFlag;
use crate::pane_group::PaneEvent;
use crate::settings::CodeSettings;

/// The read-only base pane of a file in the split layout.
pub(crate) struct SplitBaselinePane {
    pub(crate) editor: ViewHandle<LocalCodeEditorView>,
    /// The base text the pane was last filled with, used to detect a new base after a refresh.
    base: Arc<MultilineString<LF>>,
}

impl CodeReviewView {
    pub(crate) fn effective_diff_layout(ctx: &AppContext) -> DiffLayout {
        DiffLayout::effective(*CodeSettings::as_ref(ctx).diff_layout.value())
    }

    /// Builds or drops every loaded file's base pane to match the current layout.
    pub(super) fn sync_diff_layout(&mut self, ctx: &mut ViewContext<Self>) {
        let file_paths = match self.state() {
            CodeReviewViewState::Loaded(loaded) => {
                loaded.file_states.keys().cloned().collect::<Vec<_>>()
            }
            CodeReviewViewState::None
            | CodeReviewViewState::NoRepoFound
            | CodeReviewViewState::Error(_) => return,
        };
        let mut created_pane = false;
        for (index, file_path) in file_paths.iter().enumerate() {
            created_pane |= self.sync_split_pane(file_path, ctx);
            self.viewported_list_state
                .invalidate_height_for_index(index);
        }
        if created_pane {
            self.refresh_after_baseline_created(ctx);
        }
        // Panes were added or dropped, so an open query's matches belong to stale editors.
        if self.find_model.as_ref(ctx).is_find_bar_open()
            && FeatureFlag::CodeReviewFind.is_enabled()
        {
            let editors = self.editor_handles();
            self.find_model.update(ctx, |model, ctx| {
                model.run_search(editors, ctx);
            });
        }
        ctx.notify();
    }

    /// A new base pane starts without the comments already attached to its removed lines.
    pub(super) fn refresh_after_baseline_created(&mut self, ctx: &mut ViewContext<Self>) {
        self.update_editor_comment_markers(ctx);
    }

    /// Brings one file's panes in line with the current layout and the modified pane's latest
    /// diff: creates, refills or drops the base pane, and re-aligns both panes. Returns whether a
    /// new base pane was created.
    pub(super) fn sync_split_pane(&mut self, file_path: &str, ctx: &mut ViewContext<Self>) -> bool {
        let split = Self::effective_diff_layout(ctx).is_side_by_side();
        let Some(editor_state) = self.editor_state_mut(file_path) else {
            return false;
        };
        let modified = editor_state.editor.as_ref(ctx).editor().clone();

        if !split {
            if editor_state.baseline.take().is_some() {
                modified.update(ctx, |editor, ctx| {
                    editor.set_diff_decoration_mode(DiffDecorationMode::Unified, ctx);
                });
            }
            return false;
        }

        let Some(base) = modified
            .as_ref(ctx)
            .model
            .as_ref(ctx)
            .diff()
            .as_ref(ctx)
            .base()
        else {
            // The diff base arrives with the first diff, which triggers another sync.
            return false;
        };

        let (baseline, created) = match self
            .editor_state_mut(file_path)
            .and_then(|s| s.baseline.take())
        {
            Some(mut pane) => {
                if !Arc::ptr_eq(&pane.base, &base) {
                    fill_baseline(&pane.editor, &base, ctx);
                    pane.base = base;
                }
                (pane, false)
            }
            None => (self.create_baseline_pane(file_path, base, ctx), true),
        };

        modified.update(ctx, |editor, ctx| {
            editor.set_diff_decoration_mode(DiffDecorationMode::SplitModified, ctx);
        });
        let alignment = modified
            .as_ref(ctx)
            .model
            .as_ref(ctx)
            .split_diff_alignment(ctx);
        apply_alignment_to_baseline(&baseline.editor, &alignment, ctx);
        let baseline_editor = baseline.editor.as_ref(ctx).editor().clone();
        // A pane created here hasn't laid out yet, so the copied folds apply on its first layout.
        // Rebuilding now would race that layout and lay the text out twice.
        mirror_hidden_lines(
            &modified,
            &baseline_editor,
            &alignment,
            MirrorDirection::FromModified,
            !created,
            ctx,
        );

        if let Some(editor_state) = self.editor_state_mut(file_path) {
            editor_state.baseline = Some(baseline);
        }
        created
    }

    fn create_baseline_pane(
        &self,
        file_path: &str,
        base: Arc<MultilineString<LF>>,
        ctx: &mut ViewContext<Self>,
    ) -> SplitBaselinePane {
        let comment_list_save_position_id = self.code_review_list_position_id.clone();
        let code_editor_view = ctx.add_typed_action_view(|ctx| {
            let mut editor_view = CodeEditorView::new(
                None,
                None,
                CodeEditorRenderOptions::new(VerticalExpansionBehavior::InfiniteHeight)
                    .lazy_layout()
                    .line_height_override(CODE_REVIEW_EDITOR_LINE_HEIGHT_RATIO)
                    .with_show_comment_editor_provider(ShowCommentEditor {
                        comment_list_save_position_id,
                        window_id: ctx.window_id(),
                    }),
                ctx,
            )
            .with_add_context_button()
            .with_revert_diff_hunk_button()
            .with_comment_button()
            .with_collapsible_diffs(false)
            .disable_diff_indicator_expansion_on_hover()
            .with_gutter_hover_target(GutterHoverTarget::Line)
            .disable_find_and_replace();
            editor_view.set_show_nav_bar(false);
            editor_view
        });

        if let Some(repo_path) = self.repo_path() {
            let language_path = repo_path.join(file_path).path_component();
            code_editor_view.update(ctx, |editor, ctx| {
                editor.set_language_with_path(&language_path, ctx);
            });
        }

        ctx.subscribe_to_view(&code_editor_view, {
            let file_path = file_path.to_string();
            move |me, _, event, ctx| me.handle_baseline_editor_event(&file_path, event, ctx)
        });

        let editor = ctx.add_typed_action_view(|ctx| {
            LocalCodeEditorView::new(code_editor_view, None, false, None, ctx)
        });
        fill_baseline(&editor, &base, ctx);

        SplitBaselinePane { editor, base }
    }

    /// Copies collapsed sections between the two laid-out panes of a split file.
    fn mirror_baseline_hidden_lines(
        &mut self,
        file_path: &str,
        direction: MirrorDirection,
        ctx: &mut ViewContext<Self>,
    ) {
        let Some(editor_state) = self.editor_state_mut(file_path) else {
            return;
        };
        let Some(baseline) = editor_state.baseline.as_ref() else {
            return;
        };
        let modified = editor_state.editor.as_ref(ctx).editor().clone();
        let baseline = baseline.editor.as_ref(ctx).editor().clone();
        let alignment = modified
            .as_ref(ctx)
            .model
            .as_ref(ctx)
            .split_diff_alignment(ctx);
        mirror_hidden_lines(&modified, &baseline, &alignment, direction, true, ctx);
    }

    fn handle_baseline_editor_event(
        &mut self,
        file_path: &str,
        event: &CodeEditorEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            CodeEditorEvent::DiffHunkContextAdded { line_range } => {
                if let Some(modified_range) =
                    self.modified_hunk_for_base_lines(file_path, line_range, ctx)
                {
                    self.insert_diff_hunk_as_context(file_path.to_string(), modified_range, ctx);
                }
            }
            CodeEditorEvent::SplitBaselineRevertRequested { line_range } => {
                if let Some(modified_range) =
                    self.modified_hunk_for_base_lines(file_path, line_range, ctx)
                    && let Some(editor_state) = self.editor_state_mut(file_path)
                {
                    let modified = editor_state.editor.as_ref(ctx).editor().clone();
                    modified.update(ctx, |editor, ctx| {
                        editor.revert_diff_hunk(&modified_range, ctx)
                    });
                }
            }
            CodeEditorEvent::CommentSaved { comment } => {
                let Some(editor_state) = self.editor_state_mut(file_path) else {
                    return;
                };
                let local_editor = editor_state.editor.clone();
                let model = local_editor.as_ref(ctx).editor().as_ref(ctx).model.clone();
                let Some(base_line) = comment.line.line_number().map(RenderLineCount::as_usize)
                else {
                    return;
                };
                let removed_line = model
                    .as_ref(ctx)
                    .diff()
                    .as_ref(ctx)
                    .base_line_index_to_line_location(base_line);
                let Some(line) = removed_line.or_else(|| {
                    unchanged_line_location(&model.as_ref(ctx).split_diff_alignment(ctx), base_line)
                }) else {
                    return;
                };
                let diff_content = model.as_ref(ctx).get_diff_content_for_line(&line, ctx);
                let comment = EditorReviewComment::new_with_id(
                    comment.id,
                    line,
                    diff_content,
                    comment.comment_content.clone(),
                );
                self.save_editor_comment(&local_editor, comment, ctx);
            }
            CodeEditorEvent::RequestOpenComment(comment_id) => {
                let Some(existing_comment) = self.get_comment_by_id(*comment_id, ctx) else {
                    return;
                };
                let AttachedReviewCommentTarget::Line { line, .. } = &existing_comment.target
                else {
                    return;
                };
                let Some(editor_state) = self.editor_state_mut(file_path) else {
                    return;
                };
                let Some(baseline) = editor_state.baseline.as_ref() else {
                    return;
                };
                let baseline_editor = baseline.editor.as_ref(ctx).editor().clone();
                let modified = editor_state.editor.as_ref(ctx).editor().clone();
                let Some(base_line) = baseline_line_for_removed(&modified, line, ctx) else {
                    return;
                };
                let comment_text = existing_comment.content.clone();
                let origin = existing_comment.origin.clone();
                baseline_editor.update(ctx, |editor, ctx| {
                    editor.open_existing_comment(
                        comment_id,
                        &base_line,
                        &comment_text,
                        &origin,
                        ctx,
                    );
                });
            }
            CodeEditorEvent::DeleteComment { id } => {
                self.delete_comment_by_id(*id, ctx);
            }
            CodeEditorEvent::HiddenSectionExpanded => {
                self.mirror_baseline_hidden_lines(file_path, MirrorDirection::FromBase, ctx);
                self.invalidate_file_height(file_path, ctx);
            }
            // Loading the base text recomputes the base pane's own (empty) diff, and the rebuild
            // that follows drops the collapsed sections copied from the modified pane. Copy them
            // again once that lands.
            CodeEditorEvent::DiffUpdated => {
                self.mirror_baseline_hidden_lines(file_path, MirrorDirection::FromModified, ctx);
                self.invalidate_file_height(file_path, ctx);
            }
            CodeEditorEvent::Focused => {
                ctx.emit(CodeReviewViewEvent::Pane(PaneEvent::FocusSelf));
            }
            // The base pane is read-only and has no diff of its own, so these need no routing.
            CodeEditorEvent::ContentChanged { .. }
            | CodeEditorEvent::UnifiedDiffComputed
            | CodeEditorEvent::SelectionChanged
            | CodeEditorEvent::SelectionStart
            | CodeEditorEvent::SelectionEnd
            | CodeEditorEvent::CopiedEmptyText
            | CodeEditorEvent::VimEscapeInNormalMode
            | CodeEditorEvent::EscapePressed
            | CodeEditorEvent::DiffReverted
            | CodeEditorEvent::CommentEditorOpened
            | CodeEditorEvent::ViewportUpdated
            | CodeEditorEvent::DelayedRenderingFlushed
            | CodeEditorEvent::LayoutInvalidated
            | CodeEditorEvent::MouseHovered { .. }
            | CodeEditorEvent::VimGotoDefinition
            | CodeEditorEvent::VimFindReferences
            | CodeEditorEvent::VimShowHover => {}
            #[cfg(windows)]
            CodeEditorEvent::WindowsCtrlC { .. } => {}
        }
    }

    /// The modified-pane line range of the hunk that removed the given base lines.
    fn modified_hunk_for_base_lines(
        &mut self,
        file_path: &str,
        base_lines: &Range<RenderLineCount>,
        ctx: &mut ViewContext<Self>,
    ) -> Option<Range<RenderLineCount>> {
        let editor_state = self.editor_state_mut(file_path)?;
        let modified = editor_state.editor.as_ref(ctx).editor().clone();
        let spans = modified
            .as_ref(ctx)
            .model
            .as_ref(ctx)
            .diff()
            .as_ref(ctx)
            .diff_status()
            .change_spans();
        modified_range_for_base_line(&spans, base_lines.start.as_usize())
    }

    /// Shows the comments on removed lines in the base pane, where those lines are drawn.
    pub(super) fn set_split_baseline_comment_locations(
        &self,
        editor: &ViewHandle<LocalCodeEditorView>,
        comments: &[EditorReviewComment],
        ctx: &mut ViewContext<Self>,
    ) {
        let CodeReviewViewState::Loaded(loaded) = self.state() else {
            return;
        };
        let Some(baseline) = loaded
            .file_states
            .values()
            .filter_map(|file| file.editor_state.as_ref())
            .find(|editor_state| editor_state.editor.id() == editor.id())
            .and_then(|editor_state| editor_state.baseline.as_ref())
        else {
            return;
        };
        let modified = editor.as_ref(ctx).editor().clone();
        let baseline_comments = comments
            .iter()
            .filter_map(|comment| {
                let line = baseline_line_for_removed(&modified, &comment.line, ctx)?;
                let mut comment = comment.clone();
                comment.line = line;
                Some(comment)
            })
            .collect::<Vec<_>>();
        baseline
            .editor
            .as_ref(ctx)
            .editor()
            .clone()
            .update(ctx, |code_editor, ctx| {
                code_editor.set_comment_locations(baseline_comments.into_iter(), ctx);
            });
    }

    pub(super) fn invalidate_file_height(&mut self, file_path: &str, ctx: &mut ViewContext<Self>) {
        if let CodeReviewViewState::Loaded(loaded) = self.state()
            && let Some(index) = loaded.file_states.get_index_of(file_path)
        {
            self.viewported_list_state
                .invalidate_height_for_index(index);
            ctx.notify();
        }
    }

    fn editor_state_mut(&mut self, file_path: &str) -> Option<&mut CodeReviewEditorState> {
        match self.state_mut()? {
            CodeReviewViewState::Loaded(loaded) => {
                loaded.file_states.get_mut(file_path)?.editor_state.as_mut()
            }
            CodeReviewViewState::None
            | CodeReviewViewState::NoRepoFound
            | CodeReviewViewState::Error(_) => None,
        }
    }
}

/// The modified-pane lines of the change that removed or replaced `base_line`.
fn modified_range_for_base_line(
    spans: &[ChangeSpan],
    base_line: usize,
) -> Option<Range<RenderLineCount>> {
    spans
        .iter()
        .find(|span| span.base.contains(&base_line))
        .map(|span| {
            RenderLineCount::from(span.modified.start)..RenderLineCount::from(span.modified.end)
        })
}

/// The modified-pane location of an unchanged base line, which is the line on the same row.
fn unchanged_line_location(
    alignment: &DiffAlignment,
    base_line: usize,
) -> Option<EditorLineLocation> {
    let row = alignment
        .rows()
        .get(alignment.row_for_base_line(base_line)?)?;
    if row.is_change {
        return None;
    }
    let line = row.modified?;
    Some(EditorLineLocation::Current {
        line_number: RenderLineCount::from(line),
        line_range: RenderLineCount::from(line)..RenderLineCount::from(line + 1),
    })
}

fn fill_baseline(
    editor: &ViewHandle<LocalCodeEditorView>,
    base: &MultilineString<LF>,
    ctx: &mut ViewContext<CodeReviewView>,
) {
    editor.update(ctx, |local_editor, ctx| {
        let state =
            InitialBufferState::plain_text(base.as_str()).with_version(ContentVersion::new());
        local_editor.reset_with_state(state, ctx);
        local_editor.editor().update(ctx, |editor, ctx| {
            editor.set_interaction_state(InteractionState::Selectable, ctx);
        });
    });
}

/// The base pane location of a comment made on a removed line of the modified pane.
fn baseline_line_for_removed(
    modified: &ViewHandle<CodeEditorView>,
    line: &EditorLineLocation,
    ctx: &AppContext,
) -> Option<EditorLineLocation> {
    if !matches!(line, EditorLineLocation::Removed { .. }) {
        return None;
    }
    let diff = modified.as_ref(ctx).model.as_ref(ctx).diff().as_ref(ctx);
    let base_line = diff.deleted_line_to_base_line_index(line)?;
    let line_range = diff
        .diff_status()
        .change_spans()
        .into_iter()
        .find(|span| span.base.contains(&base_line))
        .map_or(base_line..base_line + 1, |span| span.base);
    Some(EditorLineLocation::Current {
        line_number: RenderLineCount::from(base_line),
        line_range: RenderLineCount::from(line_range.start)..RenderLineCount::from(line_range.end),
    })
}

fn apply_alignment_to_baseline(
    editor: &ViewHandle<LocalCodeEditorView>,
    alignment: &DiffAlignment,
    ctx: &mut ViewContext<CodeReviewView>,
) {
    let removed_lines = alignment.changed_base_lines();
    let code_editor = editor.as_ref(ctx).editor().clone();
    let mode = DiffDecorationMode::SplitBaseline {
        removed_lines: removed_lines.clone(),
        gaps: alignment.base_gaps(),
    };
    code_editor.update(ctx, |editor, ctx| {
        editor.set_split_baseline_removed_lines(Some(&removed_lines), ctx);
        editor.set_diff_decoration_mode(mode, ctx);
    });
}

/// Which pane of a split file collapsed sections are copied from.
#[derive(Clone, Copy)]
enum MirrorDirection {
    FromModified,
    FromBase,
}

/// Copies the collapsed unchanged ranges from one pane to the other. Collapsed ranges only ever
/// cover unchanged lines, which sit on the same rows in both panes.
///
/// A laid-out target only shows changed folds after a layout rebuild, which `rebuild` allows. Pass
/// false for a target that hasn't laid out yet, where the folds apply on the first layout.
fn mirror_hidden_lines(
    modified: &ViewHandle<CodeEditorView>,
    baseline: &ViewHandle<CodeEditorView>,
    alignment: &DiffAlignment,
    direction: MirrorDirection,
    rebuild: bool,
    ctx: &mut ViewContext<CodeReviewView>,
) {
    let from_modified = match direction {
        MirrorDirection::FromModified => true,
        MirrorDirection::FromBase => false,
    };
    let (source, target) = if from_modified {
        (modified, baseline)
    } else {
        (baseline, modified)
    };
    let map_line = |line: usize| {
        if from_modified {
            alignment.base_line_for_modified_line(line)
        } else {
            alignment.modified_line_for_base_line(line)
        }
    };
    let (source_line_count, target_line_count) = if from_modified {
        (alignment.modified_line_count(), alignment.base_line_count())
    } else {
        (alignment.base_line_count(), alignment.modified_line_count())
    };
    let source_ranges = source.as_ref(ctx).model.as_ref(ctx).hidden_line_ranges(ctx);
    let target_ranges: RangeSet<LineCount> = source_ranges
        .into_iter()
        .filter_map(|range: Range<usize>| {
            map_hidden_range(range, source_line_count, target_line_count, map_line)
        })
        .collect();
    let current: RangeSet<LineCount> = target
        .as_ref(ctx)
        .model
        .as_ref(ctx)
        .hidden_line_ranges(ctx)
        .into_iter()
        .map(|range| LineCount::from(range.start)..LineCount::from(range.end))
        .collect();
    if current == target_ranges {
        return;
    }
    target.update(ctx, |editor, ctx| {
        editor.model.update(ctx, |model, ctx| {
            if rebuild {
                model.set_hidden_lines_and_rebuild(target_ranges, ctx);
            } else {
                model.set_hidden_lines(target_ranges, ctx);
            }
        });
    });
}

/// Maps a collapsed range of source pane lines onto the target pane.
///
/// A collapsed range at the end of a file can include the empty line after a trailing newline,
/// which the alignment doesn't cover. Both panes have that line, so lines past the aligned ones
/// map one to one.
fn map_hidden_range(
    range: Range<usize>,
    source_line_count: usize,
    target_line_count: usize,
    map_line: impl Fn(usize) -> Option<usize>,
) -> Option<Range<LineCount>> {
    let map = |line: usize| {
        if line < source_line_count {
            map_line(line)
        } else {
            Some(target_line_count + (line - source_line_count))
        }
    };
    let start = map(range.start)?;
    let end = map(range.end.checked_sub(1)?)? + 1;
    (start < end).then(|| LineCount::from(start)..LineCount::from(end))
}

#[cfg(test)]
#[path = "split_diff_tests.rs"]
mod tests;
