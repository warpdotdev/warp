//! Shared rendering for chip editors that use a left/right zones layout.
//!
//! The agent toolbar editor and header toolbar editor share the same editable
//! chip sections (available bank, left/right drop zones, restore-default link).
//! Modal consumers wrap those sections in a title, cancel/save buttons, and blur
//! overlay, while settings consumers can render the sections inline.

use pathfinder_geometry::vector::vec2f;
use warp_core::ui::theme::Fill;
use warpui::elements::{
    Align, Border, ChildAnchor, ConstrainedBox, Container, CornerRadius, CrossAxisAlignment,
    Expanded, Flex, Hoverable, MainAxisAlignment, MainAxisSize, MouseStateHandle,
    OffsetPositioning, ParentAnchor, ParentElement, ParentOffsetBounds, Radius, Stack,
};
use warpui::platform::Cursor;
use warpui::ui_components::button::ButtonVariant;
use warpui::ui_components::components::{Coords, UiComponent, UiComponentStyles};
use warpui::{Action, Element};

use super::{ChipConfigurator, ChipConfiguratorAction};
use crate::Appearance;

const MODAL_WIDTH: f32 = 700.;
const BORDER_WIDTH: f32 = 1.;
const MODAL_TITLE_FONT_SIZE: f32 = 16.;
const MODAL_UNIFORM_PADDING: f32 = 24.;
const CORNER_RADIUS_PIXELS: f32 = 8.;
const PRIMARY_BUTTON_HEIGHT: f32 = 40.;
const SECTION_UNIFORM_PADDING: f32 = 16.;
const MARGIN_BETWEEN_MODAL_SECTIONS: f32 = 16.;
const MODAL_CONTENT_FONT_SIZE: f32 = 14.;
const RESTORE_DEFAULT_LABEL: &str = "Restore default";

/// Mouse state handles for interactive controls in chip editor sections and modals.
#[derive(Default)]
pub struct ChipEditorMouseHandles {
    pub cancel: MouseStateHandle,
    pub save: MouseStateHandle,
    pub restore_default: MouseStateHandle,
}
/// Everything that varies between chip editor modal-shell consumers.
pub struct ChipEditorModalConfig<'a, A> {
    pub title: &'a str,
    pub available_section_label: &'a str,
    pub is_at_defaults: bool,
    pub is_dirty: bool,
    pub cancel_action: A,
    pub save_action: A,
    pub reset_action: A,
    pub activate_action: A,
    pub chip_action_wrapper: fn(ChipConfiguratorAction) -> A,
    pub mouse_handles: &'a ChipEditorMouseHandles,
}
/// Everything needed to render the editable chip sections without the modal shell.
pub struct ChipEditorSectionsConfig<'a, A> {
    pub available_section_label: &'a str,
    pub is_at_defaults: bool,
    pub reset_action: A,
    pub activate_action: A,
    pub chip_action_wrapper: fn(ChipConfiguratorAction) -> A,
    pub mouse_handles: &'a ChipEditorMouseHandles,
}

/// Render a complete chip editor modal: blur overlay, centered card with title,
/// chip sections (available bank + left/right drop zones), and cancel/save buttons.
pub fn render_chip_editor_modal<A: Action + Clone + Copy + 'static>(
    chip_configurator: &ChipConfigurator,
    config: ChipEditorModalConfig<A>,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let theme = appearance.theme();

    let column = Flex::column()
        .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
        .with_child(
            Container::new(render_header(config.title, appearance))
                .with_margin_bottom(MARGIN_BETWEEN_MODAL_SECTIONS)
                .finish(),
        )
        .with_child(
            Container::new(render_chip_editor_sections(
                chip_configurator,
                ChipEditorSectionsConfig {
                    available_section_label: config.available_section_label,
                    is_at_defaults: config.is_at_defaults,
                    reset_action: config.reset_action,
                    activate_action: config.activate_action,
                    chip_action_wrapper: config.chip_action_wrapper,
                    mouse_handles: config.mouse_handles,
                },
                appearance,
            ))
            .with_margin_bottom(MARGIN_BETWEEN_MODAL_SECTIONS)
            .finish(),
        )
        .with_child(render_buttons(&config, appearance))
        .finish();

    let modal = Container::new(
        ConstrainedBox::new(column)
            .with_max_width(MODAL_WIDTH)
            .finish(),
    )
    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(CORNER_RADIUS_PIXELS)))
    .with_border(Border::all(BORDER_WIDTH).with_border_fill(theme.outline()))
    .with_background(theme.surface_1())
    .with_uniform_padding(MODAL_UNIFORM_PADDING)
    .with_margin_top(35.)
    .finish();

    let mut stack = Stack::new();
    stack.add_positioned_child(
        modal,
        OffsetPositioning::offset_from_parent(
            vec2f(0., 0.),
            ParentOffsetBounds::WindowByPosition,
            ParentAnchor::Center,
            ChildAnchor::Center,
        ),
    );

    Container::new(Align::new(stack.finish()).finish())
        .with_background_color(Fill::blur().into())
        .finish()
}

fn render_header(title: &str, appearance: &Appearance) -> Box<dyn Element> {
    appearance
        .ui_builder()
        .span(title.to_string())
        .with_style(UiComponentStyles {
            font_size: Some(MODAL_TITLE_FONT_SIZE),
            font_weight: Some(warpui::fonts::Weight::Bold),
            ..Default::default()
        })
        .build()
        .finish()
}

fn render_restore_default_button<A: Action + Clone + Copy + 'static>(
    is_at_defaults: bool,
    reset_action: A,
    mouse_handle: &MouseStateHandle,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let button = Hoverable::new(mouse_handle.clone(), |_state| {
        appearance
            .ui_builder()
            .span(RESTORE_DEFAULT_LABEL.to_string())
            .with_style(UiComponentStyles {
                font_size: Some(MODAL_CONTENT_FONT_SIZE),
                ..Default::default()
            })
            .build()
            .finish()
    })
    .on_click(move |ctx, _, _| ctx.dispatch_typed_action(reset_action))
    .with_cursor(Cursor::PointingHand);

    if is_at_defaults {
        button.disable().finish()
    } else {
        button.finish()
    }
}

fn render_section_label(label: &str, appearance: &Appearance) -> Box<dyn Element> {
    appearance
        .ui_builder()
        .span(label.to_string())
        .with_style(UiComponentStyles {
            font_size: Some(MODAL_CONTENT_FONT_SIZE),
            font_weight: Some(warpui::fonts::Weight::Semibold),
            ..Default::default()
        })
        .build()
        .finish()
}

/// Render the editable chip sections that are shared by inline editors and modals.
pub fn render_chip_editor_sections<A: Action + Clone + Copy + 'static>(
    chip_configurator: &ChipConfigurator,
    config: ChipEditorSectionsConfig<A>,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let header_row = Flex::row()
        .with_child(render_section_label(
            config.available_section_label,
            appearance,
        ))
        .with_child(render_restore_default_button(
            config.is_at_defaults,
            config.reset_action,
            &config.mouse_handles.restore_default,
            appearance,
        ))
        .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
        .with_main_axis_size(MainAxisSize::Max)
        .finish();

    let unused_chips = chip_configurator.render_unused_chips_bank(
        config.activate_action,
        config.chip_action_wrapper,
        appearance,
    );

    let left_section = Flex::column()
        .with_child(render_section_label("Left side", appearance))
        .with_child(
            Container::new(chip_configurator.render_left_drop_zone(
                config.activate_action,
                config.chip_action_wrapper,
                appearance,
            ))
            .with_margin_top(8.)
            .finish(),
        )
        .finish();

    let right_section = Flex::column()
        .with_child(render_section_label("Right side", appearance))
        .with_child(
            Container::new(chip_configurator.render_right_drop_zone(
                config.activate_action,
                config.chip_action_wrapper,
                appearance,
            ))
            .with_margin_top(8.)
            .finish(),
        )
        .finish();

    let drop_zones = Flex::row()
        .with_child(Box::new(Expanded::new(1.0, left_section)))
        .with_child(Box::new(Expanded::new(
            1.0,
            Container::new(right_section).with_margin_left(8.).finish(),
        )))
        .with_main_axis_size(MainAxisSize::Max)
        .finish();

    Container::new(
        Flex::column()
            .with_child(header_row)
            .with_child(Container::new(unused_chips).with_margin_top(10.).finish())
            .with_child(Container::new(drop_zones).with_margin_top(10.).finish())
            .finish(),
    )
    .with_uniform_padding(SECTION_UNIFORM_PADDING)
    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(CORNER_RADIUS_PIXELS)))
    .with_background(appearance.theme().surface_2())
    .finish()
}

fn render_primary_button<A: Action + Clone + Copy + 'static>(
    label: String,
    variant: ButtonVariant,
    disabled: bool,
    mouse_state_handle: &MouseStateHandle,
    on_click_action: A,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let padding = Coords {
        top: 10.,
        bottom: 10.,
        right: 140.,
        left: 140.,
    };

    let mut button = appearance
        .ui_builder()
        .button(variant, mouse_state_handle.clone())
        .with_text_label(label)
        .with_style(UiComponentStyles {
            padding: Some(padding),
            font_size: Some(MODAL_CONTENT_FONT_SIZE),
            ..Default::default()
        });

    if disabled {
        button = button.disabled();
    }

    button
        .build()
        .on_click(move |ctx, _, _| ctx.dispatch_typed_action(on_click_action))
        .with_cursor(Cursor::PointingHand)
        .finish()
}

fn render_buttons<A: Action + Clone + Copy + 'static>(
    config: &ChipEditorModalConfig<A>,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let cancel_button = render_primary_button(
        "Cancel".to_string(),
        ButtonVariant::Outlined,
        false,
        &config.mouse_handles.cancel,
        config.cancel_action,
        appearance,
    );

    let save_button = render_primary_button(
        "Save changes".to_string(),
        ButtonVariant::Accent,
        !config.is_dirty,
        &config.mouse_handles.save,
        config.save_action,
        appearance,
    );

    Flex::row()
        .with_main_axis_size(MainAxisSize::Max)
        .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
        .with_child(
            ConstrainedBox::new(cancel_button)
                .with_height(PRIMARY_BUTTON_HEIGHT)
                .finish(),
        )
        .with_child(
            ConstrainedBox::new(Container::new(save_button).with_margin_left(5.).finish())
                .with_height(PRIMARY_BUTTON_HEIGHT)
                .finish(),
        )
        .finish()
}
