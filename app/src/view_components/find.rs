use crate::appearance::Appearance;
use crate::editor::{
    EditorView, Event as EditorEvent, PropagateAndNoOpNavigationKeys, SingleLineEditorOptions,
    TextOptions,
};
use crate::send_telemetry_from_ctx;
use crate::server::telemetry::{FindOption, TelemetryEvent};
use crate::settings::InputModeSettings;
use crate::ui_components::{blended_colors, icons::Icon};
use serde::Serialize;

use crate::themes::theme::Fill;
use pathfinder_color::ColorU;
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
        Text,
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
pub(crate) const FIND_EDITOR_BORDER_WIDTH: f32 = 1.;
const FIND_EDITOR_FONT_SIZE: f32 = 12.;

pub const REGEX_TOGGLE_LABEL: &str = ". *";
pub const REGEX_TOGGLE_TOOLTIP: &str = "Regex toggle";

pub const CASE_SENSITIVE_LABEL: &str = "Aa";
pub const CASE_SENSITIVE_TOOLTIP: &str = "Case sensitive search";

pub const FIND_WITHIN_BLOCK_TOOLTIP: &str = "Find in selected block";
pub const FIND_PLACEHOLDER_TEXT: &str = "Find";

// Moving FindEvent, FindModel implementations away from terminal/.
pub enum FindEvent {
    /// Emitted a find run has been executed.
    RanFind,

    /// Emitted when the focused match in the active find run has been updated.
    UpdatedFocusedMatch,
}

pub trait FindModel {
    fn focused_match_index(&self) -> Option<usize>;
    fn match_count(&self) -> usize;
    fn default_find_direction(&self, app: &AppContext) -> FindDirection;
    fn alt_find_direction(&self, app: &AppContext) -> FindDirection {
        match self.default_find_direction(app) {
            FindDirection::Up => FindDirection::Down,
            FindDirection::Down => FindDirection::Up,
        }
    }

    /// Returns true if a find operation is currently in progress (scanning).
    /// Default implementation returns false for synchronous find implementations.
    fn is_scanning(&self) -> bool {
        false
    }
}

pub enum Event {
    CloseFindBar,
    Update { query: Option<String> },
    NextMatch { direction: FindDirection },
    ToggleFindInBlock { value: bool },
    ToggleCaseSensitivity { is_case_sensitive: bool },
    ToggleRegexSearch { is_regex_enabled: bool },
}

#[derive(Debug, Copy, Clone, PartialEq, Serialize)]
pub enum FindDirection {
    Up,
    Down,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum FindWithinBlockState {
    Enabled,
    Disabled,
    Hidden,
}

#[derive(Default)]
struct ButtonMouseStates {
    match_up: MouseStateHandle,
    match_down: MouseStateHandle,
    close: MouseStateHandle,
    toggle_find_in_block: MouseStateHandle,
    toggle_case_sensitivity: MouseStateHandle,
    toggle_regex_search: MouseStateHandle,
}

pub struct Find<T: FindModel + Entity<Event = FindEvent> + 'static> {
    editor: ViewHandle<EditorView>,
    model: ModelHandle<T>,
    button_mouse_states: ButtonMouseStates,
    pub case_sensitivity_enabled: bool,
    pub regex_search_enabled: bool,
    pub display_find_within_block: FindWithinBlockState,
}

#[derive(Copy, Clone, Debug)]
pub enum FindAction {
    Up,
    Down,
    Close,
    ToggleFindInBlock,
    ToggleCaseSensitivity,
    ToggleRegexSearch,
    CmdG,
    CmdShiftG,
}

pub fn init(app: &mut AppContext) {
    use warpui::keymap::macros::*;

    app.register_editable_bindings([
        EditableBinding::new(
            "find:find_next_occurrence",
            "Find the next occurrence of your search query",
            FindAction::CmdG,
        )
        .with_context_predicate(id!("Find"))
        // Both Intellij and VSCode use f3/shift-f3 to navigate find occurrences on windows / linux.
        // See https://www.jetbrains.com/help/idea/reference-keymap-win-default.html#find_everything.
        .with_mac_key_binding("cmd-g")
        .with_linux_or_windows_key_binding("f3"),
        EditableBinding::new(
            "find:find_prev_occurrence",
            "Find the previous occurrence of your search query",
            FindAction::CmdShiftG,
        )
        .with_context_predicate(id!("Find"))
        .with_mac_key_binding("cmd-shift-G")
        .with_linux_or_windows_key_binding("shift-f3"),
    ])
}

impl<T: FindModel + Entity<Event = FindEvent> + 'static> Find<T> {
    pub fn new(model: ModelHandle<T>, ctx: &mut ViewContext<Self>) -> Self {
        let editor = ctx.add_typed_action_view(|ctx| {
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

        ctx.subscribe_to_view(&editor, move |me, _, event, ctx| {
            me.handle_editor_event(event, ctx);
        });

        ctx.subscribe_to_model(&InputModeSettings::handle(ctx), |_, _, _, ctx| {
            ctx.notify();
        });

        ctx.subscribe_to_model(&model, |_, _, event, ctx| match event {
            FindEvent::RanFind | FindEvent::UpdatedFocusedMatch => ctx.notify(),
        });

        Self {
            editor,
            model,
            button_mouse_states: Default::default(),
            case_sensitivity_enabled: false,
            regex_search_enabled: false,
            display_find_within_block: FindWithinBlockState::Disabled,
        }
    }

    pub fn editor(&self) -> &ViewHandle<EditorView> {
        &self.editor
    }

    pub fn is_editor_focused(&self, ctx: &AppContext) -> bool {
        self.editor.as_ref(ctx).is_focused()
    }

    fn editor_text(&self, ctx: &AppContext) -> String {
        self.editor.as_ref(ctx).buffer_text(ctx)
    }

    pub fn set_query_text(&mut self, text: &str, ctx: &mut ViewContext<Self>) {
        self.editor.update(ctx, |editor, ctx| {
            editor.select_all(ctx);
            editor.insert_selected_text(text, ctx);
        });
    }

    fn handle_editor_event(&mut self, event: &EditorEvent, ctx: &mut ViewContext<Self>) {
        match event {
            EditorEvent::Edited(_) => {
                let query = self.editor_text(ctx);
                ctx.emit(Event::Update {
                    // If the query is empty, don't search for an empty string - set the query to
                    // `None`.
                    query: (!query.is_empty()).then_some(query),
                });
                self.emit_result_a11y_content(ctx);
                ctx.notify();
            }
            EditorEvent::Enter => {
                self.focus_next_match(T::default_find_direction(self.model.as_ref(ctx), ctx), ctx);
            }
            EditorEvent::ShiftEnter | EditorEvent::AltEnter => {
                self.focus_next_match(T::alt_find_direction(self.model.as_ref(ctx), ctx), ctx);
            }
            EditorEvent::Escape => {
                self.close_find_bar(ctx);
            }
            // If the user is focusing on the editor in the find bar, we
            // want to keep the current selected block
            EditorEvent::ClearParentSelections => {}
            _ => {}
        }
    }

    fn focus_next_match(&mut self, direction: FindDirection, ctx: &mut ViewContext<Self>) {
        // All of the acutal update logic for the selected match goes through the event codepath
        // but for some reason the logic for updating the match index happens below.
        ctx.emit(Event::NextMatch { direction });

        self.emit_result_a11y_content(ctx);
        ctx.notify();
    }

    /// Emits the a11y announcement informing about the current match/result.
    /// Note that it's done outside of the regular action_accessibility_contents flow,
    /// as `focus_next_match` may have multiple entrypoints (that are not Action).
    pub fn emit_result_a11y_content(&mut self, ctx: &mut ViewContext<Self>) {
        let content = if let Some(match_index) = self.model.as_ref(ctx).focused_match_index() {
            AccessibilityContent::new(
                format!(
                    "Result {} of {}.",
                    match_index + 1,
                    self.model.as_ref(ctx).match_count()
                ),
                "Use enter and shift-enter to navigate between matches. Escape to quit.",
                WarpA11yRole::UserAction,
            )
        } else {
            AccessibilityContent::new_without_help("No results.", WarpA11yRole::UserAction)
        };
        ctx.emit_a11y_content(content);
    }

    fn close_find_bar(&mut self, ctx: &mut ViewContext<Self>) {
        ctx.emit(Event::CloseFindBar);
    }

    fn toggle_find_within_block(&mut self, ctx: &mut ViewContext<Self>) {
        self.display_find_within_block = match self.display_find_within_block {
            FindWithinBlockState::Enabled => FindWithinBlockState::Disabled,
            FindWithinBlockState::Disabled => FindWithinBlockState::Enabled,
            _ => return,
        };
        send_telemetry_from_ctx!(
            TelemetryEvent::ToggleFindOption {
                option: FindOption::FindInBlock,
                enabled: self.display_find_within_block == FindWithinBlockState::Enabled,
            },
            ctx
        );
        ctx.emit(Event::ToggleFindInBlock {
            value: self.display_find_within_block == FindWithinBlockState::Enabled,
        });
    }

    fn toggle_case_sensitivity(&mut self, ctx: &mut ViewContext<Self>) {
        self.case_sensitivity_enabled = !self.case_sensitivity_enabled;
        send_telemetry_from_ctx!(
            TelemetryEvent::ToggleFindOption {
                option: FindOption::CaseSensitive,
                enabled: self.case_sensitivity_enabled
            },
            ctx
        );
        ctx.emit(Event::ToggleCaseSensitivity {
            is_case_sensitive: self.case_sensitivity_enabled,
        });
    }

    fn toggle_regex_search(&mut self, ctx: &mut ViewContext<Self>) {
        self.regex_search_enabled = !self.regex_search_enabled;
        send_telemetry_from_ctx!(
            TelemetryEvent::ToggleFindOption {
                option: FindOption::Regex,
                enabled: self.regex_search_enabled
            },
            ctx
        );
        ctx.emit(Event::ToggleRegexSearch {
            is_regex_enabled: self.regex_search_enabled,
        });
    }

    fn render_match_index(&self, appearance: &Appearance, app: &AppContext) -> Box<dyn Element> {
        let model = self.model.as_ref(app);

        // Show scanning indicator if async find is in progress.
        if model.is_scanning() {
            let match_count = model.match_count();
            let label = if match_count > 0 {
                format!("{}+ ...", match_count)
            } else {
                "Scanning...".to_string()
            };
            return Text::new_inline(label, appearance.ui_font_family(), FIND_EDITOR_FONT_SIZE)
                .with_color(blended_colors::text_sub(
                    appearance.theme(),
                    appearance.theme().surface_1(),
                ))
                .finish();
        }

        // If there is some match index, we add 1 to it since the UI is 1-indexed
        // (i.e. first match starts at index 1 out of the total number of matches).
        let index = match model.focused_match_index() {
            None => 0,
            Some(idx) => idx + 1,
        };
        let label = format!("{}/{}", index, model.match_count());
        Text::new_inline(label, appearance.ui_font_family(), FIND_EDITOR_FONT_SIZE)
            .with_color(blended_colors::text_sub(
                appearance.theme(),
                appearance.theme().surface_1(),
            ))
            .finish()
    }

    #[allow(clippy::too_many_arguments)]
    fn render_hoverable_icon_in_editor(
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
        let background_color = if hovered && self.model.as_ref(app).match_count() > 0 {
            appearance.theme().foreground_button_color()
        } else {
            Fill::Solid(ColorU::transparent_black())
        };
        let match_icon = match direction {
            FindDirection::Down => Icon::ArrowDown,
            FindDirection::Up => Icon::ArrowUp,
        };
        let icon_color = if self.model.as_ref(app).match_count() == 0 {
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

    fn render_close_button(
        &self,
        appearance: &Appearance,
        hovered: bool,
        height: f32,
    ) -> Box<dyn Element> {
        let background_color = if hovered {
            appearance.theme().foreground_button_color()
        } else {
            Fill::Solid(ColorU::transparent_black())
        };
        Container::new(
            ConstrainedBox::new(
                Icon::X
                    .to_warpui_icon(appearance.theme().active_ui_text_color())
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
}

impl<T: FindModel + Entity<Event = FindEvent> + 'static> Entity for Find<T> {
    type Event = Event;
}

impl<T: FindModel + Entity<Event = FindEvent> + 'static> TypedActionView for Find<T> {
    type Action = FindAction;

    fn handle_action(&mut self, action: &FindAction, ctx: &mut ViewContext<Self>) {
        match action {
            FindAction::Up => self.focus_next_match(FindDirection::Up, ctx),
            FindAction::CmdG => {
                self.focus_next_match(T::default_find_direction(self.model.as_ref(ctx), ctx), ctx)
            }
            FindAction::Down => self.focus_next_match(FindDirection::Down, ctx),
            FindAction::CmdShiftG => {
                self.focus_next_match(T::alt_find_direction(self.model.as_ref(ctx), ctx), ctx)
            }
            FindAction::Close => self.close_find_bar(ctx),
            FindAction::ToggleFindInBlock => self.toggle_find_within_block(ctx),
            FindAction::ToggleCaseSensitivity => self.toggle_case_sensitivity(ctx),
            FindAction::ToggleRegexSearch => self.toggle_regex_search(ctx),
        }
    }
}

impl<T: FindModel + Entity<Event = FindEvent> + 'static> View for Find<T> {
    fn ui_name() -> &'static str {
        "Find"
    }

    fn accessibility_contents(&self, _: &AppContext) -> Option<AccessibilityContent> {
        Some(AccessibilityContent::new(
            "Type searched phrase.",
            "Press escape to quit, use enter and shift-enter to navigate between matches",
            WarpA11yRole::TextareaRole,
        ))
    }

    fn on_focus(&mut self, focus_ctx: &FocusContext, ctx: &mut ViewContext<Self>) {
        if focus_ctx.is_self_focused() {
            self.editor.update(ctx, |editor, ctx| {
                editor.select_all(ctx);
            });
            ctx.focus(&self.editor);
            ctx.notify();
        }
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let editor_height = self
            .editor
            .as_ref(app)
            .line_height(app.font_cache(), appearance);
        let mut query_editor_row =
            Flex::row().with_cross_axis_alignment(CrossAxisAlignment::Center);

        let regex_icon = self.render_hoverable_icon_in_editor(
            appearance,
            Icon::Regex,
            self.regex_search_enabled,
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
                    self.case_sensitivity_enabled,
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
        let find_within_block_icon = Container::new(
            SavePosition::new(
                self.render_hoverable_icon_in_editor(
                    appearance,
                    Icon::CornersOfBox,
                    self.display_find_within_block == FindWithinBlockState::Enabled,
                    self.button_mouse_states.toggle_find_in_block.clone(),
                    FindAction::ToggleFindInBlock,
                    editor_height,
                    Some(FIND_WITHIN_BLOCK_TOOLTIP),
                    0.,
                ),
                "find_in_block_button",
            )
            .finish(),
        )
        .finish();

        let query_editor = Shrinkable::new(
            1.,
            ConstrainedBox::new(Clipped::new(ChildView::new(&self.editor).finish()).finish())
                .with_height(editor_height)
                .finish(),
        )
        .finish();
        query_editor_row.add_child(query_editor);
        query_editor_row.add_child(regex_icon);
        query_editor_row.add_child(case_sensitive_icon);
        if self.display_find_within_block != FindWithinBlockState::Hidden {
            query_editor_row.add_child(find_within_block_icon);
        }

        let mut find_row = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(
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
        find_row.add_child(
            Container::new(
                ConstrainedBox::new(self.render_match_index(appearance, app))
                    .with_height(editor_height)
                    .finish(),
            )
            .with_margin_right(HORIZONTAL_ICON_SPACING)
            .finish(),
        );
        find_row.add_child(
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
        find_row.add_child(
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
        find_row.add_child(
            // close button
            Container::new(
                Hoverable::new(self.button_mouse_states.close.clone(), |state| {
                    self.render_close_button(appearance, state.is_hovered(), editor_height)
                })
                .on_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(FindAction::Close);
                })
                .finish(),
            )
            .finish(),
        );

        let find_bar = Container::new(
            ConstrainedBox::new(
                Container::new(find_row.finish())
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

#[cfg(test)]
#[path = "find_tests.rs"]
mod tests;
