use crate::elements::{Border, Container, Element, Hoverable, MouseState, MouseStateHandle, Text};
use crate::fonts::Properties;
use crate::platform::Cursor;
use crate::ui_components::components::{UiComponent, UiComponentStyles};
use crate::EventContext;

pub type OnClickFn = Box<dyn Fn(&mut EventContext)>;

pub struct Link {
    text: String, // TODO figure out how it can be ui element (or icon?)
    /// A URL that should be opened in the user's default web browser when clicked.
    url: Option<String>,
    /// A callback that should be fired when clicked.
    /// Commonly dispatches an action from the calling view.
    callback: Option<OnClickFn>,
    styles: LinkStyles,
    hover_state: MouseStateHandle,
}

#[derive(Copy, Clone)]
pub struct LinkStyles {
    pub base: UiComponentStyles,
    pub hovered: Option<UiComponentStyles>,
    pub clicked: Option<UiComponentStyles>,
    pub soft_wrap: bool,
}

impl LinkStyles {
    fn merge(&self, style: UiComponentStyles) -> Self {
        Self {
            base: self.base.merge(style),
            hovered: Some(self.hovered.unwrap_or(self.base).merge(style)),
            clicked: Some(self.clicked.unwrap_or(self.base).merge(style)),
            soft_wrap: self.soft_wrap,
        }
    }
}

impl UiComponent for Link {
    type ElementType = Container;
    fn build(self) -> Container {
        let url = self.url.clone();
        Container::new(
            Hoverable::new(self.hover_state.clone(), |state| {
                let styles = self.styles(state);
                let mut text = Text::new(
                    self.text.clone(),
                    styles.font_family_id.unwrap(),
                    styles.font_size.unwrap_or(14.),
                )
                .soft_wrap(self.styles.soft_wrap);

                if let Some(font_color) = styles.font_color {
                    text = text.with_color(font_color);
                }

                if let Some(weight) = styles.font_weight {
                    text = text.with_style(Properties::default().weight(weight));
                }

                match (styles.border_width, styles.border_color) {
                    (Some(border_width), Some(border_color)) => Container::new(text.finish())
                        .with_border(Border::bottom(border_width).with_border_fill(border_color))
                        // Pull down the element by 1px so that the 1px border doesn't affect the
                        // vertical positioning of the element.  Without this, the text won't be
                        // vertically aligned with neighboring `Text` elements.
                        .with_margin_bottom(-border_width)
                        .finish(),
                    (_, _) => text.finish(),
                }
            })
            .on_click(move |ctx, app, _| {
                if let Some(url) = &url {
                    app.open_url(url);
                }

                if let Some(callback) = &self.callback {
                    callback(ctx);
                }
            })
            .with_cursor(Cursor::PointingHand)
            .finish(),
        )
    }

    /// Overwrites _some_ styles passed in `style` parameter
    fn with_style(self, styles: UiComponentStyles) -> Self {
        Link {
            text: self.text.clone(),
            url: self.url,
            callback: self.callback,
            styles: self.styles.merge(styles),
            hover_state: self.hover_state,
        }
    }
}

impl Link {
    pub fn new(
        text: String,
        url: Option<String>,
        callback: Option<OnClickFn>,
        mouse_state: MouseStateHandle,
        styles: LinkStyles,
    ) -> Self {
        Link {
            text,
            url,
            callback,
            styles,
            hover_state: mouse_state,
        }
    }

    fn styles(&self, state: &MouseState) -> UiComponentStyles {
        if state.is_hovered() {
            if state.is_clicked() {
                return self.styles.clicked.unwrap_or(self.styles.base);
            }
            return self.styles.hovered.unwrap_or(self.styles.base);
        }
        self.styles.base
    }

    pub fn with_hovered_style(mut self, hover_style: UiComponentStyles) -> Self {
        if let Some(style) = &mut self.styles.hovered {
            *style = style.merge(hover_style);
        }
        self
    }

    pub fn with_clicked_style(mut self, hover_style: UiComponentStyles) -> Self {
        if let Some(style) = &mut self.styles.clicked {
            *style = style.merge(hover_style);
        }
        self
    }

    pub fn soft_wrap(mut self, soft_wrap: bool) -> Self {
        self.styles.soft_wrap = soft_wrap;
        self
    }
}
