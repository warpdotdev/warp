use crate::ui_components::blended_colors;
use crate::{appearance::Appearance, themes::theme::Fill, ui_components::icons};
use pathfinder_geometry::vector::vec2f;
use warpui::{
    color::ColorU,
    elements::{
        Align, Border, ChildAnchor, ChildView, ConstrainedBox, Container, CornerRadius,
        CrossAxisAlignment, Dismiss, Element, Flex, MouseStateHandle, OffsetPositioning,
        ParentAnchor, ParentElement, ParentOffsetBounds, Radius, Shrinkable, Stack, Text,
    },
    fonts::{Properties, Weight},
    keymap::{FixedBinding, Keystroke},
    ui_components::components::{Coords, UiComponent, UiComponentStyles},
    AppContext, Entity, FocusContext, ModelHandle, SingletonEntity, TypedActionView, View,
    ViewContext, ViewHandle,
};

pub const MODAL_CORNER_RADIUS: Radius = Radius::Pixels(8.);
pub const MODAL_WIDTH: f32 = 440.;
pub const MODAL_HEADER_HEIGHT: f32 = 70.;
pub const MODAL_PADDING: f32 = 28.;
pub const MODAL_BACKDROP_OPACITY: u8 = 179;

#[derive(Clone)]
pub struct Modal<T> {
    title: Option<String>,
    header_icon: Option<icons::Icon>,
    header_icon_color: Option<Fill>,
    body: ViewHandle<T>,
    show_close_modal_button: bool,
    dismiss_on_click: bool,
    modal_styles: UiComponentStyles,
    header_styles: UiComponentStyles,
    body_styles: UiComponentStyles,
    close_modal_hover_state: MouseStateHandle,
    background_opacity: u8,
    offset_positioning: OffsetPositioning,
    /// Optional keystroke to display alongside the close button.
    dismiss_keystroke: Option<Keystroke>,
}

#[derive(Clone, Debug, Default)]
pub enum ModalState {
    Open,
    #[default]
    Closed,
}

/// Helper struct that holds the view handle and the state of the "handled" modal.
/// It's supposed to be used within places like Input or Workspace, where there are multiple
/// internal views for which we want the owner to decide whether it's open or closed  (instead of
/// multiplying the amount of members by adding extra booleans to hold that state).
pub struct ModalViewState<T> {
    pub view: ViewHandle<T>,
    state: ModalState,
}

impl<T: View> ModalViewState<T> {
    pub fn new(view: ViewHandle<T>) -> Self {
        Self {
            view,
            state: Default::default(),
        }
    }

    pub fn is_open(&self) -> bool {
        matches!(self.state, ModalState::Open)
    }

    pub fn open(&mut self) {
        self.state = ModalState::Open;
    }

    pub fn close(&mut self) {
        self.state = ModalState::Closed;
    }

    pub fn render(&self) -> Box<dyn Element> {
        ChildView::new(&self.view).finish()
    }
}

impl<T> Clone for ModalViewState<T> {
    fn clone(&self) -> Self {
        Self {
            view: self.view.clone(),
            state: self.state.clone(),
        }
    }
}

#[derive(Debug)]
pub enum ModalAction {
    Close,
}

pub fn init(app: &mut AppContext) {
    use warpui::keymap::macros::*;

    app.register_fixed_bindings(vec![FixedBinding::new(
        "escape",
        ModalAction::Close,
        id!("Modal"),
    )]);
}

#[derive(PartialEq, Eq, Debug)]
pub enum ModalEvent {
    Close,
}

/// A generic modal view that renders any view as its body.
// TODO(alokedesai): Consolidate all the various modals in our app to use this view.
impl<T: View> Modal<T> {
    pub fn new(title: Option<String>, body: ViewHandle<T>, ctx: &mut ViewContext<Self>) -> Self {
        let appearance_handle = Appearance::handle(ctx);
        ctx.observe(&appearance_handle, Self::handle_appearance_update);

        let appearance = appearance_handle.as_ref(ctx);
        let theme = appearance.theme();
        Self {
            title,
            header_icon: None,
            header_icon_color: None,
            body,
            show_close_modal_button: true,
            dismiss_on_click: false,
            modal_styles: UiComponentStyles {
                border_radius: Some(CornerRadius::with_all(MODAL_CORNER_RADIUS)),
                border_width: Some(1.),
                border_color: Some(theme.outline().into()),
                width: Some(MODAL_WIDTH),
                height: Some(480.),
                ..Default::default()
            },
            header_styles: UiComponentStyles {
                font_family_id: Some(appearance.header_font_family()),
                font_color: Some(theme.active_ui_text_color().into()),
                font_size: Some(appearance.header_font_size()),
                font_weight: Some(Weight::Normal),
                height: Some(MODAL_HEADER_HEIGHT),
                padding: Some(Coords {
                    top: 20.,
                    bottom: 15.,
                    left: MODAL_PADDING,
                    right: MODAL_PADDING,
                }),
                ..Default::default()
            },
            body_styles: UiComponentStyles {
                background: Some(theme.surface_2().into()),
                padding: Some(Coords {
                    top: MODAL_PADDING,
                    bottom: MODAL_PADDING,
                    left: MODAL_PADDING,
                    right: MODAL_PADDING,
                }),
                height: Some(410.),
                ..Default::default()
            },
            close_modal_hover_state: Default::default(),
            background_opacity: MODAL_BACKDROP_OPACITY,
            offset_positioning: Self::default_offset_positioning(),
            dismiss_keystroke: None,
        }
    }

    pub fn close_modal_button_disabled(mut self) -> Self {
        self.show_close_modal_button = false;
        self
    }

    /// Turn on "Dismiss on click" behavior
    /// outside of the Modal
    pub fn with_dismiss_on_click(mut self) -> Self {
        self.dismiss_on_click = true;
        self
    }

    /// Overwrites _some_ styles passed in `modal_style` parameter
    pub fn with_modal_style(mut self, styles: UiComponentStyles) -> Self {
        self.modal_styles = self.modal_styles.merge(styles);
        self
    }

    /// Overwrites _some_ styles passed in `header_style` parameter
    pub fn with_header_style(mut self, styles: UiComponentStyles) -> Self {
        self.header_styles = self.header_styles.merge(styles);
        self
    }

    /// Overwrites _some_ styles passed in `body_style` parameter
    pub fn with_body_style(mut self, styles: UiComponentStyles) -> Self {
        self.body_styles = self.body_styles.merge(styles);
        self
    }

    pub fn with_background_opacity(mut self, opacity: u8) -> Self {
        self.background_opacity = opacity;
        self
    }

    /// Set the keystroke to display alongside the close button.
    pub fn with_dismiss_keystroke(mut self, keystroke: Keystroke) -> Self {
        self.dismiss_keystroke = Some(keystroke);
        self
    }

    pub fn set_title(&mut self, title: Option<String>) {
        self.title = title;
    }

    pub fn set_header_icon(&mut self, icon: Option<icons::Icon>) {
        self.header_icon = icon;
    }

    pub fn set_header_icon_color(&mut self, color: Option<Fill>) {
        self.header_icon_color = color;
    }

    pub fn set_offset_positioning(&mut self, offset_positioning: OffsetPositioning) {
        self.offset_positioning = offset_positioning;
    }

    fn handle_appearance_update(
        &mut self,
        handle: ModelHandle<Appearance>,
        ctx: &mut ViewContext<Self>,
    ) {
        let appearance = handle.as_ref(ctx);
        let theme = appearance.theme();
        // Update theme dependent styles
        self.modal_styles = self.modal_styles.merge(UiComponentStyles {
            border_color: Some(theme.outline().into()),
            ..Default::default()
        });
        self.header_styles = self.header_styles.merge(UiComponentStyles {
            font_family_id: Some(appearance.ui_font_family()),
            font_color: Some(theme.active_ui_text_color().into()),
            ..Default::default()
        });
        self.body_styles = self.body_styles.merge(UiComponentStyles {
            background: Some(theme.surface_2().into()),
            ..Default::default()
        });
    }

    fn render_close_modal_button(&self, appearance: &Appearance) -> Box<dyn Element> {
        use ui_components::{keyboard_shortcut, Component};

        const BUTTON_DIAMETER: f32 = 24.;
        let close_button = appearance
            .ui_builder()
            .close_button(BUTTON_DIAMETER, self.close_modal_hover_state.clone())
            .build()
            .on_click(|ctx, _, _| ctx.dispatch_typed_action(ModalAction::Close))
            .finish();

        if let Some(keystroke) = &self.dismiss_keystroke {
            let theme = appearance.theme();
            Flex::row()
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_child(close_button)
                .with_child(
                    Container::new(keyboard_shortcut::KeyboardShortcut.render(
                        appearance,
                        keyboard_shortcut::Params {
                            keystroke: keystroke.clone(),
                            options: keyboard_shortcut::Options {
                                font_color: Some(theme.nonactive_ui_text_color().into()),
                                background: Some(blended_colors::neutral_3(theme).into()),
                                border_fill: None,
                                sizing: keyboard_shortcut::Sizing {
                                    font_size: 11.,
                                    padding: 5.,
                                },
                            },
                        },
                    ))
                    .with_margin_left(4.)
                    .finish(),
                )
                .finish()
        } else {
            close_button
        }
    }

    fn render_header_icon(&self, appearance: &Appearance) -> Option<Box<dyn Element>> {
        if let Some(icon) = self.header_icon {
            let icon_size = self
                .header_styles
                .font_size
                .unwrap_or(appearance.header_font_size());
            // first check if there's an icon color override, otherwise fall back to the
            // header font color, otherwise fall back to the theme's active_ui_text_color
            let icon_color = self.header_icon_color.unwrap_or(
                self.header_styles
                    .font_color
                    .unwrap_or(appearance.theme().active_ui_text_color().into())
                    .into(),
            );
            Some(
                Align::new(
                    Container::new(
                        ConstrainedBox::new(icon.to_warpui_icon(icon_color).finish())
                            .with_width(icon_size)
                            .with_height(icon_size)
                            .finish(),
                    )
                    .with_margin_right(icon_size / 2.)
                    .finish(),
                )
                .finish(),
            )
        } else {
            None
        }
    }

    fn render_header(&self, appearance: &Appearance) -> Option<Box<dyn Element>> {
        if let Some(title) = &self.title {
            let mut header = Flex::row().with_cross_axis_alignment(CrossAxisAlignment::Center);

            if let Some(icon) = self.render_header_icon(appearance) {
                header.add_child(icon);
            }

            header.add_child(
                Shrinkable::new(
                    1.0,
                    Align::new(
                        Text::new_inline(
                            title.clone(),
                            self.header_styles
                                .font_family_id
                                .unwrap_or(appearance.ui_font_family()),
                            self.header_styles
                                .font_size
                                .unwrap_or(appearance.header_font_size()),
                        )
                        .with_color(
                            self.header_styles
                                .font_color
                                .unwrap_or(appearance.theme().active_ui_text_color().into()),
                        )
                        .with_style(
                            Properties::default()
                                .weight(self.header_styles.font_weight.unwrap_or(Weight::Normal)),
                        )
                        .finish(),
                    )
                    .left()
                    .finish(),
                )
                .finish(),
            );

            if self.show_close_modal_button {
                header.add_child(self.render_close_modal_button(appearance))
            }

            let mut container = Container::new(header.finish());
            if let Some(background) = self.header_styles.background {
                container = container.with_background(background);
            }
            if let Some(border_radius) = self.header_styles.border_radius {
                container = container.with_corner_radius(border_radius.top());
            }

            if let Some(padding) = self.header_styles.padding {
                container = container
                    .with_padding_left(padding.left)
                    .with_padding_top(padding.top)
                    .with_padding_right(padding.right)
                    .with_padding_bottom(padding.bottom);
            }

            Some(
                ConstrainedBox::new(container.finish())
                    .with_max_height(self.header_styles.height.unwrap_or(MODAL_HEADER_HEIGHT))
                    .finish(),
            )
        } else {
            None
        }
    }

    fn render_body(&self) -> Box<dyn Element> {
        let mut container = Container::new(ChildView::new(&self.body).finish());

        if self.title.is_some() {
            container = container
                .with_corner_radius(self.modal_styles.border_radius.unwrap_or_default().bottom());
        } else {
            // If there is no title, then the body has to have its top rounded
            container =
                container.with_corner_radius(self.modal_styles.border_radius.unwrap_or_default());
        };

        if let Some(padding) = self.body_styles.padding {
            container = container
                .with_padding_left(padding.left)
                .with_padding_top(padding.top)
                .with_padding_right(padding.right)
                .with_padding_bottom(padding.bottom);
        }
        ConstrainedBox::new(container.finish())
            .with_max_height(self.body_styles.height.unwrap())
            .finish()
    }

    pub fn body(&self) -> &ViewHandle<T> {
        &self.body
    }

    fn default_offset_positioning() -> OffsetPositioning {
        OffsetPositioning::offset_from_parent(
            vec2f(0., 0.),
            ParentOffsetBounds::WindowByPosition,
            ParentAnchor::Center,
            ChildAnchor::Center,
        )
    }
}

impl<T: View> Entity for Modal<T> {
    type Event = ModalEvent;
}

impl<T: View> TypedActionView for Modal<T> {
    type Action = ModalAction;

    fn handle_action(&mut self, action: &ModalAction, ctx: &mut ViewContext<Self>) {
        match action {
            ModalAction::Close => {
                ctx.emit(ModalEvent::Close);
            }
        }
    }
}

impl<T: View> View for Modal<T> {
    fn ui_name() -> &'static str {
        "Modal"
    }

    fn on_focus(&mut self, focus_ctx: &FocusContext, ctx: &mut ViewContext<Self>) {
        if focus_ctx.is_self_focused() {
            ctx.focus(&self.body);
            ctx.notify();
        }
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let header = self.render_header(appearance);
        let contents = if let Some(header) = header {
            Flex::column()
                .with_child(header)
                .with_child(self.render_body())
                .finish()
        } else {
            self.render_body()
        };

        let mut modal = ConstrainedBox::new(
            Container::new(contents)
                .with_background(blended_colors::neutral_2(appearance.theme()))
                .with_corner_radius(self.modal_styles.border_radius.unwrap_or_default())
                .with_border(
                    Border::all(self.modal_styles.border_width.unwrap())
                        .with_border_fill(self.modal_styles.border_color.unwrap()),
                )
                .with_margin_top(35.)
                .finish(),
        )
        .with_max_width(self.modal_styles.width.unwrap())
        .with_max_height(self.modal_styles.height.unwrap())
        .finish();

        if self.dismiss_on_click {
            modal = Dismiss::new(modal)
                .on_dismiss(|ctx, _app| ctx.dispatch_typed_action(ModalAction::Close))
                .finish();
        }

        // Stack needed so that modal can get bounds information,
        // specifically to ensure no overlap with the window's traffic lights
        let mut stack = Stack::new();
        stack.add_positioned_child(modal, self.offset_positioning.clone());

        Container::new(Align::new(stack.finish()).finish())
            .with_background_color(ColorU::new(0, 0, 0, self.background_opacity))
            .with_corner_radius(app.windows().window_corner_radius())
            .finish()
    }
}
