use pathfinder_color::ColorU;
use warp_core::ui::theme::Fill;
use warpui::clipboard::ClipboardContent;
use warpui::elements::{
    Align, Border, ChildView, ClippedScrollStateHandle, ClippedScrollable, ConstrainedBox,
    Container, CornerRadius, CrossAxisAlignment, DropShadow, Flex, MainAxisAlignment, MainAxisSize,
    MouseStateHandle, ParentElement, Radius, ScrollbarWidth, Text,
};
use warpui::fonts::{Properties, Weight};
use warpui::keymap::FixedBinding;
use warpui::platform::Cursor;
use warpui::ui_components::components::UiComponent;
use warpui::ui_components::text::Span;
use warpui::{
    AppContext, Element, Entity, SingletonEntity, TypedActionView, View, ViewContext, ViewHandle,
};

use crate::appearance::Appearance;
use crate::view_components::action_button::{ActionButton, PrimaryTheme, SecondaryTheme};

const MODAL_WIDTH: f32 = 400.;
const ERROR_MAX_HEIGHT: f32 = 240.;

pub fn init(app: &mut AppContext) {
    use warpui::keymap::macros::*;

    app.register_fixed_bindings([
        FixedBinding::new(
            "escape",
            IapRefreshFailureModalAction::Dismiss,
            id!(IapRefreshFailureModal::ui_name()),
        ),
        FixedBinding::new(
            "enter",
            IapRefreshFailureModalAction::LogIn,
            id!(IapRefreshFailureModal::ui_name()),
        ),
    ]);
}

#[derive(Clone, Debug)]
pub enum IapRefreshFailureModalAction {
    CopyError,
    Dismiss,
    LogIn,
    ToggleSnooze,
}

#[derive(Clone, Debug)]
pub enum IapRefreshFailureModalEvent {
    Dismiss,
    LogIn,
}

pub struct IapRefreshFailureModal {
    message: Option<String>,
    snooze: bool,
    on_snooze: Option<Box<dyn Fn()>>,
    scroll_state: ClippedScrollStateHandle,
    checkbox_mouse_state: MouseStateHandle,
    copy_button: ViewHandle<ActionButton>,
    dismiss_button: ViewHandle<ActionButton>,
    login_button: ViewHandle<ActionButton>,
}

impl IapRefreshFailureModal {
    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        let copy_button = ctx.add_view(|_| {
            ActionButton::new("Copy Error", SecondaryTheme)
                .on_click(|ctx| ctx.dispatch_typed_action(IapRefreshFailureModalAction::CopyError))
        });
        let dismiss_button = ctx.add_view(|_| {
            ActionButton::new("Dismiss", SecondaryTheme)
                .on_click(|ctx| ctx.dispatch_typed_action(IapRefreshFailureModalAction::Dismiss))
        });
        let login_button = ctx.add_view(|_| {
            ActionButton::new("Log in", PrimaryTheme)
                .on_click(|ctx| ctx.dispatch_typed_action(IapRefreshFailureModalAction::LogIn))
        });

        Self {
            message: None,
            snooze: true,
            on_snooze: None,
            scroll_state: Default::default(),
            checkbox_mouse_state: Default::default(),
            copy_button,
            dismiss_button,
            login_button,
        }
    }

    pub fn show(
        &mut self,
        message: String,
        on_snooze: impl Fn() + 'static,
        ctx: &mut ViewContext<Self>,
    ) {
        self.message = Some(message);
        self.snooze = true;
        self.on_snooze = Some(Box::new(on_snooze));
        self.scroll_state = Default::default();
        ctx.focus_self();
        ctx.notify();
    }

    pub fn is_open(&self) -> bool {
        self.message.is_some()
    }

    fn close(&mut self, event: IapRefreshFailureModalEvent, ctx: &mut ViewContext<Self>) {
        self.message = None;
        if self.snooze {
            if let Some(on_snooze) = self.on_snooze.take() {
                on_snooze();
            }
        } else {
            self.on_snooze = None;
        }
        ctx.emit(event);
        ctx.notify();
    }

    fn render_error(&self, appearance: &Appearance) -> Box<dyn Element> {
        let theme = appearance.theme();
        let error = Text::new(
            self.message.clone().unwrap_or_default(),
            appearance.monospace_font_family(),
            12.,
        )
        .with_style(Properties::default().weight(Weight::Normal))
        .with_color(theme.main_text_color(theme.surface_2()).into_solid())
        .finish();
        let scrollable = ClippedScrollable::vertical(
            self.scroll_state.clone(),
            Container::new(error)
                .with_uniform_padding(12.)
                .with_margin_right(8.)
                .finish(),
            ScrollbarWidth::Custom(4.),
            theme.nonactive_ui_text_color().into(),
            theme.active_ui_text_color().into(),
            warpui::elements::Fill::None,
        )
        .with_overlayed_scrollbar()
        .with_padding_start(0.)
        .with_padding_end(0.)
        .finish();

        ConstrainedBox::new(
            Container::new(scrollable)
                .with_background(theme.surface_2())
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)))
                .finish(),
        )
        .with_height(ERROR_MAX_HEIGHT)
        .finish()
    }

    fn render_checkbox(&self, appearance: &Appearance) -> Box<dyn Element> {
        appearance
            .ui_builder()
            .checkbox(self.checkbox_mouse_state.clone(), Some(14.))
            .with_label(Span::new(
                "Don't show again for 5 mins.".to_string(),
                Default::default(),
            ))
            .check(self.snooze)
            .build()
            .with_cursor(Cursor::PointingHand)
            .on_click(|ctx, _, _| {
                ctx.dispatch_typed_action(IapRefreshFailureModalAction::ToggleSnooze)
            })
            .finish()
    }

    fn render_buttons(&self) -> Box<dyn Element> {
        Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_main_axis_alignment(MainAxisAlignment::End)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(8.)
            .with_child(ChildView::new(&self.copy_button).finish())
            .with_child(ChildView::new(&self.dismiss_button).finish())
            .with_child(ChildView::new(&self.login_button).finish())
            .finish()
    }
}

impl Entity for IapRefreshFailureModal {
    type Event = IapRefreshFailureModalEvent;
}

impl View for IapRefreshFailureModal {
    fn ui_name() -> &'static str {
        "IapRefreshFailureModal"
    }

    fn on_focus(&mut self, _focus_ctx: &warpui::FocusContext, ctx: &mut ViewContext<Self>) {
        ctx.focus_self();
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        let background = theme.surface_3();
        let title = Text::new(
            "IAP credential refresh failed",
            appearance.ui_font_family(),
            18.,
        )
        .with_style(Properties::default().weight(Weight::Bold))
        .with_color(theme.main_text_color(background).into_solid())
        .finish();
        let content = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_spacing(16.)
            .with_child(title)
            .with_child(self.render_error(appearance))
            .with_child(self.render_checkbox(appearance))
            .with_child(self.render_buttons())
            .finish();
        let card = ConstrainedBox::new(
            Container::new(content)
                .with_background(background)
                .with_border(Border::all(1.).with_border_fill(theme.outline()))
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(8.)))
                .with_drop_shadow(DropShadow::default())
                .with_uniform_padding(24.)
                .finish(),
        )
        .with_width(MODAL_WIDTH)
        .finish();

        Container::new(Align::new(card).finish())
            .with_background(Fill::Solid(ColorU::new(97, 97, 97, 255)).with_opacity(50))
            .finish()
    }
}

impl TypedActionView for IapRefreshFailureModal {
    type Action = IapRefreshFailureModalAction;

    fn handle_action(
        &mut self,
        action: &IapRefreshFailureModalAction,
        ctx: &mut ViewContext<Self>,
    ) {
        match action {
            IapRefreshFailureModalAction::CopyError => {
                if let Some(message) = &self.message {
                    ctx.clipboard()
                        .write(ClipboardContent::plain_text(message.clone()));
                }
            }
            IapRefreshFailureModalAction::Dismiss => {
                self.close(IapRefreshFailureModalEvent::Dismiss, ctx);
            }
            IapRefreshFailureModalAction::LogIn => {
                self.close(IapRefreshFailureModalEvent::LogIn, ctx);
            }
            IapRefreshFailureModalAction::ToggleSnooze => {
                self.snooze = !self.snooze;
                ctx.notify();
            }
        }
    }
}
