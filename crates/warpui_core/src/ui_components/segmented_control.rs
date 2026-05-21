use itertools::Itertools;

use crate::{
    color::ColorU,
    elements::{
        Align, Border, ConstrainedBox, Container, CornerRadius, CrossAxisAlignment, Fill, Flex,
        Icon, MainAxisAlignment, MouseStateHandle, ParentElement, Radius, Text,
    },
    fonts::FamilyId,
    platform::Cursor,
    ui_components::{
        button::Button,
        components::{Coords, UiComponent, UiComponentStyles},
        tool_tip::{Tooltip, TooltipWithSublabel},
    },
    AppContext, Element, Entity, TypedActionView, View, ViewContext,
};

use core::fmt;
use std::{borrow::Cow, boxed::Box};

use super::button::ButtonTooltipPosition;
const MAX_WIDTH: f32 = 300.0;

/// A segmented control component with multiple selectable options
pub struct SegmentedControl<T> {
    options: Vec<T>,
    selected_option: T,
    build_option_config: BuildRenderableOptionConfig<T>,
    mouse_states: Vec<MouseStateHandle>,
    styles: UiComponentStyles,

    /// If Some, we will set the control to disabled and use the tooltip text provided
    disabled_tooltip: Option<Cow<'static, str>>,
}

#[derive(Debug)]
pub enum SegmentedControlAction<T: SegmentedControlOption> {
    SelectOption(T),
}

pub enum SegmentedControlEvent<T: SegmentedControlOption> {
    OptionSelected(T),
}

pub struct LabelConfig {
    pub label: Cow<'static, str>,
    pub width_override: Option<f32>,
    pub color: ColorU,
}

pub struct TooltipConfig {
    pub text: Cow<'static, str>,
    pub sub_text: Option<Cow<'static, str>>,
    pub text_color: ColorU,
    pub background_color: ColorU,
    pub border_color: ColorU,
}

/// Config for rendering an option within the control.
pub struct RenderableOptionConfig {
    pub icon_path: &'static str,
    pub icon_color: ColorU,
    pub label: Option<LabelConfig>,
    pub tooltip: Option<TooltipConfig>,
    pub background: Fill,
}

/// Trait for data types that may be used as options within a segmented control.
///
/// This basically exists to ensure options are `Copy` and support checking for value equality.
pub trait SegmentedControlOption:
    fmt::Debug + Copy + Clone + PartialEq + Eq + Send + Sync + 'static
{
}

impl<T> SegmentedControlOption for T where
    T: fmt::Debug + Copy + Clone + PartialEq + Eq + Send + Sync + 'static
{
}

/// Type alias for function used to construct a [`RenderableOptionConfig`] to do determine how to
/// render an option within the segmented control, called at render time.
///
/// The first param is the option `T` being rendered, the second param is a boolean indicating
/// whether the option is currently selected.
///
/// If the returned value is [`None`], the option will not be rendered.
pub type BuildRenderableOptionConfig<T> =
    Box<dyn Fn(T, bool, &AppContext) -> Option<RenderableOptionConfig>>;

impl<T: SegmentedControlOption> SegmentedControl<T> {
    pub fn new<F>(
        options: Vec<T>,
        build_option_config_fn: F,
        mut default_option: T,
        styles: UiComponentStyles,
    ) -> Self
    where
        F: Fn(T, bool, &AppContext) -> Option<RenderableOptionConfig> + 'static,
    {
        debug_assert!(
            options.contains(&default_option),
            "Default option must be one of the provided options"
        );

        if !options.contains(&default_option) {
            default_option = options[0];
        }

        let mouse_states = options
            .iter()
            .map(|_| MouseStateHandle::default())
            .collect();

        Self {
            options,
            build_option_config: Box::new(build_option_config_fn),
            selected_option: default_option,
            mouse_states,
            styles,
            disabled_tooltip: None,
        }
    }

    /// Get the value of the currently selected option
    pub fn selected_option(&self) -> T {
        self.selected_option
    }

    /// Set the selected option.
    ///
    /// If `option` is not present in `self.options`, does nothing.
    pub fn set_selected_option(&mut self, option: T, ctx: &mut ViewContext<Self>) {
        if !self.options.iter().contains(&option) {
            return;
        }
        self.selected_option = option;
        ctx.notify();
    }

    pub fn set_styles(&mut self, styles: UiComponentStyles, ctx: &mut ViewContext<Self>) {
        self.styles = styles;
        ctx.notify();
    }

    /// Enable/disable the segmented control (disables click selection but retains hover/tooltip)
    pub fn set_disabled_tooltip(
        &mut self,
        disabled_tooltip: Option<Cow<'static, str>>,
        ctx: &mut ViewContext<Self>,
    ) {
        self.disabled_tooltip = disabled_tooltip;
        ctx.notify();
    }

    /// Update the available options in the control.
    ///
    /// If the currently selected option is not present in the new list, selects the first option by default.
    pub fn update_options(&mut self, updated_options: Vec<T>, ctx: &mut ViewContext<Self>) {
        debug_assert!(
            !updated_options.is_empty(),
            "Cannot pass empty options to SegmentedControl"
        );
        if updated_options.is_empty() {
            log::error!("Attempted to update SegmentedControl with empty options");
            return;
        }

        let should_update_selected = !updated_options.contains(&self.selected_option);
        self.options = updated_options;

        self.mouse_states = self
            .options
            .iter()
            .map(|_| MouseStateHandle::default())
            .collect();

        if should_update_selected {
            self.set_selected_option(self.options[0], ctx);
        }
        ctx.notify();
    }
}

impl<T: SegmentedControlOption> Entity for SegmentedControl<T> {
    type Event = SegmentedControlEvent<T>;
}

impl<T: SegmentedControlOption> View for SegmentedControl<T> {
    fn ui_name() -> &'static str {
        "SegmentedControl"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let is_disabled = self.disabled_tooltip.is_some();
        let mut options_container = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_alignment(MainAxisAlignment::Start);

        for (index, option) in self.options.iter().enumerate() {
            let is_selected = *option == self.selected_option;
            let Some(mut option_config) = (self.build_option_config)(*option, is_selected, app)
            else {
                continue;
            };

            // If globally disabled and an override tooltip is set, replace the tooltip text
            if let Some(disabled_text) = self.disabled_tooltip.clone() {
                // Override tooltip text with disabled text
                if let Some(tooltip_config) = option_config.tooltip.as_mut() {
                    tooltip_config.text = disabled_text;
                    // Clear keybinding/subtext when disabled
                    tooltip_config.sub_text = None;
                }
            }

            let mouse_state = self.mouse_states[index].clone();

            let button_styles = UiComponentStyles {
                background: Some(option_config.background),
                // Slightly tighter padding to keep controls compact in narrow headers.
                padding: Some(Coords::uniform(2.0)),
                border_width: None,
                border_radius: Some(CornerRadius::with_all(Radius::Pixels(3.0))),
                margin: None,
                ..self.styles
            };

            let mut button = Button::new(
                mouse_state,
                button_styles,
                None, // hover styles
                None, // clicked styles
                None, // disabled styles
            );

            if let Some(label_config) = option_config.label.take() {
                let font_size = if cfg!(any(
                    windows,
                    any(target_os = "linux", target_os = "freebsd")
                )) {
                    // Reduce the font size by one to avoid text being cut off on Windows and Linux.
                    self.styles.font_size.unwrap_or(12.0) - 1.0
                } else {
                    self.styles.font_size.unwrap_or(12.0)
                };
                let icon_size = font_size * 1.4;
                let font_family_id = self.styles.font_family_id.unwrap_or(FamilyId(0));

                let mut text = ConstrainedBox::new(
                    Container::new(
                        Align::new(
                            Text::new(label_config.label, font_family_id, font_size)
                                .with_color(option_config.icon_color)
                                .finish(),
                        )
                        .finish(),
                    )
                    // Account for icon margins due to viewbox difference in the SVG
                    .with_padding_right(icon_size * 0.2)
                    .finish(),
                );

                if let Some(width_override) = label_config.width_override {
                    // Scale label width by the same ratio as font size for proper zoom behavior
                    let font_size = self.styles.font_size.unwrap_or(12.0);
                    let base_font_size = 10.0; // Match the base font size used in universal_developer_input.rs
                    let ui_scalar = font_size / base_font_size;
                    text = text.with_width(width_override * ui_scalar);
                }

                let text = text.finish();

                if option_config.icon_path.is_empty() {
                    button = button.with_custom_label(text);
                } else {
                    let icon = ConstrainedBox::new(
                        Container::new(
                            Icon::new(option_config.icon_path, option_config.icon_color).finish(),
                        )
                        .finish(),
                    )
                    .with_width(icon_size)
                    .with_height(icon_size)
                    .finish();

                    let button_label = Flex::row()
                        .with_cross_axis_alignment(CrossAxisAlignment::Center)
                        .with_child(icon)
                        .with_child(text)
                        .finish();

                    button = button.with_custom_label(button_label);
                }
            } else {
                button = button
                    .with_icon_label(Icon::new(option_config.icon_path, option_config.icon_color));
            }

            if let Some(tooltip_config) = option_config.tooltip.as_ref() {
                let styles = self.styles;
                let tooltip = tooltip_config.text.clone();
                let subtext = tooltip_config.sub_text.clone();
                let text_color = tooltip_config.text_color;
                let background_color = tooltip_config.background_color;
                let border_color = tooltip_config.border_color;
                button = button.with_tooltip(move || {
                    let styles = UiComponentStyles {
                        font_color: Some(text_color),
                        background: Some(Fill::Solid(background_color)),
                        border_radius: Some(CornerRadius::with_all(Radius::Pixels(4.0))),
                        border_width: Some(1.0),
                        border_color: Some(Fill::Solid(border_color)),
                        font_family_id: styles.font_family_id,
                        font_size: styles.font_size.map(|size| size - 2.0),
                        padding: Some(Coords {
                            top: 4.,
                            bottom: 4.,
                            left: 8.,
                            right: 8.,
                        }),
                        ..Default::default()
                    };
                    if let Some(subtext) = subtext {
                        TooltipWithSublabel::new(tooltip.into(), subtext.into(), styles)
                            .build()
                            .finish()
                    } else {
                        Tooltip::new(tooltip.into(), styles).build().finish()
                    }
                });
            }

            button = button.with_tooltip_position(ButtonTooltipPosition::AboveLeft);

            let option_copy = *option;
            let mut hoverable = button.build().with_cursor(if is_disabled {
                Cursor::Arrow
            } else {
                Cursor::PointingHand
            });

            // Buttons should not be clickable if they are disabled
            if !is_disabled {
                hoverable = hoverable.on_click({
                    move |ctx, _, _| {
                        ctx.dispatch_typed_action(SegmentedControlAction::SelectOption(
                            option_copy,
                        ));
                    }
                });
            }

            options_container = options_container.with_child(hoverable.finish());
        }

        let mut container = Container::new(
            ConstrainedBox::new(options_container.finish())
                .with_max_width(MAX_WIDTH)
                .finish(),
        );

        // Apply styles from UiComponentStyles
        if let Some(background) = self.styles.background {
            container = container.with_background(background);
        }
        if let Some(border_width) = self.styles.border_width {
            if let Some(border_color) = self.styles.border_color {
                container =
                    container.with_border(Border::all(border_width).with_border_fill(border_color));
            }
        }
        if let Some(border_radius) = self.styles.border_radius {
            container = container.with_corner_radius(border_radius);
        }
        if let Some(margin) = self.styles.margin {
            container = container
                .with_margin_left(margin.left)
                .with_margin_right(margin.right)
                .with_margin_top(margin.top)
                .with_margin_bottom(margin.bottom);
        }

        container.finish()
    }
}

impl<T: SegmentedControlOption> TypedActionView for SegmentedControl<T> {
    type Action = SegmentedControlAction<T>;

    fn handle_action(&mut self, action: &SegmentedControlAction<T>, ctx: &mut ViewContext<Self>) {
        match action {
            SegmentedControlAction::SelectOption(option) => {
                self.set_selected_option(*option, ctx);
                ctx.emit(SegmentedControlEvent::OptionSelected(*option));
            }
        }
    }
}
