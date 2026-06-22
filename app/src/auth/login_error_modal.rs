use std::borrow::Cow;

use pathfinder_color::ColorU;
use warpui::elements::{
    Align, Border, ConstrainedBox, Container, CornerRadius, Flex, ParentElement, Shrinkable,
};
use warpui::ui_components::components::{UiComponent, UiComponentStyles};
use warpui::ui_components::text::Span;
use warpui::{AppContext, Element, SingletonEntity as _};

use crate::appearance::Appearance;
use crate::modal::MODAL_CORNER_RADIUS;
use crate::root_view::unthemed_window_border;
use crate::themes::theme::{Blend, Fill};

/// A full-window login error.
///
/// This is used for uncommon login error states, such as:
/// * A user needing to link SSO after logging in with an incorrect Firebase provider.
/// * An error importing the user from a host web application.
pub struct LoginErrorModal {
    modal_styles: UiComponentStyles,
    header_styles: UiComponentStyles,
    header: Option<Cow<'static, str>>,
    detail_styles: UiComponentStyles,
    detail: Option<Cow<'static, str>>,

    action: Option<Box<dyn Element>>,

    window_corner_radius: CornerRadius,
}

impl LoginErrorModal {
    pub fn new(app: &AppContext) -> Self {
        let appearance = Appearance::as_ref(app);
        let modal_styles = UiComponentStyles {
            width: Some(480.),
            height: Some(280.),
            border_color: Some(Fill::black().blend(&Fill::white().with_opacity(15)).into()),
            border_width: Some(1.),
            ..Default::default()
        };
        let header_styles = UiComponentStyles {
            font_family_id: Some(appearance.ui_font_family()),
            font_size: Some(appearance.header_font_size()),
            ..Default::default()
        };
        let detail_styles = UiComponentStyles {
            font_family_id: Some(appearance.ui_font_family()),
            font_size: Some(appearance.ui_font_size()),
            font_color: Some(appearance.theme().nonactive_ui_text_color().into()),
            ..Default::default()
        };

        LoginErrorModal {
            modal_styles,
            header_styles,
            detail_styles,
            window_corner_radius: app.windows().window_corner_radius(),
            header: None,
            detail: None,
            action: None,
        }
    }

    pub fn with_header(mut self, header: impl Into<Cow<'static, str>>) -> Self {
        self.header = Some(header.into());
        self
    }

    pub fn with_detail(mut self, detail: impl Into<Cow<'static, str>>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    pub fn with_action(mut self, action: Box<dyn Element>) -> Self {
        self.action = Some(action);
        self
    }
}

impl UiComponent for LoginErrorModal {
    type ElementType = Container;

    fn build(self) -> Self::ElementType {
        let mut contents = Flex::column();
        if let Some(header) = self.header {
            contents.add_child(
                Shrinkable::new(
                    1.,
                    Align::new(Span::new(header, self.header_styles).build().finish()).finish(),
                )
                .finish(),
            );
        }
        if let Some(detail) = self.detail {
            contents.add_child(
                Shrinkable::new(
                    1.,
                    Align::new(Span::new(detail, self.detail_styles).build().finish()).finish(),
                )
                .finish(),
            );
        }
        if let Some(action) = self.action {
            contents.add_child(Shrinkable::new(1., Align::new(action).finish()).finish());
        }
        let modal = Container::new(
            ConstrainedBox::new(contents.finish())
                .with_width(self.modal_styles.width.unwrap_or_default())
                .with_height(self.modal_styles.height.unwrap_or_default())
                .finish(),
        )
        .with_border(
            Border::all(self.modal_styles.border_width.unwrap_or_default())
                .with_border_fill(self.modal_styles.border_color.unwrap_or_default()),
        )
        .with_corner_radius(CornerRadius::with_all(MODAL_CORNER_RADIUS))
        .finish();

        Container::new(Align::new(modal).finish())
            .with_background_color(ColorU::black())
            .with_corner_radius(self.window_corner_radius)
            .with_border(unthemed_window_border())
    }

    fn with_style(mut self, style: UiComponentStyles) -> Self {
        self.modal_styles = self.modal_styles.merge(style);
        self.header_styles = self.header_styles.merge(style);
        self.detail_styles = self.detail_styles.merge(style);
        self
    }
}
