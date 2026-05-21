//! Find References UI component for displaying LSP textDocument/references results.
//!
//! This module provides a hover card that shows all references to a symbol
//! as a flat list with file info, line numbers, and syntax-highlighted code snippets.

use std::collections::HashMap;
use std::path::PathBuf;

use lsp::ReferenceLocation;
use pathfinder_geometry::vector::Vector2F;
use string_offset::CharOffset;
use warp_core::ui::appearance::Appearance;
use warp_core::ui::icons::Icon as WarpIcon;
use warp_core::ui::theme::color::internal_colors;
use warp_editor::content::buffer::InitialBufferState;
use warp_editor::render::element::VerticalExpansionBehavior;
use warp_files::FileModel;
use warpui::elements::{
    Border, ChildAnchor, ChildView, ClippedScrollStateHandle, ClippedScrollable, ConstrainedBox,
    Container, CornerRadius, CrossAxisAlignment, Fill, Flex, Hoverable, MouseStateHandle,
    OffsetPositioning, ParentAnchor, ParentElement, ParentOffsetBounds, Radius, ScrollbarWidth,
    Shrinkable, Stack, Text,
};
use warpui::keymap::FixedBinding;
use warpui::platform::Cursor;
use warpui::prelude::Align;
use warpui::ui_components::components::UiComponent;
use warpui::{
    AppContext, Element, Entity, SingletonEntity, TypedActionView, View, ViewContext, ViewHandle,
};

use super::editor::view::{CodeEditorRenderOptions, CodeEditorView};
use super::global_buffer_model::GlobalBufferModel;
use crate::editor::InteractionState;
use crate::search::result_renderer::ItemHighlightState;

/// Maximum height for the find references card.
pub const FIND_REFERENCES_CARD_MAX_HEIGHT: f32 = 300.;

const HAS_REFERENCES: &str = "HasReferences";

pub fn init(app: &mut AppContext) {
    use warpui::keymap::macros::*;

    app.register_fixed_bindings([
        FixedBinding::new(
            "escape",
            FindReferencesViewAction::Close,
            id!(FindReferencesView::ui_name()),
        ),
        FixedBinding::new(
            "enter",
            FindReferencesViewAction::SelectReference,
            id!(FindReferencesView::ui_name()) & id!(HAS_REFERENCES),
        ),
        FixedBinding::new(
            "numpadenter",
            FindReferencesViewAction::SelectReference,
            id!(FindReferencesView::ui_name()) & id!(HAS_REFERENCES),
        ),
        FixedBinding::new(
            "up",
            FindReferencesViewAction::ArrowUp,
            id!(FindReferencesView::ui_name()) & id!(HAS_REFERENCES),
        ),
        FixedBinding::new(
            "down",
            FindReferencesViewAction::ArrowDown,
            id!(FindReferencesView::ui_name()) & id!(HAS_REFERENCES),
        ),
    ]);
}

/// Actions that FindReferencesView can handle.
#[derive(Clone, Debug)]
pub enum FindReferencesViewAction {
    /// Navigate to the reference at the given index.
    GotoReference(usize),
    /// Close the references card.
    Close,
    /// Move selection up (arrow up).
    ArrowUp,
    /// Move selection down (arrow down).
    ArrowDown,
    /// Select the currently highlighted reference (enter).
    SelectReference,
}

/// Events emitted by FindReferencesView.
pub enum FindReferencesViewEvent {
    /// User requested navigation to a reference.
    GotoReference(usize),
    /// User requested to close the card.
    CloseRequested,
}

/// View for displaying find references results with async line loading.
pub struct FindReferencesView {
    /// Flat list of all references with their UI state.
    references: Vec<ReferenceEntryWithUi>,
    /// The offset where references were requested (used for positioning).
    request_offset: CharOffset,
    /// Scroll state for the card content.
    scroll_state: ClippedScrollStateHandle,
    back_mouse_state: MouseStateHandle,
    /// The index of the currently selected reference for keyboard navigation.
    selected_reference_index: usize,
}

/// A reference entry bundled with its UI state.
pub struct ReferenceEntryWithUi {
    /// The reference data.
    pub entry: FlatReferenceEntry,
    /// Mouse state for the entry row hover.
    pub entry_mouse_state: MouseStateHandle,
    /// Mouse state for the file name tooltip.
    pub file_name_mouse_state: MouseStateHandle,
    /// Editor view for syntax highlighting.
    pub editor_view: ViewHandle<CodeEditorView>,
}

impl ReferenceEntryWithUi {
    /// Updates the line content and refreshes the editor view.
    /// Trims whitespace from the start of the line and adjusts the column accordingly.
    pub fn update_line_content(&mut self, line: &str, ctx: &mut ViewContext<FindReferencesView>) {
        let trimmed = line.trim_start();
        let trim_offset = line.len() - trimmed.len();
        self.entry.line_content = Some(trimmed.to_string());
        self.entry.column = self.entry.column.saturating_sub(trim_offset);

        // Update the editor view with the loaded content
        let content = trimmed.to_string();
        let file_path = self.entry.file_path.clone();
        self.editor_view.update(ctx, |view, ctx| {
            view.set_language_with_local_path(&file_path, ctx);
            let state = InitialBufferState::plain_text(&content);
            view.reset(state, ctx);
        });
    }
}

impl FindReferencesView {
    /// Creates a new FindReferencesView with async line loading.
    /// Lines are loaded asynchronously from GlobalBufferModel (fast) or disk (slow).
    ///
    /// Converts raw ReferenceLocation to FlatReferenceEntry and loads line content.
    pub fn new(
        raw_references: Vec<ReferenceLocation>,
        workspace_root: Option<PathBuf>,
        request_offset: CharOffset,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        // Convert ReferenceLocation to ReferenceEntryWithUi (without line content - will load async)
        let mut references: Vec<ReferenceEntryWithUi> = raw_references
            .into_iter()
            .map(|r| {
                let file_path = r.file_path.clone();

                // Get just the file name (leaf) for display
                let file_name = file_path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| file_path.to_string_lossy().to_string());

                // Guard against empty file names from malformed LSP URIs
                let file_name = if file_name.is_empty() {
                    "[unknown]".to_string()
                } else {
                    file_name
                };

                // Get relative path from workspace root for tooltip
                let display_path = workspace_root
                    .as_ref()
                    .and_then(|root| file_path.strip_prefix(root).ok())
                    .map(|p| p.to_string_lossy().to_string())
                    .unwrap_or_else(|| file_path.to_string_lossy().to_string());

                let line_number = r.range.start.line + 1; // Convert 0-based to 1-based

                let entry = FlatReferenceEntry {
                    file_path,
                    file_name,
                    display_path,
                    line_number,
                    column: r.range.start.column,
                    line_content: None, // Will be loaded async
                };

                // Create editor view for this reference
                let editor_view = Self::create_editor_view_for_reference(&entry, ctx);

                ReferenceEntryWithUi {
                    entry,
                    entry_mouse_state: MouseStateHandle::default(),
                    file_name_mouse_state: MouseStateHandle::default(),
                    editor_view,
                }
            })
            .collect();

        // Batch references by file for efficient loading
        let mut file_to_refs: HashMap<PathBuf, Vec<usize>> = HashMap::new();
        for (idx, reference) in references.iter().enumerate() {
            if reference.entry.line_content.is_none() {
                file_to_refs
                    .entry(reference.entry.file_path.clone())
                    .or_default()
                    .push(idx);
            }
        }

        // Try to load lines from GlobalBufferModel first (fast, in-memory)
        let global_buffer = GlobalBufferModel::handle(ctx);
        for (file_path, ref_indices) in file_to_refs.iter() {
            // Collect all line numbers we need from this file (0-based)
            let line_numbers: Vec<usize> = ref_indices
                .iter()
                .map(|&idx| references[idx].entry.line_number - 1)
                .collect();

            if line_numbers.is_empty() {
                continue;
            }

            if let Some(lines) = global_buffer.update(ctx, |model, ctx| {
                model.get_lines_for_file(file_path, line_numbers.clone(), ctx)
            }) {
                // Build a map from line number to content for quick lookup
                let line_map: HashMap<usize, &String> =
                    lines.iter().map(|(ln, content)| (*ln, content)).collect();

                // Successfully loaded from buffer - update references
                for &ref_idx in ref_indices {
                    let reference = &mut references[ref_idx];
                    let line_num = reference.entry.line_number - 1; // 0-based
                    if let Some(line) = line_map.get(&line_num) {
                        reference.update_line_content(line, ctx);
                    }
                }
            }
        }

        // For any references still without line_content, spawn async file reads
        for (file_path, ref_indices) in file_to_refs {
            // Check if any refs for this file still need loading
            let needs_loading: Vec<usize> = ref_indices
                .iter()
                .copied()
                .filter(|&idx| references[idx].entry.line_content.is_none())
                .collect();

            if needs_loading.is_empty() {
                continue;
            }

            // Collect line numbers for this file (0-based)
            let line_numbers: Vec<usize> = needs_loading
                .iter()
                .map(|&idx| references[idx].entry.line_number - 1)
                .collect();

            let file_path_clone = file_path.clone();
            ctx.spawn(
                async move { FileModel::read_lines_async(&file_path_clone, line_numbers).await },
                move |me, result, ctx| {
                    if let Ok(lines) = result {
                        // Build a map from line number to content for quick lookup
                        let line_map: HashMap<usize, &String> =
                            lines.iter().map(|(ln, content)| (*ln, content)).collect();

                        for &ref_idx in &needs_loading {
                            if ref_idx >= me.references.len() {
                                continue;
                            }
                            let reference = &mut me.references[ref_idx];
                            let line_num = reference.entry.line_number - 1; // 0-based
                            if let Some(line) = line_map.get(&line_num) {
                                reference.update_line_content(line, ctx);
                            }
                        }
                        ctx.notify(); // Trigger re-render
                    }
                },
            );
        }

        Self {
            references,
            request_offset,
            scroll_state: ClippedScrollStateHandle::default(),
            back_mouse_state: MouseStateHandle::default(),
            selected_reference_index: 0,
        }
    }

    /// Returns the request offset for positioning the card.
    pub fn request_offset(&self) -> CharOffset {
        self.request_offset
    }

    /// Gets a reference by index.
    pub fn get_reference(&self, index: usize) -> Option<&FlatReferenceEntry> {
        self.references.get(index).map(|r| &r.entry)
    }

    /// Creates a read-only code editor view for a single reference line.
    fn create_editor_view_for_reference(
        reference: &FlatReferenceEntry,
        ctx: &mut ViewContext<Self>,
    ) -> ViewHandle<CodeEditorView> {
        let view = ctx.add_typed_action_view(|ctx| {
            let mut editor_view = CodeEditorView::new(
                None,
                None,
                CodeEditorRenderOptions::new(VerticalExpansionBehavior::InfiniteHeight),
                ctx,
            )
            .with_can_show_diff_ui(false)
            .with_show_line_numbers(false)
            .with_horizontal_scrollbar_appearance(
                warpui::elements::new_scrollable::ScrollableAppearance::new(
                    warpui::elements::ScrollbarWidth::None,
                    false,
                ),
            );

            editor_view.set_vertical_scrollbar_appearance(
                warpui::elements::new_scrollable::ScrollableAppearance::new(
                    warpui::elements::ScrollbarWidth::None,
                    false,
                ),
            );

            editor_view
        });

        // Initialize the editor with the reference line content (or empty if still loading)
        let content = reference.line_content.clone().unwrap_or_default();
        let file_path = reference.file_path.clone();
        view.update(ctx, |view, ctx| {
            view.set_show_current_line_highlights(false, ctx);
            view.set_interaction_state(InteractionState::Disabled, ctx);

            // Set up syntax highlighting based on file extension
            view.set_language_with_local_path(&file_path, ctx);

            // Reset with the reference line content
            let state = InitialBufferState::plain_text(&content);
            view.reset(state, ctx);
        });

        view
    }
}

impl Entity for FindReferencesView {
    type Event = FindReferencesViewEvent;
}

impl TypedActionView for FindReferencesView {
    type Action = FindReferencesViewAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            FindReferencesViewAction::GotoReference(index) => {
                ctx.emit(FindReferencesViewEvent::GotoReference(*index));
            }
            FindReferencesViewAction::Close => {
                ctx.emit(FindReferencesViewEvent::CloseRequested);
            }
            FindReferencesViewAction::ArrowUp => {
                if !self.references.is_empty() {
                    self.selected_reference_index =
                        (self.selected_reference_index + self.references.len() - 1)
                            % self.references.len();
                    ctx.notify();
                }
            }
            FindReferencesViewAction::ArrowDown => {
                if !self.references.is_empty() {
                    self.selected_reference_index =
                        (self.selected_reference_index + 1) % self.references.len();
                    ctx.notify();
                }
            }
            FindReferencesViewAction::SelectReference => {
                if self.selected_reference_index < self.references.len() {
                    ctx.emit(FindReferencesViewEvent::GotoReference(
                        self.selected_reference_index,
                    ));
                }
            }
        }
    }
}

impl View for FindReferencesView {
    fn ui_name() -> &'static str {
        "FindReferencesView"
    }

    fn keymap_context(&self, _app: &AppContext) -> warpui::keymap::Context {
        let mut context = Self::default_keymap_context();
        if !self.references.is_empty() {
            context.set.insert(HAS_REFERENCES);
        }
        context
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        if self.references.is_empty() {
            return warpui::elements::Empty::new().finish();
        }

        let appearance = Appearance::handle(app).as_ref(app);
        let theme = appearance.theme();

        // Header with back arrow and "Showing X references"
        let header = render_header(self.back_mouse_state.clone(), self.references.len(), app);

        // Content: flat list of reference entries
        let mut content_column =
            Flex::column().with_cross_axis_alignment(CrossAxisAlignment::Stretch);

        for (index, reference) in self.references.iter().enumerate() {
            let is_selected = index == self.selected_reference_index;
            let entry = render_reference_entry(
                &reference.entry,
                reference.entry_mouse_state.clone(),
                reference.file_name_mouse_state.clone(),
                &reference.editor_view,
                index,
                is_selected,
                app,
            );
            content_column.add_child(entry);
        }

        // Make content scrollable
        let scrollable_content = ClippedScrollable::vertical(
            self.scroll_state.clone(),
            content_column.finish(),
            ScrollbarWidth::None,
            theme.disabled_ui_text_color().into(),
            theme.active_ui_text_color().into(),
            Fill::None,
        )
        .finish();

        // Combine header and content
        let card_content = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_child(header)
            .with_child(Shrinkable::new(1., scrollable_content).finish())
            .finish();

        ConstrainedBox::new(
            Container::new(card_content)
                .with_background_color(internal_colors::neutral_1(theme))
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(8.)))
                .with_border(Border::all(1.).with_border_fill(internal_colors::neutral_4(theme)))
                .finish(),
        )
        .with_max_height(FIND_REFERENCES_CARD_MAX_HEIGHT)
        .finish()
    }
}

/// A single reference entry with file information for flat list display.
#[derive(Debug, Clone)]
pub struct FlatReferenceEntry {
    /// The absolute file path.
    pub file_path: PathBuf,
    /// The display file name (just the file name, not full path).
    pub file_name: String,
    /// The relative path from workspace root (for tooltip display).
    pub display_path: String,
    /// 1-based line number where the reference appears.
    pub line_number: usize,
    /// 0-based column number where the reference starts (adjusted for trimmed whitespace).
    pub column: usize,
    /// The content of the line containing the reference (trimmed).
    /// None indicates the line is still loading from disk.
    pub line_content: Option<String>,
}

/// Renders the card header with back arrow and "Showing X references" text.
fn render_header(
    back_mouse_state: MouseStateHandle,
    total_refs: usize,
    app: &AppContext,
) -> Box<dyn Element> {
    let appearance = Appearance::handle(app).as_ref(app);
    let theme = appearance.theme();

    // "Showing X references" title
    let title_text = if total_refs == 1 {
        "Showing 1 reference".to_string()
    } else {
        format!("Showing {total_refs} references")
    };

    let title = Align::new(
        Text::new_inline(
            title_text,
            appearance.ui_font_family(),
            appearance.ui_font_size(),
        )
        .with_color(theme.active_ui_text_color().into())
        .finish(),
    )
    .left()
    .finish();

    // Close (X) button on the right
    let icon_color = theme.sub_text_color(theme.background());
    let close_button = Hoverable::new(back_mouse_state, move |state| {
        let close_icon = ConstrainedBox::new(
            warpui::elements::Icon::new(WarpIcon::X.into(), icon_color).finish(),
        )
        .with_width(16.)
        .with_height(16.)
        .finish();

        let container = Container::new(close_icon)
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)))
            .with_uniform_padding(2.);

        if state.is_hovered() {
            container
                .with_background(internal_colors::fg_overlay_2(theme))
                .finish()
        } else {
            container.finish()
        }
    })
    .on_click(|ctx, _, _| {
        ctx.dispatch_typed_action(FindReferencesViewAction::Close);
    })
    .with_cursor(Cursor::PointingHand)
    .finish();

    Container::new(
        Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(Shrinkable::new(1., title).finish())
            .with_child(close_button)
            .finish(),
    )
    .with_horizontal_padding(12.)
    .with_vertical_padding(8.)
    .with_background(theme.background())
    .with_corner_radius(CornerRadius::with_top(Radius::Pixels(8.)))
    .finish()
}

/// Renders a single reference entry row with file info, line number, and code snippet.
fn render_reference_entry(
    entry: &FlatReferenceEntry,
    entry_mouse_state: MouseStateHandle,
    file_name_mouse_state: MouseStateHandle,
    editor_view: &ViewHandle<CodeEditorView>,
    index: usize,
    is_selected: bool,
    app: &AppContext,
) -> Box<dyn Element> {
    let appearance = Appearance::handle(app).as_ref(app);
    let theme = appearance.theme();
    let file_name = entry.file_name.clone();
    let display_path = entry.display_path.clone();
    let line_number = entry.line_number;
    let line_content = entry.line_content.clone();

    ConstrainedBox::new(
        Hoverable::new(entry_mouse_state, move |state| {
            // File icon - use language-specific icon
            let file_icon = ConstrainedBox::new(crate::search::files::icon::icon_from_file_path(
                &file_name,
                appearance,
                ItemHighlightState::Default,
            ))
            .with_width(16.)
            .with_height(16.)
            .finish();

            // File name with tooltip showing full path
            let file_name_with_tooltip =
                Hoverable::new(file_name_mouse_state.clone(), move |file_name_state| {
                    let file_name_text = Text::new_inline(
                        file_name.clone(),
                        appearance.monospace_font_family(),
                        appearance.monospace_font_size(),
                    )
                    .with_color(theme.sub_text_color(theme.background()).into())
                    .finish();

                    if file_name_state.is_hovered() {
                        let mut stack = Stack::new().with_child(file_name_text);
                        let tooltip = appearance
                            .ui_builder()
                            .tool_tip(display_path.clone())
                            .build()
                            .finish();
                        stack.add_positioned_overlay_child(
                            tooltip,
                            OffsetPositioning::offset_from_parent(
                                Vector2F::new(0., 4.),
                                ParentOffsetBounds::Unbounded,
                                ParentAnchor::BottomMiddle,
                                ChildAnchor::TopMiddle,
                            ),
                        );
                        stack.finish()
                    } else {
                        file_name_text
                    }
                })
                .finish();

            // Line number
            let line_num_text = Text::new_inline(
                line_number.to_string(),
                appearance.monospace_font_family(),
                appearance.monospace_font_size(),
            )
            .with_color(theme.sub_text_color(theme.background()).into())
            .finish();

            // Code content: use editor view for syntax highlighting, or show loading indicator
            // Cap the content element's height to prevent the Align from inflating
            // the Flex::row (Align::layout always takes constraint.max as its size).
            // Use the monospace line height so the editor matches the text children.
            let content_max_height =
                appearance.monospace_font_size() * appearance.line_height_ratio();
            let content_element: Box<dyn Element> = if line_content.is_some() {
                ConstrainedBox::new(
                    Align::new(ChildView::new(editor_view).finish())
                        .top_left()
                        .finish(),
                )
                .with_max_height(content_max_height)
                .finish()
            } else {
                // Show loading indicator when line_content is None
                Text::new_inline(
                    "Loading...",
                    appearance.monospace_font_family(),
                    appearance.monospace_font_size(),
                )
                .with_color(theme.sub_text_color(theme.background()).into())
                .finish()
            };

            // Layout: [file_icon] [file_name] [line_number] [code_snippet]
            let row = Flex::row()
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_child(Container::new(file_icon).with_margin_right(6.).finish())
                .with_child(
                    ConstrainedBox::new(file_name_with_tooltip)
                        .with_width(120.)
                        .finish(),
                )
                .with_child(
                    Container::new(
                        ConstrainedBox::new(Align::new(line_num_text).right().finish())
                            .with_width(40.)
                            .finish(),
                    )
                    .with_padding_right(4.)
                    .finish(),
                )
                .with_child(Shrinkable::new(1., content_element).finish())
                .finish();

            let container = Container::new(row)
                .with_horizontal_padding(8.)
                .with_vertical_padding(6.);

            if state.is_hovered() || is_selected {
                container
                    .with_background(internal_colors::fg_overlay_2(theme))
                    .finish()
            } else {
                container.finish()
            }
        })
        .on_click(move |ctx, _, _| {
            ctx.dispatch_typed_action(FindReferencesViewAction::GotoReference(index));
        })
        .with_cursor(Cursor::PointingHand)
        .finish(),
    )
    .with_min_height(28.)
    .finish()
}
