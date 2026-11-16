use warp_core::ui::appearance::Appearance;
use warpui::elements::{
    Border, Container, CrossAxisAlignment, Element, Empty, Fill, Flex, MouseStateHandle,
    ParentElement,
};
use warpui::ui_components::button::ButtonVariant;
use warpui::ui_components::components::{Coords, UiComponent, UiComponentStyles};

use crate::ui_components::blended_colors;

pub struct SettingsTab {
    pub label: String,
    pub mouse_state: MouseStateHandle,
}

impl SettingsTab {
    pub fn new(label: impl Into<String>, mouse_state: MouseStateHandle) -> Self {
        Self {
            label: label.into(),
            mouse_state,
        }
    }
}

/// Render a tab selector with a row of tabs and a bottom border indicator.
pub fn render_tab_selector<F>(
    tabs: Vec<SettingsTab>,
    selected_label: &str,
    on_select: F,
    appearance: &Appearance,
) -> Box<dyn Element>
where
    // The on select function will take in the click event context and the selected label,
    // and will then presumably change the passed in selected label.
    F: Fn(&str, &mut warpui::EventContext) + 'static + Clone,
{
    let mut tabs_row = Flex::row()
        .with_spacing(12.)
        .with_cross_axis_alignment(CrossAxisAlignment::End);

    for tab in tabs {
        let is_selected = tab.label == selected_label;

        let tab_button_styles = UiComponentStyles {
            font_color: Some(if is_selected {
                appearance.theme().active_ui_text_color().into()
            } else {
                blended_colors::text_sub(appearance.theme(), appearance.theme().surface_1())
            }),
            font_size: Some(16.),
            padding: Some(Coords {
                top: 4.,
                bottom: 4.,
                left: 0.,
                right: 0.,
            }),
            ..Default::default()
        };

        let on_select_clone = on_select.clone();
        let button = appearance
            .ui_builder()
            .button(ButtonVariant::Link, tab.mouse_state)
            .with_style(tab_button_styles)
            .with_text_label(tab.label.clone())
            .build()
            .on_click(move |ctx, _, _| {
                on_select_clone(&tab.label, ctx);
            })
            .finish();

        let tab_with_border = Container::new(button)
            .with_border(Border::bottom(2.).with_border_fill(if is_selected {
                Fill::Solid(appearance.theme().accent().into_solid())
            } else {
                Fill::None
            }))
            .finish();

        tabs_row = tabs_row.with_child(tab_with_border);
    }

    let separator = Container::new(Empty::new().finish())
        .with_border(Border::bottom(2.).with_border_fill(appearance.theme().outline()))
        .with_margin_top(-2.);

    Container::new(
        Flex::column()
            .with_child(tabs_row.finish())
            .with_child(separator.finish())
            .finish(),
    )
    .with_margin_bottom(16.)
    .finish()
}
