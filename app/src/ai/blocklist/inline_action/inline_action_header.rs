use std::borrow::Cow;
use std::rc::Rc;

use pathfinder_color::ColorU;
use warp_core::ui::appearance::Appearance;
use warp_core::ui::theme::color::internal_colors;
use warpui::elements::{
    Border, ConstrainedBox, Container, CornerRadius, CrossAxisAlignment, DispatchEventResult,
    EventHandler, Expanded, Flex, FormattedTextElement, Hoverable, MainAxisAlignment, MainAxisSize,
    MouseStateHandle, ParentElement, Radius, Shrinkable, SizeConstraintCondition,
    SizeConstraintSwitch, Text,
};
use warpui::fonts::FamilyId;
use warpui::platform::Cursor;
use warpui::{AppContext, Element, EventContext, SingletonEntity};

use crate::ai::blocklist::inline_action::inline_action_icons::icon_size;
use crate::ui_components::blended_colors;
use crate::view_components::compactible_action_button::{
    RenderCompactibleActionButton, render_compact_and_regular_button_rows, render_expansion_icon,
};

/// Same padding constants as the original for consistency
pub const INLINE_ACTION_HORIZONTAL_PADDING: f32 = 16.;
/// The vertical padding applied to the requested action row's content body (usually a command).
pub const INLINE_ACTION_VERTICAL_PADDING: f32 = 12.;
pub const INLINE_ACTION_HEADER_VERTICAL_PADDING: f32 = 10.;
pub const ICON_MARGIN: f32 = 8.;

pub type OnToggleExpandedCallback = Rc<dyn Fn(&mut EventContext) + 'static>;
pub type OnRightClickCallback = Rc<dyn Fn(&mut EventContext) + 'static>;

/// Configuration for manual expansion behavior
#[derive(Clone)]
pub struct ExpandedConfig {
    pub is_expanded: bool,
    /// Optional callback for when expansion toggle is clicked
    pub on_toggle_expanded: Option<OnToggleExpandedCallback>,
    /// Optional callback for when right-click is triggered
    pub on_right_click: Option<OnRightClickCallback>,
    /// Mouse state handle for expansion toggle
    pub toggle_mouse_state: MouseStateHandle,
    pub expands_upwards: bool,
}

impl ExpandedConfig {
    pub fn new(is_expanded: bool, toggle_mouse_state: MouseStateHandle) -> Self {
        Self {
            is_expanded,
            on_toggle_expanded: None,
            on_right_click: None,
            toggle_mouse_state,
            expands_upwards: false,
        }
    }

    pub fn with_expands_upwards(mut self) -> Self {
        self.expands_upwards = true;
        self
    }

    pub fn with_toggle_callback<F>(mut self, callback: F) -> Self
    where
        F: Fn(&mut EventContext) + 'static,
    {
        self.on_toggle_expanded = Some(Rc::new(callback));
        self
    }

    pub fn with_right_click_callback<F>(mut self, callback: F) -> Self
    where
        F: Fn(&mut EventContext) + 'static,
    {
        self.on_right_click = Some(Rc::new(callback));
        self
    }
}

/// Configuration for when we want a right clickable element,
/// but that element isn't necessarily expandable.
#[derive(Clone)]
pub struct RightClickConfig {
    pub on_right_click: OnRightClickCallback,
    pub header_mouse_state: MouseStateHandle,
}

impl RightClickConfig {
    pub fn new(on_right_click: OnRightClickCallback, header_mouse_state: MouseStateHandle) -> Self {
        Self {
            on_right_click,
            header_mouse_state,
        }
    }
}

#[derive(Clone)]
pub enum InteractionMode {
    /// Renders action buttons.
    ActionButtons {
        action_buttons: Vec<Rc<dyn RenderCompactibleActionButton>>,
        size_switch_threshold: f32,
    },
    /// Renders expansion chevron, with caller-specified click handler.
    ManuallyExpandable(ExpandedConfig),
    /// Renders a right-clickable element, with caller-specified right-click handler.
    RightClickable(RightClickConfig),
}

#[derive(Clone)]
pub struct HeaderConfig {
    pub title: Cow<'static, str>,
    pub font_family: FamilyId,
    /// Whether to parse the title as markdown when rendering.
    pub use_markdown: bool,
    pub icon: Option<warpui::elements::Icon>,
    pub badge: Option<String>,
    pub interaction_mode: Option<InteractionMode>,
    pub is_text_selectable: bool,
    pub font_color_override: Option<ColorU>,
    pub corner_radius_override: Option<CornerRadius>,
    pub soft_wrap_title: bool,
}

impl HeaderConfig {
    pub fn new(title: impl Into<Cow<'static, str>>, app: &AppContext) -> Self {
        Self {
            title: title.into(),
            font_family: Appearance::as_ref(app).ui_font_family(),
            use_markdown: false,
            icon: None,
            badge: None,
            interaction_mode: None,
            is_text_selectable: false,
            font_color_override: None,
            corner_radius_override: None,
            soft_wrap_title: false,
        }
    }

    pub fn with_soft_wrap_title(mut self) -> Self {
        self.soft_wrap_title = true;
        self
    }

    pub fn with_font_family(mut self, font: FamilyId) -> Self {
        self.font_family = font;
        self
    }

    pub fn with_icon(mut self, icon: warpui::elements::Icon) -> Self {
        self.icon = Some(icon);
        self
    }

    pub fn with_badge(mut self, badge: String) -> Self {
        self.badge = Some(badge);
        self
    }

    pub fn with_interaction_mode(mut self, interaction_mode: InteractionMode) -> Self {
        self.interaction_mode = Some(interaction_mode);
        self
    }

    pub fn with_selectable_text(mut self) -> Self {
        self.is_text_selectable = true;
        self
    }

    pub fn with_font_color(mut self, font_color: ColorU) -> Self {
        self.font_color_override = Some(font_color);
        self
    }

    pub fn with_corner_radius_override(mut self, corner_radius: CornerRadius) -> Self {
        self.corner_radius_override = Some(corner_radius);
        self
    }

    /// Parses the title as markdown when rendering.
    pub fn with_markdown(mut self) -> Self {
        self.use_markdown = true;
        self
    }

    pub fn render_header(
        self,
        app: &AppContext,
        interaction_mode_content: Option<Box<dyn Element>>,
    ) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        let header_background = theme.surface_2();

        let mut header_row = Flex::row()
            .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
            .with_main_axis_size(MainAxisSize::Max)
            .with_cross_axis_alignment(CrossAxisAlignment::Center);

        let mut left_content_container = Flex::row()
            .with_main_axis_alignment(MainAxisAlignment::Start)
            .with_cross_axis_alignment(CrossAxisAlignment::Center);

        if let Some(icon) = self.icon {
            left_content_container.add_child(
                Container::new(
                    ConstrainedBox::new(icon.finish())
                        .with_width(icon_size(app))
                        .with_height(icon_size(app))
                        .finish(),
                )
                .with_margin_right(ICON_MARGIN)
                .finish(),
            )
        }

        let text_color = self
            .font_color_override
            .unwrap_or_else(|| blended_colors::text_main(appearance.theme(), header_background));

        let mut title_element = Text::new_inline(
            self.title.clone(),
            self.font_family,
            appearance.monospace_font_size(),
        )
        .soft_wrap(self.soft_wrap_title)
        .with_selectable(self.is_text_selectable)
        .with_color(text_color)
        .finish();

        if self.use_markdown
            && let Ok(formatted_text) = markdown_parser::parse_markdown(&self.title)
        {
            let mut element = FormattedTextElement::new(
                formatted_text,
                appearance.monospace_font_size(),
                self.font_family,
                appearance.monospace_font_family(),
                text_color,
                Default::default(),
            )
            .set_selectable(self.is_text_selectable);
            if !self.soft_wrap_title {
                element = element.with_no_text_wrapping();
            }
            title_element = element.finish();
        }

        left_content_container.add_child(
            Expanded::new(
                1.,
                Container::new(title_element).with_margin_right(8.).finish(),
            )
            .finish(),
        );

        if let Some(badge) = self.badge {
            left_content_container.add_child(
                Container::new(
                    Text::new(
                        badge,
                        appearance.ui_font_family(),
                        appearance.ui_font_size(),
                    )
                    .with_color(internal_colors::fg_overlay_5(theme).into())
                    .finish(),
                )
                .with_horizontal_padding(8.)
                .with_vertical_padding(4.)
                .with_margin_right(8.)
                .with_border(Border::all(1.0).with_border_fill(theme.outline()))
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(2.)))
                .with_background(theme.surface_1())
                .finish(),
            );
        }

        header_row.add_child(Shrinkable::new(1., left_content_container.finish()).finish());

        if let Some(interaction_mode_content) = interaction_mode_content {
            header_row.add_child(interaction_mode_content);
        }

        let looks_expanded_downwards =
            self.interaction_mode
                .as_ref()
                .is_some_and(|mode| match mode {
                    InteractionMode::ActionButtons { .. } => true,
                    InteractionMode::ManuallyExpandable(expansion_config) => {
                        expansion_config.is_expanded && !expansion_config.expands_upwards
                    }
                    InteractionMode::RightClickable(..) => false,
                });
        let container = Container::new(header_row.finish())
            .with_padding_left(INLINE_ACTION_HORIZONTAL_PADDING)
            .with_padding_right(INLINE_ACTION_HORIZONTAL_PADDING)
            .with_vertical_padding(INLINE_ACTION_HEADER_VERTICAL_PADDING)
            .with_background(header_background)
            .with_corner_radius(
                if let Some(corner_radius_override) = self.corner_radius_override {
                    corner_radius_override
                } else if looks_expanded_downwards {
                    CornerRadius::with_top(Radius::Pixels(8.))
                } else {
                    CornerRadius::with_all(Radius::Pixels(8.))
                },
            )
            .finish();

        if let Some(InteractionMode::ManuallyExpandable(expansion_config)) = &self.interaction_mode
        {
            let element = if let Some(callback) = &expansion_config.on_toggle_expanded {
                let callback = Rc::clone(callback);
                let mouse_state = expansion_config.toggle_mouse_state.clone();

                Hoverable::new(mouse_state, |_| container)
                    .on_click(move |ctx, _, _| {
                        callback(ctx);
                    })
                    .with_cursor(Cursor::PointingHand)
                    .finish()
            } else {
                container
            };

            // Wrap in EventHandler to allow right-click event propagation
            match expansion_config.on_right_click.clone() {
                Some(right_click_callback) => {
                    return EventHandler::new(element)
                        .on_right_mouse_down(move |ctx, _, _| {
                            right_click_callback(ctx);
                            DispatchEventResult::PropagateToParent
                        })
                        .finish();
                }
                _ => {
                    return element;
                }
            }
        } else if let Some(InteractionMode::RightClickable(right_click_config)) =
            &self.interaction_mode
        {
            let right_click_callback = right_click_config.on_right_click.clone();
            let header_mouse_state = right_click_config.header_mouse_state.clone();

            let hoverable = Hoverable::new(header_mouse_state, |_| container).finish();
            return EventHandler::new(hoverable)
                .on_right_mouse_down(move |ctx, _, _| {
                    right_click_callback(ctx);
                    DispatchEventResult::PropagateToParent
                })
                .finish();
        }

        container
    }

    pub fn render(self, app: &AppContext) -> Box<dyn Element> {
        match self.interaction_mode.clone() {
            Some(interaction_mode) => {
                let appearance: &Appearance = Appearance::as_ref(app);
                match interaction_mode {
                    InteractionMode::ActionButtons {
                        action_buttons,
                        size_switch_threshold,
                    } => {
                        // Convert boxed trait objects into trait object references expected by the renderer
                        let button_refs: Vec<&dyn RenderCompactibleActionButton> =
                            action_buttons.iter().map(|b| b.as_ref()).collect();

                        let (regular_row, compact_row) = render_compact_and_regular_button_rows(
                            button_refs,
                            None,
                            appearance,
                            app,
                        );

                        let regular_header = self.clone().render_header(app, Some(regular_row));
                        let compact_header = self.render_header(app, Some(compact_row));

                        let size_switch_threshold =
                            size_switch_threshold * appearance.monospace_ui_scalar();
                        SizeConstraintSwitch::new(
                            regular_header,
                            vec![(
                                SizeConstraintCondition::WidthLessThan(size_switch_threshold),
                                compact_header,
                            )],
                        )
                        .finish()
                    }
                    InteractionMode::ManuallyExpandable(expansion_config) => {
                        let expanded_icon = ConstrainedBox::new(render_expansion_icon(
                            expansion_config.is_expanded,
                            expansion_config.expands_upwards,
                            appearance,
                            app,
                        ))
                        .with_height(icon_size(app))
                        .with_width(icon_size(app))
                        .finish();

                        self.render_header(app, Some(expanded_icon))
                    }
                    InteractionMode::RightClickable(_) => self.render_header(app, None),
                }
            }
            _ => self.render_header(app, None),
        }
    }
}
