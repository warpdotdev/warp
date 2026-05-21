//! This module contains common utilities for rendering Blocklist AI UI.
use std::sync::LazyLock;

use pathfinder_color::ColorU;
use pathfinder_geometry::vector::vec2f;
use warp_core::ui::appearance::Appearance;
use warpui::{
    elements::{
        ConstrainedBox, Container, CrossAxisAlignment, Flex, MainAxisAlignment, MainAxisSize,
        MouseStateHandle, ParentElement,
    },
    fonts::Weight,
    ui_components::{
        button::ButtonVariant,
        components::{UiComponent, UiComponentStyles},
        text::Span,
    },
    AppContext, Element, EntityId, EventContext, SingletonEntity,
};

use crate::{
    themes::theme::{AnsiColorIdentifier, Fill, WarpTheme},
    ui_components::icons::Icon,
};

use warpui::elements::ChildAnchor;
use warpui::elements::Hoverable;
use warpui::elements::OffsetPositioning;
use warpui::elements::ParentAnchor;
use warpui::elements::ParentOffsetBounds;
use warpui::elements::Stack;

const PROVIDER_BUTTON_ICON_SIZE: f32 = 14.;
const PROVIDER_BUTTON_ICON_TEXT_GAP: f32 = 8.;

/// Text to use as a label throughout the app for user interactions that will attach selected
/// block(s) or text selections to a new AI query.
pub static ATTACH_AS_AGENT_MODE_CONTEXT_TEXT: LazyLock<&'static str> =
    LazyLock::new(|| "Attach as agent context");

/// Label we use for the the command palette action to create a new local Oz agent pane.
pub static NEW_AGENT_PANE_LABEL: LazyLock<&'static str> = LazyLock::new(|| "New Agent Pane");

/// Claude/Anthropic brand color (official brand orange #D97757).
/// Reference: https://github.com/anthropics/skills/blob/main/skills/brand-guidelines/SKILL.md
pub const CLAUDE_ORANGE: ColorU = ColorU {
    r: 0xD9,
    g: 0x77,
    b: 0x57,
    a: 0xFF,
};

/// Returns the color to be used for various AI signifiers
/// input with AI mode).
pub fn ai_brand_color(theme: &WarpTheme) -> ColorU {
    AnsiColorIdentifier::Magenta
        .to_ansi_color(&theme.terminal_colors().normal)
        .into()
}

/// Returns the color to be used for error UI throughout Agent Mode (like the "request limit
/// exceeded" chip).
pub fn error_color(theme: &WarpTheme) -> ColorU {
    AnsiColorIdentifier::Red
        .to_ansi_color(&theme.terminal_colors().normal)
        .into()
}

/// Returns the AI icon element to be rendered in AI output blocks and the terminal input when in
/// AI mode. Takes a color parameter as the solid fill for the icon. We use [ai_brand_color] in most
/// cases.
pub fn render_ai_agent_mode_icon(app: &AppContext, color: impl Into<Fill>) -> Box<dyn Element> {
    render_input_icon(Icon::AgentMode, color.into(), app)
}

/// Returns the icon element to be rendered in the terminal input when
/// the user is making a follow up AI query in an existing conversation. Takes a color parameter as the solid fill for the icon.
pub fn render_ai_follow_up_icon(
    mouse_state: MouseStateHandle,
    app: &AppContext,
) -> Box<dyn Element> {
    let appearance = Appearance::as_ref(app);
    Hoverable::new(mouse_state, |state| {
        let mut stack = Stack::new().with_child(render_input_icon(
            Icon::CornerRight,
            appearance.theme().foreground(),
            app,
        ));
        if state.is_hovered() {
            let tooltip_background = appearance.theme().tooltip_background();
            let tool_tip = appearance
                .ui_builder()
                .tool_tip(
                    crate::i18n::tr_static(app, "Follow up with existing conversation").to_owned(),
                )
                .with_style(UiComponentStyles {
                    font_size: Some(12.),
                    background: Some(warpui::elements::Fill::Solid(tooltip_background)),
                    font_color: Some(appearance.theme().background().into_solid()),
                    ..Default::default()
                });
            stack.add_positioned_overlay_child(
                tool_tip.build().finish(),
                OffsetPositioning::offset_from_parent(
                    vec2f(0., -4.),
                    ParentOffsetBounds::WindowByPosition,
                    ParentAnchor::TopLeft,
                    ChildAnchor::BottomLeft,
                ),
            );
        }
        stack.finish()
    })
    .finish()
}

fn render_input_icon(icon: Icon, color: Fill, app: &AppContext) -> Box<dyn Element> {
    // Since the icon is rendered next to monospace text content, its size should scale to
    // based on the current font size -- specifically, its height must match the editor text line
    // height.
    let icon_size = ai_indicator_height(app);
    ConstrainedBox::new(
        Container::new(icon.to_warpui_icon(color).finish())
            .with_uniform_padding(icon_size / 8.)
            .finish(),
    )
    .with_width(icon_size)
    .with_height(icon_size)
    .finish()
}

/// Returns the size to be used for the AI icon in AI output blocks and the terminal input when in
/// AI mode.
///
/// This size is computed based on the user's current font size and line height ratio, such that the
/// size of the icon matches the user's text line height.  This is necessary because the AI icon in
/// the input is rendered next to text in the editor.
pub fn ai_indicator_height(app: &AppContext) -> f32 {
    let appearance = Appearance::as_ref(app);
    app.font_cache().line_height(
        appearance.monospace_font_size(),
        appearance.line_height_ratio(),
    )
}

/// Returns the saved position ID of the attached blocks chip inside the [`AIBlock`] header.
pub fn get_attached_blocks_chip_element_position_id(view_id: EntityId) -> String {
    format!("aiblock:{view_id}.attached_block_chip_position")
}

/// Returns the saved position ID of the overflow menu inside the [`AIBlock`] header.
pub fn get_ai_block_overflow_menu_element_position_id(view_id: EntityId) -> String {
    format!("aiblock:{view_id}.overflow_menu_position")
}

/// Formats credit count to display as whole numbers when the value is effectively a whole number,
/// otherwise displays with one decimal place.
/// Returns a formatted string with proper pluralization ("credit" vs "credits").
pub fn format_credits(credits: f32) -> String {
    // If the first part of the decimal is 0, we just display the whole number.
    if credits.fract() < 0.1 {
        let whole = credits.trunc() as i32;
        if whole == 1 {
            format!("{whole} credit")
        } else {
            format!("{whole} credits")
        }
    } else {
        format!("{credits:.1} credits")
    }
}

/// Renders a secondary button with an MCP/skill provider icon and a text label.
pub(crate) fn render_provider_icon_button<F>(
    button_label: &str,
    button_handle: MouseStateHandle,
    appearance: &Appearance,
    icon: Icon,
    color: Fill,
    on_click: F,
) -> Box<dyn Element>
where
    F: FnMut(&mut EventContext) + 'static,
{
    let theme = appearance.theme();
    let font_color = theme.foreground().into_solid();
    let mut label_children = vec![ConstrainedBox::new(icon.to_warpui_icon(color).finish())
        .with_width(PROVIDER_BUTTON_ICON_SIZE)
        .with_height(PROVIDER_BUTTON_ICON_SIZE)
        .finish()];
    label_children.push(
        Container::new(
            Span::new(
                button_label.to_string(),
                UiComponentStyles {
                    font_family_id: Some(appearance.ui_font_family()),
                    font_size: Some(appearance.ui_font_size()),
                    font_weight: Some(Weight::Semibold),
                    font_color: Some(font_color),
                    ..Default::default()
                },
            )
            .build()
            .finish(),
        )
        .with_padding_left(PROVIDER_BUTTON_ICON_TEXT_GAP)
        .finish(),
    );
    let label = Flex::row()
        .with_children(label_children)
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .with_main_axis_alignment(MainAxisAlignment::Center)
        .with_main_axis_size(MainAxisSize::Min)
        .finish();
    let mut on_click = on_click;
    appearance
        .ui_builder()
        .button(ButtonVariant::Secondary, button_handle)
        .with_custom_label(label)
        .with_style(UiComponentStyles {
            font_weight: Some(Weight::Semibold),
            ..Default::default()
        })
        .build()
        .on_click(move |ctx, _, _| {
            on_click(ctx);
        })
        .finish()
}
