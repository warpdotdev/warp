mod gutter_button;
pub use gutter_button::{AddAsContextButton, CommentButton, RevertHunkButton};

use std::{
    ops::Range,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

use parking_lot::Mutex;
use pathfinder_color::ColorU;
use pathfinder_geometry::{
    rect::RectF,
    vector::{vec2f, Vector2F},
};
use warp_core::ui::{
    appearance::Appearance,
    theme::{color::internal_colors, Fill},
};
use warp_editor::{
    editor::EditorView,
    render::{
        element::{
            lens_element::RichTextElementLens, RenderableBlock, RichTextElement,
            VerticalExpansionBehavior,
        },
        model::{
            gutter_expansion_button_types, BlockLocation, ExpansionType, LineCount, RenderState,
        },
    },
};
use warpui::{
    elements::{
        new_scrollable::{NewScrollableElement, ScrollableAxis},
        Align, Axis, Border, ChildAnchor, ConstrainedBox, Container, CornerRadius, Empty, F32Ext,
        Flex, MainAxisSize, OffsetPositioning, ParentAnchor, ParentElement, ParentOffsetBounds,
        Point, Radius, ScrollData, Stack, Text, ZIndex,
    },
    event::DispatchedEvent,
    fonts::FamilyId,
    ui_components::components::UiComponent,
    units::{IntoPixels, Pixels},
    AfterLayoutContext, AppContext, ClipBounds, Element, Event, EventContext, LayoutContext,
    ModelHandle, PaintContext, SingletonEntity, SizeConstraint,
};

use super::diff::{DiffHunkDisplay, DiffStatus};
use super::model::DiffNavigationState;
use crate::code::editor::element::gutter_button::GutterButton;
use crate::{
    code::editor::{
        line::EditorLineLocation,
        view::{CodeEditorViewAction, SavedComment},
    },
    view_components::action_button::{ActionButtonTheme, SecondaryTheme},
};
use warp_core::features::FeatureFlag;
use warpui::elements::{Hoverable, MouseStateHandle};

pub const GUTTER_WIDTH: f32 = 94.;
const VERTICAL_DIFF_HUNK_INDICATOR_WIDTH: f32 = 3.;
const VERTICAL_DIFF_HUNK_INDICATOR_HOVERED_WIDTH: f32 = 8.;

fn highlight_element(appearance: &Appearance) -> Box<dyn Element> {
    let border_color = appearance.theme().accent();
    Container::new(Empty::new().finish())
        .with_border(Border::all(2.).with_border_fill(border_color))
        .finish()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeType {
    Add,
    Remove,
}

#[derive(Debug, Clone, Copy)]
pub enum GutterElementType {
    DiffHunk {
        hunk: Option<DiffHunkDisplay>,
        change_type: ChangeType,
    },
    HiddenSection {
        expansion_type: ExpansionType,
    },
}

/// The inner editor element wrapped by the EditorWrapper. Currently we support a full scrollable
/// editor or a lens element into a section of the buffer.
pub enum InnerEditor<V: EditorView> {
    FullEditor(RichTextElement<V>),
    Lens(RichTextElementLens<V>),
}

impl<V: EditorView> InnerEditor<V> {
    fn blocks(&self) -> Option<&[Box<dyn RenderableBlock>]> {
        match self {
            InnerEditor::FullEditor(element) => element.blocks(),
            InnerEditor::Lens(element) => element.blocks(),
        }
    }

    fn model(&self) -> &ModelHandle<RenderState> {
        match self {
            InnerEditor::FullEditor(element) => &element.model,
            InnerEditor::Lens(element) => &element.model,
        }
    }
}

impl<V: EditorView> Element for InnerEditor<V> {
    fn layout(
        &mut self,
        constraint: SizeConstraint,
        ctx: &mut LayoutContext,
        app: &AppContext,
    ) -> Vector2F {
        match self {
            InnerEditor::FullEditor(element) => element.layout(constraint, ctx, app),
            InnerEditor::Lens(element) => element.layout(constraint, ctx, app),
        }
    }

    fn after_layout(&mut self, ctx: &mut AfterLayoutContext, app: &AppContext) {
        match self {
            InnerEditor::FullEditor(element) => element.after_layout(ctx, app),
            InnerEditor::Lens(element) => element.after_layout(ctx, app),
        }
    }

    fn paint(&mut self, origin: Vector2F, ctx: &mut PaintContext, app: &AppContext) {
        match self {
            InnerEditor::FullEditor(element) => element.paint(origin, ctx, app),
            InnerEditor::Lens(element) => element.paint(origin, ctx, app),
        };
    }

    fn size(&self) -> Option<Vector2F> {
        None
    }

    fn origin(&self) -> Option<Point> {
        None
    }

    fn dispatch_event(
        &mut self,
        event: &DispatchedEvent,
        ctx: &mut EventContext,
        app: &AppContext,
    ) -> bool {
        match self {
            InnerEditor::FullEditor(element) => element.dispatch_event(event, ctx, app),
            InnerEditor::Lens(element) => element.dispatch_event(event, ctx, app),
        }
    }
}

struct GutterElement {
    element: Box<dyn Element>,
    offset: Pixels,
    height: f32,
    hovered: bool,
    line: EditorLineLocation,
    element_type: GutterElementType,
    /// Optional background fill for removed-line (temporary) blocks.
    overlay: Option<Fill>,
}

impl GutterElement {
    /// Checks if the given position falls within this gutter element's bounds.
    /// Returns a GutterRange if the position is contained, None otherwise.
    /// When `check_y_axis_only` is true, a position is considered within the element
    /// if it's anywhere within the `EditorWrapper`'s full line width.
    fn contains_position(
        &self,
        position: Vector2F,
        wrapper_origin: Vector2F,
        wrapper_size: Vector2F,
        check_y_axis_only: bool,
    ) -> Option<GutterRange> {
        let gutter_origin = wrapper_origin + vec2f(0., self.offset.as_f32());

        match self.element_type {
            GutterElementType::HiddenSection { expansion_type } => {
                let does_contain = if check_y_axis_only {
                    // We can count the position if it's within the editor wrapper's whole line width
                    let line_origin = Vector2F::new(wrapper_origin.x(), gutter_origin.y());
                    let line_size = Vector2F::new(wrapper_size.x(), self.height);
                    RectF::new(line_origin, line_size).contains_point(position)
                } else {
                    // Hidden sections use the full gutter width
                    let size = vec2f(GUTTER_WIDTH, self.height);
                    RectF::new(gutter_origin, size).contains_point(position)
                };
                if does_contain {
                    Some(GutterRange::HiddenSection {
                        line: self.line.clone(),
                        expansion_type,
                    })
                } else {
                    None
                }
            }
            GutterElementType::DiffHunk { .. } => {
                let does_contain = if check_y_axis_only {
                    // We can count the position if it's within the editor wrapper's whole line width
                    let line_origin = Vector2F::new(wrapper_origin.x(), gutter_origin.y());
                    let line_size = Vector2F::new(wrapper_size.x(), self.height);
                    RectF::new(line_origin, line_size).contains_point(position)
                } else {
                    // For diff hunks, check if position is in the sliver or the full gutter
                    // Get the sliver size (always use expanded=true for hit testing)
                    let sliver_size = self.diff_hunk_size(true)?;
                    let sliver_rect = RectF::new(gutter_origin, sliver_size);
                    sliver_rect.contains_point(position)
                };

                if does_contain {
                    // Position is in the sliver
                    Some(GutterRange::DiffHunk {
                        line: self.line.clone(),
                        in_sliver: true,
                    })
                } else {
                    // Check if position is in the full gutter width (but not in sliver)
                    let full_gutter_rect =
                        RectF::new(gutter_origin, vec2f(GUTTER_WIDTH, self.height));
                    if full_gutter_rect.contains_point(position) {
                        Some(GutterRange::DiffHunk {
                            line: self.line.clone(),
                            in_sliver: false,
                        })
                    } else {
                        None
                    }
                }
            }
        }
    }

    /// The size of the diff hunk element in gutter (if exists).
    fn diff_hunk_size(&self, gutter_element_is_hovered: bool) -> Option<Vector2F> {
        match self.element_type {
            GutterElementType::HiddenSection { .. } => {
                // Hidden section elements are always horizontal.
                Some(vec2f(GUTTER_WIDTH, self.height))
            }
            GutterElementType::DiffHunk { hunk: ref diff, .. } => {
                let vertical_indicator_width = if gutter_element_is_hovered {
                    VERTICAL_DIFF_HUNK_INDICATOR_HOVERED_WIDTH
                } else {
                    VERTICAL_DIFF_HUNK_INDICATOR_WIDTH
                };

                diff.as_ref()
                    .map(|_| vec2f(vertical_indicator_width, self.height))
            }
        }
    }

    // The color of the diff hunk indicator element in gutter (if it exists).
    fn diff_indicator_color(&self, diff_hunks_are_expanded: bool) -> Option<ColorU> {
        if let GutterElementType::DiffHunk {
            hunk: Some(diff_hunk),
            change_type,
        } = self.element_type
        {
            match &diff_hunk {
                DiffHunkDisplay::Add(color) | DiffHunkDisplay::Remove(color) => Some(*color),
                DiffHunkDisplay::Replacement {
                    collapsed_color: change_color,
                    add_color,
                    remove_color,
                } => {
                    if diff_hunks_are_expanded {
                        match change_type {
                            ChangeType::Add => Some(*add_color),
                            ChangeType::Remove => Some(*remove_color),
                        }
                    } else {
                        Some(*change_color)
                    }
                }
            }
        } else {
            None
        }
    }
}

/// States that need to live in between frames.
#[derive(Default)]
pub struct EditorWrapperState {
    /// The line range of the hovered diff hunk.
    hovered_diff_hunk: Mutex<Option<EditorLineLocation>>,
    /// Whether there is an active click.
    in_click: AtomicBool,
    /// Mouse state handle for the plus button.
    add_as_context_mouse_state: MouseStateHandle,
    /// Mouse state handle for the revert button.
    revert_mouse_state: MouseStateHandle,
    /// Mouse state handle for the comment button.
    comment_mouse_state: MouseStateHandle,
    /// Tracks the line range where the add context button was last clicked,
    /// so we don't show the button again until a different range is hovered.
    last_clicked_range: Mutex<Option<Range<LineCount>>>,
}

impl EditorWrapperState {
    /// Record that a range has been clicked (add context button was used)
    pub fn record_clicked_range(&self, range: Range<LineCount>) {
        *self.last_clicked_range.lock() = Some(range);
    }

    /// Clear the clicked range (when hovering over a new range)
    pub fn clear_clicked_range(&self) {
        *self.last_clicked_range.lock() = None;
    }

    /// Check if a range has been clicked
    pub fn is_range_clicked(&self, range: &Range<LineCount>) -> bool {
        self.last_clicked_range.lock().as_ref() == Some(range)
    }
}

pub type EditorWrapperStateHandle = Arc<EditorWrapperState>;

pub enum GutterRange {
    DiffHunk {
        line: EditorLineLocation,
        // Whether the position is in the sliver (the colored part) of the diff hunk element.
        in_sliver: bool,
    },
    HiddenSection {
        line: EditorLineLocation,
        expansion_type: ExpansionType,
    },
}

impl GutterRange {
    pub fn line(&self) -> &EditorLineLocation {
        match self {
            GutterRange::DiffHunk { line, .. } => line,
            GutterRange::HiddenSection { line, .. } => line,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum GutterHoverTarget {
    // The entire line covered by the gutter is considered the hover target.
    Line,
    // Only the gutter element itself is considered the hover target.
    GutterElement,
}

/// The caller defined handler for a click event.
type EditorWrapperClickHandler = Box<dyn FnMut(GutterRange, &mut EventContext)>;

/// UI Config for rendering line number.
pub struct LineNumberConfig {
    pub font_family: FamilyId,
    pub font_size: f32,
    pub text_color: ColorU,
    pub highlight_text_color: ColorU,
    pub starting_line_number: Option<usize>,
}

struct CommentBox {
    line_highlight_element: Box<dyn Element>,
    line: EditorLineLocation,
}

pub struct EditorWrapper<V: EditorView> {
    editor: InnerEditor<V>,
    element_size: Option<Vector2F>,
    element_origin: Option<Point>,
    /// Whether the editor should expand vertically to fill the available space.
    vertical_expansion_behavior: VerticalExpansionBehavior,
    /// If there is no [`LineNumberConfig`], the entire left gutter won't be rendered.
    line_number_config: Option<LineNumberConfig>,
    gutter_elements: Option<Vec<GutterElement>>,
    diff_status: DiffStatus,
    state_handle: EditorWrapperStateHandle,
    click_handler: EditorWrapperClickHandler,
    /// The current state of diff navigation in the editor
    diff_navigation_state: DiffNavigationState,
    /// The line range of the focused diff hunk (if any)
    focused_diff_line_range: Option<Range<LineCount>>,
    should_handle_scroll_wheel: bool,
    /// This helps us handle events properly on stacks. A stack will always
    /// put its children on higher z-indexes than its origin, so a hit test using the standard
    /// `z_index` method would always result in the event being covered (by the children of the
    /// stack). Instead, we track the upper-bound of z-indexes _contained by_ the child element.
    /// Then we use that upper bound to do the hit testing, which means a parent will always get
    /// events from its children, regardless of whether they are stacks or not.
    child_max_z_index: Option<ZIndex>,
    /// Display state of the "add as agent context" button shown next to diff hunks.
    add_hunk_as_context_button: Option<AddAsContextButton>,
    /// Display state of the "revert" button shown next to diff hunks.
    revert_hunk_button: Option<RevertHunkButton>,
    /// Display state of the "comment" button shown next to diff hunks.
    comment_button: Option<CommentButton>,
    // Todo: kc combine all comment related fields into a struct.
    comment_box: Option<CommentBox>,
    /// Lines with saved comments attached. These lines always have an
    /// indicator in the gutter element.
    saved_comments: Vec<SavedComment>,
    gutter_element_hover_target: GutterHoverTarget,
    expand_diff_indicator_width_on_hover: bool,
    comment_save_position_id: String,
    find_references_save_position_id: String,
    /// The line where find references card is anchored (if active).
    find_references_anchor: Option<EditorLineLocation>,
}

impl<V: EditorView> EditorWrapper<V> {
    fn paint_removed_line_overlays(
        &self,
        origin: Vector2F,
        wrapper_size: Vector2F,
        ctx: &mut PaintContext,
    ) {
        // Removed lines: group consecutive Remove-type gutter elements by line_range and
        // draw one full-width rect per group.
        let Some(gutter_elements) = &self.gutter_elements else {
            return;
        };

        struct Group {
            start_y: f32,
            end_y: f32,
            line_range: Range<LineCount>,
            overlay: Fill,
        }

        let mut group: Option<Group> = None;

        let mut flush = |group: &mut Option<Group>| {
            let Some(group) = group.take() else {
                return;
            };
            ctx.scene
                .draw_rect_without_hit_recording(RectF::new(
                    origin + vec2f(0., group.start_y),
                    vec2f(wrapper_size.x(), group.end_y - group.start_y),
                ))
                .with_background(group.overlay);
        };

        for element in gutter_elements.iter() {
            let is_remove = matches!(
                element.element_type,
                GutterElementType::DiffHunk {
                    change_type: ChangeType::Remove,
                    ..
                }
            );

            if !is_remove {
                flush(&mut group);
                continue;
            }

            let Some(overlay) = element.overlay else {
                flush(&mut group);
                continue;
            };

            let current_range = element.line.line_range().clone();
            let start_y = element.offset.as_f32();
            let end_y = start_y + element.height;

            match &mut group {
                Some(group) if group.line_range == current_range && group.overlay == overlay => {
                    group.end_y = end_y;
                }
                _ => {
                    flush(&mut group);
                    group = Some(Group {
                        start_y,
                        end_y,
                        line_range: current_range,
                        overlay,
                    });
                }
            }
        }

        flush(&mut group);
    }

    #[allow(clippy::too_many_arguments)]
    pub fn new(
        editor: InnerEditor<V>,
        vertical_expansion_behavior: VerticalExpansionBehavior,
        line_number_config: Option<LineNumberConfig>,
        diff_status: DiffStatus,
        state_handle: EditorWrapperStateHandle,
        click_handler: EditorWrapperClickHandler,
        should_handle_scroll_wheel: bool,
        diff_navigation_state: DiffNavigationState,
        focused_diff_line_range: Option<Range<LineCount>>,
        add_diff_as_context_button: Option<AddAsContextButton>,
        revert_hunk_button: Option<RevertHunkButton>,
        comment_button: Option<CommentButton>,
        saved_comments: Vec<SavedComment>,
        expand_diff_indicator_width_on_hover: bool,
        gutter_element_hover_target: GutterHoverTarget,
        comment_save_position_id: String,
        find_references_save_position_id: String,
    ) -> Self {
        Self {
            editor,
            vertical_expansion_behavior,
            element_size: None,
            element_origin: None,
            line_number_config,
            gutter_elements: None,
            diff_status,
            state_handle,
            diff_navigation_state,
            focused_diff_line_range,
            click_handler,
            should_handle_scroll_wheel,
            child_max_z_index: None,
            add_hunk_as_context_button: add_diff_as_context_button,
            revert_hunk_button,
            comment_button,
            expand_diff_indicator_width_on_hover,
            gutter_element_hover_target,
            comment_box: None,
            saved_comments,
            comment_save_position_id,
            find_references_save_position_id,
            find_references_anchor: None,
        }
    }

    /// Set the comment box to be displayed at a specific line
    pub fn set_comment_box(&mut self, line: EditorLineLocation, app: &AppContext) {
        let appearance = Appearance::as_ref(app);
        self.comment_box = Some(CommentBox {
            line_highlight_element: highlight_element(appearance),
            line,
        });
    }

    /// Set the find references anchor line for position caching
    pub fn set_find_references_anchor(&mut self, anchor: Option<EditorLineLocation>) {
        self.find_references_anchor = anchor;
    }

    /// True iff the diff hunks are expanded in the underlying editor model.
    fn diff_hunks_are_expanded(&self) -> bool {
        !matches!(self.diff_navigation_state, DiffNavigationState::Collapsed)
    }

    /// Returns the saved comment for the given line, if any.
    fn find_saved_comment_for_line(&self, line: &EditorLineLocation) -> Option<SavedComment> {
        self.saved_comments
            .iter()
            .find(|comment| comment.location().is_same_line(line))
            .cloned()
    }

    /// Returning **no** gutter means the gutter shouldn't be rendered at all.
    /// Returning an **empty** gutter means the gutter should be rendered with no contents.
    fn gutter_elements(&self, app: &AppContext) -> Option<Vec<GutterElement>> {
        let appearance = Appearance::as_ref(app);
        let Some(line_number_config) = &self.line_number_config else {
            return None;
        };
        let Some(blocks) = self.editor.blocks() else {
            return Some(Vec::new());
        };

        let mut elements = Vec::new();
        let model = self.model().as_ref(app);
        let hovered_range = self.state_handle.hovered_diff_hunk.lock();
        let line_decorations = model.decorations().line_decoration_ranges();
        let last_block_idx = blocks.len().saturating_sub(1);
        // Track the index of removal blocks with the same line_count
        let mut removed_hunk_line_number: Option<LineCount> = None;
        let mut removed_hunk_line_index: usize = 0;
        for (block_idx, block) in blocks.iter().enumerate() {
            let Some(line_count) = model.start_line_index(&**block) else {
                continue;
            };

            // For lens element, we need to use use the content offset - scroll top instead of the render model's viewport
            // offset since we are only rendering a section of the editor.
            let offset = match &self.editor {
                InnerEditor::Lens(element) => (block.viewport_item().content_offset.as_f32()
                    - element.starting_renderable_block_offset().unwrap_or(0.))
                .into_pixels(),
                InnerEditor::FullEditor(_) => block.viewport_item().viewport_offset,
            };
            let diff_hunk = self.diff_status.diff_hunk(line_count, appearance);
            let is_removal = matches!(diff_hunk, Some(DiffHunkDisplay::Remove(_)));

            let current_line =
                line_count.as_usize() + line_number_config.starting_line_number.unwrap_or(1);

            // If the block is temporary, don't render line number.
            // Currently, all temporary blocks are removal hunks, either from a deleted section,
            // or the old lines from a replacement hunk.
            if block.is_temporary() {
                let diff_range = self.diff_status.removed_diff_range(line_count);
                let range_already_clicked = diff_range
                    .as_ref()
                    .is_some_and(|range| self.state_handle.is_range_clicked(range));

                // If we are expanding diff hunks and the current block is a removal hunk, render
                // the gutter element with the line decoration.
                if self.diff_hunks_are_expanded()
                    && (is_removal
                        || matches!(diff_hunk, Some(DiffHunkDisplay::Replacement { .. })))
                {
                    // Track the index for this removal line
                    if removed_hunk_line_number == Some(line_count) {
                        removed_hunk_line_index += 1;
                    } else {
                        removed_hunk_line_index = 0;
                        removed_hunk_line_number = Some(line_count);
                    }

                    let height = block.viewport_item().content_size.y();

                    // Get the first line height for the plus icon (like we do for regular blocks)
                    let first_line_height = model.first_line_height(&**block).unwrap_or(height);

                    let line = EditorLineLocation::Removed {
                        line_number: line_count,
                        line_range: diff_range.unwrap_or(line_count..line_count),
                        index: removed_hunk_line_index,
                    };
                    // Show comment button only on the specific hovered line with matching index
                    let is_diff_line = self.diff_hunks_are_expanded() && diff_hunk.is_some();
                    let range_hovered = hovered_range
                        .as_ref()
                        .map(|hovered_line| hovered_line.line_range() == line.line_range())
                        .unwrap_or(false);
                    let is_this_line_hovered = hovered_range
                        .as_ref()
                        .is_some_and(|hovered_line| hovered_line.is_same_line(&line));
                    let attached_comment = self.find_saved_comment_for_line(&line);

                    let is_comment_on_current_line = attached_comment.is_some();

                    let is_comment_box_open = self.comment_box.is_some();
                    let is_comment_box_open_on_current_line = self
                        .comment_box
                        .as_ref()
                        .is_some_and(|comment_box| comment_box.line.is_same_line(&line));
                    let is_comment_box_open_on_different_line =
                        is_comment_box_open && !is_comment_box_open_on_current_line;

                    // We want to show the gutter buttons if either:
                    // 1) This line is part of a diff hunk that is being hovered and the comment box
                    // isn't open on another line.
                    // 2) We're currently on a line where the comment box is open.
                    let show_gutter_buttons = (is_diff_line
                        && is_this_line_hovered
                        && !range_already_clicked
                        && !is_comment_box_open_on_different_line)
                        || is_comment_box_open_on_current_line;

                    // Compute comment visibility independently of diff-hunk buttons
                    let should_show_comment_button = (is_this_line_hovered
                        && !is_comment_box_open_on_different_line)
                        || is_comment_box_open_on_current_line;

                    let element = self.render_gutter_element(
                        None,
                        line_number_config,
                        show_gutter_buttons,
                        should_show_comment_button,
                        is_comment_on_current_line,
                        attached_comment,
                        first_line_height,
                        height,
                        &line,
                        block.overlay_decoration(),
                        true, // is diff line
                        appearance,
                    );

                    elements.push(GutterElement {
                        element,
                        height,
                        offset,
                        hovered: range_hovered,
                        line,
                        element_type: GutterElementType::DiffHunk {
                            hunk: diff_hunk,
                            change_type: ChangeType::Remove,
                        },
                        overlay: block.overlay_decoration(),
                    });
                }

                continue;
            }

            if block.is_hidden_section() {
                let line_range = self
                    .model()
                    .as_ref(app)
                    .line_range(&**block)
                    .unwrap_or_default();

                let range_hovered = hovered_range
                    .as_ref()
                    .map(|line| line.line_range())
                    .is_some_and(|hovered_line_range| *hovered_line_range == line_range);
                let range_length = line_range.end - line_range.start;
                let height = block.viewport_item().content_size.y();

                let block_location = if block_idx == 0 {
                    BlockLocation::Start
                } else if block_idx >= last_block_idx {
                    BlockLocation::End
                } else {
                    BlockLocation::Middle
                };

                let expand_button_types =
                    gutter_expansion_button_types(&block_location, range_length.as_usize());
                let mut new_gutter_elements = if expand_button_types.len() == 1 {
                    vec![self.construct_expand_hidden_section_gutter_element(
                        height,
                        expand_button_types[0],
                        range_hovered,
                        appearance,
                        line_number_config,
                        line_range,
                        offset,
                    )]
                } else {
                    let half_len = range_length.as_usize() / 2;
                    let midpoint = line_range.start + half_len;

                    let (first_half_range, second_half_range) =
                        (line_range.start..midpoint, midpoint..line_range.end);

                    let first_range_hovered = hovered_range
                        .as_ref()
                        .map(|line| line.line_range().contains(&line_count))
                        .unwrap_or(false);

                    let second_range_hovered = hovered_range
                        .as_ref()
                        .map(|line| line.line_range().contains(&midpoint))
                        .unwrap_or(false);

                    // The editor will render a double-height line,
                    // so the buttons should split the available vertical space.
                    let height = height / 2.0;
                    vec![
                        self.construct_expand_hidden_section_gutter_element(
                            height,
                            expand_button_types[0],
                            first_range_hovered,
                            appearance,
                            line_number_config,
                            first_half_range,
                            offset,
                        ),
                        self.construct_expand_hidden_section_gutter_element(
                            height,
                            expand_button_types[1],
                            second_range_hovered,
                            appearance,
                            line_number_config,
                            second_half_range,
                            offset + Pixels::new(height),
                        ),
                    ]
                };

                elements.append(&mut new_gutter_elements);

                continue;
            }
            let diff_range = self.diff_status.added_diff_range(line_count);
            let range_already_clicked = diff_range
                .as_ref()
                .is_some_and(|range| self.state_handle.is_range_clicked(range));

            // If the corresponding line in the editor element has a line decoration, we should apply the decoration
            // in the wrapper as well. This does assume the line could only have a single decoration. I think it's fine
            // given this is a wrapper specific to our code pane implementation.
            let overlay = line_decorations
                .iter()
                .filter(|decoration| decoration.start <= line_count && decoration.end > line_count)
                .map(|decoration| decoration.overlay)
                .next();

            let Some(height) = model.first_line_height(&**block) else {
                continue;
            };

            // Check if this line is part of any diff hunk when diff hunks are expanded and hovered
            let is_diff_line = self.diff_hunks_are_expanded() && diff_hunk.is_some();

            let line = EditorLineLocation::Current {
                line_number: line_count,
                line_range: diff_range.unwrap_or(line_count..line_count + 1),
            };

            let range_hovered = hovered_range
                .as_ref()
                .is_some_and(|hovered_line| hovered_line.line_range() == line.line_range());

            // Show comment button only on the specific hovered line
            let is_this_line_hovered = hovered_range
                .as_ref()
                .is_some_and(|hovered_line| hovered_line.is_same_line(&line));
            let attached_comment = self.find_saved_comment_for_line(&line);

            let is_comment_box_open = self.comment_box.is_some();
            let is_comment_box_open_on_current_line = self
                .comment_box
                .as_ref()
                .is_some_and(|comment_box| comment_box.line.is_same_line(&line));
            let is_comment_box_open_on_different_line =
                is_comment_box_open && !is_comment_box_open_on_current_line;

            // We want to show the gutter buttons if either:
            // 1) This line is part of a diff hunk that is being hovered and the comment box
            // isn't open on another line.
            // 2) We're currently on a line where the comment box is open.
            let should_show_diff_hunk_button = (is_diff_line
                && is_this_line_hovered
                && !is_removal
                && !range_already_clicked
                && !is_comment_box_open_on_different_line)
                || is_comment_box_open_on_current_line;

            let is_comment_on_current_line = attached_comment.is_some();

            // The gutter element should take the same height as block's first line.
            // Compute comment visibility for any hovered/current line, not just diff lines
            // For non-diff lines, check the feature flag before showing comment button
            let should_show_comment_button = {
                if !is_diff_line && !FeatureFlag::ContextLineReviewComments.is_enabled() {
                    // Only show comment button if there's already a comment on this line
                    is_comment_box_open_on_current_line
                } else {
                    // Show comment button normally for diff lines or when feature flag is enabled
                    (is_this_line_hovered && !is_comment_box_open_on_different_line)
                        || is_comment_box_open_on_current_line
                }
            };

            let element = self.render_gutter_element(
                Some(current_line),
                line_number_config,
                should_show_diff_hunk_button,
                should_show_comment_button,
                is_comment_on_current_line,
                attached_comment,
                height,
                height,
                &line,
                overlay,
                is_diff_line,
                appearance,
            );

            let diff_hunk = if is_removal && self.diff_hunks_are_expanded() {
                None
            } else {
                diff_hunk
            };
            elements.push(GutterElement {
                element,
                height,
                offset,
                hovered: range_hovered,
                // We can skip rendering this removal gutter element if its hunk is expanded since
                // the gutter is rendered on the temporary block.
                line,
                element_type: GutterElementType::DiffHunk {
                    hunk: diff_hunk,
                    change_type: ChangeType::Add,
                },
                overlay: None,
            });
        }
        Some(elements)
    }

    #[allow(clippy::too_many_arguments)]
    fn construct_expand_hidden_section_gutter_element(
        &self,
        height: f32,
        expansion_type: ExpansionType,
        range_hovered: bool,
        appearance: &Appearance,
        line_number_config: &LineNumberConfig,
        line_range: Range<LineCount>,
        offset: Pixels,
    ) -> GutterElement {
        // Use a slightly stronger overlay when hovered for better visual feedback
        let theme = appearance.theme();
        let gutter_background_color = if range_hovered {
            internal_colors::fg_overlay_2(theme)
        } else {
            internal_colors::fg_overlay_1(theme)
        };

        let icon = ConstrainedBox::new(
            warpui::elements::Icon::new(
                expansion_type.icon().into(),
                line_number_config.text_color,
            )
            .finish(),
        )
        .with_width(16.)
        .with_height(16.)
        .finish();

        let element = Container::new(
            ConstrainedBox::new(Align::new(icon).finish())
                .with_height(height)
                .with_width(GUTTER_WIDTH)
                .finish(),
        )
        .with_background_color(gutter_background_color.into())
        .finish();

        GutterElement {
            element,
            offset,
            height,
            hovered: range_hovered,
            line: EditorLineLocation::Collapsed { line_range },
            element_type: GutterElementType::HiddenSection { expansion_type },
            overlay: None,
        }
    }

    fn render_gutter_button(
        &self,
        mouse_state: MouseStateHandle,
        gutter_element_height: f32,
        on_click_action: Option<CodeEditorViewAction>,
        appearance: &Appearance,
        gutter_button: &dyn GutterButton,
    ) -> Box<dyn Element> {
        let vertical_padding = if FeatureFlag::InlineCodeReview.is_enabled() {
            2.
        } else {
            4.
        };

        let button_size = gutter_element_height;
        let icon_size = button_size - (vertical_padding * 2.);
        let enabled = gutter_button.is_enabled();

        let mut button = Hoverable::new(mouse_state, |state| {
            let button_background = gutter_button.background_color(state, appearance);
            let icon_color = gutter_button.icon_color(state, appearance);

            let border = SecondaryTheme.border(appearance);

            let container = Container::new(
                ConstrainedBox::new(
                    warpui::elements::Icon::new(gutter_button.icon().into(), icon_color).finish(),
                )
                .with_width(icon_size)
                .with_height(icon_size)
                .finish(),
            )
            .with_uniform_padding(2.)
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)))
            .with_background(button_background);

            let container = if let Some(border) = border {
                container.with_border(Border::all(1.0).with_border_fill(border))
            } else {
                container
            };

            let mut stack = Stack::new().with_child(container.finish());
            if state.is_hovered() {
                if let Some(text) = gutter_button.tooltip_text() {
                    let tooltip = appearance
                        .ui_builder()
                        .tool_tip(text.into())
                        .build()
                        .finish();
                    stack.add_positioned_overlay_child(
                        tooltip,
                        OffsetPositioning::offset_from_parent(
                            vec2f(0., 8.),
                            ParentOffsetBounds::Unbounded,
                            ParentAnchor::BottomLeft,
                            ChildAnchor::TopLeft,
                        ),
                    );
                }
            }

            stack.finish()
        });

        if enabled {
            button = button.with_cursor(warpui::platform::Cursor::PointingHand);

            if let Some(on_click_action) = on_click_action {
                let action = on_click_action.clone();
                button = button.on_click(move |event, _app, _position| {
                    event.dispatch_typed_action(action.clone());
                });
            }
        }

        button.finish()
    }

    /// Renders the plus button for adding a diff as Agent context.
    fn render_plus_button(
        &self,
        add_as_context_button: &AddAsContextButton,
        gutter_element_height: f32,
        diff_line_range: &Range<LineCount>,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let on_click_action = Some(CodeEditorViewAction::AddDiffHunkContext {
            line_range: diff_line_range.to_owned(),
        });

        self.render_gutter_button(
            self.state_handle.add_as_context_mouse_state.clone(),
            gutter_element_height,
            on_click_action,
            appearance,
            add_as_context_button,
        )
    }

    /// Renders the revert button for reverting a specific diff hunk.
    fn render_revert_button(
        &self,
        revert_hunk_button: &RevertHunkButton,
        gutter_element_height: f32,
        diff_line_range: &Range<LineCount>,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let on_click_action = Some(CodeEditorViewAction::RevertDiffHunk {
            line_range: diff_line_range.to_owned(),
        });

        self.render_gutter_button(
            self.state_handle.revert_mouse_state.clone(),
            gutter_element_height,
            on_click_action,
            appearance,
            revert_hunk_button,
        )
    }

    /// Renders the comment button for adding comments to diff hunks.
    fn render_comment_button(
        &self,
        gutter_element_height: f32,
        line: &EditorLineLocation,
        attached_comment: Option<SavedComment>,
        appearance: &Appearance,
        comment_button: &CommentButton,
    ) -> Box<dyn Element> {
        let (on_click_action, comment_button, mouse_state) = if self
            .comment_box
            .as_ref()
            .is_some_and(|comment_box| comment_box.line.is_same_line(line))
        {
            let comment_button = if attached_comment.is_some() {
                CommentButton::EditorOpenedToUpdateComment
            } else {
                CommentButton::EditorOpenedToCreateNewComment
            };
            // If the comment box is already open for this line, don't do anything on click
            (
                None,
                comment_button,
                self.state_handle.comment_mouse_state.clone(),
            )
        } else {
            match attached_comment {
                Some(saved_comment) => (
                    Some(CodeEditorViewAction::RequestOpenSavedComment {
                        uuid: saved_comment.uuid(),
                    }),
                    CommentButton::AddedComment,
                    saved_comment.mouse_state().clone(),
                ),
                None => (
                    Some(CodeEditorViewAction::NewCommentOnLine { line: line.clone() }),
                    *comment_button,
                    self.state_handle.comment_mouse_state.clone(),
                ),
            }
        };

        self.render_gutter_button(
            mouse_state,
            gutter_element_height,
            on_click_action,
            appearance,
            &comment_button,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn render_gutter_element(
        &self,
        current_line: Option<usize>,
        line_number_config: &LineNumberConfig,
        should_show_diff_hunk_icons: bool,
        should_show_comment_button: bool,
        is_active_comment_on_current_line: bool,
        attached_comment: Option<SavedComment>,
        line_height: f32,
        gutter_element_height: f32,
        line: &EditorLineLocation,
        overlay: Option<Fill>,
        highlight_text: bool,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let base_content = match current_line {
            Some(line) => {
                // Highlight the line number if it's a diff line or has an overlay
                let text_color = if highlight_text || overlay.is_some() {
                    line_number_config.highlight_text_color
                } else {
                    line_number_config.text_color
                };
                Align::new(
                    Text::new_inline(
                        line.to_string(),
                        line_number_config.font_family,
                        line_number_config.font_size,
                    )
                    .with_selectable(true)
                    .with_color(text_color)
                    .finish(),
                )
                .finish()
            }
            None => {
                // If no current line, render empty element
                Empty::new().finish()
            }
        };

        let constrained_base = ConstrainedBox::new(base_content)
            .with_height(gutter_element_height)
            .with_width(GUTTER_WIDTH)
            .finish();

        let show_add_as_context_button = self.add_hunk_as_context_button.is_some();
        let show_revert_diff_hunk =
            FeatureFlag::RevertDiffHunk.is_enabled() && self.revert_hunk_button.is_some();

        // Show comment button independently of diff hunk state when requested
        let show_comment_button = FeatureFlag::InlineCodeReview.is_enabled()
            && self.comment_button.is_some()
            && (should_show_comment_button || is_active_comment_on_current_line);

        if should_show_diff_hunk_icons || is_active_comment_on_current_line || show_comment_button {
            let mut buttons = Flex::row().with_main_axis_size(MainAxisSize::Min);
            if let Some(comment_button) =
                self.comment_button.as_ref().filter(|_| show_comment_button)
            {
                buttons.add_child(self.render_comment_button(
                    line_height,
                    line,
                    attached_comment,
                    appearance,
                    comment_button,
                ));
            }

            if should_show_diff_hunk_icons {
                if let Some(revert_hunk_button) = self
                    .revert_hunk_button
                    .as_ref()
                    .filter(|_| show_revert_diff_hunk)
                {
                    buttons.add_child(self.render_revert_button(
                        revert_hunk_button,
                        line_height,
                        line.line_range(),
                        appearance,
                    ));
                }

                if let Some(add_as_context_button) = self
                    .add_hunk_as_context_button
                    .as_ref()
                    .filter(|_| show_add_as_context_button)
                {
                    buttons.add_child(self.render_plus_button(
                        add_as_context_button,
                        line_height,
                        line.line_range(),
                        appearance,
                    ));
                }
            }

            let offset = if self.expand_diff_indicator_width_on_hover {
                vec2f(VERTICAL_DIFF_HUNK_INDICATOR_HOVERED_WIDTH, 0.)
            } else {
                vec2f(VERTICAL_DIFF_HUNK_INDICATOR_WIDTH, 0.)
            };

            // Create a stack with the constrained base and overlay the buttons
            let mut main_stack = Stack::new().with_child(constrained_base);
            main_stack.add_positioned_child(
                buttons.finish(),
                OffsetPositioning::offset_from_parent(
                    offset,
                    ParentOffsetBounds::Unbounded,
                    ParentAnchor::TopLeft,
                    ChildAnchor::TopLeft,
                ),
            );
            main_stack.finish()
        } else {
            constrained_base
        }
    }

    /// Returns whether a diff hunk range contains the given mouse position on the screen.
    fn gutter_element_range_containing_position(
        &self,
        position: Vector2F,
        check_y_axis: bool,
    ) -> Option<GutterRange> {
        let wrapper_origin = self.element_origin?.xy();
        let wrapper_size = self.size()?;

        if let Some(gutter_elements) = &self.gutter_elements {
            for gutter_element in gutter_elements {
                let gutter_range = gutter_element.contains_position(
                    position,
                    wrapper_origin,
                    wrapper_size,
                    check_y_axis,
                );
                if let Some(gutter_range) = gutter_range {
                    return Some(gutter_range);
                }
            }
        }

        None
    }

    fn model(&self) -> &ModelHandle<RenderState> {
        self.editor.model()
    }

    fn size_buffer(&self) -> Vector2F {
        let is_gutter_present = self.line_number_config.is_some();
        if is_gutter_present {
            GUTTER_WIDTH.along(Axis::Horizontal)
        } else {
            Vector2F::zero()
        }
    }
}

impl<V: EditorView> Element for EditorWrapper<V> {
    fn layout(
        &mut self,
        constraint: SizeConstraint,
        ctx: &mut LayoutContext,
        app: &AppContext,
    ) -> Vector2F {
        let size_buffer = self.size_buffer();
        let content_constraint = SizeConstraint::new(
            (constraint.min - size_buffer).max(Vector2F::zero()),
            (constraint.max - size_buffer).max(Vector2F::zero()),
        );

        // Layout the editor element first so we can read the laid out visible blocks.
        let editor_size = self.editor.layout(content_constraint, ctx, app);

        let size = match self.vertical_expansion_behavior {
            VerticalExpansionBehavior::GrowToMaxHeight
            | VerticalExpansionBehavior::InfiniteHeight => {
                Vector2F::new(constraint.max.x(), constraint.max.y().min(editor_size.y()))
            }
            VerticalExpansionBehavior::FillMaxHeight => constraint.max,
        };

        let mut gutter_elements = self.gutter_elements(app);
        if let Some(gutter_elements) = &mut gutter_elements {
            for gutter_element in gutter_elements {
                let gutter_element_size = gutter_element.element.layout(constraint, ctx, app);

                if FeatureFlag::InlineCodeReview.is_enabled() {
                    if let Some(comment_box) = &mut self.comment_box {
                        let highlight_line = &comment_box.line;
                        if gutter_element.line == *highlight_line {
                            let highlight_width = size.x();
                            let highlight_height = gutter_element_size.y();
                            comment_box.line_highlight_element.layout(
                                SizeConstraint {
                                    min: vec2f(0.0, 0.0),
                                    max: vec2f(highlight_width, highlight_height),
                                },
                                ctx,
                                app,
                            );
                        }
                    }
                }
            }
        }

        self.gutter_elements = gutter_elements;
        self.element_size = Some(size);
        size
    }

    fn after_layout(&mut self, ctx: &mut AfterLayoutContext, app: &AppContext) {
        self.editor.after_layout(ctx, app);
    }

    fn paint(&mut self, origin: Vector2F, ctx: &mut PaintContext, app: &AppContext) {
        // Save the element origin for hit testing.
        let element_origin = Point::from_vec2f(origin, ctx.scene.z_index());
        self.element_origin = Some(element_origin);

        let size_buffer = self.size_buffer();
        let wrapper_size = self.size().unwrap_or_default();

        // Pre-pass: Draw full-width overlay rects for diff highlighting.
        // Drawing before the inner editor and gutter elements so they appear behind text.
        // Clip to the wrapper bounds so overlays don't bleed outside the element
        // (important for Lens mode where LineDecoration ranges may exceed the visible range).
        let overlay_clip = RectF::new(origin, wrapper_size);
        ctx.scene
            .start_layer(ClipBounds::BoundedByActiveLayerAnd(overlay_clip));

        // Added/replaced lines: one rect per LineDecoration range.
        {
            let model = self.model().as_ref(app);
            let content = model.content();
            let y_adjustment = match &self.editor {
                InnerEditor::Lens(element) => element
                    .starting_renderable_block_offset()
                    .unwrap_or(0.)
                    .into_pixels(),
                InnerEditor::FullEditor(_) => model.viewport().scroll_top(),
            };
            for decoration in model.decorations().line_decoration_ranges() {
                let start_y = content.y_offset_at_line(decoration.start);
                let end_y = content.y_offset_at_line(decoration.end);
                ctx.scene
                    .draw_rect_without_hit_recording(RectF::new(
                        origin + vec2f(0., (start_y - y_adjustment).as_f32()),
                        vec2f(wrapper_size.x(), (end_y - start_y).as_f32()),
                    ))
                    .with_background(decoration.overlay);
            }
        }

        self.paint_removed_line_overlays(origin, wrapper_size, ctx);

        ctx.scene.stop_layer();

        self.editor.paint(origin + size_buffer, ctx, app);

        let diff_hunks_are_expanded = self.diff_hunks_are_expanded();
        let gutter_width = self.size_buffer().x();
        let gutter_bounds = RectF::new(origin, vec2f(gutter_width, wrapper_size.y()));

        // Track the offset and height of the gutter element under which we should render the
        // inline comment box.
        let mut inline_comment_gutter_element: Option<(f32, f32)> = None;
        // Track the offset and height of the gutter element for find references anchor.
        let mut find_references_gutter_element: Option<(f32, f32)> = None;

        if let Some(gutter_elements) = &mut self.gutter_elements {
            // Start clipping layer for gutter elements to prevent overflow
            // at the bottom of the editor.
            // Use BoundedByActiveLayerAnd to intersect with parent scrollable's clipping.
            ctx.scene
                .start_layer(ClipBounds::BoundedByActiveLayerAnd(gutter_bounds));

            // Combined pass: compute grouped diff indicator sliver rects and
            // track positions for inline comment / find references.
            // Adjacent indicators in the same diff range are grouped into one tall rect.
            struct SliverGroup<'a> {
                start_y: f32,
                end_y: f32,
                range: &'a Range<LineCount>,
                color: ColorU,
                width: f32,
            }

            let mut sliver_rects: Vec<(RectF, ColorU)> = Vec::new();
            let mut group: Option<SliverGroup<'_>> = None;

            for element in gutter_elements.iter() {
                let gutter_y = element.offset.as_f32();

                // For rendering, only expand the gutter element size if it is
                // hovered or it is the active diff hunk.
                let is_hovered = self.expand_diff_indicator_width_on_hover
                    && (element.hovered
                        || self
                            .focused_diff_line_range
                            .as_ref()
                            .is_some_and(|r| r == element.line.line_range()));

                if let Some(size) = element.diff_hunk_size(is_hovered) {
                    if let Some(color) = element.diff_indicator_color(diff_hunks_are_expanded) {
                        let current_range = element.line.line_range();
                        let same_group = group.as_ref().is_some_and(|g| {
                            g.range == current_range
                                && g.color == color
                                && (g.width - size.x()).abs() < f32::EPSILON
                        });
                        if same_group {
                            group.as_mut().unwrap().end_y = gutter_y + element.height;
                        } else {
                            // Flush previous group.
                            if let Some(g) = group.take() {
                                sliver_rects.push((
                                    RectF::new(
                                        origin + vec2f(0., g.start_y),
                                        vec2f(g.width, g.end_y - g.start_y),
                                    ),
                                    g.color,
                                ));
                            }
                            // Start new group.
                            group = Some(SliverGroup {
                                start_y: gutter_y,
                                end_y: gutter_y + element.height,
                                range: current_range,
                                color,
                                width: size.x(),
                            });
                        }
                    }
                }

                // Track positions for inline comment and find references.
                if !matches!(
                    element.element_type,
                    GutterElementType::HiddenSection { .. }
                ) {
                    // If this is the gutter element for the inline comment box,
                    // save its position for later rendering.
                    if self
                        .comment_box
                        .as_ref()
                        .is_some_and(|cb| element.line == cb.line)
                    {
                        inline_comment_gutter_element = Some((gutter_y, element.height));
                    }

                    // If this is the gutter element for find references anchor,
                    // save its position for caching.
                    if self
                        .find_references_anchor
                        .as_ref()
                        .is_some_and(|anchor| element.line.is_same_line(anchor))
                    {
                        find_references_gutter_element = Some((gutter_y, element.height));
                    }
                }
            }
            // Flush last group.
            if let Some(g) = group.take() {
                sliver_rects.push((
                    RectF::new(
                        origin + vec2f(0., g.start_y),
                        vec2f(g.width, g.end_y - g.start_y),
                    ),
                    g.color,
                ));
            }

            // Draw all buffered sliver rects (behind gutter elements).
            for (rect, color) in &sliver_rects {
                ctx.scene
                    .draw_rect_with_hit_recording(*rect)
                    .with_background(*color);
            }

            // Paint gutter elements.
            for gutter_element in gutter_elements.iter_mut() {
                let gutter_origin = origin + vec2f(0., gutter_element.offset.as_f32());
                gutter_element.element.paint(gutter_origin, ctx, app);
            }

            ctx.scene.stop_layer();

            if FeatureFlag::InlineCodeReview.is_enabled() {
                if let Some(comment_box) = &mut self.comment_box {
                    if let Some((offset, height)) = inline_comment_gutter_element {
                        let gutter_origin = origin + vec2f(0., offset);

                        // Highlight the selected line.
                        comment_box
                            .line_highlight_element
                            .paint(gutter_origin, ctx, app);

                        // Cache a single pixel where the comment editor would be located
                        // if were to render it within this element.
                        let rect = RectF::new(gutter_origin, vec2f(1., height));
                        ctx.position_cache.cache_position_for_one_frame(
                            self.comment_save_position_id.clone(),
                            rect,
                        );
                    }
                }
            }
        }

        // Cache find references anchor position if we have one.
        if self.find_references_anchor.is_some() {
            if let Some((offset, height)) = find_references_gutter_element {
                let gutter_origin = origin + vec2f(0., offset);

                // Cache the gutter position for the find references anchor.
                // This will be used to determine if the card should be shown.
                let rect = RectF::new(gutter_origin, vec2f(1., height));
                ctx.position_cache.cache_position_for_one_frame(
                    self.find_references_save_position_id.clone(),
                    rect,
                );
            }
        }

        self.child_max_z_index = Some(ctx.scene.max_active_z_index());
    }

    fn size(&self) -> Option<Vector2F> {
        self.element_size
    }

    fn origin(&self) -> Option<Point> {
        self.element_origin
    }

    fn dispatch_event(
        &mut self,
        event: &DispatchedEvent,
        ctx: &mut EventContext,
        app: &AppContext,
    ) -> bool {
        let Some(z_index) = self
            .child_max_z_index
            .or_else(|| self.element_origin.map(|origin| origin.z_index()))
        else {
            return false;
        };

        // Then, dispatch events to gutter elements that are in the hovered range
        // This is important for Hoverable elements like the plus button
        let mut gutter_handled = false;
        if let Some(gutter_elements) = &mut self.gutter_elements {
            let hovered_range = self.state_handle.hovered_diff_hunk.lock().clone();

            for gutter_element in gutter_elements {
                let should_dispatch = hovered_range
                    .as_ref()
                    .is_some_and(|hovered_line| hovered_line.is_same_line(&gutter_element.line));

                if should_dispatch && gutter_element.element.dispatch_event(event, ctx, app) {
                    gutter_handled = true;
                }
            }
        }

        // Handle mouse events for hover state and clicks
        match event.at_z_index(z_index, ctx) {
            Some(Event::MouseMoved { position, .. }) => {
                let only_check_y_axis =
                    matches!(self.gutter_element_hover_target, GutterHoverTarget::Line);
                let hovered_line = self
                    .gutter_element_range_containing_position(*position, only_check_y_axis)
                    .map(|gutter_range| gutter_range.line().clone());
                let mut hovered_diff_hunk = self.state_handle.hovered_diff_hunk.lock();
                if hovered_diff_hunk.as_ref() != hovered_line.as_ref() {
                    // When hovering over a new range, clear the previously clicked range
                    // so the add context button can appear again
                    if hovered_line.is_some() {
                        self.state_handle.clear_clicked_range();
                    }
                    *hovered_diff_hunk = hovered_line;
                    ctx.notify();
                }
            }
            Some(Event::LeftMouseDown { position, .. }) => {
                if !gutter_handled {
                    let in_bound = self
                        .gutter_element_range_containing_position(*position, false)
                        .is_some();
                    self.state_handle
                        .in_click
                        .store(in_bound, Ordering::Relaxed);
                }
            }
            Some(Event::LeftMouseUp { position, .. }) => {
                if !gutter_handled {
                    let was_clicking = self.state_handle.in_click.swap(false, Ordering::Relaxed);

                    if was_clicking {
                        if let Some(gutter_range) =
                            self.gutter_element_range_containing_position(*position, false)
                        {
                            (self.click_handler)(gutter_range, ctx);
                        }
                    }
                }
            }
            _ => (),
        };

        let dispatch_to_editor = self.editor.dispatch_event(event, ctx, app);

        // Always dispatch to both the gutter elements and the editor.
        dispatch_to_editor || gutter_handled
    }
}

impl<V: EditorView> NewScrollableElement for EditorWrapper<V> {
    fn scroll_data(&self, axis: Axis, app: &AppContext) -> Option<ScrollData> {
        // TODO: Support scrolling in editor lens.
        match &self.editor {
            InnerEditor::FullEditor(element) => element.scroll_data(axis, app),
            InnerEditor::Lens(_) => None,
        }
    }

    fn scroll(&mut self, delta: Pixels, axis: Axis, ctx: &mut EventContext) {
        if let InnerEditor::FullEditor(element) = &mut self.editor {
            element.scroll(delta, axis, ctx)
        }
    }

    fn axis_should_handle_scroll_wheel(&self, _axis: Axis) -> bool {
        self.should_handle_scroll_wheel
    }

    fn axis(&self) -> ScrollableAxis {
        ScrollableAxis::Both
    }
}
