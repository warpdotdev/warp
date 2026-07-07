use pathfinder_color::ColorU;
use warpui::elements::{
    ConstrainedBox, Container, Element, Empty, Flex, MouseStateHandle, ParentElement, Shrinkable,
};
use warpui::ui_components::components::{UiComponent, UiComponentStyles};
use warpui::{AppContext, Entity, SingletonEntity, TypedActionView, View, ViewContext};

use super::{
    SectionAction, SectionView, CHEVRON_ICON_SIZE, DESCRIPTION_FONT_SIZE, ICON_PADDING,
    ITEM_PADDING_BOTTOM, SECTION_SPACING,
};
use crate::appearance::Appearance;
use crate::resource_center::{ContentItem, ContentSectionData};
use crate::util::color::{ContrastingColor, MinimumAllowedContrast};

#[derive(Default)]
struct ContentMouseStateHandles {
    item_handles: Vec<MouseStateHandle>,
    top_bar_mouse_state: MouseStateHandle,
}

pub struct ContentSectionView {
    content_section_data: ContentSectionData,
    content_button_mouse_states: ContentMouseStateHandles,
    is_expanded: bool,
}

impl Entity for ContentSectionView {
    type Event = ();
}

impl TypedActionView for ContentSectionView {
    type Action = SectionAction;

    fn handle_action(&mut self, action: &SectionAction, ctx: &mut ViewContext<Self>) {
        use SectionAction::*;
        match action {
            OpenUrl(url) => {
                ctx.open_url(url.as_str());
            }
            ToggleExpanded => self.toggle_expanded(ctx),
            _ => {}
        }
    }
}

impl ContentSectionView {
    pub fn new(
        content_section_data: ContentSectionData,
        is_expanded: bool,
        _ctx: &mut ViewContext<Self>,
    ) -> Self {
        let content_button_mouse_states = ContentMouseStateHandles {
            item_handles: content_section_data
                .items
                .iter()
                .map(|_| Default::default())
                .collect(),
            ..Default::default()
        };

        Self {
            content_section_data,
            content_button_mouse_states,
            is_expanded,
        }
    }

    fn render_link_button(
        &self,
        item: &ContentItem,
        appearance: &Appearance,
        mouse_state_handle: MouseStateHandle,
    ) -> Box<dyn Element> {
        let theme = appearance.theme();
        let default_link_styles = UiComponentStyles {
            font_size: Some(13.),
            font_family_id: Some(appearance.ui_font_family()),
            font_color: Some(ColorU::from(
                theme
                    .accent()
                    .on_background(theme.surface_2(), MinimumAllowedContrast::Text),
            )),
            ..Default::default()
        };

        let hovered_and_clicked_styles = UiComponentStyles {
            font_color: Some(ColorU::from(theme.active_ui_text_color())),
            ..default_link_styles
        };

        Flex::row()
            .with_child(
                appearance
                    .ui_builder()
                    .link(
                        item.button_label.to_string(),
                        Some(item.url.into()),
                        None,
                        mouse_state_handle,
                    )
                    .soft_wrap(false)
                    .with_style(default_link_styles)
                    .with_hovered_style(hovered_and_clicked_styles)
                    .with_clicked_style(hovered_and_clicked_styles)
                    .build()
                    .finish(),
            )
            .with_child(Shrinkable::new(1., Empty::new().finish()).finish())
            .finish()
    }

    fn render_content_item(
        &self,
        item: &ContentItem,
        appearance: &Appearance,
        index: usize,
    ) -> Box<dyn Element> {
        let mut element = Flex::column();
        let mouse_state = self.content_button_mouse_states.item_handles[index].clone();
        let link_button = self.render_link_button(item, appearance, mouse_state);

        // title
        element.add_child(
            Container::new(
                appearance
                    .ui_builder()
                    .wrappable_text(item.title.to_string(), true)
                    .with_style(UiComponentStyles {
                        font_size: Some(DESCRIPTION_FONT_SIZE),
                        ..Default::default()
                    })
                    .build()
                    .finish(),
            )
            .with_padding_bottom(ITEM_PADDING_BOTTOM)
            .finish(),
        );

        // description
        element.add_child(
            Container::new(
                appearance
                    .ui_builder()
                    .wrappable_text(item.description.to_string(), true)
                    .with_style(UiComponentStyles {
                        font_size: Some(DESCRIPTION_FONT_SIZE),
                        font_color: Some(ColorU::from(
                            appearance.theme().nonactive_ui_text_color(),
                        )),
                        ..Default::default()
                    })
                    .build()
                    .finish(),
            )
            .with_padding_bottom(ITEM_PADDING_BOTTOM)
            .finish(),
        );

        // link
        element.add_child(link_button);

        Container::new(element.finish())
            .with_margin_bottom(SECTION_SPACING)
            .finish()
    }
}

impl SectionView for ContentSectionView {
    fn is_expanded(&self) -> bool {
        self.is_expanded
    }

    fn toggle_expanded(&mut self, ctx: &mut ViewContext<Self>) {
        self.is_expanded = !self.is_expanded;
        ctx.notify();
    }

    fn section_progress_indicator(
        &self,
        _show_gamified: bool,
        _appearance: &Appearance,
        _ctx: &AppContext,
    ) -> Option<Box<dyn Element>> {
        None
    }

    fn section_link(&self, _appearance: &Appearance) -> Option<Box<dyn Element>> {
        None
    }
}

impl View for ContentSectionView {
    fn ui_name() -> &'static str {
        "ResourceCenterContentSectionView"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let header = self.render_section_header(
            self.content_section_data.section_name,
            false,
            appearance,
            self.content_button_mouse_states.top_bar_mouse_state.clone(),
            app,
        );

        let mut section = Flex::column().with_child(header);
        if self.is_expanded {
            let content_section =
                Container::new(
                    Flex::column()
                        .with_children(
                            self.content_section_data.items.iter().enumerate().map(
                                |(index, item)| self.render_content_item(item, appearance, index),
                            ),
                        )
                        .finish(),
                )
                .with_uniform_margin(SECTION_SPACING)
                .with_margin_left(SECTION_SPACING + CHEVRON_ICON_SIZE + ICON_PADDING);

            section.add_child(content_section.finish());
        }

        ConstrainedBox::new(Container::new(section.finish()).finish()).finish()
    }
}
