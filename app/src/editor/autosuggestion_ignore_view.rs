//! This module contains the code for the ignore button shown inline next to autosuggestions.

use crate::appearance::Appearance;
use crate::ui_components::blended_colors;
use crate::ui_components::icons::Icon;
use pathfinder_geometry::vector::vec2f;
use warp_core::ui::theme::Fill;
use warpui::elements::{
    ChildAnchor, ConstrainedBox, Container, CornerRadius, Element, Hoverable, MouseStateHandle,
    OffsetPositioning, ParentAnchor, ParentElement, ParentOffsetBounds, Radius, Stack,
};
use warpui::platform::Cursor;
use warpui::ui_components::components::UiComponent;
use warpui::{Entity, TypedActionView, View};
use warpui::{SingletonEntity, ViewContext};

use super::EditorElement;

pub const AUTOSUGGESTION_IGNORE_MINIMUM_HEIGHT: f32 = 12.;

pub struct AutosuggestionIgnore {
    autosuggestion_ignore_mouse_handle: MouseStateHandle,
    current_autosuggestion: Option<String>,
}

pub enum AutosuggestionIgnoreEvent {
    IgnoreAutosuggestion { suggestion: String },
}

impl Default for AutosuggestionIgnore {
    fn default() -> Self {
        Self::new()
    }
}

impl Entity for AutosuggestionIgnore {
    type Event = AutosuggestionIgnoreEvent;
}

impl AutosuggestionIgnore {
    pub fn new() -> Self {
        Self {
            autosuggestion_ignore_mouse_handle: Default::default(),
            current_autosuggestion: None,
        }
    }

    pub fn set_current_autosuggestion(&mut self, suggestion: Option<String>) {
        self.current_autosuggestion = suggestion;
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AutosuggestionIgnoreAction {
    IgnoreCurrentAutosuggestion,
}

impl TypedActionView for AutosuggestionIgnore {
    type Action = AutosuggestionIgnoreAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            AutosuggestionIgnoreAction::IgnoreCurrentAutosuggestion => {
                if let Some(suggestion) = &self.current_autosuggestion {
                    ctx.emit(AutosuggestionIgnoreEvent::IgnoreAutosuggestion {
                        suggestion: suggestion.clone(),
                    });
                }
            }
        }
        ctx.notify();
    }
}

impl View for AutosuggestionIgnore {
    fn ui_name() -> &'static str {
        "AutosuggestionIgnore"
    }

    fn render(&self, ctx: &warpui::AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(ctx);
        // Because the icon's origin is NOT the top of the line but the top of the cursor,
        // we should render it with line height ratio no larger than DEFAULT_UI_LINE_HEIGHT_RATIO.
        // With larger line height ratios, there wouldn't be enough space in the icon to render the text.
        // But we do need to account for smaller line height ratios so the text can be rendered in the smaller space.
        let line_height_ratio = appearance
            .line_height_ratio()
            .min(warpui::elements::DEFAULT_UI_LINE_HEIGHT_RATIO);
        // We want the ignore icon to be the same height as the cursor in the input.
        let height =
            EditorElement::cursor_height(appearance.monospace_font_size(), line_height_ratio)
                .max(AUTOSUGGESTION_IGNORE_MINIMUM_HEIGHT);
        let disabled_color = blended_colors::semantic_text_disabled(appearance.theme());
        let border_width = 1.;

        // The stack contains the ignore button and tooltip that shows upon mouse hover.
        let mut stack = Stack::new();

        Hoverable::new(self.autosuggestion_ignore_mouse_handle.clone(), |state| {
            // Colors are inverted when hovered.
            let (icon_color, background_color) = if state.is_hovered() {
                (
                    appearance.theme().background().into(),
                    Some(blended_colors::semantic_text_disabled(appearance.theme())),
                )
            } else {
                (
                    blended_colors::semantic_text_disabled(appearance.theme()),
                    None,
                )
            };

            let height_without_border = height - border_width * 2.;
            let close_icon = Container::new(
                ConstrainedBox::new(Icon::X.to_warpui_icon(Fill::Solid(icon_color)).finish())
                    .with_height(height_without_border)
                    .with_width(height_without_border)
                    .finish(),
            )
            .finish();

            let mut ignore_button = Container::new(close_icon)
                .with_uniform_padding(2.)
                .with_border(
                    warpui::elements::Border::all(border_width).with_border_color(disabled_color),
                )
                .with_corner_radius(CornerRadius::with_all(Radius::Percentage(25.)));

            if let Some(background_color) = background_color {
                ignore_button = ignore_button.with_background_color(background_color);
            }

            let ignore_button_element = ConstrainedBox::new(ignore_button.finish())
                .with_max_height(height)
                .finish();

            stack.add_child(ignore_button_element);

            if state.is_hovered() {
                let tool_tip = appearance
                    .ui_builder()
                    .autosuggestion_tool_tip(
                        crate::i18n::tr_static(ctx, "Ignore this suggestion").into(),
                    )
                    .build()
                    .finish();
                stack.add_positioned_overlay_child(
                    tool_tip,
                    OffsetPositioning::offset_from_parent(
                        vec2f(0., -5.),
                        ParentOffsetBounds::WindowByPosition,
                        ParentAnchor::TopMiddle,
                        ChildAnchor::BottomMiddle,
                    ),
                );
            }
            stack.finish()
        })
        .on_click(|ctx, _, _| {
            ctx.dispatch_typed_action(AutosuggestionIgnoreAction::IgnoreCurrentAutosuggestion);
        })
        .with_cursor(Cursor::PointingHand)
        .finish()
    }
}
