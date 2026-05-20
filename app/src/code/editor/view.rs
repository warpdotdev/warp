#![cfg_attr(target_family = "wasm", allow(dead_code, unused_imports))]
// Adding this file level gate as some of the code around editability is not used in WASM yet.

use crate::code::editor::{
    comment_editor::{CommentEditor, CommentEditorEvent},
    comments::PendingComment,
    diff::DiffStatus,
    element::{
        AddAsContextButton, CommentButton, EditorWrapper, EditorWrapperStateHandle,
        GutterHoverTarget, GutterRange, InnerEditor, LineNumberConfig, RevertHunkButton,
    },
    find::view::{CodeEditorFind as Find, Event as FindViewEvent},
    goto_line::view::{Event as GoToLineEvent, GoToLineView},
    line::EditorLineLocation,
    model::{CodeEditorModel, CodeEditorModelEvent, HoverableLink, LineBound, StableEditorLine},
    nav_bar::{NavBar, NavBarBehavior, NavBarEvent},
    scroll::{ScrollPosition, ScrollTrigger, ScrollWheelBehavior},
};
use crate::code::{
    editor::EditorReviewComment, DiffResult, NoopCommentEditorProvider,
    NoopFindReferencesCardProvider, ShowCommentEditorProvider, ShowFindReferencesCardProvider,
};
use crate::{
    appearance::Appearance,
    code_review::comments::{CommentId, CommentOrigin},
    editor::InteractionState,
    features::FeatureFlag,
    notebooks::editor::rich_text_styles,
    settings::{AppEditorSettings, FontSettings},
    view_components::find::FindDirection,
};
use ai::diff_validation::DiffDelta;
use lazy_static::lazy_static;
use num_traits::SaturatingSub;
use pathfinder_geometry::vector::vec2f;
use std::fmt::Debug;
use std::rc::Rc;
use std::{collections::HashMap, ops::Range};
use std::{collections::HashSet, path::Path};
use string_offset::CharOffset;
use vec1::{vec1, Vec1};
use vim::vim::{Direction, InsertPosition, VimMode, VimModel, VimState, VimSubscriber};
use warp_core::platform::SessionPlatform;
use warp_editor::{
    content::{
        buffer::{
            Buffer, BufferEditAction, EditOrigin, InitialBufferState, ToBufferCharOffset as _,
            ToBufferPoint,
        },
        text::IndentUnit,
        version::BufferVersion,
    },
    model::{CoreEditorModel, PlainTextEditorModel},
    multiline::AnyMultilineString,
    render::{
        element::{
            lens_element::RichTextElementLens, DisplayOptions, DisplayStateHandle, RichTextElement,
            VerticalExpansionBehavior,
        },
        model::{
            AutoScrollMode, BlockSpacing, Decoration, ExpansionType, LineCount, ParagraphStyles,
            RichTextStyles, CODE_EDITOR_HIDDEN_SECTION_EXPANSION_LINES,
        },
    },
    search::{SearchEvent, Searcher, MATCH_FILL, SELECTED_MATCH_FILL},
};
use warp_util::content_version::ContentVersion;
use warpui::{
    elements::{
        new_scrollable::{
            AxisConfiguration, DualAxisConfig, NewScrollableElement, ScrollableAppearance,
        },
        ChildAnchor, ChildView, Dismiss, Fill, Flex, Margin, MouseStateHandle, NewScrollable,
        OffsetPositioning, Padding, ParentAnchor, ParentElement, ParentOffsetBounds,
        ScrollStateHandle, Shrinkable, Stack,
    },
    event::ModifiersState,
    keymap::Keystroke,
    platform::Cursor,
    prelude::RectF,
    text::point::Point,
    units::Pixels,
    AppContext, BlurContext, CursorInfo, Element, Entity, FocusContext, ModelHandle,
    SingletonEntity, View, ViewContext, ViewHandle, WeakViewHandle, WindowId,
};

mod actions;
pub use actions::init;
pub(super) use actions::CodeEditorViewAction;

mod vim_handler;

/// Limit the keybindings that conflict with the Agent Mode embedded editor.
const NON_EDITABLE_KEYMAP_CONTEXT: &str = "NonEditableKeymapContext";

lazy_static! {
    static ref AUTOCOMPLETE_SYMBOLS: HashMap<char, char> =
        HashMap::from([('(', ')'), ('[', ']'), ('{', '}'), ('\'', '\''), ('"', '"'),]);
    static ref CLOSING_SYMBOLS: HashSet<char> = AUTOCOMPLETE_SYMBOLS.values().cloned().collect();
}

pub enum CodeEditorEvent {
    Focused,
    ContentChanged {
        origin: EditOrigin,
    },
    UnifiedDiffComputed(Rc<DiffResult>),
    SelectionChanged,
    SelectionStart,
    SelectionEnd,
    CopiedEmptyText,
    /// Emitted when the editor is in Vim mode and a user Escape while already in Normal mode.
    VimEscapeInNormalMode,
    /// Emitted when escape key is pressed (regardless of Vim state).
    /// This allows parent views to handle escape for closing overlays.
    EscapePressed,
    /// Emitted when diff decorations are updated (line highlights, removed lines, etc.)
    DiffUpdated,
    /// Emitted when the plus icon is clicked to add diff hunk context
    DiffHunkContextAdded {
        #[allow(dead_code)]
        line_range: Range<LineCount>,
    },
    /// Emitted when a diff hunk is reverted
    DiffReverted,
    HiddenSectionExpanded,
    /// Emitted when a comment is saved. This gets propagated up so that it
    /// can be augmented with the file and repo paths and saved to the comment model.
    CommentSaved {
        comment: EditorReviewComment,
    },
    RequestOpenComment(CommentId),
    /// Emitted when the viewport is updated after layout
    ViewportUpdated,
    DelayedRenderingFlushed,
    /// Emitted when the render state layout has been updated.
    LayoutInvalidated,
    #[cfg(windows)]
    WindowsCtrlC {
        /// True if the `ctrl-c` action was used to copy an active selection.
        copied_selection: bool,
    },
    MouseHovered {
        offset: CharOffset,
        cmd: bool,
        clamped: bool,
        /// Whether the mouse move event was covered by an element above the editor.
        is_covered: bool,
    },
    DeleteComment {
        id: CommentId,
    },
    VimGotoDefinition,
    VimFindReferences,
    VimShowHover,
}

/// Store all states related to displaying the editor content.
#[derive(Default)]
struct DisplayHandles {
    vertical_scroll_state: ScrollStateHandle,
    horizontal_scroll_state: ScrollStateHandle,
    display_state: DisplayStateHandle,
    wrapper_state_handle: EditorWrapperStateHandle,
}

#[derive(Debug, Clone, Copy)]
struct CodeEditorViewDisplayOptions {
    vertical_expansion_behavior: VerticalExpansionBehavior,
    /// If `false`, we will never show the diff UI in this code editor.
    can_show_diff_ui: bool,
    collapsible_diffs: bool,
    show_line_numbers: bool,
    scroll_wheel_behavior: ScrollWheelBehavior,
    horizontal_scrollbar_appearance: ScrollableAppearance,
    vertical_scrollbar_appearance: ScrollableAppearance,
    show_nav_bar: bool,
    /// The add as context button, or `None` if it is not currently visible.
    diff_hunk_as_context: Option<AddAsContextButton>,
    /// The revert diff button, or `None` if it is not currently visible.
    revert_diff_hunk: Option<RevertHunkButton>,
    /// The add comment button, or `None` if it is not currently visible.
    comment_button: Option<CommentButton>,
    /// Whether to expand the width of the diff indicator in the gutter on hover.
    expand_diff_indicator_width_on_hover: bool,
    // Provides a starting line number for markdown code blocks.
    starting_line_number: Option<usize>,
    gutter_hover_target: GutterHoverTarget,
    line_height_override: Option<f32>,
}

#[derive(Clone, Debug)]
pub(super) struct SavedComment {
    uuid: CommentId,
    location: EditorLineLocation,
    mouse_state: MouseStateHandle,
}

impl SavedComment {
    pub fn location(&self) -> &EditorLineLocation {
        &self.location
    }

    pub fn mouse_state(&self) -> &MouseStateHandle {
        &self.mouse_state
    }

    pub fn uuid(&self) -> CommentId {
        self.uuid
    }
}

#[derive(Debug)]
pub struct CodeEditorRenderOptions {
    vertical_expansion_behavior: VerticalExpansionBehavior,
    line_height_override: Option<f32>,
    lazy_layout: bool,
    show_comment_editor_provider: Box<dyn ShowCommentEditorProvider>,
    show_find_references_provider: Box<dyn ShowFindReferencesCardProvider>,
}

impl CodeEditorRenderOptions {
    pub fn new(vertical_expansion_behavior: VerticalExpansionBehavior) -> Self {
        Self {
            vertical_expansion_behavior,
            line_height_override: None,
            lazy_layout: false,
            show_comment_editor_provider: Box::new(NoopCommentEditorProvider),
            show_find_references_provider: Box::new(NoopFindReferencesCardProvider),
        }
    }

    pub fn lazy_layout(mut self) -> Self {
        self.lazy_layout = true;
        self
    }

    pub fn line_height_override(mut self, line_height: f32) -> Self {
        self.line_height_override = Some(line_height);
        self
    }

    pub fn with_show_comment_editor_provider(
        mut self,
        comment_editor_provider: impl ShowCommentEditorProvider,
    ) -> Self {
        self.show_comment_editor_provider = Box::new(comment_editor_provider);
        self
    }

    pub fn with_show_find_references_provider(
        mut self,
        find_references_provider: impl ShowFindReferencesCardProvider,
    ) -> Self {
        self.show_find_references_provider = Box::new(find_references_provider);
        self
    }
}

pub struct CodeEditorView {
    pub model: ModelHandle<CodeEditorModel>,
    pub searcher: ModelHandle<Searcher>,
    find_bar: Option<ViewHandle<Find>>,
    goto_line_dialog: ViewHandle<GoToLineView>,
    nav_bar: ViewHandle<NavBar>,
    display_states: DisplayHandles,
    is_selecting: bool,
    self_handle: WeakViewHandle<Self>,
    display_options: CodeEditorViewDisplayOptions,
    pending_scroll: Option<ScrollTrigger>,
    supports_vim_mode: bool,
    vim_model: ModelHandle<VimModel>,
    // Track the most recent Vim search direction to determine how to cycle (n/N) thereafter.
    last_search_direction: Direction,
    active_comment_editor: ViewHandle<CommentEditor>,
    /// TODO: maybe turn into a map for fast UUID or range lookup
    comment_locations: Vec<SavedComment>,
    /// Save position of the comment button rendered within this code editor view.
    comment_save_position_id: String,
    show_comment_editor_provider: Box<dyn ShowCommentEditorProvider>,
    /// Save position of the anchor point for find references card.
    find_references_save_position_id: String,
    show_find_references_provider: Box<dyn ShowFindReferencesCardProvider>,
    /// The offset where find references card is anchored (if showing).
    find_references_anchor_offset: Option<CharOffset>,
    window_id: WindowId,
}

impl CodeEditorView {
    /// A [`SessionPlatform`] is used to determine the default line ending for
    /// the code editor. It should be provided if the code editor can modify
    /// files on the user's system.
    pub fn new(
        session_platform: Option<SessionPlatform>,
        buffer: Option<ModelHandle<Buffer>>,
        render_options: CodeEditorRenderOptions,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        let appearance_handle = Appearance::handle(ctx);
        let font_settings_handle = FontSettings::handle(ctx);
        let initial_styles = code_text_styles(
            appearance_handle.as_ref(ctx),
            font_settings_handle.as_ref(ctx),
            render_options.line_height_override,
        );
        ctx.subscribe_to_model(&appearance_handle, |me, _, _, ctx| {
            me.handle_appearance_or_font_change(ctx);
        });
        ctx.subscribe_to_model(&font_settings_handle, |me, _, _, ctx| {
            me.handle_appearance_or_font_change(ctx);
        });

        let model = ctx.add_model(|ctx| {
            CodeEditorModel::new(
                initial_styles,
                session_platform,
                render_options.lazy_layout,
                buffer,
                ctx,
            )
        });
        ctx.subscribe_to_model(&model, |me, _, event, ctx| {
            me.handle_editor_model_event(event, ctx);
        });

        // Creates a new model for searching the editor
        let buffer = model.as_ref(ctx).buffer().clone();
        let selection_model = model.as_ref(ctx).buffer_selection_model().clone();
        let searcher = ctx.add_model(|ctx| Searcher::new(buffer, selection_model, ctx));
        // Subscribes to events from Searcher model so that we can handle
        ctx.subscribe_to_model(&searcher, |me, _, event, ctx| {
            me.handle_searcher_event(event, ctx);
        });

        let find_bar = ctx.add_typed_action_view(|ctx| Find::new(searcher.clone(), ctx));
        ctx.subscribe_to_view(&find_bar, move |me, _, event, ctx| {
            me.handle_find_event(event, ctx);
        });

        let goto_line_dialog = ctx.add_typed_action_view(GoToLineView::new);
        ctx.subscribe_to_view(&goto_line_dialog, |me, _, event, ctx| {
            me.handle_goto_line_event(event, ctx);
        });

        let nav_bar = ctx.add_typed_action_view(|ctx| NavBar::new(model.clone(), ctx));
        ctx.subscribe_to_view(&nav_bar, |me, _, event, ctx| match event {
            NavBarEvent::Close => {
                me.toggle_diff_nav(None, ctx);
                ctx.notify();
            }
        });

        // If feature flag is enabled, enable vim mode.
        let supports_vim_mode = FeatureFlag::VimCodeEditor.is_enabled();

        let vim_model = ctx.add_model(|_| VimModel::new());
        ctx.subscribe_to_model(&vim_model, Self::handle_vim_event);

        // Ensure CodeEditorView starts in Normal mode when Vim keybindings are enabled.
        if supports_vim_mode && AppEditorSettings::as_ref(ctx).vim_mode_enabled() {
            vim_model.update(ctx, |vim_model, ctx| {
                if let Ok(escape) = Keystroke::parse("escape") {
                    vim_model.keypress(&escape, ctx);
                }
            });
        }
        // Ensure that we re-render when the rendering model changes.
        ctx.observe(&model.as_ref(ctx).render_state().clone(), |_, _, ctx| {
            ctx.notify();
        });

        let comment_model = model.as_ref(ctx).comments().clone();
        let comment_editor =
            ctx.add_typed_action_view(|ctx| CommentEditor::new(ctx, comment_model));
        ctx.subscribe_to_view(&comment_editor, |me, _, event, ctx| {
            me.handle_comment_editor_event(event, ctx);
        });

        Self {
            searcher,
            find_bar: Some(find_bar),
            goto_line_dialog,
            model,
            display_states: Default::default(),
            is_selecting: false,
            self_handle: ctx.handle(),
            nav_bar,
            comment_locations: Vec::new(),
            display_options: CodeEditorViewDisplayOptions {
                vertical_expansion_behavior: render_options.vertical_expansion_behavior,
                can_show_diff_ui: true,
                collapsible_diffs: true,
                show_line_numbers: true,
                starting_line_number: None,
                show_nav_bar: true,
                diff_hunk_as_context: Default::default(),
                revert_diff_hunk: Default::default(),
                comment_button: Default::default(),
                // By default expand diff indicators on hover.
                expand_diff_indicator_width_on_hover: true,
                scroll_wheel_behavior: ScrollWheelBehavior::AlwaysHandle,
                // By default, we should render the horizontal scrollbar as overlay to prevent it
                // from truncating space for the code editor. We should not render it as an overlay
                // for small code editors.
                horizontal_scrollbar_appearance: ScrollableAppearance::new(
                    warpui::elements::ScrollbarWidth::Auto,
                    false,
                ),
                vertical_scrollbar_appearance: ScrollableAppearance::new(
                    warpui::elements::ScrollbarWidth::Auto,
                    false,
                ),
                gutter_hover_target: GutterHoverTarget::GutterElement,
                line_height_override: render_options.line_height_override,
            },
            pending_scroll: None,
            supports_vim_mode,
            vim_model,
            last_search_direction: Direction::Forward,
            active_comment_editor: comment_editor,
            comment_save_position_id: format!("code_editor_comment_{}", ctx.view_id()),
            show_comment_editor_provider: render_options.show_comment_editor_provider,
            find_references_save_position_id: format!(
                "code_editor_find_references_{}",
                ctx.view_id()
            ),
            show_find_references_provider: render_options.show_find_references_provider,
            find_references_anchor_offset: None,
            window_id: ctx.window_id(),
        }
    }

    pub fn set_find_references_anchor_offset(&mut self, offset: Option<CharOffset>) {
        self.find_references_anchor_offset = offset;
    }

    pub fn find_references_save_position_id(&self) -> &str {
        &self.find_references_save_position_id
    }

    pub fn show_find_references_provider(&self) -> &dyn ShowFindReferencesCardProvider {
        self.show_find_references_provider.as_ref()
    }

    pub fn window_id(&self) -> WindowId {
        self.window_id
    }

    pub fn set_add_diff_hunk_as_context_button(
        &mut self,
        enabled: bool,
        ctx: &mut ViewContext<Self>,
    ) {
        self.display_options.diff_hunk_as_context = Some(AddAsContextButton::new(enabled));
        ctx.notify();
    }

    /// Enables the add context button (plus icon) in diff hunks. Only enable this for code review views.
    pub fn with_add_context_button(mut self) -> Self {
        self.display_options.diff_hunk_as_context =
            Some(AddAsContextButton::new(true /* enabled */));
        self
    }

    /// Enables the "revert" button on diff hunks. Only enable this for code review views.
    pub fn with_revert_diff_hunk_button(mut self) -> Self {
        self.display_options.revert_diff_hunk =
            Some(RevertHunkButton::new(true /* is_enabled */));
        self
    }

    /// Enables the "comment" button on diff hunks. Only enable this for code review views.
    pub fn with_comment_button(mut self) -> Self {
        self.display_options.comment_button = Some(CommentButton::default());
        self
    }

    /// Disables the diff indicator expanding on hover.
    pub fn disable_diff_indicator_expansion_on_hover(mut self) -> Self {
        self.display_options.expand_diff_indicator_width_on_hover = false;
        self
    }

    pub fn with_gutter_hover_target(mut self, target: GutterHoverTarget) -> Self {
        self.display_options.gutter_hover_target = target;
        self
    }

    /// Enables clicking on diff hunk gutter elements to collapse changed sections.
    pub fn with_collapsible_diffs(mut self, enabled: bool) -> Self {
        self.display_options.collapsible_diffs = enabled;
        self
    }

    pub(crate) fn disable_find_and_replace(mut self) -> Self {
        self.find_bar = None;
        self
    }

    fn find_bar_open(&self, ctx: &mut ViewContext<Self>) -> bool {
        if let Some(find) = &self.find_bar {
            find.as_ref(ctx).is_open()
        } else {
            false
        }
    }

    fn show_find_bar(&mut self, ctx: &mut ViewContext<Self>) {
        let Some(find_bar) = &self.find_bar else {
            return;
        };

        // Pre-populate the search field with the currently selected text if it's a single line
        let should_populate_query = self.model.as_ref(ctx).selection_is_single_range(ctx);

        if should_populate_query {
            let selection_offsets = *self.model.as_ref(ctx).selections(ctx).first();
            let selected_text = self
                .model
                .as_ref(ctx)
                .buffer()
                .as_ref(ctx)
                .text_in_range(selection_offsets.head..selection_offsets.tail)
                .into_string();
            // Only use the selected text if it's a single line (no newlines)
            if !selected_text.contains('\n') && !selected_text.trim().is_empty() {
                find_bar.update(ctx, |find_bar, ctx| {
                    find_bar.set_find_query(ctx, &selected_text);
                });
            }
        }

        find_bar.update(ctx, |find_bar, _ctx| {
            find_bar.set_open(true);
        });
        self.update_decorations_and_position(ctx);
        ctx.focus(find_bar);
        ctx.notify();
    }

    pub fn changed_lines(&self, app: &AppContext) -> Vec<Range<usize>> {
        self.model
            .as_ref(app)
            .diff()
            .as_ref(app)
            .modified_lines()
            .collect()
    }

    pub fn close_find_bar(&mut self, should_focus_editor: bool, ctx: &mut ViewContext<Self>) {
        if let Some(find_bar) = &self.find_bar {
            let should_update = find_bar.update(ctx, |find_bar, _ctx| {
                if find_bar.is_open() {
                    find_bar.set_open(false);
                    true
                } else {
                    false
                }
            });

            if should_update {
                self.update_decorations_and_position(ctx);
            }
        }

        if should_focus_editor {
            self.focus(ctx);
        }

        ctx.notify();
    }

    pub fn hide_lines_outside_of_active_diff(
        &self,
        context_lines: usize,
        ctx: &mut ViewContext<Self>,
    ) {
        self.model.update(ctx, |model, ctx| {
            model.hide_lines_outside_of_active_diff(context_lines, ctx)
        });
    }

    pub fn set_base(&self, base: &str, recompute_diff: bool, ctx: &mut ViewContext<Self>) {
        self.model
            .update(ctx, |model, ctx| model.set_base(base, recompute_diff, ctx));
    }

    pub fn lens_for_line_range(
        &self,
        line_range: Range<EditorLineLocation>,
        ctx: &AppContext,
    ) -> Box<dyn Element> {
        let render_state = self.model.as_ref(ctx).render_state();
        let lens = RichTextElementLens::<Self>::new(
            line_range.start.into_render_line_location()
                ..line_range.end.into_render_line_location(),
            render_state.clone(),
            self.self_handle.clone(),
            Default::default(),
        );

        let line_number_config = self.line_number_config(ctx);

        let diff_status = if self.display_options.can_show_diff_ui {
            self.model.as_ref(ctx).diff_status(ctx)
        } else {
            DiffStatus::default()
        };

        EditorWrapper::new(
            InnerEditor::Lens(lens),
            self.display_options.vertical_expansion_behavior,
            line_number_config,
            diff_status,
            Default::default(), /* Do not reuse the same hover state handle as the editor view */
            Box::new(|_, _| {}),
            false,
            self.model.as_ref(ctx).diff_navigation_state().clone(),
            None,
            Default::default(),
            Default::default(),
            Default::default(),
            vec![],
            false,
            self.display_options.gutter_hover_target,
            self.comment_save_position_id.clone(),
            self.find_references_save_position_id.clone(),
        )
        .finish()
    }

    fn show_goto_line(&mut self, ctx: &mut ViewContext<Self>) {
        if self.find_bar_open(ctx) {
            self.close_find_bar(false, ctx);
        }
        self.goto_line_dialog.update(ctx, |dialog, ctx| {
            dialog.open(ctx);
        });
        ctx.focus(&self.goto_line_dialog);
        ctx.notify();
    }

    fn close_goto_line_dialog(&mut self, ctx: &mut ViewContext<Self>) {
        self.goto_line_dialog.update(ctx, |dialog, ctx| {
            dialog.close(ctx);
        });
        self.focus(ctx);
        ctx.notify();
    }

    fn handle_goto_line_event(&mut self, event: &GoToLineEvent, ctx: &mut ViewContext<Self>) {
        match event {
            GoToLineEvent::Close => {
                self.close_goto_line_dialog(ctx);
            }
            GoToLineEvent::Confirm { input } => {
                let trimmed = input.trim().to_string();
                if trimmed.is_empty() {
                    self.goto_line_dialog.update(ctx, |dialog, ctx| {
                        dialog.set_error("Please enter a line number".to_string(), ctx);
                    });
                    return;
                }
                let (line_str, col_str) = match trimmed.split_once(':') {
                    Some((l, c)) => (l, Some(c)),
                    None => (trimmed.as_str(), None),
                };
                let line_number = match line_str.parse::<usize>() {
                    Ok(n) if n >= 1 => n,
                    _ => {
                        self.goto_line_dialog.update(ctx, |dialog, ctx| {
                            dialog.set_error("Please enter a valid line number".to_string(), ctx);
                        });
                        return;
                    }
                };
                let column = match col_str {
                    Some(c) if !c.is_empty() => match c.parse::<usize>() {
                        Ok(n) => Some(n),
                        Err(_) => {
                            self.goto_line_dialog.update(ctx, |dialog, ctx| {
                                dialog.set_error(
                                    "Please enter a valid column number".to_string(),
                                    ctx,
                                );
                            });
                            return;
                        }
                    },
                    _ => None,
                };
                let line_count = self.model.as_ref(ctx).line_count(ctx);
                let clamped_line = line_number.min(line_count + 1);
                self.model.update(ctx, |model, ctx| {
                    model.jump_to_line_column(clamped_line, column, ctx);
                });
                self.close_goto_line_dialog(ctx);
            }
        }
    }

    fn escape(&mut self, ctx: &mut ViewContext<Self>) {
        if self.goto_line_dialog.as_ref(ctx).is_open() {
            self.close_goto_line_dialog(ctx);
            return;
        }

        // If find bar is open, close it (editor handles escape)
        if self.find_bar_open(ctx) {
            self.close_find_bar(true, ctx);
            return;
        }

        // If vim mode is enabled, let vim handle escape
        if self.vim_mode_enabled(ctx) {
            self.enter_vim_normal_mode(ctx);
            return;
        }

        // Editor didn't handle escape - emit event for parent views (e.g., close overlays)
        ctx.emit(CodeEditorEvent::EscapePressed);
    }

    fn update_decorations_and_position(&self, ctx: &mut ViewContext<Self>) {
        // Updating search decorations based on matches
        let search_decorations = match self.find_bar_open(ctx) {
            true => self.searcher.as_ref(ctx).result_decorations(),
            false => Vec::new(),
        };
        self.model.update(ctx, |model, ctx| {
            model.render_state().update(ctx, |render, ctx| {
                render.set_text_decorations(search_decorations, ctx);
            });
        });

        // Updating position on code editor view based on match location
        if let Some(autoscroll_match) = self.searcher.as_ref(ctx).selected_match_range() {
            self.model
                .as_ref(ctx)
                .render_state()
                .clone()
                .update(ctx, |render_state, _ctx| {
                    render_state.request_autoscroll_to(AutoScrollMode::ScrollOffsetsIntoViewport(
                        autoscroll_match.clone(),
                    ));
                });
        }
    }

    pub fn set_pending_scroll(&mut self, trigger: ScrollTrigger) {
        self.pending_scroll = Some(trigger);
    }

    pub fn set_show_nav_bar(&mut self, show_nav_bar: bool) {
        self.display_options.show_nav_bar = show_nav_bar;
    }

    pub fn set_nav_bar_behavior(&self, behavior: NavBarBehavior, ctx: &mut ViewContext<Self>) {
        self.nav_bar.update(ctx, |nav_bar, _ctx| {
            nav_bar.set_behavior(behavior);
        });
    }

    pub fn set_show_current_line_highlights(
        &self,
        show_current_line_highlights: bool,
        ctx: &mut ViewContext<Self>,
    ) {
        self.model.update(ctx, |model, _ctx| {
            model.set_show_current_line_highlights(show_current_line_highlights);
        })
    }

    pub fn set_scroll_wheel_behavior(&mut self, behavior: ScrollWheelBehavior) {
        self.display_options.scroll_wheel_behavior = behavior;
    }

    pub fn set_vertical_scrollbar_appearance(&mut self, appearance: ScrollableAppearance) {
        self.display_options.vertical_scrollbar_appearance = appearance;
    }

    pub fn set_horizontal_scrollbar_appearance(&mut self, appearance: ScrollableAppearance) {
        self.display_options.horizontal_scrollbar_appearance = appearance;
    }

    pub fn set_show_find_references_provider(
        &mut self,
        provider: impl ShowFindReferencesCardProvider,
    ) {
        self.show_find_references_provider = Box::new(provider);
    }

    pub fn set_find_highlights(
        &self,
        ranges: Vec<Range<CharOffset>>,
        selected_range_index: Option<usize>,
        ctx: &mut ViewContext<Self>,
    ) {
        let decorations: Vec<Decoration> = ranges
            .into_iter()
            .enumerate()
            .map(|(index, range)| {
                let fill = if Some(index) == selected_range_index {
                    *SELECTED_MATCH_FILL
                } else {
                    *MATCH_FILL
                };
                Decoration::new(range.start, range.end).with_background(fill)
            })
            .collect();

        self.model.update(ctx, |model, ctx| {
            model.render_state().update(ctx, |render, ctx| {
                render.set_text_decorations(decorations, ctx);
            });
        });
    }

    /// Set diagnostic decorations (e.g., error/warning underlines) on the editor.
    pub fn set_diagnostic_decorations(
        &self,
        decorations: Vec<Decoration>,
        ctx: &mut ViewContext<Self>,
    ) {
        self.model.update(ctx, |model, ctx| {
            model.render_state().update(ctx, |render, ctx| {
                render.set_text_decorations(decorations, ctx);
            });
        });
    }

    pub fn character_bounds_in_viewport(
        &self,
        offset: CharOffset,
        app: &AppContext,
    ) -> Option<RectF> {
        let render_state = self.model.as_ref(app).render_state().as_ref(app);

        // Get the bounds of the character in viewport coordinates.
        let bounds = render_state.character_bounds_in_viewport(offset)?;

        // The render_state bounds are relative to the editor content area, but the
        // CodeEditorView also includes a gutter (line numbers). We need to offset
        // the bounds by the gutter width when line numbers are shown.
        let gutter_offset = if self.display_options.show_line_numbers {
            super::element::GUTTER_WIDTH
        } else {
            0.0
        };

        Some(RectF::new(
            bounds.origin() + vec2f(gutter_offset, 0.0),
            bounds.size(),
        ))
    }

    /// Returns the current viewport height in pixels, or None if not yet laid out.
    pub fn viewport_height(&self, app: &AppContext) -> Option<f32> {
        let render_state = self.model.as_ref(app).render_state().as_ref(app);
        let height = render_state.viewport().height();
        // Return None if the viewport hasn't been laid out yet (height is zero)
        if height.as_f32() > 0.0 {
            Some(height.as_f32())
        } else {
            None
        }
    }

    #[allow(clippy::single_range_in_vec_init)]
    fn expand_hidden_section(
        &mut self,
        line_range: Range<LineCount>,
        expansion_type: &ExpansionType,
        ctx: &mut ViewContext<Self>,
    ) {
        let hidden_section_start = line_range.start.as_usize();
        let hidden_section_end = line_range.end.as_usize();
        let lines_to_unhide = match expansion_type {
            ExpansionType::Both => {
                warp_editor::content::text::LineCount::from(hidden_section_start)
                    ..warp_editor::content::text::LineCount::from(hidden_section_end)
            }
            ExpansionType::ExpandDown => {
                let end = hidden_section_end
                    .min(hidden_section_start + CODE_EDITOR_HIDDEN_SECTION_EXPANSION_LINES);
                warp_editor::content::text::LineCount::from(hidden_section_start)
                    ..warp_editor::content::text::LineCount::from(end)
            }
            ExpansionType::ExpandUp => {
                let start = hidden_section_start.max(
                    hidden_section_end.saturating_sub(CODE_EDITOR_HIDDEN_SECTION_EXPANSION_LINES),
                );
                warp_editor::content::text::LineCount::from(start)
                    ..warp_editor::content::text::LineCount::from(hidden_section_end)
            }
        };
        self.model.update(ctx, |model, ctx| {
            model.set_visible_line_range(lines_to_unhide, ctx);
        });
        ctx.emit(CodeEditorEvent::HiddenSectionExpanded);
    }

    pub(crate) fn with_can_show_diff_ui(mut self, can_show_diff_ui: bool) -> Self {
        self.display_options.can_show_diff_ui = can_show_diff_ui;
        self
    }

    pub(crate) fn with_show_line_numbers(mut self, show_line_numbers: bool) -> Self {
        self.display_options.show_line_numbers = show_line_numbers;
        self
    }

    pub(crate) fn with_horizontal_scrollbar_appearance(
        mut self,
        scrollbar_appearance: ScrollableAppearance,
    ) -> Self {
        self.display_options.horizontal_scrollbar_appearance = scrollbar_appearance;
        self
    }

    pub(crate) fn starting_line_number(&self) -> Option<usize> {
        self.display_options.starting_line_number
    }

    pub(crate) fn set_starting_line_number(&mut self, starting_line_number: Option<usize>) {
        self.display_options.starting_line_number = starting_line_number;
    }

    fn handle_searcher_event(&mut self, _event: &SearchEvent, ctx: &mut ViewContext<Self>) {
        // Only update decorations if the find bar is open.
        if self.find_bar_open(ctx) {
            // TODO: Add some indicator (maybe a border?) for invalid queries.
            self.update_decorations_and_position(ctx);
        }
    }

    /// Move the editor cursor to the start of the currently-selected search match, if any.
    fn move_cursor_to_selected_match(&mut self, ctx: &mut ViewContext<Self>) {
        if let Some(match_range) = self.searcher.as_ref(ctx).selected_match_range() {
            self.model.update(ctx, |model, ctx| {
                model.selection().update(ctx, |selection_model, ctx| {
                    selection_model.set_cursor(match_range.start, ctx);
                });
            });
        }
    }

    pub fn selection_position_anchor(&self, app: &AppContext) -> OffsetPositioning {
        self.model.as_ref(app).positioning(app)
    }

    fn handle_find_event(&mut self, event: &FindViewEvent, ctx: &mut ViewContext<Self>) {
        let Some(find_bar) = &self.find_bar else {
            return;
        };

        match event {
            FindViewEvent::CloseFindBar => {
                // If vim mode is enabled and there is a selected search match, move the cursor
                // to the start of that match before closing the find bar.
                if self.vim_mode_enabled(ctx) {
                    self.move_cursor_to_selected_match(ctx);
                }
                self.close_find_bar(true, ctx);
            }
            FindViewEvent::Update { query } => {
                let query_str = query.clone().unwrap_or_default();
                self.run_find(&query_str, ctx);
            }
            FindViewEvent::NextMatch { direction } => {
                self.searcher.update(ctx, |searcher, ctx| match direction {
                    FindDirection::Up => searcher.select_previous_result(ctx),
                    FindDirection::Down => searcher.select_next_result(ctx),
                });

                self.move_cursor_to_selected_match(ctx);

                // After navigating via the find bar, only return focus to the editor in Vim mode.
                if self.vim_mode_enabled(ctx) {
                    self.focus(ctx);
                }
            }
            FindViewEvent::VimEnterAndFocusEditor => {
                // Shift focus back to the main editor when Enter is pressed in Vim mode in the find bar.
                // Select the nearest match from the cursor position and move to it.
                self.searcher
                    .update(ctx, |searcher, ctx| match self.last_search_direction {
                        Direction::Forward => searcher.select_next_from_cursor(ctx),
                        Direction::Backward => searcher.select_prev_from_cursor(ctx),
                    });
                self.move_cursor_to_selected_match(ctx);
                self.focus(ctx);
                ctx.notify();
            }
            FindViewEvent::SelectAll => {
                // Get all search results from the find model
                if let Some(results) = self.searcher.as_ref(ctx).results() {
                    if !results.matches.is_empty() {
                        // Convert all match ranges to selection offsets
                        let selection_offsets: Vec<warp_editor::content::buffer::SelectionOffsets> =
                            results
                                .matches
                                .iter()
                                .map(|match_result| {
                                    warp_editor::content::buffer::SelectionOffsets {
                                        head: match_result.end,
                                        tail: match_result.start,
                                    }
                                })
                                .collect();

                        // Set multiple selections on the editor to highlight all matches
                        if let Ok(selections) = vec1::Vec1::try_from_vec(selection_offsets) {
                            self.model.update(ctx, |model, ctx| {
                                model.selection().update(ctx, |selection_model, ctx| {
                                    selection_model.update_selection(
                                        warp_editor::content::buffer::BufferSelectAction::SetSelectionOffsets { selections },
                                        warp_editor::content::buffer::AutoScrollBehavior::Selection,
                                        ctx,
                                    );
                                });
                            });
                        }
                    }
                }
                self.close_find_bar(true, ctx);
            }
            FindViewEvent::ReplaceSelected => {
                let replace_query =
                    find_bar.update(ctx, |find_bar, ctx| find_bar.replace_query(ctx));
                if let Some(match_range) = self.searcher.as_ref(ctx).selected_match_range() {
                    let final_replace_text = if find_bar.as_ref(ctx).is_preserve_case_enabled() {
                        self.preserve_case(match_range.clone(), &replace_query, ctx)
                    } else {
                        replace_query.clone()
                    };

                    let edits = vec1![(final_replace_text, match_range)];
                    let selection_model = self.model.as_ref(ctx).buffer_selection_model().clone();
                    // Replace current selection in the editor with replace query
                    self.model.update(ctx, |model, ctx| {
                        model.update_content( |mut content_model, ctx| {
                            content_model.apply_edit(
                                warp_editor::content::buffer::BufferEditAction::InsertAtCharOffsetRanges { edits: &edits },
                                EditOrigin::UserInitiated,
                                selection_model,
                                ctx,
                            )
                        }, ctx);
                    });
                    // After selection is replaced, emit an accessibility announcement
                    find_bar.update(ctx, |find_bar, ctx| {
                        find_bar.emit_replace_a11y_content(ctx);
                    })
                }
            }
            FindViewEvent::ReplaceAll => {
                let (replace_query, preserve_case_enabled) =
                    find_bar.update(ctx, |find_bar, ctx| {
                        (
                            find_bar.replace_query(ctx),
                            find_bar.is_preserve_case_enabled(),
                        )
                    });
                if !replace_query.is_empty() {
                    // Get all search results from the find model
                    if let Some(results) = self.searcher.as_ref(ctx).results() {
                        if !results.matches.is_empty() {
                            // Convert all match ranges to selection offsets with case preservation
                            let edits: Vec<(String, Range<CharOffset>)> = results
                                .matches
                                .iter()
                                .map(|match_result| {
                                    let match_range = Range {
                                        start: match_result.start,
                                        end: match_result.end,
                                    };

                                    let final_replace_text = if preserve_case_enabled {
                                        self.preserve_case(match_range.clone(), &replace_query, ctx)
                                    } else {
                                        replace_query.clone()
                                    };

                                    (final_replace_text, match_range)
                                })
                                .collect();

                            if let Ok(edits) = vec1::Vec1::try_from_vec(edits) {
                                let selection_model =
                                    self.model.as_ref(ctx).buffer_selection_model().clone();
                                // Replace selections in the editor with replace query
                                self.model.update(ctx, |model, ctx| {
                                    model.update_content(|mut content_model, ctx| {
                                        content_model.apply_edit(
                                            warp_editor::content::buffer::BufferEditAction::InsertAtCharOffsetRanges { edits: &edits },
                                            EditOrigin::UserInitiated,
                                            selection_model,
                                            ctx,
                                        )
                                    }, ctx);
                                });
                            }
                        }
                    }
                }
            }
        }
    }

    fn handle_comment_editor_event(
        &mut self,
        event: &CommentEditorEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            CommentEditorEvent::ContentChanged => {
                // Handle comment content changes if needed
                ctx.notify();
            }
            CommentEditorEvent::CommentSaved {
                id,
                comment_text,
                line,
            } => {
                let Some(line) = line else {
                    debug_assert!(false, "Comment saved event missing line information");
                    return;
                };
                self.save_comment(*id, comment_text, line, ctx);
            }
            CommentEditorEvent::CloseEditor => {
                // Close the comment editor by updating the pending comment state to Closed
                self.model.update(ctx, |model, ctx| {
                    model.comments().update(ctx, |comments, _| {
                        comments.pending_comment = PendingComment::Closed;
                    });
                });
                ctx.notify();
            }
            CommentEditorEvent::DeleteComment { id } => {
                ctx.emit(CodeEditorEvent::DeleteComment { id: *id });
            }
        }
    }

    fn save_comment(
        &mut self,
        id: Option<CommentId>,
        comment_text: &str,
        line: &EditorLineLocation,
        ctx: &mut ViewContext<Self>,
    ) {
        let line_content = self.model.as_ref(ctx).get_diff_content_for_line(line, ctx);

        let review_comment = match id {
            Some(id) => EditorReviewComment::new_with_id(
                id,
                line.to_owned(),
                line_content,
                comment_text.to_owned(),
            ),
            None => {
                EditorReviewComment::new(line.to_owned(), line_content, comment_text.to_owned())
            }
        };

        self.comment_locations.push(SavedComment {
            uuid: review_comment.id,
            location: line.to_owned(),
            mouse_state: MouseStateHandle::default(),
        });

        ctx.emit(CodeEditorEvent::CommentSaved {
            comment: review_comment,
        });
        ctx.notify();
    }

    /// Update all comment locations in this editor.
    pub fn set_comment_locations(
        &mut self,
        comments: impl Iterator<Item = EditorReviewComment>,
        ctx: &mut ViewContext<Self>,
    ) {
        self.comment_locations.clear();
        for comment in comments {
            self.comment_locations.push(SavedComment {
                uuid: comment.id,
                location: comment.line.clone(),
                mouse_state: MouseStateHandle::default(),
            });
        }
        ctx.notify();
    }

    /// Clear all comment locations in this editor.
    pub fn clear_comment_locations(&mut self, ctx: &mut ViewContext<Self>) {
        self.comment_locations.clear();
        ctx.notify();
    }

    fn line_number_config(&self, ctx: &AppContext) -> Option<LineNumberConfig> {
        let appearance = Appearance::as_ref(ctx);
        let theme = appearance.theme();
        if self.display_options.show_line_numbers {
            Some(LineNumberConfig {
                font_family: appearance.monospace_font_family(),
                font_size: appearance.monospace_font_size(),
                text_color: theme.sub_text_color(theme.background()).into(),
                highlight_text_color: theme.main_text_color(theme.background()).into(),
                starting_line_number: self.display_options.starting_line_number,
            })
        } else {
            None
        }
    }

    fn run_find(&mut self, query: &str, ctx: &mut ViewContext<Self>) {
        self.searcher.update(ctx, |searcher, ctx| {
            searcher.set_query(query.to_string(), ctx);
        });
    }

    fn handle_editor_model_event(
        &mut self,
        event: &CodeEditorModelEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            CodeEditorModelEvent::DiffUpdated => {
                self.nav_bar.update(ctx, |_, ctx| ctx.notify());
                ctx.emit(CodeEditorEvent::DiffUpdated);
                ctx.notify()
            }
            CodeEditorModelEvent::SyntaxHighlightingUpdated => {
                self.nav_bar.update(ctx, |_, ctx| ctx.notify());
                ctx.notify()
            }
            CodeEditorModelEvent::SelectionChanged => {
                self.reset_for_editing_change();
                self.vim_maybe_enforce_cursor_line_cap(ctx);
                ctx.emit(CodeEditorEvent::SelectionChanged);
            }
            CodeEditorModelEvent::ContentChanged { origin } => {
                if origin.from_user() {
                    self.reset_for_editing_change();
                    self.vim_maybe_enforce_cursor_line_cap(ctx);
                }
                ctx.emit(CodeEditorEvent::ContentChanged { origin: *origin });
            }
            CodeEditorModelEvent::UnifiedDiffComputed(diff) => {
                ctx.emit(CodeEditorEvent::UnifiedDiffComputed(diff.clone()));
            }
            CodeEditorModelEvent::ViewportUpdated(version) => {
                if let Some(trigger) = self
                    .pending_scroll
                    .take_if(|trigger| trigger.minimum_applicable_version <= *version)
                {
                    match trigger.position {
                        ScrollPosition::LineAndColumn(line_col) => {
                            self.jump_to_line_column(line_col.line_num, line_col.column_num, ctx);
                        }
                        ScrollPosition::FocusedDiffHunk => {
                            self.navigate_current_diff_hunk(ctx);
                        }
                    }
                }
                ctx.emit(CodeEditorEvent::ViewportUpdated);
            }
            CodeEditorModelEvent::InteractionStateChanged => (),
            CodeEditorModelEvent::DelayedRenderingFlushed => {
                ctx.emit(CodeEditorEvent::DelayedRenderingFlushed);
            }
            CodeEditorModelEvent::LayoutInvalidated => {
                ctx.emit(CodeEditorEvent::LayoutInvalidated);
            }
            #[cfg(windows)]
            CodeEditorModelEvent::WindowsCtrlC { copied_selection } => {
                ctx.emit(CodeEditorEvent::WindowsCtrlC {
                    copied_selection: *copied_selection,
                });
            }
        }
    }

    pub fn toggle_diff_nav(
        &self,
        line_range: Option<Range<LineCount>>,
        ctx: &mut ViewContext<Self>,
    ) {
        if !self.display_options.can_show_diff_ui {
            return;
        }

        let toggle_to_on = self
            .model
            .update(ctx, |model, ctx| model.toggle_diff_nav(line_range, ctx));

        if toggle_to_on {
            self.nav_bar.update(ctx, |nav_bar, ctx| {
                nav_bar.autoscroll(ctx);
            });
        }
    }

    /// Expands all diff hunks without focusing any specific diff hunk.
    /// All diff hunks will be shown expanded with normal highlighting.
    pub fn expand_diffs(&self, ctx: &mut ViewContext<Self>) {
        if !self.display_options.can_show_diff_ui {
            return;
        }

        self.model.update(ctx, |model, ctx| {
            model.expand_diffs(ctx);
        });
    }

    /// Handles [`Appearance`] changes by updating the render model.
    fn handle_appearance_or_font_change(&mut self, ctx: &mut ViewContext<Self>) {
        let new_styles = code_text_styles(
            Appearance::as_ref(ctx),
            FontSettings::as_ref(ctx),
            self.display_options.line_height_override,
        );
        self.model.update(ctx, move |model, ctx| {
            model.handle_appearance_or_font_change(new_styles, ctx);
        });
    }

    pub fn is_focused(&self, app: &AppContext) -> bool {
        let Some(handle) = self.self_handle.upgrade(app) else {
            return false;
        };

        // If our window is not active, we don't have user focus, even if we're focused within the app.
        if app.windows().state().active_window != Some(handle.window_id(app)) {
            return false;
        }

        //  Check if the editor is focused directly.
        if handle.is_focused(app) {
            return true;
        }

        false
    }

    pub fn focus(&self, ctx: &mut ViewContext<Self>) {
        ctx.focus_self();
        ctx.emit(CodeEditorEvent::Focused);
    }

    pub fn indent_unit(&self, ctx: &AppContext) -> IndentUnit {
        self.model
            .as_ref(ctx)
            .buffer()
            .as_ref(ctx)
            .indent_unit_at_plain_text()
            .unwrap_or_default()
    }

    fn selection_start(
        &mut self,
        offset: CharOffset,
        modifiers: ModifiersState,
        ctx: &mut ViewContext<Self>,
    ) {
        // Clicking into the editor should restore focus.
        self.focus(ctx);

        // If there is a hovered symbol range, don't handle the cmd-click.
        if modifiers.cmd {
            if let Some(range) = self.model.as_ref(ctx).hovered_symbol_range() {
                if range.range().contains(&offset) {
                    return;
                }
            }
        }

        let multiselect = modifiers.alt && FeatureFlag::RichTextMultiselect.is_enabled();
        self.model.update(ctx, |model, ctx| {
            model.select_at(offset, multiselect, ctx);
        });
        self.is_selecting = true;
        ctx.emit(CodeEditorEvent::SelectionStart);
    }

    /// Extend the selection that is currently being dragged.  This should be called after
    /// `selection_start` and before `selection_end`.
    fn selection_update(&mut self, offset: CharOffset, ctx: &mut ViewContext<Self>) {
        self.model.update(ctx, |model, ctx| {
            model.update_pending_selection(offset, ctx);
        });
    }

    fn selection_end(&mut self, ctx: &mut ViewContext<Self>) {
        self.is_selecting = false;
        self.model.update(ctx, |model, ctx| {
            model.end_selection(ctx);
        });
        ctx.emit(CodeEditorEvent::SelectionEnd);
        ctx.notify();
    }

    pub fn is_selecting(&self) -> bool {
        self.is_selecting
    }

    /// Extend the selection to the given offset.  This is used for shift-clicking to extend the
    /// selection, and not for dragging the selection.
    fn selection_extend(&mut self, offset: CharOffset, ctx: &mut ViewContext<Self>) {
        self.model.update(ctx, |model, ctx| {
            model.set_last_selection_head(offset, ctx);
        });
    }

    pub fn set_language_with_path(&mut self, path: &Path, ctx: &mut ViewContext<Self>) {
        self.model.update(ctx, |model, ctx| {
            model.set_language_with_path(path, ctx);
        });
    }

    pub fn set_language_with_name(&mut self, name: &str, ctx: &mut ViewContext<Self>) {
        self.model.update(ctx, |model, ctx| {
            model.set_language_with_name(name, ctx);
        });
    }

    fn jump_to_line_column(&self, line: usize, column: Option<usize>, ctx: &mut ViewContext<Self>) {
        self.model.update(ctx, |model, ctx| {
            model.jump_to_line_column(line, column, ctx)
        })
    }

    /// Reset editor content using InitialBufferState.
    /// This is the preferred method for resetting editor content as it consolidates all parameters.
    pub fn reset(&self, state: InitialBufferState, ctx: &mut ViewContext<Self>) {
        self.model.update(ctx, |model, ctx| {
            model.reset_content(state, ctx);
        });
    }

    pub fn apply_diffs(&self, diffs: Vec<DiffDelta>, ctx: &mut ViewContext<Self>) {
        self.model.update(ctx, |model, ctx| {
            model.apply_diffs(diffs, ctx);
        });
    }

    fn reset_for_editing_change(&mut self) {
        self.display_states.display_state.reset_cursor_blink_timer();
    }

    pub fn text(&self, ctx: &AppContext) -> AnyMultilineString {
        self.model.as_ref(ctx).content_string(ctx)
    }

    pub fn word_range_at_offset(
        &self,
        offset: CharOffset,
        app: &AppContext,
    ) -> Option<Range<CharOffset>> {
        self.model.as_ref(app).word_range_at_offset(offset, app)
    }

    /// Returns the character at the given offset in the buffer, if it exists.
    pub fn char_at(&self, offset: CharOffset, ctx: &AppContext) -> Option<char> {
        self.model.as_ref(ctx).char_at(offset, ctx)
    }

    /// Check whether the given version matches the version of the underlying buffer.
    pub fn version_match(&self, version: &ContentVersion, ctx: &AppContext) -> bool {
        self.model
            .as_ref(ctx)
            .content()
            .as_ref(ctx)
            .version_match(version)
    }

    /// Return the ContentVersion of the underlying `Buffer`.
    pub fn version(&self, ctx: &AppContext) -> ContentVersion {
        self.model.as_ref(ctx).content().as_ref(ctx).version()
    }

    pub fn buffer_version(&self, ctx: &AppContext) -> BufferVersion {
        self.model.as_ref(ctx).buffer_version(ctx)
    }

    /// Append text to the end of the buffer regardless of cursor position.
    /// This is used for streaming content where we always want to append at the end,
    /// not at the current cursor position since the user may select text while it's streaming.
    pub fn append_at_end(&self, text: &str, ctx: &mut ViewContext<Self>) {
        self.model.update(ctx, |model, ctx| {
            // Use append_at_end to insert at the end of buffer regardless of cursor position.
            // This ensures streaming code blocks always append at the end, even when user
            // has clicked somewhere else in the editor.
            model.append_at_end(text, ctx);
        });
    }

    pub fn system_append_autoscroll_vertical_only(&self, text: &str, ctx: &mut ViewContext<Self>) {
        self.model.update(ctx, |model, ctx| {
            model.system_insert_autoscroll_vertical_only(text, ctx);
        });
    }

    pub fn truncate(&self, len: usize, ctx: &mut ViewContext<Self>) {
        self.model.update(ctx, |model, ctx| {
            model.truncate(len, ctx);
        });
    }

    pub fn retrieve_unified_diff(&self, file_name: String, ctx: &mut ViewContext<Self>) {
        self.model.update(ctx, |model, ctx| {
            model.retrieve_unified_diff(file_name, ctx);
        });
    }

    /// Identifies which line (current or removed) is at the given content-space
    /// vertical offset. Delegates to [`CodeEditorModel::line_at_vertical_offset`].
    #[allow(dead_code)]
    pub fn line_at_vertical_offset(
        &self,
        offset: Pixels,
        ctx: &mut ViewContext<Self>,
    ) -> Option<(StableEditorLine, Pixels)> {
        self.model
            .update(ctx, |model, ctx| model.line_at_vertical_offset(offset, ctx))
    }

    /// Returns the content-space vertical offset of the top of the given line.
    /// Delegates to [`CodeEditorModel::line_top`].
    pub fn line_top(&self, line: &StableEditorLine, ctx: &AppContext) -> Option<Pixels> {
        self.model.as_ref(ctx).line_top(line, ctx)
    }

    /// Returns the total content height of the editor.
    pub fn content_height(&self, ctx: &AppContext) -> Pixels {
        self.model.as_ref(ctx).render_state().as_ref(ctx).height()
    }

    pub fn interaction_state(&self, ctx: &AppContext) -> InteractionState {
        self.model.as_ref(ctx).interaction_state()
    }

    pub fn set_interaction_state(&self, state: InteractionState, ctx: &mut ViewContext<Self>) {
        self.model.update(ctx, |model, ctx| {
            model.set_interaction_state(state, ctx);
        });
    }

    pub fn is_editable(&self, app: &AppContext) -> bool {
        self.model.as_ref(app).interaction_state() == InteractionState::Editable
    }

    pub fn navigate_next_diff_hunk(&self, ctx: &mut ViewContext<Self>) {
        self.nav_bar.update(ctx, |nav_bar, ctx| {
            nav_bar.navigate_down(ctx);
        });
    }

    pub fn navigate_previous_diff_hunk(&self, ctx: &mut ViewContext<Self>) {
        self.nav_bar.update(ctx, |nav_bar, ctx| {
            nav_bar.navigate_up(ctx);
        });
    }

    fn navigate_current_diff_hunk(&self, ctx: &mut ViewContext<Self>) {
        self.nav_bar.update(ctx, |nav_bar, ctx| {
            nav_bar.autoscroll(ctx);
        });
    }

    pub fn set_vertical_expansion_behavior(
        &mut self,
        behavior: VerticalExpansionBehavior,
        ctx: &mut ViewContext<Self>,
    ) {
        self.display_options.vertical_expansion_behavior = behavior;
        ctx.notify();
    }

    pub fn selected_text(&self, ctx: &AppContext) -> Option<String> {
        let selected_text = self
            .model
            .as_ref(ctx)
            .read_selected_text_as_clipboard_content(ctx);
        if selected_text.plain_text.is_empty() {
            return None;
        }
        Some(selected_text.plain_text)
    }

    pub fn clear_selection(&mut self, ctx: &mut ViewContext<Self>) {
        // Same as [`Self::selection_start`] but without the focus.
        self.model.update(ctx, |model, ctx| {
            model.clear_selections(ctx);
        });
        self.selection_end(ctx);
    }

    pub fn line_location_to_offsets(
        &self,
        line: &EditorLineLocation,
        ctx: &AppContext,
    ) -> (CharOffset, CharOffset) {
        let render_state = self.model.as_ref(ctx).render_state();
        let render_state_ref = render_state.as_ref(ctx);

        let line_number = match line {
            EditorLineLocation::Current { line_number, .. } => *line_number,
            EditorLineLocation::Removed { line_number, .. } => {
                // Scroll to where the removal hunk appears in the diff view
                *line_number
            }
            EditorLineLocation::Collapsed { line_range } => line_range.start,
        };

        render_state_ref.line_number_to_offset_range(line_number)
    }

    pub fn offset_to_lsp_position(
        &self,
        offset: CharOffset,
        ctx: &AppContext,
    ) -> lsp::types::Location {
        let buffer = self.model.as_ref(ctx).content().as_ref(ctx);

        let point = offset.to_buffer_point(buffer);
        let line = point.row.saturating_sub(1);

        lsp::types::Location {
            line: line as usize,
            column: point.column as usize,
        }
    }

    pub fn lsp_location_to_offset(
        &self,
        location: &lsp::types::Location,
        ctx: &AppContext,
    ) -> CharOffset {
        let buffer = self.model.as_ref(ctx).content().as_ref(ctx);

        let line = location.line + 1;
        let column = location.column;
        let point = Point::new(line as u32, column as u32);
        point.to_buffer_char_offset(buffer)
    }

    /// Returns the current cursor position as an LSP location.
    pub fn cursor_lsp_position(&self, ctx: &AppContext) -> lsp::types::Location {
        let selection = *self.model.as_ref(ctx).selections(ctx).first();
        self.offset_to_lsp_position(selection.head, ctx)
    }

    /// Returns the buffer offset at the current cursor head.
    pub fn cursor_head_offset(&self, ctx: &AppContext) -> CharOffset {
        self.model.as_ref(ctx).selections(ctx).first().head
    }

    pub fn hovered_symbol_range<'a>(
        &'a self,
        ctx: &'a AppContext,
    ) -> Option<&'a Range<CharOffset>> {
        self.model
            .as_ref(ctx)
            .hovered_symbol_range()
            .map(|link| link.range())
    }

    pub fn cursor_at(&self, point: Point, ctx: &mut ViewContext<Self>) {
        let offset = point.to_buffer_char_offset(self.model.as_ref(ctx).content().as_ref(ctx));
        self.model.update(ctx, |model, ctx| {
            model.cursor_at(offset, ctx);
        });
    }

    pub fn clear_hovered_symbol_range(&mut self, ctx: &mut ViewContext<Self>) -> bool {
        self.set_hovered_symbol_range(None, ctx)
    }

    pub fn set_hovered_symbol_range(
        &mut self,
        range: Option<HoverableLink>,
        ctx: &mut ViewContext<Self>,
    ) -> bool {
        let cursor_shape = if range.is_some() {
            Cursor::PointingHand
        } else {
            Cursor::IBeam
        };

        let updated = self
            .model
            .update(ctx, |model, _ctx| model.set_hovered_symbol_range(range));

        if updated {
            ctx.set_cursor_shape(cursor_shape);
        }

        updated
    }

    pub fn apply_edits(
        &mut self,
        edits: Vec1<(String, Range<CharOffset>)>,
        ctx: &mut ViewContext<Self>,
    ) {
        self.model.update(ctx, |model, ctx| {
            let selection_model = model.buffer_selection_model().clone();
            model.update_content(
                |mut content, ctx| {
                    content.apply_edit(
                        BufferEditAction::InsertAtCharOffsetRanges { edits: &edits },
                        EditOrigin::UserInitiated,
                        selection_model,
                        ctx,
                    );
                },
                ctx,
            );
        });
    }

    /// Splits text based on pascal case, camel case, hyphens, or underscores.
    /// Returns a tuple of (parts, delimiter) where delimiter is Some(char) if a delimiter was used.
    fn split_text(text: &str) -> (Vec<String>, Option<char>) {
        if text.is_empty() {
            return (vec![], None);
        }

        // Check for hyphen or underscore delimiters first
        if text.contains('-') {
            let parts = text.split('-').map(|s| s.to_string()).collect();
            return (parts, Some('-'));
        }
        if text.contains('_') {
            let parts = text.split('_').map(|s| s.to_string()).collect();
            return (parts, Some('_'));
        }

        // Split on camelCase/PascalCase boundaries
        let mut parts = Vec::new();
        let mut current_part = String::new();
        let mut chars = text.chars().peekable();

        while let Some(ch) = chars.next() {
            if ch.is_uppercase() && !current_part.is_empty() {
                // Check if this is a sequence of uppercase letters (like "HTML" in "HTMLParser")
                if let Some(&next_ch) = chars.peek() {
                    if next_ch.is_lowercase() {
                        // This is the start of a new word (like "P" in "HTMLParser")
                        parts.push(current_part);
                        current_part = ch.to_string();
                    } else {
                        // Continue the current part for sequences like "HTML"
                        current_part.push(ch);
                    }
                } else {
                    // Last character - check if it should be part of the previous uppercase sequence
                    if current_part
                        .chars()
                        .last()
                        .is_some_and(|c| c.is_uppercase())
                    {
                        // Previous char was uppercase, append to current part
                        current_part.push(ch);
                    } else {
                        // Previous char was lowercase, start new part
                        parts.push(current_part);
                        current_part = ch.to_string();
                    }
                }
            } else {
                current_part.push(ch);
            }
        }

        if !current_part.is_empty() {
            parts.push(current_part);
        }

        // No delimiter used for camelCase/PascalCase
        (parts, None)
    }

    /// Preserves the case of a single part based on the original part's casing.
    fn preserve_case_for_part(original_part: &str, replace_part: &str) -> String {
        if original_part.chars().all(|c| c.is_lowercase()) {
            replace_part.to_lowercase()
        } else if original_part.chars().all(|c| c.is_uppercase()) {
            replace_part.to_uppercase()
        } else if let Some(first_original) = original_part.chars().next() {
            if first_original.is_uppercase() {
                // Preserve uppercase first character
                let mut result = String::new();
                let mut replace_chars = replace_part.chars();
                if let Some(mut first_replace) = replace_chars.next() {
                    first_replace = if first_replace.is_uppercase() {
                        first_replace
                    } else {
                        {
                            first_replace.to_uppercase().next().unwrap_or(first_replace)
                        }
                    };
                    result.push(first_replace);
                    result.push_str(&replace_chars.collect::<String>().to_lowercase());
                }
                result
            } else {
                // Mixed case but starts with lowercase - keep replace_part as-is
                replace_part.to_string()
            }
        } else {
            // Empty original part - keep replace_part as-is
            replace_part.to_string()
        }
    }

    /// Whether the editor needs a vertical constraint when rendering. This is when the editor
    /// has a vertical expansion behavior that is not infinite height.
    pub fn needs_vertical_constraint(&self) -> bool {
        !matches!(
            self.display_options.vertical_expansion_behavior,
            VerticalExpansionBehavior::InfiniteHeight
        )
    }

    /// Preserves the case pattern of the original text when replacing it with new text.
    /// The case pattern is preserved in several ways:
    /// - For simple text: "begin" -> "end" will preserve uppercase ("Begin" -> "End")
    ///   and all-caps ("BEGIN" -> "END")
    /// - For camelCase/PascalCase: The replacement text's casing pattern is used
    ///   ("oneTwoThree" -> "fourFiveSix", "OneTwoThree" -> "FourFiveSix")
    /// - Delimiters like hyphens and underscores are preserved
    fn preserve_case(
        &self,
        original_range: Range<CharOffset>,
        replace_text: &str,
        ctx: &AppContext,
    ) -> String {
        // Get the original text from the buffer
        let original_text = self
            .model
            .as_ref(ctx)
            .buffer()
            .as_ref(ctx)
            .text_in_range(original_range.start..original_range.end)
            .into_string();

        // Split both original and replacement text
        let (mut original_parts, mut original_delimiter) = Self::split_text(&original_text);
        let (mut replace_parts, replace_delimiter) = Self::split_text(replace_text);
        // If the replaced text is delimited differently than incoming text, we shouldn't split either of them.
        if original_delimiter != replace_delimiter {
            (original_parts, original_delimiter) = (vec![original_text.to_string()], None);
            replace_parts = vec![replace_text.to_string()];
        }

        // Apply case preservation to each part
        let mut result_parts = Vec::new();
        for (i, replace_part) in replace_parts.iter().enumerate() {
            if let Some(original_part) = original_parts.get(i) {
                result_parts.push(Self::preserve_case_for_part(original_part, replace_part));
            } else {
                // If we have more replacement parts than original parts, keep them as-is
                result_parts.push(replace_part.clone());
            }
        }

        // Join the parts back together using the original delimiter if it exists
        let delimiter = original_delimiter
            .map(|d| d.to_string())
            .unwrap_or_default();
        result_parts.join(&delimiter)
    }

    pub fn diff_hunks_changed_lines(&self, app: &AppContext) -> (usize, usize) {
        let model = self.model.as_ref(app);
        let diff = model.diff().as_ref(app);
        diff.diff_status().get_diff_lines()
    }

    /// If there's a single selection, returns its starting and ending line numbers.
    pub fn selected_lines(&self, app: &AppContext) -> Option<(u32, u32)> {
        // Query the buffer model directly to determine if we have a selection
        let selection_model = self.model.as_ref(app).buffer_selection_model().as_ref(app);

        if !selection_model.is_single_selection() {
            return None;
        }

        let offsets = selection_model.selection_offsets();
        let selection_offsets = offsets.first();
        if selection_offsets.head == selection_offsets.tail {
            return None;
        }

        let buffer = self.model.as_ref(app).buffer().as_ref(app);
        let (start_offset, end_offset) = (
            selection_offsets.head.min(selection_offsets.tail),
            selection_offsets.head.max(selection_offsets.tail),
        );
        let start_line = start_offset.to_buffer_point(buffer).row;
        let end_line = end_offset.to_buffer_point(buffer).row;
        Some((start_line, end_line))
    }

    /// If vim keybindings are enabled, return the [`VimMode`]. Otherwise, return None.
    pub fn vim_mode(&self, ctx: &AppContext) -> Option<VimMode> {
        self.vim_state(ctx).map(|state| state.mode)
    }

    /// If vim keybindings are enabled, return the [`VimState`]. Otherwise, return None.
    pub fn vim_state<'a>(&self, ctx: &'a AppContext) -> Option<VimState<'a>> {
        self.vim_mode_enabled(ctx)
            .then(|| self.vim_model.as_ref(ctx).state())
    }

    pub fn vim_mode_enabled(&self, ctx: &AppContext) -> bool {
        self.supports_vim_mode && AppEditorSettings::as_ref(ctx).vim_mode_enabled()
    }

    pub fn enter_vim_normal_mode(&mut self, ctx: &mut ViewContext<Self>) {
        if self.vim_mode_enabled(ctx) {
            self.vim_escape(ctx);
        }
    }

    /// Send the 'escape' keystroke to the VimFSA.
    fn vim_escape(&mut self, ctx: &mut ViewContext<Self>) {
        self.vim_keystroke(&Keystroke::parse("escape").expect("escape parses"), ctx)
    }

    fn vim_apply_insert_position(
        &mut self,
        position: &InsertPosition,
        ctx: &mut ViewContext<Self>,
    ) {
        match position {
            InsertPosition::AtCursor => {}
            InsertPosition::AfterCursor => {
                self.model.update(ctx, |model, ctx| {
                    model.vim_move_horizontal_by_offset(
                        1,
                        &Direction::Forward,
                        false, // keep_selection
                        true,  // stop_at_line_boundary
                        ctx,
                    );
                });
            }
            InsertPosition::LineFirstNonWhitespace => self.model.update(ctx, |model, ctx| {
                model.vim_move_to_first_nonwhitespace(false, ctx);
            }),
            InsertPosition::LineEnd => self.model.update(ctx, |model, ctx| {
                model.vim_move_to_line_bound(LineBound::End, false, ctx);
            }),
            InsertPosition::LineAbove => {
                self.model.update(ctx, |model, ctx| {
                    model.vim_newline(true, ctx);
                });
            }
            InsertPosition::LineBelow => {
                self.model.update(ctx, |model, ctx| {
                    model.vim_newline(false, ctx);
                });
            }
        }
    }

    /// When in Vim mode, specifically normal mode, the block cursor cannot go past the last
    /// character on the line as the beam cursor can. We call this "line capping." This helper
    /// method determines if line capping needs to be enforced, and if so, enforces it.
    fn vim_maybe_enforce_cursor_line_cap(&mut self, ctx: &mut ViewContext<Self>) {
        if let Some(VimMode::Normal) = self.vim_mode(ctx) {
            if self.model.as_ref(ctx).vim_needs_line_capping(ctx) {
                self.model.update(ctx, |model, ctx| {
                    model.vim_enforce_cursor_line_cap(ctx);
                });
            }
        }
    }

    fn user_insert(&mut self, typed: &str, ctx: &mut ViewContext<Self>) {
        if ctx.is_self_focused() {
            if let Some(first_char) = typed.chars().next() {
                if typed.chars().count() == 1 {
                    let all_cursors_next_character_matches_char =
                        self.model.update(ctx, |model, ctx| {
                            model.all_cursors_next_character_matches_char(first_char, ctx)
                        });

                    // If the character is a closing symbol, we want to potentially step over it incase it's already a closing symbol.
                    if CLOSING_SYMBOLS.contains(&first_char)
                        && all_cursors_next_character_matches_char
                    {
                        let buffer = self.model.as_ref(ctx).buffer().as_ref(ctx);
                        let selection_model =
                            self.model.as_ref(ctx).buffer_selection_model().as_ref(ctx);
                        if selection_model.all_single_cursors() {
                            let selections = selection_model.selection_offsets();
                            let should_step_over = selections.iter().all(|sel| {
                                buffer.char_at(sel.head).is_some_and(|c| c == first_char)
                            });
                            if should_step_over {
                                self.model.update(ctx, |model, ctx| {
                                    model.selection_model().update(ctx, |selection, ctx| {
                                        selection.update_selection(
                                            warp_editor::content::buffer::BufferSelectAction::MoveRight,
                                            warp_editor::content::buffer::AutoScrollBehavior::Selection,
                                            ctx,
                                        );
                                    });
                                });
                                return;
                            }
                        }
                    }

                    // If the character is opening autocomplete symbol, we want to autocomplete it with a closing symbol.
                    if let Some(close) = AUTOCOMPLETE_SYMBOLS.get(&first_char) {
                        self.model.update(ctx, |model, ctx| {
                            model.autocomplete_symbol(first_char, *close, ctx);
                        });
                        return;
                    }
                }
            }

            self.model.update(ctx, |model, ctx| {
                model.user_insert(typed, ctx);
            });
        }
    }

    pub fn undo(&mut self, ctx: &mut ViewContext<Self>) {
        self.model.update(ctx, |model, ctx| {
            model.undo(ctx);
        });
    }

    fn delete_line_left(&self, ctx: &mut ViewContext<Self>) {
        self.model.update(ctx, |model, ctx| {
            model.delete_all_left(ctx);
        })
    }

    fn vim_user_insert(&mut self, text: &str, ctx: &mut ViewContext<Self>) {
        // Only handle Vim input when this editor is focused.
        if !ctx.is_self_focused() {
            return;
        }
        self.vim_model.update(ctx, |vim_model, ctx| {
            for c in text.chars() {
                vim_model.typed_character(c, ctx);
            }
        });
    }

    /// Similar to Self::vim_user_insert, but for keystrokes which aren't represented by a char.
    pub fn vim_keystroke(&mut self, keystroke: &Keystroke, ctx: &mut ViewContext<Self>) {
        // Only handle Vim keystrokes when this editor is focused.
        if !ctx.is_self_focused() {
            return;
        }
        self.vim_model.update(ctx, |vim_model, ctx| {
            vim_model.keypress(keystroke, ctx);
        });
    }

    pub fn open_existing_comment(
        &mut self,
        id: &CommentId,
        location: &EditorLineLocation,
        comment_text: &str,
        origin: &CommentOrigin,
        ctx: &mut ViewContext<Self>,
    ) {
        let comment_exists = self
            .comment_locations
            .iter()
            .any(|saved_comment| saved_comment.uuid == *id);

        if !comment_exists {
            log::warn!(
                "open_existing_comment: no saved comment found for id {:?}",
                id
            );
            return;
        }

        self.active_comment_editor
            .update(ctx, |comment_editor, ctx| {
                comment_editor.reopen_saved_comment(
                    id,
                    Some(location.clone()),
                    comment_text,
                    origin,
                    ctx,
                );
            });

        self.model.update(ctx, |editor_model, ctx| {
            editor_model.reopen_comment_line(id, location, comment_text, origin, ctx);
        });

        ctx.notify();
    }
}

impl Entity for CodeEditorView {
    type Event = CodeEditorEvent;
}

impl View for CodeEditorView {
    fn ui_name() -> &'static str {
        "CodeEditorView"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        let editable = self.is_editable(app);

        let focused = self.is_focused(app);
        let blink_cursors = AppEditorSettings::as_ref(app).cursor_blink_enabled();

        let display_options = DisplayOptions {
            debug_bounds: false,
            focused,
            blink_cursors,
            vertical_expansion_behavior: self.display_options.vertical_expansion_behavior,
            editable,
            ..Default::default()
        };

        let line_number_config = self.line_number_config(app);

        let diff_status = if self.display_options.can_show_diff_ui {
            self.model.as_ref(app).diff_status(app)
        } else {
            DiffStatus::default()
        };

        let render_state = self.model.as_ref(app).render_state();

        // Align Vim visual tails with the render state's selection coordinate system.
        // The render model stores selection head/tail as (buffer_offset - 1), so apply the same
        // transformation to the visual tails to avoid off-by-one mismatches when highlighting.
        let vim_visual_tails = self
            .model
            .as_ref(app)
            .vim_visual_tails()
            .iter()
            .map(|t| t.saturating_sub(&CharOffset::from(1)))
            .collect::<Vec<_>>();
        let editor_rich_content = RichTextElement::<Self>::new(
            render_state.clone(),
            self.self_handle.clone(),
            display_options,
            self.display_states.display_state.clone(),
            self.vim_mode(app),
            vim_visual_tails,
        );

        let collapsible_diffs = self.display_options.collapsible_diffs;

        let mut code_editor = EditorWrapper::new(
            InnerEditor::FullEditor(editor_rich_content),
            self.display_options.vertical_expansion_behavior,
            line_number_config,
            diff_status,
            self.display_states.wrapper_state_handle.clone(),
            Box::new(move |click, ctx| match click {
                GutterRange::DiffHunk {
                    line,
                    in_sliver: true,
                } => {
                    if collapsible_diffs {
                        ctx.dispatch_typed_action(CodeEditorViewAction::ToggleDiffNav(Some(
                            line.line_range().clone(),
                        )))
                    }
                }
                GutterRange::HiddenSection {
                    line: line_info,
                    expansion_type,
                } => ctx.dispatch_typed_action(CodeEditorViewAction::HiddenSectionExpansion {
                    line_range: line_info.line_range().clone(),
                    expansion_type,
                }),
                _ => {}
            }),
            self.display_options
                .scroll_wheel_behavior
                .should_handle(focused),
            self.model.as_ref(app).diff_navigation_state().clone(),
            if let Some(index) = self.model.as_ref(app).focused_diff_index() {
                self.model
                    .as_ref(app)
                    .diff()
                    .as_ref(app)
                    .line_range_by_diff_hunk_index(index)
                    .map(|line_count| {
                        LineCount::from(line_count.start)..LineCount::from(line_count.end)
                    })
            } else {
                None
            },
            self.display_options.diff_hunk_as_context,
            self.display_options.revert_diff_hunk,
            self.display_options.comment_button,
            self.comment_locations.clone(),
            self.display_options.expand_diff_indicator_width_on_hover,
            self.display_options.gutter_hover_target,
            self.comment_save_position_id.clone(),
            self.find_references_save_position_id.clone(),
        );

        let pending_comment = &self
            .model
            .as_ref(app)
            .comments()
            .as_ref(app)
            .pending_comment;
        // Check if there's an open comment in the model and set the comment box
        if let PendingComment::Open { line, .. } = pending_comment {
            code_editor.set_comment_box(line.clone(), app);
        }

        // Set find references anchor if there's an active request
        if let Some(offset) = &self.find_references_anchor_offset {
            let render_state_ref = render_state.as_ref(app);
            let softwrap_point = render_state_ref.offset_to_softwrap_point(*offset);
            let line_number = LineCount::from(softwrap_point.row() as usize + 1); // Convert 0-indexed to 1-indexed
                                                                                  // Create a simple EditorLineLocation::Current with the line number
                                                                                  // We don't have hunk range info here, so use a single-line range
            let anchor_line = EditorLineLocation::Current {
                line_number,
                line_range: line_number..line_number + LineCount::from(1),
            };
            code_editor.set_find_references_anchor(Some(anchor_line));
        }

        let config = DualAxisConfig::Manual {
            horizontal: AxisConfiguration::Manual(
                self.display_states.horizontal_scroll_state.clone(),
            ),
            vertical: AxisConfiguration::Manual(self.display_states.vertical_scroll_state.clone()),
            child: NewScrollableElement::finish_scrollable(code_editor),
        };

        let scrollable = NewScrollable::horizontal_and_vertical(
            config,
            theme.disabled_text_color(theme.background()).into(),
            theme.main_text_color(theme.background()).into(),
            Fill::None,
        )
        .with_horizontal_scrollbar(self.display_options.horizontal_scrollbar_appearance)
        .with_vertical_scrollbar(self.display_options.vertical_scrollbar_appearance)
        .with_propagate_mousewheel_if_not_handled(true)
        .finish();

        let inner = match self.display_options.vertical_expansion_behavior {
            VerticalExpansionBehavior::InfiniteHeight => scrollable,
            VerticalExpansionBehavior::FillMaxHeight
            | VerticalExpansionBehavior::GrowToMaxHeight => {
                Shrinkable::new(1., scrollable).finish()
            }
        };
        let mut col = Flex::column().with_child(inner);
        if self.model.as_ref(app).diff_nav_is_active() && self.display_options.show_nav_bar {
            col.add_child(ChildView::new(&self.nav_bar).finish());
        }

        let mut stack = Stack::new()
            .with_constrain_absolute_children()
            .with_child(col.finish());
        if let Some(find_bar) = &self.find_bar {
            if find_bar.as_ref(app).is_open() {
                stack.add_overlay_child(ChildView::new(find_bar).finish());
            }
        }
        if self.goto_line_dialog.as_ref(app).is_open() {
            let dialog = Dismiss::new(ChildView::new(&self.goto_line_dialog).finish())
                .on_dismiss(|ctx, _app| {
                    ctx.dispatch_typed_action(CodeEditorViewAction::Escape);
                })
                .finish();
            stack.add_overlay_child(dialog);
        }

        if !FeatureFlag::EmbeddedCodeReviewComments.is_enabled() {
            // Render the open comment editor.
            if let PendingComment::Open { line, .. } = pending_comment {
                let render_state_ref = render_state.as_ref(app);
                let vertical_offset = render_state_ref
                    .vertical_offset_at_render_location(line.clone().into_render_line_location())
                    .unwrap_or_default()
                    + render_state_ref.styles().base_line_height();

                let line_location = app.element_position_by_id_at_last_frame(
                    self.window_id,
                    &self.comment_save_position_id,
                );

                let should_render_comment_editor = match line_location {
                    Some(line_location) => self
                        .show_comment_editor_provider
                        .should_show_comment_editor(line_location, app),
                    None => true,
                };

                if should_render_comment_editor {
                    stack.add_positioned_child(
                        ChildView::new(&self.active_comment_editor).finish(),
                        OffsetPositioning::offset_from_parent(
                            vec2f(0., vertical_offset.as_f32()),
                            ParentOffsetBounds::ParentByPosition,
                            ParentAnchor::TopLeft,
                            ChildAnchor::TopLeft,
                        ),
                    );
                }
            }
        }
        stack.finish()
    }

    fn on_focus(&mut self, focus_ctx: &FocusContext, ctx: &mut ViewContext<Self>) {
        if focus_ctx.is_self_focused() && self.goto_line_dialog.as_ref(ctx).is_open() {
            ctx.focus(&self.goto_line_dialog);
        }
    }

    fn active_cursor_position(&self, ctx: &ViewContext<Self>) -> Option<CursorInfo> {
        let render_state = self.model.as_ref(ctx).render_state().as_ref(ctx);
        let cursor_id = render_state.saved_positions().cursor_id();
        let font_size = render_state.styles().base_text.font_size;

        ctx.element_position_by_id(cursor_id.as_str())
            .map(|position| CursorInfo {
                position,
                font_size,
            })
    }

    fn on_blur(&mut self, blur_ctx: &BlurContext, ctx: &mut ViewContext<Self>) {
        if blur_ctx.is_self_blurred() {
            ctx.notify();
        }
    }

    fn keymap_context(&self, app: &AppContext) -> warpui::keymap::Context {
        let mut context = Self::default_keymap_context();

        if self.interaction_state(app) != InteractionState::Editable {
            context.set.insert(NON_EDITABLE_KEYMAP_CONTEXT);
        }
        if let Some(vim_mode) = self.vim_mode(app) {
            context.set.insert("Vim");
            match vim_mode {
                VimMode::Normal => {
                    context.set.insert("VimNormalMode");
                }
                VimMode::Visual(_) => {
                    context.set.insert("VimVisualMode");
                }
                _ => {}
            }
        }
        if self.find_bar.is_some() {
            context.set.insert("FindBarAvailable");
        }
        context
    }
}

pub fn code_text_styles(
    appearance: &Appearance,
    font_settings: &FontSettings,
    line_height_override: Option<f32>,
) -> RichTextStyles {
    let mut styling = rich_text_styles(appearance, font_settings);
    let theme = appearance.theme();
    styling.base_text = ParagraphStyles {
        font_size: appearance.monospace_font_size(),
        line_height_ratio: line_height_override.unwrap_or(appearance.line_height_ratio()),
        font_family: appearance.monospace_font_family(),
        font_weight: Default::default(),
        text_color: theme.main_text_color(theme.background()).into_solid(),
        baseline_ratio: 0.8,
        fixed_width_tab_size: Some(4),
    };
    styling.block_spacings.text = BlockSpacing {
        margin: Margin::uniform(0.).with_left(1.),
        padding: Padding::uniform(0.),
    };
    styling.show_placeholder_text_on_empty_block = false;
    styling.minimum_paragraph_height = None;
    styling.cursor_width = 2.;
    // URLs are not clickable in code editors, so we should not highlight them.
    styling.highlight_urls = false;
    styling
}

#[cfg(feature = "integration_tests")]
impl CodeEditorView {
    pub fn open_goto_line_for_test(&mut self, ctx: &mut ViewContext<Self>) {
        self.show_goto_line(ctx);
    }

    pub fn goto_line_confirm_for_test(&mut self, input: &str, ctx: &mut ViewContext<Self>) {
        self.show_goto_line(ctx);
        let event = GoToLineEvent::Confirm {
            input: input.to_string(),
        };
        self.handle_goto_line_event(&event, ctx);
    }
}

#[cfg(test)]
mod view_tests;
