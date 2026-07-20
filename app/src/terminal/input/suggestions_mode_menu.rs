//! Rendering logic for the input suggestions menu.
//!
//! This module contains all rendering functions for the various input suggestion modes:
//! - HistoryUp
//! - CompletionSuggestions
//! - StaticWorkflowEnumSuggestions
//! - DynamicWorkflowEnumSuggestions

use warpui::elements::{
    Align, Border, ConstrainedBox, Container, CornerRadius, CrossAxisAlignment, DragBarSide,
    DropShadow, Element, Empty, Flex, ParentElement, Radius, Resizable, Shrinkable,
    SizeConstraintCondition, SizeConstraintSwitch, Text,
};
use warpui::presenter::ChildView;
use warpui::ui_components::components::{Coords, UiComponent, UiComponentStyles};

use super::{
    DYNAMIC_ENUM_FAILURE_MESSAGE, DYNAMIC_ENUM_GENERATE_MESSAGE,
    DYNAMIC_ENUM_HORIZONTAL_TEXT_PADDING, DYNAMIC_ENUM_MENU_HEIGHT_OFFSET,
    DYNAMIC_ENUM_MENU_PADDING, DYNAMIC_ENUM_NO_RESULTS_MESSAGE, DYNAMIC_ENUM_PENDING_MESSAGE,
    DYNAMIC_ENUM_RUN_MESSAGE, DynamicEnumSuggestionStatus, HISTORY_DETAILS_VIEW_WIDTH_REQUIREMENT,
    Input, InputAction, MenuPositioning, RUN_DYNAMIC_ENUM_COMMAND_KEYSTROKE,
    TERMINAL_VIEW_PADDING_LEFT,
};
use crate::appearance::Appearance;
use crate::input_suggestions::{
    DETAILS_PANEL_MARGIN, DETAILS_PANEL_PADDING, HISTORY_DETAILS_PANEL_WIDTH,
    LABEL_PADDING as InputSuggestionsLabelPadding,
};
use crate::themes::theme::WarpTheme;

enum SuggestionsResizeConfig {
    WidthAndHeight,
    WidthOnly,
    HeightOnly,
}

impl Input {
    pub(super) fn render_history_up_menu(
        &self,
        appearance: &Appearance,
        menu_positioning: MenuPositioning,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let radius = Radius::Pixels(6.);

        let corner_radius = match menu_positioning {
            MenuPositioning::AboveInputBox => CornerRadius::with_top(radius),
            MenuPositioning::BelowInputBox => CornerRadius::with_bottom(radius),
        };

        let input_suggestions_border = 2.0;
        let margin =
            *TERMINAL_VIEW_PADDING_LEFT - input_suggestions_border - InputSuggestionsLabelPadding;

        let content = ChildView::new(&self.input_suggestions).finish();

        SizeConstraintSwitch::new(
            Flex::row()
                .with_children([
                    Shrinkable::new(
                        1.,
                        self.render_suggestions_container(
                            margin,
                            corner_radius,
                            theme,
                            SuggestionsResizeConfig::HeightOnly,
                            menu_positioning,
                            content,
                        ),
                    )
                    .finish(),
                    ConstrainedBox::new(Empty::new().finish())
                        .with_width(
                            HISTORY_DETAILS_PANEL_WIDTH
                                + DETAILS_PANEL_MARGIN * 2.
                                + DETAILS_PANEL_PADDING * 2.,
                        )
                        .finish(),
                ])
                .finish(),
            vec![(
                SizeConstraintCondition::WidthLessThan(HISTORY_DETAILS_VIEW_WIDTH_REQUIREMENT),
                self.render_suggestions_container(
                    margin,
                    corner_radius,
                    theme,
                    SuggestionsResizeConfig::HeightOnly,
                    menu_positioning,
                    ChildView::new(&self.input_suggestions).finish(),
                ),
            )],
        )
        .finish()
    }

    pub(super) fn render_completion_suggestions_menu(
        &self,
        appearance: &Appearance,
        menu_positioning: MenuPositioning,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let corner_radius = CornerRadius::with_all(Radius::Pixels(6.));

        let content = ChildView::new(&self.input_suggestions).finish();

        self.render_suggestions_container(
            0.,
            corner_radius,
            theme,
            SuggestionsResizeConfig::WidthAndHeight,
            menu_positioning,
            content,
        )
    }

    pub(super) fn render_workflow_enum_suggestions_menu(
        &self,
        appearance: &Appearance,
        menu_positioning: MenuPositioning,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let corner_radius = CornerRadius::with_all(Radius::Pixels(6.));

        // Same styling as CompletionSuggestions
        let margin = 0.;

        let content = ChildView::new(&self.input_suggestions).finish();

        self.render_suggestions_container(
            margin,
            corner_radius,
            theme,
            SuggestionsResizeConfig::WidthAndHeight,
            menu_positioning,
            content,
        )
    }

    pub(super) fn render_dynamic_workflow_enum_menu(
        &self,
        appearance: &Appearance,
        menu_positioning: MenuPositioning,
        command: String,
        status: DynamicEnumSuggestionStatus,
        suggestions: &[String],
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let corner_radius = CornerRadius::with_all(Radius::Pixels(6.));

        let (content, resize_config) = match status {
            DynamicEnumSuggestionStatus::Unapproved => (
                self.render_dynamic_enum_command_approval(command, appearance),
                SuggestionsResizeConfig::WidthOnly,
            ),
            DynamicEnumSuggestionStatus::Pending => (
                self.render_dynamic_enum_status_message(DYNAMIC_ENUM_PENDING_MESSAGE, appearance),
                SuggestionsResizeConfig::WidthAndHeight,
            ),
            DynamicEnumSuggestionStatus::Failure => (
                self.render_dynamic_enum_status_message(DYNAMIC_ENUM_FAILURE_MESSAGE, appearance),
                SuggestionsResizeConfig::WidthAndHeight,
            ),
            DynamicEnumSuggestionStatus::Success if suggestions.is_empty() => (
                self.render_dynamic_enum_status_message(
                    DYNAMIC_ENUM_NO_RESULTS_MESSAGE,
                    appearance,
                ),
                SuggestionsResizeConfig::WidthAndHeight,
            ),
            DynamicEnumSuggestionStatus::Success => (
                ChildView::new(&self.input_suggestions).finish(),
                SuggestionsResizeConfig::WidthAndHeight,
            ),
        };

        self.render_suggestions_container(
            0.,
            corner_radius,
            theme,
            resize_config,
            menu_positioning,
            content,
        )
    }

    /// Renders the suggestions container with configurable resize behavior.
    fn render_suggestions_container(
        &self,
        margin: f32,
        corner_radius: CornerRadius,
        theme: &WarpTheme,
        resize_config: SuggestionsResizeConfig,
        menu_positioning: MenuPositioning,
        content: Box<dyn Element>,
    ) -> Box<dyn Element> {
        // Inner container for the visual menu content (background, border, shadow)
        // This is wrapped by Resizable so the resize handles align with the border.
        let inner_container = Container::new(content)
            .with_background(theme.surface_2())
            .with_corner_radius(corner_radius)
            .with_drop_shadow(DropShadow::default())
            .with_border(Border::all(1.0).with_border_fill(theme.outline()))
            .finish();

        // Apply width resizing based on config
        let horizontal_resizable = match resize_config {
            SuggestionsResizeConfig::WidthAndHeight | SuggestionsResizeConfig::WidthOnly => {
                let width_handle_for_end = self.completions_menu_resizable_width.clone();
                Resizable::new(
                    self.completions_menu_resizable_width.clone(),
                    inner_container,
                )
                .with_dragbar_side(DragBarSide::Right)
                .with_dragbar_offset(7.0)
                .with_bounds_callback(Box::new(|window_size| (200.0, window_size.x())))
                .on_resize(move |ctx, _| {
                    ctx.notify();
                })
                .on_end_resizing(move |ctx, _| {
                    let new_width = width_handle_for_end
                        .lock()
                        .expect("width handle lock poisoned")
                        .size();
                    ctx.dispatch_typed_action(InputAction::UpdateCompletionsMenuWidth(new_width));
                })
                .finish()
            }
            SuggestionsResizeConfig::HeightOnly => inner_container,
        };

        // Apply height resizing based on config
        // Skip vertical resizing for command approval to show full command because we want users
        // to see the entire command they are about to execute.
        let resizable_element = match resize_config {
            SuggestionsResizeConfig::WidthOnly => horizontal_resizable,
            SuggestionsResizeConfig::WidthAndHeight | SuggestionsResizeConfig::HeightOnly => {
                let dragbar_side = match menu_positioning {
                    MenuPositioning::AboveInputBox => DragBarSide::Top,
                    MenuPositioning::BelowInputBox => DragBarSide::Bottom,
                };

                let height_handle_for_end = self.completions_menu_resizable_height.clone();
                Resizable::new(
                    self.completions_menu_resizable_height.clone(),
                    horizontal_resizable,
                )
                .with_dragbar_side(dragbar_side)
                .with_bounds_callback(Box::new(|window_size| (100.0, window_size.y())))
                .on_resize(move |ctx, _| {
                    ctx.notify();
                })
                .on_end_resizing(move |ctx, _| {
                    let new_height = height_handle_for_end
                        .lock()
                        .expect("height handle lock poisoned")
                        .size();
                    ctx.dispatch_typed_action(InputAction::UpdateCompletionsMenuHeight(new_height));
                })
                .finish()
            }
        };

        Container::new(resizable_element)
            .with_margin_top(12.0)
            .with_margin_left(margin)
            .with_margin_right(margin)
            .finish()
    }

    /// Renders the command approval UI for dynamic enum suggestions.
    fn render_dynamic_enum_command_approval(
        &self,
        command: String,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let keybinding_style = UiComponentStyles {
            height: Some(15.),
            font_size: Some(appearance.ui_font_size()),
            margin: Some(Coords {
                left: 2.,
                right: 2.,
                ..Default::default()
            }),
            ..Default::default()
        };

        Flex::column()
            .with_child(
                ConstrainedBox::new(
                    Container::new(
                        Text::new_inline(
                            String::from(DYNAMIC_ENUM_GENERATE_MESSAGE),
                            appearance.ui_font_family(),
                            appearance.ui_font_size(),
                        )
                        .with_color(theme.main_text_color(theme.surface_2()).into_solid())
                        .finish(),
                    )
                    .with_uniform_padding(DYNAMIC_ENUM_MENU_PADDING)
                    .finish(),
                )
                .with_max_height(appearance.ui_font_size() + DYNAMIC_ENUM_MENU_HEIGHT_OFFSET)
                .finish(),
            )
            .with_child(
                Container::new(
                    Text::new(
                        command.clone(),
                        appearance.monospace_font_family(),
                        appearance.monospace_font_size(),
                    )
                    .with_color(theme.main_text_color(theme.background()).into_solid())
                    .finish(),
                )
                .with_uniform_padding(DYNAMIC_ENUM_MENU_PADDING)
                .with_background(theme.background())
                .finish(),
            )
            .with_child(
                ConstrainedBox::new(
                    Flex::row()
                        .with_child(
                            Shrinkable::new(
                                1.,
                                Container::new(
                                    appearance
                                        .ui_builder()
                                        .keyboard_shortcut(&RUN_DYNAMIC_ENUM_COMMAND_KEYSTROKE)
                                        .with_style(keybinding_style)
                                        .build()
                                        .finish(),
                                )
                                .with_uniform_padding(DYNAMIC_ENUM_HORIZONTAL_TEXT_PADDING)
                                .finish(),
                            )
                            .finish(),
                        )
                        .with_child(
                            Shrinkable::new(
                                1.,
                                Container::new(
                                    Text::new_inline(
                                        String::from(DYNAMIC_ENUM_RUN_MESSAGE),
                                        appearance.ui_font_family(),
                                        appearance.ui_font_size(),
                                    )
                                    .with_color(theme.sub_text_color(theme.surface_2()).into())
                                    .finish(),
                                )
                                .with_uniform_padding(DYNAMIC_ENUM_HORIZONTAL_TEXT_PADDING)
                                .finish(),
                            )
                            .finish(),
                        )
                        .with_cross_axis_alignment(CrossAxisAlignment::Center)
                        .finish(),
                )
                .with_max_height(appearance.ui_font_size() + DYNAMIC_ENUM_MENU_HEIGHT_OFFSET)
                .finish(),
            )
            .finish()
    }

    /// Renders a status message for dynamic enum suggestions.
    fn render_dynamic_enum_status_message(
        &self,
        message: &str,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        ConstrainedBox::new(
            Align::new(
                Container::new(
                    Text::new_inline(
                        String::from(message),
                        appearance.monospace_font_family(),
                        appearance.monospace_font_size(),
                    )
                    .with_color(theme.sub_text_color(theme.surface_2()).into())
                    .finish(),
                )
                .with_uniform_padding(DYNAMIC_ENUM_HORIZONTAL_TEXT_PADDING)
                .finish(),
            )
            .finish(),
        )
        .with_max_height(appearance.monospace_font_size() + DYNAMIC_ENUM_MENU_HEIGHT_OFFSET)
        .finish()
    }
}
