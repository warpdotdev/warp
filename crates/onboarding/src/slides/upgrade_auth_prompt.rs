use warp_core::ui::appearance::Appearance;
use warp_core::ui::icons::Icon;
use warpui_core::Element;
use warpui_core::elements::{
    Border, ConstrainedBox, Container, CrossAxisAlignment, Flex, MainAxisSize, MouseStateHandle,
    ParentElement,
};
use warpui_core::ui_components::components::{UiComponent as _, UiComponentStyles};
use warpui_core::ui_components::link::OnClickFn;

pub(super) fn render_upgrade_auth_prompt_bar(
    appearance: &Appearance,
    copy_url_mouse_state: MouseStateHandle,
    paste_token_mouse_state: MouseStateHandle,
    on_copy_url: OnClickFn,
    on_paste_token: OnClickFn,
) -> Box<dyn Element> {
    const BAR_HEIGHT: f32 = 40.;
    const ICON_SIZE: f32 = 14.;
    const FONT_SIZE: f32 = 12.;

    let theme = appearance.theme();
    let bar_bg = theme.surface_1();
    let bar_bg_solid = bar_bg.into_solid();
    let text_color = warp_core::ui::theme::color::internal_colors::text_sub(theme, bar_bg_solid);
    let ui_builder = appearance.ui_builder();

    let text_styles = UiComponentStyles {
        font_color: Some(text_color),
        font_size: Some(FONT_SIZE),
        ..Default::default()
    };
    let link_styles = UiComponentStyles {
        font_size: Some(FONT_SIZE),
        ..Default::default()
    };

    let icon = ConstrainedBox::new(Box::new(
        Icon::AlertCircle.to_warpui_icon(text_color.into()),
    ))
    .with_width(ICON_SIZE)
    .with_height(ICON_SIZE)
    .finish();

    let copy_url_link = ui_builder
        .link(
            "copy the URL".into(),
            None,
            Some(on_copy_url),
            copy_url_mouse_state,
        )
        .soft_wrap(false)
        .with_style(link_styles)
        .build()
        .finish();

    let paste_token_link = ui_builder
        .link(
            "Click here".into(),
            None,
            Some(on_paste_token),
            paste_token_mouse_state,
        )
        .soft_wrap(false)
        .with_style(link_styles)
        .build()
        .finish();

    let text_row = Flex::row()
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .with_child(icon)
        .with_child(
            Container::new(
                ui_builder
                    .span("If your browser hasn't launched, ")
                    .with_style(text_styles)
                    .build()
                    .finish(),
            )
            .with_margin_left(8.)
            .finish(),
        )
        .with_child(copy_url_link)
        .with_child(
            ui_builder
                .span(" and open the page manually. ")
                .with_style(text_styles)
                .build()
                .finish(),
        )
        .with_child(paste_token_link)
        .with_child(
            ui_builder
                .span(" to paste your token from the browser.")
                .with_style(text_styles)
                .build()
                .finish(),
        )
        .finish();

    let row = Flex::row()
        .with_main_axis_size(MainAxisSize::Max)
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .with_child(text_row)
        .finish();

    ConstrainedBox::new(
        Container::new(row)
            .with_background(bar_bg)
            .with_border(Border::top(1.).with_border_color(
                warp_core::ui::theme::color::internal_colors::neutral_4(theme),
            ))
            .with_horizontal_padding(16.)
            .finish(),
    )
    .with_min_height(BAR_HEIGHT)
    .finish()
}
