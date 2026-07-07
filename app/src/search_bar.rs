use warpui::elements::{
    Border, ChildView, Clipped, ConstrainedBox, Container, CornerRadius, CrossAxisAlignment,
    DispatchEventResult, EventHandler, Flex, Icon, ParentElement, Radius, Shrinkable,
};
use warpui::ui_components::components::{Coords, UiComponentStyles};
use warpui::{Element, Entity, SingletonEntity, TypedActionView, View, ViewContext, ViewHandle};

use crate::appearance::Appearance;
use crate::editor::EditorView;

const SEARCH_ICON_PATH: &str = "bundled/svg/search.svg";
const ICON_SIZE: f32 = 12.;

pub struct SearchBar {
    editor: ViewHandle<EditorView>,
    custom_styles: UiComponentStyles,
}

#[derive(Debug)]
pub enum SearchBarAction {
    SearchBarClicked,
}

pub enum SearchBarEvent {}

impl TypedActionView for SearchBar {
    type Action = SearchBarAction;

    fn handle_action(&mut self, action: &SearchBarAction, ctx: &mut ViewContext<Self>) {
        match action {
            SearchBarAction::SearchBarClicked => self.focus_search_bar(ctx),
        }
    }
}

impl Entity for SearchBar {
    type Event = SearchBarEvent;
}

impl View for SearchBar {
    fn ui_name() -> &'static str {
        "SearchBar"
    }

    fn render(&self, app: &warpui::AppContext) -> Box<dyn Element> {
        let styles = {
            let appearance = Appearance::as_ref(app);
            let context_styles = UiComponentStyles {
                font_size: Some(appearance.ui_font_size()),
                font_family_id: Some(appearance.ui_font_family()),
                padding: Some(Coords {
                    top: 10.,
                    bottom: 10.,
                    left: 10.,
                    right: 10.,
                }),
                border_width: Some(1.),
                border_radius: Some(CornerRadius::with_all(Radius::Pixels(3.))),
                background: Some(appearance.theme().background().into()),
                border_color: Some(appearance.theme().foreground().with_opacity(20).into()),
                font_color: Some(
                    appearance
                        .theme()
                        .main_text_color(appearance.theme().surface_2())
                        .into(),
                ),
                ..Default::default()
            };
            context_styles.merge(self.custom_styles)
        };

        let search_icon = Icon::new(
            SEARCH_ICON_PATH,
            styles.font_color.expect("Should get font colour"),
        );
        let search_bar_element = Container::new(
            ConstrainedBox::new(search_icon.finish())
                .with_height(ICON_SIZE)
                .with_width(ICON_SIZE)
                .finish(),
        )
        .with_padding_right(5.)
        .finish();

        let mut container = Container::new(
            Clipped::new(
                Flex::row()
                    .with_child(search_bar_element)
                    .with_child(
                        Clipped::new(
                            Shrinkable::new(1., ChildView::new(&self.editor).finish()).finish(),
                        )
                        .finish(),
                    )
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .finish(),
            )
            .finish(),
        );

        // Setting up the border
        if let Some(corner) = styles.border_radius {
            container = container.with_corner_radius(corner);
        }

        let mut border = Border::all(styles.border_width.unwrap_or_default());
        if let Some(border_color) = styles.border_color {
            border = border.with_border_fill(border_color);
        }
        container = container.with_border(border);

        // Position-related settings
        if let Some(padding) = styles.padding {
            container = container
                .with_padding_left(padding.left)
                .with_padding_top(padding.top)
                .with_padding_right(padding.right)
                .with_padding_bottom(padding.bottom);
        }
        if let Some(margin) = styles.margin {
            container = container
                .with_margin_left(margin.left)
                .with_margin_top(margin.top)
                .with_margin_right(margin.right)
                .with_margin_bottom(margin.bottom);
        }

        if let Some(background) = styles.background {
            container = container.with_background(background);
        }

        let container_with_event_handler = EventHandler::new(container.finish())
            .on_left_mouse_down(move |ctx, _, _| {
                ctx.dispatch_typed_action(SearchBarAction::SearchBarClicked);
                DispatchEventResult::StopPropagation
            })
            .finish();

        match (styles.height, styles.width) {
            (None, None) => ConstrainedBox::new(container_with_event_handler),
            (_, _) => {
                let mut constrained_box = ConstrainedBox::new(container_with_event_handler);
                if let Some(height) = styles.height {
                    constrained_box = constrained_box.with_height(height);
                }
                if let Some(width) = styles.width {
                    constrained_box = constrained_box.with_width(width);
                }
                constrained_box
            }
        }
        .finish()
    }
}

impl SearchBar {
    pub fn new(search_editor: ViewHandle<EditorView>) -> Self {
        SearchBar {
            editor: search_editor,
            custom_styles: UiComponentStyles::default(),
        }
    }

    pub fn with_style(&mut self, styles: UiComponentStyles) {
        self.custom_styles = self.custom_styles.merge(styles);
    }

    pub fn focus_search_bar(&mut self, ctx: &mut ViewContext<Self>) {
        ctx.focus(&self.editor);
        ctx.notify();
    }
}
