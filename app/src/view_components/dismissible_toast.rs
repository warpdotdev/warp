use std::rc::Rc;
use std::time::Duration;

use pathfinder_color::ColorU;
use pathfinder_geometry::vector::vec2f;
use uuid::Uuid;
use warp_core::ui::builder::UiBuilder;
use warp_core::ui::theme::color::internal_colors;
use warpui::elements::ChildView;
use warpui::keymap::Keystroke;
use warpui::r#async::Timer;
use warpui::{
    elements::{
        Border, ChildAnchor, ClippedScrollStateHandle, ClippedScrollable, ConstrainedBox,
        Container, CornerRadius, CrossAxisAlignment, DispatchEventResult, EventHandler,
        Fill as ElementFill, Flex, Hoverable, Icon, MainAxisAlignment, MainAxisSize,
        MouseStateHandle, OffsetPositioning, ParentElement, PositionedElementAnchor,
        PositionedElementOffsetBounds, Radius, SavePosition, ScrollbarWidth, Shrinkable, Stack,
    },
    fonts::Weight,
    r#async::SpawnedFutureHandle,
    ui_components::components::{Coords, UiComponent, UiComponentStyles},
    units::Pixels,
    AppContext, Element, Entity, SingletonEntity, TypedActionView, View, ViewContext,
};
use warpui::{Action, ViewHandle};

use crate::{appearance::Appearance, themes::theme::Fill};

use super::action_button::ActionButton;

const TOAST_WIDTH: f32 = 464.;
const TOAST_CORNER_RADIUS: f32 = 4.;
const TEXT_MARGIN: f32 = 16.;
const VERTICAL_PADDING: f32 = 8.;
const HORIZONTAL_PADDING: f32 = 12.;
const ICON_RIGHT_MARGIN: f32 = 8.;
const CLOSE_BUTTON_SIZE: f32 = 16.;

const SUCCESS_ICON_PATH: &str = "bundled/svg/check-skinny.svg";
const ERROR_ICON_PATH: &str = "bundled/svg/alert-circle.svg";

struct ToastData<A: Action + Clone> {
    /// The toast itself.
    dismissible_toast: DismissibleToast<A>,

    /// Each toast is stored with its abort handle so we can abort the
    /// timeout-based dismissal if a manual dismissal happens first.
    abort_handle: Option<SpawnedFutureHandle>,

    /// Unique identifier for the toast. Used for finding the toast to dismiss from the
    /// stack.
    uuid: Uuid,
}
/// This View is a stack of toasts, each of which holds some "main text" on the left, and optionally
/// a hyperlink on the right. They can either be manually dismissed by clicking the X button, or
/// automatically dismissed by a timeout (of configurable duration). It is a stack b/c there may be
/// multiple toasts in existence (one might get added before a previous one is dismissed), and should
/// be rendered according to the order they were generated.
pub struct DismissibleToastStack<A: Action + Clone = ()> {
    timeout: Duration,
    scroll_state: ClippedScrollStateHandle,
    /// A vector of individual toasts. Manual dismissals dismiss the specific toast that was
    /// clicked, while timeouts pass the toast's UUID to the dismiss method.
    /// Since the user may close any arbitrary toast, we use a vector, and assign UUIDs to
    /// each toast to identify them. Each toast is stored together with its abort handle
    /// so we can abort the timeout-based dismissal if a manual dismissal happens.
    toasts: Vec<ToastData<A>>,
}

impl<A: Action + Clone> DismissibleToastStack<A> {
    pub fn new(timeout: Duration) -> Self {
        Self {
            timeout,
            scroll_state: Default::default(),
            toasts: Vec::new(),
        }
    }

    /// Put a new ephemeral toast in the front of the stack.
    /// The toast will go away when:
    /// - the configurable timeout is reached
    /// - the toast is manually dismissed
    /// whichever comes first.
    pub fn add_ephemeral_toast(&mut self, toast: DismissibleToast<A>, ctx: &mut ViewContext<Self>) {
        let uuid = Uuid::new_v4();
        let abort_handle = ctx.spawn_abortable(
            Timer::after(self.timeout),
            move |view, _, ctx| view.dismiss_toast_by_uuid(&uuid, ctx),
            |_, _| {},
        );

        if let Some(object_id) = &toast.object_id {
            self.dismiss_older_toasts(object_id, ctx);
        }

        self.toasts.push(ToastData {
            dismissible_toast: toast,
            abort_handle: Some(abort_handle),
            uuid,
        });
        self.scroll_state.scroll_to(Pixels::zero());

        ctx.notify();
    }

    /// Put a new persistent toast at the top of the stack.
    /// The toast will only go away when the toast is manually dismissed.
    pub fn add_persistent_toast(
        &mut self,
        toast: DismissibleToast<A>,
        ctx: &mut ViewContext<Self>,
    ) {
        if let Some(object_id) = &toast.object_id {
            self.dismiss_older_toasts(object_id, ctx);
        }

        self.toasts.push(ToastData {
            dismissible_toast: toast,
            abort_handle: None,
            uuid: Uuid::new_v4(),
        });
        self.scroll_state.scroll_to(Pixels::zero());

        ctx.notify();
    }

    /// Find a toast by uuid and removed it from the stack.
    pub fn dismiss_toast_by_uuid(&mut self, uuid: &Uuid, ctx: &mut ViewContext<Self>) {
        if let Some(index) = self.toasts.iter().position(|toast| toast.uuid == *uuid) {
            let toast = self.toasts.remove(index);
            if let Some(abort_handle) = toast.abort_handle {
                abort_handle.abort();
            }
            ctx.notify();
        }
    }

    /// Find all toasts pertaining to a particular object, and remove them from the stack.
    pub fn dismiss_older_toasts(&mut self, object_id: &str, ctx: &mut ViewContext<Self>) {
        self.toasts.retain(|toast| {
            if let Some(other_object_id) = &toast.dismissible_toast.object_id {
                return object_id != other_object_id;
            }

            true
        });
        ctx.notify();
    }

    /// Dismiss all toasts whose `object_id` starts with the given prefix.
    pub fn dismiss_toasts_by_prefix(&mut self, prefix: &str, ctx: &mut ViewContext<Self>) {
        let before = self.toasts.len();
        self.toasts.retain(|toast| {
            toast
                .dismissible_toast
                .object_id
                .as_ref()
                .is_none_or(|id| !id.starts_with(prefix))
        });
        if self.toasts.len() != before {
            ctx.notify();
        }
    }

    pub fn clear_toasts(&mut self, ctx: &mut ViewContext<Self>) {
        self.toasts.clear();
        ctx.notify();
    }

    /// Returns whether the stack currently has any toasts.
    pub fn has_toasts(&self) -> bool {
        !self.toasts.is_empty()
    }
}

impl<A: Action + Clone> View for DismissibleToastStack<A> {
    fn ui_name() -> &'static str {
        "DismissibleToastStack"
    }

    /// Shows nothing if there are no toasts. If there are one or more, show them all in a
    /// stacked column with the most recent one at the top.
    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let mut rendered_toasts =
            Flex::column().with_cross_axis_alignment(CrossAxisAlignment::Center);
        // For loop over the toasts in reverse order so that the most recent toast is
        // rendered first. Pass in the toast's UUID to the render method so that it is
        // piped to the dismiss action when the close button is clicked. The handler will
        // use this UUID to determine which toast in the stack to close.
        for toast in self.toasts.iter().rev() {
            rendered_toasts.add_child(
                Container::new(toast.dismissible_toast.render(app, toast.uuid))
                    .with_margin_bottom(5.)
                    .finish(),
            );
        }
        if self.toasts.is_empty() {
            return rendered_toasts.finish();
        }

        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        let scrollable_contents = Container::new(rendered_toasts.finish())
            .with_padding_top(4.)
            .with_padding_right(4.)
            .finish();

        ClippedScrollable::vertical(
            self.scroll_state.clone(),
            scrollable_contents,
            ScrollbarWidth::Auto,
            theme.nonactive_ui_detail().into(),
            theme.active_ui_detail().into(),
            ElementFill::None,
        )
        .with_overlayed_scrollbar()
        .finish()
    }
}

impl<A: Action + Clone> Entity for DismissibleToastStack<A> {
    type Event = ();
}

#[derive(Debug)]
pub enum DismissibleToastAction {
    ClickDismissButton(Uuid),
    ClickBody(Uuid),
}

impl<A: Action + Clone> TypedActionView for DismissibleToastStack<A> {
    type Action = DismissibleToastAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            DismissibleToastAction::ClickDismissButton(uuid) => {
                self.dismiss_toast_by_uuid(uuid, ctx);
            }
            DismissibleToastAction::ClickBody(uuid) => {
                if let Some(index) = self.toasts.iter().position(|t| t.uuid == *uuid) {
                    let toast = self.toasts.remove(index);
                    if let Some(abort_handle) = toast.abort_handle {
                        abort_handle.abort();
                    }
                    if let Some(on_body_click) = &toast.dismissible_toast.on_body_click {
                        on_body_click(ctx);
                    }
                    ctx.notify();
                }
            }
        }
    }
}

/// The hyperlink in a toast.
#[derive(Clone)]
pub struct ToastLink<A: Action + Clone> {
    text: String,
    href: Option<String>,
    action: Option<A>,
    keystroke: Option<Keystroke>,
    mouse_hover_state: MouseStateHandle,
}

impl<A: Action + Clone> ToastLink<A> {
    pub fn new(text: String) -> Self {
        Self {
            text,
            href: None,
            action: None,
            keystroke: None,
            mouse_hover_state: Default::default(),
        }
    }

    #[allow(dead_code)]
    pub fn with_href(mut self, href: String) -> Self {
        self.href = Some(href);
        self
    }

    pub fn with_onclick_action(mut self, action: A) -> Self {
        self.action = Some(action);
        self
    }

    pub fn with_keystroke(mut self, keystroke: Keystroke) -> Self {
        self.keystroke = Some(keystroke);
        self
    }

    fn render(&self, ui_builder: &UiBuilder, font_color: ColorU) -> Box<dyn Element> {
        let action = self.action.clone();

        let mut row = Flex::row().with_cross_axis_alignment(CrossAxisAlignment::Center);
        row.add_child(
            ui_builder
                .link(
                    self.text.clone(),
                    self.href.clone(),
                    Some(Box::new(move |ctx| {
                        if let Some(action) = &action {
                            ctx.dispatch_typed_action(action.clone());
                        }
                    })),
                    self.mouse_hover_state.clone(),
                )
                .soft_wrap(false)
                .with_style(UiComponentStyles {
                    font_color: Some(font_color),
                    font_weight: Some(Weight::Bold),
                    ..Default::default()
                })
                .soft_wrap(true)
                .build()
                .finish(),
        );

        if let Some(keystroke) = &self.keystroke {
            row.add_child(
                Container::new(ui_builder.keyboard_shortcut(keystroke).build().finish())
                    .with_margin_left(4.)
                    .finish(),
            );
        }

        row.finish()
    }
}

/// Callback type for body click actions.
/// Note: Rc is used to allow Clone on DismissibleToast.
pub type OnBodyClickCallback<A> = Rc<dyn Fn(&mut ViewContext<DismissibleToastStack<A>>)>;

/// Holds the data and logic needed to render an individual toast in the stack.
#[derive(Clone)]
pub struct DismissibleToast<A: Action + Clone> {
    flavor: ToastFlavor,
    main_text: String,
    link: Option<ToastLink<A>>,
    close_button_mouse_state: MouseStateHandle,
    close_button_hover_state: MouseStateHandle,
    /// An optional string-based ID representing the object that is the subject of this toast.
    /// Future toasts added to the stack will auto-dismiss any toasts still in the stack with the
    /// same ID, as it's likely the older ones are now out-of-date.
    object_id: Option<String>,
    action_button: Option<ViewHandle<ActionButton>>,
    /// Optional callback invoked when the toast body is clicked.
    pub(crate) on_body_click: Option<OnBodyClickCallback<A>>,
}

pub enum ToastType {
    CloudObjectNotFound,
}

impl<A: Action + Clone> DismissibleToast<A> {
    pub fn new(main_text: String, flavor: ToastFlavor) -> Self {
        Self {
            flavor,
            main_text,
            link: None,
            close_button_mouse_state: Default::default(),
            close_button_hover_state: Default::default(),
            object_id: Default::default(),
            action_button: Default::default(),
            on_body_click: None,
        }
    }

    pub fn default(main_text: String) -> Self {
        Self::new(main_text, ToastFlavor::Default)
    }

    pub fn success(main_text: String) -> Self {
        Self::new(main_text, ToastFlavor::Success)
    }

    pub fn error(main_text: String) -> Self {
        Self::new(main_text, ToastFlavor::Error)
    }

    pub fn with_link(mut self, link: ToastLink<A>) -> Self {
        self.link = Some(link);
        self
    }

    pub fn with_object_id(mut self, object_id: String) -> Self {
        self.object_id = Some(object_id);
        self
    }

    /// Inserts an action button to the right of the toast.
    pub fn with_action_button(mut self, button: ViewHandle<ActionButton>) -> Self {
        self.action_button = Some(button);
        self
    }

    /// Sets a callback to be invoked when the toast body is clicked.
    /// When set, the entire toast body becomes clickable.
    pub fn with_on_body_click<F>(mut self, callback: F) -> Self
    where
        F: Fn(&mut ViewContext<DismissibleToastStack<A>>) + 'static,
    {
        self.on_body_click = Some(Rc::new(callback));
        self
    }

    fn is_clickable(&self) -> bool {
        self.on_body_click.is_some()
    }

    fn position_id(&self, uuid: Uuid) -> String {
        format!("toast_id_{uuid}")
    }

    fn render(&self, app: &AppContext, uuid: Uuid) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let ui_builder = appearance.ui_builder();

        let mut left_aligned = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_size(MainAxisSize::Min);

        if let Some(icon) = self.render_icon(appearance.ui_font_size() * 1.2, appearance) {
            left_aligned.add_child(icon);
        }

        left_aligned.add_child(
            Shrinkable::new(
                1.,
                ui_builder
                    .wrappable_text(self.main_text.clone(), true)
                    .with_style(UiComponentStyles {
                        font_size: Some(appearance.ui_font_size() * 1.2),
                        font_color: Some(self.flavor.text_color(appearance)),
                        ..Default::default()
                    })
                    .build()
                    .finish(),
            )
            .finish(),
        );

        let mut right_aligned = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_alignment(MainAxisAlignment::End)
            .with_main_axis_size(MainAxisSize::Min);

        if let Some(link) = &self.link {
            right_aligned.add_child(
                Container::new(
                    ConstrainedBox::new(
                        link.render(ui_builder, self.flavor.text_color(appearance)),
                    )
                    .with_max_width(TOAST_WIDTH / 3.)
                    .finish(),
                )
                .with_margin_left(TEXT_MARGIN)
                .finish(),
            );
        }

        if let Some(right_aligned_button) = &self.action_button {
            right_aligned.add_child(
                Container::new(
                    ConstrainedBox::new(ChildView::new(right_aligned_button).finish()).finish(),
                )
                .with_margin_left(TEXT_MARGIN)
                .finish(),
            );
        }

        let row = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
            .with_main_axis_size(MainAxisSize::Min)
            .with_child(Shrinkable::new(1., left_aligned.finish()).finish())
            .with_child(Shrinkable::new(2., right_aligned.finish()).finish());

        let is_clickable = self.is_clickable();
        // On mobile devices, always show close button since hover effects don't work with touch
        let is_mobile = warpui::platform::is_mobile_device();

        Hoverable::new(self.close_button_hover_state.clone(), move |mouse_state| {
            let toast_container = Container::new(row.finish())
                .with_vertical_padding(VERTICAL_PADDING)
                .with_horizontal_padding(HORIZONTAL_PADDING)
                .with_background(self.flavor.bg_color(appearance))
                .with_corner_radius(warpui::elements::CornerRadius::with_all(Radius::Pixels(
                    TOAST_CORNER_RADIUS,
                )))
                .with_border(Border::all(1.).with_border_fill(self.flavor.border_color(appearance)))
                .finish();

            let toast_element: Box<dyn Element> = if is_clickable {
                EventHandler::new(toast_container)
                    .on_left_mouse_down(move |ctx, _, _| {
                        ctx.dispatch_typed_action(DismissibleToastAction::ClickBody(uuid));
                        DispatchEventResult::StopPropagation
                    })
                    .finish()
            } else {
                toast_container
            };

            let mut stack = Stack::new()
                .with_child(SavePosition::new(toast_element, &self.position_id(uuid)).finish());

            if mouse_state.is_hovered() || is_mobile {
                stack.add_positioned_overlay_child(
                    self.render_close_button(ui_builder, uuid, appearance),
                    OffsetPositioning::offset_from_save_position_element(
                        self.position_id(uuid),
                        vec2f(4., -4.),
                        PositionedElementOffsetBounds::WindowByPosition,
                        PositionedElementAnchor::TopRight,
                        ChildAnchor::TopRight,
                    ),
                );
            }
            stack.finish()
        })
        .with_hover_out_delay(Duration::from_millis(500))
        .finish()
    }

    fn render_icon(&self, icon_size: f32, appearance: &Appearance) -> Option<Box<dyn Element>> {
        self.flavor.icon_path().map(|path| {
            Container::new(
                ConstrainedBox::new(Icon::new(path, self.flavor.text_color(appearance)).finish())
                    .with_max_height(icon_size)
                    .with_max_width(icon_size)
                    .finish(),
            )
            .with_margin_right(ICON_RIGHT_MARGIN)
            .finish()
        })
    }

    fn render_close_button(
        &self,
        ui_builder: &UiBuilder,
        uuid: Uuid,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        Container::new(
            ui_builder
                .close_button(CLOSE_BUTTON_SIZE, self.close_button_mouse_state.clone())
                .with_style(UiComponentStyles {
                    font_color: Some(appearance.theme().foreground().into()),
                    background: Some(ToastFlavor::Default.bg_color(appearance).into()),
                    border_radius: Some(CornerRadius::with_all(Radius::Percentage(50.))),
                    border_width: Some(1.),
                    border_color: Some(ToastFlavor::Default.border_color(appearance).into()),
                    padding: Some(Coords {
                        top: 2.,
                        bottom: 2.,
                        left: 2.,
                        right: 2.,
                    }),
                    ..Default::default()
                })
                .build()
                .on_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(DismissibleToastAction::ClickDismissButton(uuid))
                })
                .finish(),
        )
        .finish()
    }
}

/// Represents the type of toast. Controls color and icon in order to communicate success, error,
/// etc.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ToastFlavor {
    Default,
    Success,
    Error,
}

impl ToastFlavor {
    fn icon_path(&self) -> Option<&'static str> {
        match self {
            Self::Default => None,
            Self::Success => Some(SUCCESS_ICON_PATH),
            Self::Error => Some(ERROR_ICON_PATH),
        }
    }

    fn text_color(&self, appearance: &Appearance) -> ColorU {
        let theme = appearance.theme();
        match self {
            ToastFlavor::Default => theme.main_text_color(theme.background()).into(),
            _ => theme.background().into(),
        }
    }

    fn bg_color(&self, appearance: &Appearance) -> Fill {
        let theme = appearance.theme();
        match self {
            Self::Default => internal_colors::neutral_4(theme).into(),
            Self::Success => theme.ansi_fg_green().into(),
            Self::Error => theme.ansi_fg_red().into(),
        }
    }

    fn border_color(&self, appearance: &Appearance) -> Fill {
        let theme = appearance.theme();
        match self {
            ToastFlavor::Default => internal_colors::neutral_3(theme).into(),
            ToastFlavor::Success => theme.ansi_bg_green().into(),
            ToastFlavor::Error => theme.ansi_bg_red().into(),
        }
    }
}
