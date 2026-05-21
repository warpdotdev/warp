#![cfg_attr(target_family = "wasm", allow(dead_code, unused_imports))]
// Adding this file level gate as some of the code around editability is not used in WASM yet.

use crate::appearance::Appearance;
use crate::editor::{
    EditorView, Event as EditorEvent, InteractionState, PropagateAndNoOpNavigationKeys,
    SingleLineEditorOptions, TextOptions,
};
use crate::send_telemetry_from_ctx;
use crate::server::telemetry::{FindOption, TelemetryEvent};
use crate::themes::theme::Fill;
use crate::ui_components::{blended_colors, icons::Icon};
use crate::view_components::action_button::{ActionButton, DisabledSecondaryTheme, SecondaryTheme};
use crate::view_components::find::FindDirection;
use crate::{features::FeatureFlag, settings::AppEditorSettings};
use pathfinder_color::ColorU;
use warp_editor::editor::NavigationKey;
use warp_editor::search::{SearchEvent, Searcher};
use warpui::elements::MainAxisAlignment;
use warpui::elements::{ChildAnchor, OffsetPositioning, Radius, SavePosition, Shrinkable};
use warpui::keymap::EditableBinding;
use warpui::ui_components::components::UiComponent;
pub use warpui::{
    accessibility::{AccessibilityContent, WarpA11yRole},
    elements::{ParentElement as _, Stack},
    geometry::vector::vec2f,
    AppContext,
};
use warpui::{
    elements::{
        Align, Border, Clipped, ConstrainedBox, Container, CornerRadius, CrossAxisAlignment,
        DropShadow, Element, Flex, Hoverable, MouseStateHandle, ParentAnchor, ParentOffsetBounds,
        Rect, Text,
    },
    Entity, SingletonEntity, TypedActionView, View,
};
use warpui::{presenter::ChildView, ViewContext, ViewHandle};
use warpui::{FocusContext, ModelHandle};

pub const FIND_BAR_WIDTH: f32 = 500.;
const ICON_PADDING: f32 = 4.;
const HORIZONTAL_ICON_SPACING: f32 = 4.;
const ICON_CONTAINER_CORNER_RADIUS: f32 = 4.;
pub const FIND_BAR_PADDING: f32 = 4.;
const FIND_EDITOR_PADDING: f32 = 6.;
pub const FIND_EDITOR_BORDER_RADIUS: f32 = 6.;
const FIND_EDITOR_BORDER_WIDTH: f32 = 1.;
const FIND_EDITOR_FONT_SIZE: f32 = 12.;
const FIND_EDITOR_ROW_SPACING: f32 = 4.;

pub const REGEX_TOGGLE_TOOLTIP: &str = "Regex toggle";
pub const CASE_SENSITIVE_TOOLTIP: &str = "Case sensitive search";
pub const PRESERVE_CASE_TOOLTIP: &str = "Preserve case";
pub const FIND_PLACEHOLDER_TEXT: &str = "Find";
pub const REPLACE_PLACEHOLDER_TEXT: &str = "Replace";

#[derive(Default)]
struct ButtonMouseStates {
    match_up: MouseStateHandle,
    match_down: MouseStateHandle,
    close: MouseStateHandle,
    toggle_case_sensitivity: MouseStateHandle,
    toggle_regex_search: MouseStateHandle,
    toggle_replace_open: MouseStateHandle,
    toggle_preserve_case: MouseStateHandle,
}

#[derive(Debug)]
pub enum Event {
    CloseFindBar,
    Update { query: Option<String> },
    NextMatch { direction: FindDirection },
    SelectAll,
    ReplaceSelected,
    ReplaceAll,
    VimEnterAndFocusEditor,
}

pub struct CodeEditorFind {
    find_editor: ViewHandle<EditorView>,
    replace_editor: ViewHandle<EditorView>,
    searcher: ModelHandle<Searcher>,
    button_mouse_states: ButtonMouseStates,
    preserve_case_enabled: bool,
    is_open: bool,
    is_replace_open: bool,
    select_all_button: ViewHandle<ActionButton>,
    replace_all_button: ViewHandle<ActionButton>,
}

#[derive(Copy, Clone, Debug)]
pub enum FindAction {
    Up,
    Down,
    Close,
    ToggleCaseSensitivity,
    ToggleRegexSearch,
    CmdG,
    CmdShiftG,
    SelectAll,
    ToggleReplaceOpen,
    ReplaceAll,
    TogglePreserveCase,
}

pub fn init(app: &mut AppContext) {
    use warpui::keymap::macros::*;
    app.register_editable_bindings([
        EditableBinding::new(
            "find:find_next_occurrence",
            "Find the next occurrence of your search query",
            FindAction::CmdG,
        )
        .with_context_predicate(id!("CodeEditorFind"))
        // Both Intellij and VSCode use f3/shift-f3 to navigate find occurrences on windows / linux.
        // See https://www.jetbrains.com/help/idea/reference-keymap-win-default.html#find_everything.
        .with_mac_key_binding("cmd-g")
        .with_linux_or_windows_key_binding("f3"),
        EditableBinding::new(
            "find:find_prev_occurrence",
            "Find the previous occurrence of your search query",
            FindAction::CmdShiftG,
        )
        .with_context_predicate(id!("CodeEditorFind"))
        .with_mac_key_binding("cmd-shift-G")
        .with_linux_or_windows_key_binding("shift-f3"),
    ])
}

impl CodeEditorFind {
    pub fn new(searcher: ModelHandle<Searcher>, ctx: &mut ViewContext<Self>) -> Self {
        let find_editor = ctx.add_typed_action_view(|ctx| {
            let appearance = Appearance::as_ref(ctx);
            let mut editor = EditorView::single_line(
                SingleLineEditorOptions {
                    text: TextOptions::ui_text(Some(FIND_EDITOR_FONT_SIZE), appearance),
                    select_all_on_focus: true,
                    clear_selections_on_blur: true,
                    propagate_and_no_op_vertical_navigation_keys:
                        PropagateAndNoOpNavigationKeys::Always,
                    ..Default::default()
                },
                ctx,
            );
            editor.set_placeholder_text(FIND_PLACEHOLDER_TEXT, ctx);
            editor
        });

        let replace_editor = ctx.add_typed_action_view(|ctx| {
            let appearance = Appearance::as_ref(ctx);
            let mut replace_editor = EditorView::single_line(
                SingleLineEditorOptions {
                    text: TextOptions::ui_text(Some(FIND_EDITOR_FONT_SIZE), appearance),
                    select_all_on_focus: true,
                    clear_selections_on_blur: true,
                    propagate_and_no_op_vertical_navigation_keys:
                        PropagateAndNoOpNavigationKeys::Always,
                    ..Default::default()
                },
                ctx,
            );
            replace_editor.set_placeholder_text(REPLACE_PLACEHOLDER_TEXT, ctx);
            replace_editor
        });

        // Get the line height from the find editor's font metrics
        let line_height = find_editor
            .as_ref(ctx)
            .line_height(ctx.font_cache(), Appearance::as_ref(ctx));
        // Total editor height is:
        // - line_height: The height of a single line of text
        // - 2 * FIND_EDITOR_PADDING: Padding above and below the text (6px * 2 = 12px)
        // - 5px: Additional spacing to account for button border width
        let editor_height = line_height + (2. * FIND_EDITOR_PADDING) + 5.;

        let select_all_button = ctx.add_typed_action_view(|ctx| {
            ActionButton::new(crate::i18n::tr_static(ctx, "Select all"), SecondaryTheme)
                .on_click(|ctx| {
                    ctx.dispatch_typed_action(FindAction::SelectAll);
                })
                .with_width(72.)
                .with_height(editor_height)
                .with_disabled_theme(DisabledSecondaryTheme)
        });

        let replace_all_button = ctx.add_typed_action_view(|ctx| {
            let mut button =
                ActionButton::new(crate::i18n::tr_static(ctx, "Replace all"), SecondaryTheme)
                    .on_click(|ctx| {
                        ctx.dispatch_typed_action(FindAction::ReplaceAll);
                    })
                    .with_width(72.)
                    .with_height(editor_height)
                    .with_disabled_theme(DisabledSecondaryTheme);
            button.set_disabled(true, ctx);
            button
        });

        ctx.subscribe_to_view(&find_editor, move |me, _, event, ctx| {
            me.handle_find_editor_event(event, ctx);
        });

        ctx.subscribe_to_view(&replace_editor, move |me, _, event, ctx| {
            me.handle_replace_editor_event(event, ctx);
        });

        ctx.subscribe_to_model(&searcher, |me, _, event, ctx| {
            // Handle search result updates. The searcher emits three events:
            // - Updated: Search results have changed, handled here to update the select all button
            // - InvalidQuery: The search query is invalid (e.g. malformed regex), TODO: show error tooltip
            // - SelectedResultChanged: The selected match changed, handled by the editor view
            match event {
                SearchEvent::Updated => {
                    let has_matches = me.searcher.as_ref(ctx).match_count() > 0;
                    me.select_all_button.update(ctx, |button, ctx| {
                        button.set_disabled(!has_matches, ctx);
                    });
                    ctx.notify();
                }
                SearchEvent::SelectedResultChanged => {
                    // The selected match index changed (e.g., via Vim n/N), needed to re-render index label.
                    ctx.notify();
                }
                SearchEvent::InvalidQuery => {}
            }
        });

        Self {
            find_editor,
            replace_editor,
            searcher,
            button_mouse_states: Default::default(),
            preserve_case_enabled: false,
            is_open: false,
            is_replace_open: false,
            select_all_button,
            replace_all_button,
        }
    }

    pub fn set_open(&mut self, is_open: bool) {
        self.is_open = is_open;
        // Replace menu doesn't persist open state
        if !is_open {
            self.is_replace_open = false;
        }
    }

    pub fn is_open(&self) -> bool {
        self.is_open
    }

    pub fn is_preserve_case_enabled(&self) -> bool {
        self.preserve_case_enabled
    }

    pub fn replace_query(&self, ctx: &mut ViewContext<Self>) -> String {
        self.replace_editor.as_ref(ctx).buffer_text(ctx)
    }

    pub fn set_find_query(&self, ctx: &mut ViewContext<Self>, query: &str) {
        self.find_editor.update(ctx, |editor, ctx| {
            editor.set_buffer_text(query, ctx);
        });
    }

    /// Returns true if the find input is currently editable.
    pub fn is_find_input_editable(&self, app: &AppContext) -> bool {
        self.find_editor.as_ref(app).can_edit(app)
    }

    /// Enable or disable the find input editor's interactivity.
    pub fn set_find_input_editable(&self, ctx: &mut ViewContext<Self>, is_editable: bool) {
        self.find_editor.update(ctx, |editor, ctx| {
            let state = if is_editable {
                InteractionState::Editable
            } else {
                InteractionState::Disabled
            };
            editor.set_interaction_state(state, ctx);
        });
    }
    fn handle_find_editor_event(&mut self, event: &EditorEvent, ctx: &mut ViewContext<Self>) {
        match event {
            EditorEvent::Edited(_) => {
                let query = self.find_editor.as_ref(ctx).buffer_text(ctx);
                ctx.emit(Event::Update {
                    // If the query is empty, don't search for an empty string - set the query to
                    // `None`.
                    query: (!query.is_empty()).then_some(query),
                });

                self.update_replace_button_state(ctx);
                self.emit_result_a11y_content(ctx);
                ctx.notify();
            }
            EditorEvent::Enter => {
                let vim_enabled = FeatureFlag::VimCodeEditor.is_enabled()
                    && AppEditorSettings::as_ref(ctx).vim_mode_enabled();

                if !vim_enabled {
                    self.focus_next_match(FindDirection::Down, ctx);
                } else {
                    // Vim: treat "enter" as ending the search query entry and shift focus back to the editor
                    self.find_editor.update(ctx, |editor, ctx| {
                        editor.clear_selections(ctx);
                        editor.set_interaction_state(InteractionState::Disabled, ctx);
                    });
                    ctx.emit(Event::VimEnterAndFocusEditor);
                }
            }
            EditorEvent::ShiftEnter | EditorEvent::AltEnter => {
                let vim_enabled = FeatureFlag::VimCodeEditor.is_enabled()
                    && AppEditorSettings::as_ref(ctx).vim_mode_enabled();
                if !vim_enabled {
                    self.focus_next_match(FindDirection::Up, ctx);
                }
            }
            EditorEvent::Escape => {
                self.close_find_bar(ctx);
            }
            EditorEvent::Navigate(NavigationKey::Tab) => {
                // If replace editor is currently open and the user presses 'tab', focus on the find editor
                if self.is_replace_open {
                    ctx.focus(&self.replace_editor);
                }
            }
            _ => {}
        }
    }

    fn handle_replace_editor_event(&mut self, event: &EditorEvent, ctx: &mut ViewContext<Self>) {
        match event {
            EditorEvent::Edited(_) => {
                self.update_replace_button_state(ctx);
            }
            EditorEvent::Enter => {
                ctx.emit(Event::ReplaceSelected);
            }
            EditorEvent::Escape => {
                self.close_find_bar(ctx);
            }
            // If the user is focused on the replace editor and presses 'tab', focus should shift back to the find editor
            EditorEvent::Navigate(NavigationKey::Tab) => ctx.focus(&self.find_editor),
            _ => (),
        }
    }

    fn update_replace_button_state(&mut self, ctx: &mut ViewContext<Self>) {
        // Enable the replace all button only if there is replacement text and at least one match found
        let (has_replace_query, has_match) = (
            !self.replace_query(ctx).is_empty(),
            self.searcher.as_ref(ctx).match_count() > 0,
        );
        let should_enable_replace = has_replace_query && has_match;
        self.replace_all_button.update(ctx, |button, ctx| {
            button.set_disabled(!should_enable_replace, ctx);
        });
    }

    fn focus_next_match(&mut self, direction: FindDirection, ctx: &mut ViewContext<Self>) {
        // All of the actual update logic for the selected match goes through the event codepath
        // but for some reason the logic for updating the match index happens below.
        ctx.emit(Event::NextMatch { direction });

        self.emit_result_a11y_content(ctx);
        ctx.notify();
    }

    /// Emits the a11y announcement informing about the current match/result.
    /// Note that it's done outside of the regular action_accessibility_contents flow,
    /// as `focus_next_match` may have multiple entrypoints (that are not Action).
    pub fn emit_result_a11y_content(&mut self, ctx: &mut ViewContext<Self>) {
        let content = if let Some(match_index) = self.searcher.as_ref(ctx).selected_match() {
            AccessibilityContent::new(
                format!(
                    "Result {} of {}.",
                    match_index + 1,
                    self.searcher.as_ref(ctx).match_count()
                ),
                "Use enter and shift-enter to navigate between matches. Escape to quit.",
                WarpA11yRole::UserAction,
            )
        } else {
            AccessibilityContent::new_without_help("No results.", WarpA11yRole::UserAction)
        };
        ctx.emit_a11y_content(content);
    }

    /// Emits the a11y announcement informing about replace operation results.
    /// Called when Enter is pressed in the replace field to provide feedback on the operation.
    pub fn emit_replace_a11y_content(&mut self, ctx: &mut ViewContext<Self>) {
        let content = if let Some(match_index) = self.searcher.as_ref(ctx).selected_match() {
            let remaining_matches = self.searcher.as_ref(ctx).match_count();
            AccessibilityContent::new(
                format!(
                    "Successfully replaced match. Selected match is {match_index} of {remaining_matches}"
                ),
                "Continue pressing Enter to replace more matches, or use up/down arrows to navigate.",
                WarpA11yRole::UserAction,
            )
        } else {
            AccessibilityContent::new_without_help(
                "Successfully replaced the last match.",
                WarpA11yRole::UserAction,
            )
        };
        ctx.emit_a11y_content(content);
    }

    fn close_find_bar(&mut self, ctx: &mut ViewContext<Self>) {
        ctx.emit(Event::CloseFindBar);
    }

    fn toggle_case_sensitivity(&mut self, ctx: &mut ViewContext<Self>) {
        let new_case_sensitive = !self.searcher.as_ref(ctx).is_case_sensitive();
        self.searcher.update(ctx, |searcher, ctx| {
            searcher.set_case_sensitive(new_case_sensitive, ctx);
        });
        send_telemetry_from_ctx!(
            TelemetryEvent::ToggleFindOption {
                option: FindOption::CaseSensitive,
                enabled: new_case_sensitive
            },
            ctx
        );
    }

    fn toggle_regex_search(&mut self, ctx: &mut ViewContext<Self>) {
        let new_regex_enabled = !self.searcher.as_ref(ctx).is_regex();
        self.searcher.update(ctx, |searcher, ctx| {
            searcher.set_regex(new_regex_enabled, ctx);
        });
        send_telemetry_from_ctx!(
            TelemetryEvent::ToggleFindOption {
                option: FindOption::Regex,
                enabled: new_regex_enabled
            },
            ctx
        );
    }

    fn render_match_index(&self, appearance: &Appearance, app: &AppContext) -> Box<dyn Element> {
        // If there is some match index, we add 1 to it since the UI is 1-indexed
        // (i.e. first match starts at index 1 out of the total number of matches).
        let index = match self.searcher.as_ref(app).selected_match() {
            None => 0,
            Some(idx) => idx + 1,
        };
        let label = format!(
            "{}/{}",
            if index > 0 {
                index.to_string()
            } else {
                "?".to_string()
            },
            self.searcher.as_ref(app).match_count()
        );
        Text::new_inline(label, appearance.ui_font_family(), FIND_EDITOR_FONT_SIZE)
            .with_color(blended_colors::text_sub(
                appearance.theme(),
                appearance.theme().surface_1(),
            ))
            .finish()
    }

    #[allow(clippy::too_many_arguments)]
    pub fn render_hoverable_icon_in_editor(
        &self,
        appearance: &Appearance,
        icon: Icon,
        is_selected: bool,
        mouse_state_handle: MouseStateHandle,
        on_click_action: FindAction,
        size: f32,
        tooltip_text: Option<&str>,
        right_margin: f32,
    ) -> Box<dyn Element> {
        Hoverable::new(mouse_state_handle, |state| {
            let (border, background) = if is_selected {
                (
                    Border::all(1.).with_border_fill(appearance.theme().accent()),
                    appearance.theme().find_bar_button_selection_color(),
                )
            } else if state.is_hovered() {
                let hover_color = appearance.theme().foreground_button_color();
                (Border::all(1.).with_border_fill(hover_color), hover_color)
            } else {
                let transparent = Fill::Solid(ColorU::transparent_black());
                (Border::all(1.).with_border_fill(transparent), transparent)
            };
            let icon = Container::new(
                ConstrainedBox::new(
                    icon.to_warpui_icon(appearance.theme().active_ui_text_color())
                        .finish(),
                )
                .with_height(size)
                .with_width(size)
                .finish(),
            )
            .with_uniform_padding(ICON_PADDING)
            .with_vertical_margin(HORIZONTAL_ICON_SPACING)
            .with_margin_right(right_margin)
            .with_border(border)
            .with_background(background)
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(
                ICON_CONTAINER_CORNER_RADIUS,
            )))
            .finish();

            let mut stack = Stack::new().with_child(icon);
            if let (Some(tooltip_text), true) = (tooltip_text, state.is_hovered()) {
                let tooltip = appearance
                    .ui_builder()
                    .tool_tip(tooltip_text.to_string())
                    .build()
                    .finish();

                stack.add_positioned_overlay_child(
                    tooltip,
                    OffsetPositioning::offset_from_parent(
                        vec2f(0., -5.),
                        ParentOffsetBounds::Unbounded,
                        ParentAnchor::TopMiddle,
                        ChildAnchor::BottomMiddle,
                    ),
                );
            }
            stack.finish()
        })
        .on_click(move |ctx, _app, _| ctx.dispatch_typed_action(on_click_action))
        .finish()
    }

    fn render_next_match_button(
        &self,
        appearance: &Appearance,
        hovered: bool,
        direction: FindDirection,
        height: f32,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let background_color = if hovered && self.searcher.as_ref(app).match_count() > 0 {
            appearance.theme().foreground_button_color()
        } else {
            Fill::Solid(ColorU::transparent_black())
        };
        let match_icon = match direction {
            FindDirection::Down => Icon::ArrowDown,
            FindDirection::Up => Icon::ArrowUp,
        };
        let icon_color = if self.searcher.as_ref(app).match_count() == 0 {
            appearance.theme().nonactive_ui_text_color()
        } else {
            appearance.theme().active_ui_text_color()
        };
        Container::new(
            ConstrainedBox::new(match_icon.to_warpui_icon(icon_color).finish())
                .with_height(height)
                .with_width(height)
                .finish(),
        )
        .with_background(background_color)
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(
            ICON_CONTAINER_CORNER_RADIUS,
        )))
        .with_uniform_padding(ICON_PADDING)
        .finish()
    }

    fn render_search_button(
        &self,
        appearance: &Appearance,
        height: f32,
        app: &AppContext,
    ) -> Box<dyn Element> {
        // Always use transparent background (default state)
        let background_color = Fill::Solid(ColorU::transparent_black());
        let icon_color = if self.searcher.as_ref(app).match_count() == 0 {
            appearance.theme().nonactive_ui_text_color()
        } else {
            appearance.theme().active_ui_text_color()
        };
        Container::new(
            ConstrainedBox::new(Icon::Search.to_warpui_icon(icon_color).finish())
                .with_height(height)
                .with_width(height)
                .finish(),
        )
        .with_background(background_color)
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(
            ICON_CONTAINER_CORNER_RADIUS,
        )))
        .with_uniform_padding(ICON_PADDING)
        .finish()
    }

    fn render_hoverable_icon_button(
        &self,
        appearance: &Appearance,
        hovered: bool,
        icon: Icon,
        height: f32,
    ) -> Box<dyn Element> {
        let background_color = if hovered {
            appearance.theme().foreground_button_color()
        } else {
            Fill::Solid(ColorU::transparent_black())
        };
        Container::new(
            ConstrainedBox::new(
                icon.to_warpui_icon(appearance.theme().active_ui_text_color())
                    .finish(),
            )
            .with_height(height)
            .with_width(height)
            .finish(),
        )
        .with_background(background_color)
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(
            ICON_CONTAINER_CORNER_RADIUS,
        )))
        .with_uniform_padding(ICON_PADDING)
        .finish()
    }

    fn render_option_row(
        &self,
        appearance: &Appearance,
        editor_height: f32,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let mut option_row = Flex::row().with_cross_axis_alignment(CrossAxisAlignment::Center);
        option_row.add_child(
            // down button
            Container::new(
                Hoverable::new(self.button_mouse_states.match_down.clone(), |state| {
                    self.render_next_match_button(
                        appearance,
                        state.is_hovered(),
                        FindDirection::Down,
                        editor_height,
                        app,
                    )
                })
                .on_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(FindAction::Down);
                })
                .finish(),
            )
            .with_margin_left(HORIZONTAL_ICON_SPACING)
            .finish(),
        );
        option_row.add_child(
            // up button
            Container::new(
                Hoverable::new(self.button_mouse_states.match_up.clone(), |state| {
                    self.render_next_match_button(
                        appearance,
                        state.is_hovered(),
                        FindDirection::Up,
                        editor_height,
                        app,
                    )
                })
                .on_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(FindAction::Up);
                })
                .finish(),
            )
            .with_margin_right(HORIZONTAL_ICON_SPACING)
            .finish(),
        );
        option_row.add_child(
            // vertical divider
            Container::new(
                ConstrainedBox::new(
                    Rect::new()
                        .with_background(appearance.theme().surface_3())
                        .finish(),
                )
                .with_width(1.)
                .with_height(editor_height)
                .finish(),
            )
            .with_margin_right(HORIZONTAL_ICON_SPACING)
            .finish(),
        );
        option_row.add_child(
            // search icon
            Container::new(self.render_search_button(appearance, editor_height, app))
                .with_margin_right(HORIZONTAL_ICON_SPACING)
                .finish(),
        );
        option_row.add_child(
            // match index
            Shrinkable::new(
                1.,
                Container::new(
                    Align::new(
                        ConstrainedBox::new(self.render_match_index(appearance, app))
                            .with_height(editor_height)
                            .finish(),
                    )
                    .right()
                    .finish(),
                )
                .with_margin_right(HORIZONTAL_ICON_SPACING)
                .finish(),
            )
            .finish(),
        );
        option_row.add_child(
            // close button
            Container::new(
                Hoverable::new(self.button_mouse_states.close.clone(), |state| {
                    self.render_hoverable_icon_button(
                        appearance,
                        state.is_hovered(),
                        Icon::X,
                        editor_height,
                    )
                })
                .on_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(FindAction::Close);
                })
                .finish(),
            )
            .finish(),
        );
        option_row.finish()
    }

    fn render_find_row(
        &self,
        appearance: &Appearance,
        editor_height: f32,
        app: &AppContext,
    ) -> Box<dyn Element> {
        // Create the query editor row with find editor and icons
        let regex_icon = self.render_hoverable_icon_in_editor(
            appearance,
            Icon::Regex,
            self.searcher.as_ref(app).is_regex(),
            self.button_mouse_states.toggle_regex_search.clone(),
            FindAction::ToggleRegexSearch,
            editor_height,
            Some(REGEX_TOGGLE_TOOLTIP),
            ICON_PADDING,
        );
        let case_sensitive_icon = Container::new(
            SavePosition::new(
                self.render_hoverable_icon_in_editor(
                    appearance,
                    Icon::CaseSensitivity,
                    self.searcher.as_ref(app).is_case_sensitive(),
                    self.button_mouse_states.toggle_case_sensitivity.clone(),
                    FindAction::ToggleCaseSensitivity,
                    editor_height,
                    Some(CASE_SENSITIVE_TOOLTIP),
                    ICON_PADDING,
                ),
                "case_sensitive_button",
            )
            .finish(),
        )
        .finish();

        let mut query_editor_row = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(
                Shrinkable::new(
                    1.,
                    ConstrainedBox::new(
                        Clipped::new(ChildView::new(&self.find_editor).finish()).finish(),
                    )
                    .with_height(editor_height)
                    .finish(),
                )
                .finish(),
            );
        query_editor_row.add_child(regex_icon);
        query_editor_row.add_child(case_sensitive_icon);

        // Create the find row with replace toggle, query editor, and select all button
        let mut find_row = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(
                // replace toggle
                Container::new(
                    Hoverable::new(
                        self.button_mouse_states.toggle_replace_open.clone(),
                        |state| {
                            self.render_hoverable_icon_button(
                                appearance,
                                state.is_hovered(),
                                if self.is_replace_open {
                                    Icon::ListOpen
                                } else {
                                    Icon::ListCollapsed
                                },
                                editor_height,
                            )
                        },
                    )
                    .on_click(move |ctx, _, _| {
                        ctx.dispatch_typed_action(FindAction::ToggleReplaceOpen);
                    })
                    .finish(),
                )
                .with_margin_left(HORIZONTAL_ICON_SPACING)
                .with_margin_right(HORIZONTAL_ICON_SPACING)
                .finish(),
            );
        find_row.add_child(
            Shrinkable::new(
                1.,
                Container::new(query_editor_row.finish())
                    .with_padding_right(4.)
                    .with_padding_left(8.)
                    .with_background(appearance.theme().surface_1())
                    .with_border(
                        Border::all(FIND_EDITOR_BORDER_WIDTH)
                            .with_border_fill(appearance.theme().surface_3()),
                    )
                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(
                        FIND_EDITOR_BORDER_RADIUS,
                    )))
                    .with_margin_right(2. * HORIZONTAL_ICON_SPACING)
                    .finish(),
            )
            .finish(),
        );
        find_row
            .add_child(Container::new(ChildView::new(&self.select_all_button).finish()).finish());
        find_row.finish()
    }

    fn render_replace_row(&self, appearance: &Appearance, editor_height: f32) -> Box<dyn Element> {
        // Create the replace editor row with preserve case toggle
        let mut replace_editor_row = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(
                Shrinkable::new(
                    1.,
                    ConstrainedBox::new(
                        Clipped::new(ChildView::new(&self.replace_editor).finish()).finish(),
                    )
                    .with_height(editor_height)
                    .finish(),
                )
                .finish(),
            );
        let preserve_case_icon = self.render_hoverable_icon_in_editor(
            appearance,
            Icon::PreserveCase,
            self.preserve_case_enabled,
            self.button_mouse_states.toggle_preserve_case.clone(),
            FindAction::TogglePreserveCase,
            editor_height,
            Some(PRESERVE_CASE_TOOLTIP),
            ICON_PADDING,
        );
        replace_editor_row.add_child(preserve_case_icon);

        // Create the replace row with replacement query editor and replace all button
        let mut replace_row = Flex::row().with_cross_axis_alignment(CrossAxisAlignment::Center);
        replace_row.add_child(
            Shrinkable::new(
                1.,
                Container::new(replace_editor_row.finish())
                    .with_padding_right(4.)
                    .with_padding_left(8.)
                    .with_background(appearance.theme().surface_1())
                    .with_border(
                        Border::all(FIND_EDITOR_BORDER_WIDTH)
                            .with_border_fill(appearance.theme().surface_3()),
                    )
                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(
                        FIND_EDITOR_BORDER_RADIUS,
                    )))
                    .with_margin_left(2. * HORIZONTAL_ICON_SPACING + editor_height + 8.)
                    .with_margin_right(2. * HORIZONTAL_ICON_SPACING)
                    .finish(),
            )
            .finish(),
        );
        replace_row
            .add_child(Container::new(ChildView::new(&self.replace_all_button).finish()).finish());
        replace_row.finish()
    }
}

impl Entity for CodeEditorFind {
    type Event = Event;
}

impl TypedActionView for CodeEditorFind {
    type Action = FindAction;

    fn handle_action(&mut self, action: &FindAction, ctx: &mut ViewContext<Self>) {
        match action {
            FindAction::Down | FindAction::CmdG => self.focus_next_match(FindDirection::Down, ctx),
            FindAction::Up | FindAction::CmdShiftG => self.focus_next_match(FindDirection::Up, ctx),
            FindAction::Close => self.close_find_bar(ctx),
            FindAction::ToggleCaseSensitivity => self.toggle_case_sensitivity(ctx),
            FindAction::ToggleRegexSearch => self.toggle_regex_search(ctx),
            FindAction::SelectAll => ctx.emit(Event::SelectAll),
            FindAction::ReplaceAll => ctx.emit(Event::ReplaceAll),
            FindAction::ToggleReplaceOpen => {
                self.is_replace_open = !self.is_replace_open;
                ctx.notify();
            }
            FindAction::TogglePreserveCase => {
                self.preserve_case_enabled = !self.preserve_case_enabled;
                ctx.notify();
            }
        }
    }
}

impl View for CodeEditorFind {
    fn ui_name() -> &'static str {
        "CodeEditorFind"
    }

    fn accessibility_contents(&self, app: &AppContext) -> Option<AccessibilityContent> {
        let match_count = self.searcher.as_ref(app).match_count();
        let selected_match = self.searcher.as_ref(app).selected_match();
        let description = match (match_count, selected_match) {
            (0, _) | (_, None) => "Find bar for searching text in the editor.".to_string(),
            (count, Some(current)) => format!(
                "Find bar with {} matches found. Currently on match {} of {}.",
                count,
                current + 1,
                count
            ),
        };

        let is_replace_focused = self.is_replace_open && self.replace_editor.is_focused(app);
        let help_text = if is_replace_focused {
            "Replace field focused. Type replacement text, press Enter to replace current match, Tab to return to find field. Use up/down arrows to navigate matches, Escape to close."
        } else {
            "Find field focused. Type to search text. Use Enter and Shift-Enter or up/down arrows to navigate between matches. Press Escape to close find bar."
        };

        Some(AccessibilityContent::new(
            description,
            help_text,
            WarpA11yRole::TextareaRole,
        ))
    }

    fn on_focus(&mut self, focus_ctx: &FocusContext, ctx: &mut ViewContext<Self>) {
        self.searcher.update(ctx, |searcher, _ctx| {
            searcher.set_auto_select(true);
        });
        if focus_ctx.is_self_focused() {
            self.find_editor.update(ctx, |editor, ctx| {
                editor.set_interaction_state(InteractionState::Editable, ctx);
                editor.select_all(ctx);
            });
            ctx.focus(&self.find_editor);
            ctx.notify();
        }
    }

    fn on_blur(&mut self, _blur_ctx: &warpui::BlurContext, ctx: &mut ViewContext<Self>) {
        // Check if the currently focused view is one of our child components
        let focused_view_id = ctx.focused_view_id(ctx.window_id());
        let is_focus_within_find_bar = [
            self.find_editor.id(),
            self.replace_editor.id(),
            self.select_all_button.id(),
            self.replace_all_button.id(),
        ]
        .iter()
        .any(|entity_id| focused_view_id == Some(*entity_id));

        // On blur, should clear selected result and disable auto-select.
        // Exception: in vim mode, Enter moves focus back to the editor while the find bar stays
        // open as a status indicator; we want to preserve the selected match so it stays
        // highlighted and n/N can cycle from it.
        let vim_enabled = FeatureFlag::VimCodeEditor.is_enabled()
            && AppEditorSettings::as_ref(ctx).vim_mode_enabled();
        let keep_selection_for_vim = vim_enabled && self.is_open;

        if !is_focus_within_find_bar && !keep_selection_for_vim {
            self.searcher.update(ctx, |searcher, ctx| {
                searcher.clear_selected_result(ctx);
                searcher.set_auto_select(false);
            });
            ctx.notify();
        }
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let editor_height = self
            .find_editor
            .as_ref(app)
            .line_height(app.font_cache(), appearance);

        let option_row = self.render_option_row(appearance, editor_height, app);
        let find_row = self.render_find_row(appearance, editor_height, app);
        let replace_row = self.render_replace_row(appearance, editor_height);

        let mut find_rows = Flex::column()
            .with_main_axis_alignment(MainAxisAlignment::SpaceEvenly)
            .with_child(option_row)
            .with_child(find_row);

        if self.is_replace_open {
            find_rows.add_child(
                Container::new(replace_row)
                    .with_margin_top(FIND_EDITOR_ROW_SPACING)
                    .finish(),
            );
        }

        let find_bar = Container::new(
            ConstrainedBox::new(
                Container::new(find_rows.finish())
                    .with_background(appearance.theme().surface_2())
                    .finish(),
            )
            .with_height(editor_height + (2. * FIND_EDITOR_PADDING) + (2. * FIND_BAR_PADDING))
            .with_width(FIND_BAR_WIDTH)
            .finish(),
        )
        .with_uniform_padding(FIND_BAR_PADDING)
        .with_background(appearance.theme().surface_2())
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(
            FIND_EDITOR_BORDER_RADIUS,
        )))
        .with_drop_shadow(DropShadow::default())
        .finish();

        Align::new(
            Container::new(find_bar)
                .with_padding_top(10.)
                .with_padding_right(20.)
                .finish(),
        )
        .top_right()
        .finish()
    }
}
