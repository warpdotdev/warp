//! Banner shown when the remote-server binary check, installation, or connection fails on the remote host.
//! We fall back to the existing Warpification behavior and display this banner so the user knows why advanced features are unavailable.

use warp_core::ui::theme::color::internal_colors;
use warpui::{
    elements::{
        ConstrainedBox, Container, CrossAxisAlignment, Flex, Hoverable, MainAxisAlignment,
        MainAxisSize, MouseStateHandle, ParentElement, Shrinkable, Text,
    },
    platform::Cursor,
    AppContext, Element, Entity, SingletonEntity, TypedActionView, View, ViewContext,
};

use crate::{terminal::model::session::SessionId, ui_components::icons::Icon, Appearance};

#[derive(Clone, Debug)]
pub enum SshRemoteServerFailedBannerAction {
    Dismiss,
}

#[derive(Clone, Debug)]
pub enum SshRemoteServerFailedBannerEvent {
    Dismissed,
}

pub struct SshRemoteServerFailedBanner {
    session_id: SessionId,
    close_mouse_state: MouseStateHandle,
}

impl SshRemoteServerFailedBanner {
    pub fn new(session_id: SessionId) -> Self {
        Self {
            session_id,
            close_mouse_state: MouseStateHandle::default(),
        }
    }

    pub fn session_id(&self) -> SessionId {
        self.session_id
    }
}

impl Entity for SshRemoteServerFailedBanner {
    type Event = SshRemoteServerFailedBannerEvent;
}

impl View for SshRemoteServerFailedBanner {
    fn ui_name() -> &'static str {
        "SshRemoteServerFailedBanner"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        let fg_color = theme.foreground().into_solid();
        let muted_color = internal_colors::neutral_5(theme);
        let font_size = appearance.monospace_font_size();
        let small_font_size = font_size - 2.;

        // Alert-circle icon
        let icon = Container::new(
            ConstrainedBox::new(Icon::AlertCircle.to_warpui_icon(fg_color.into()).finish())
                .with_width(16.)
                .with_height(16.)
                .finish(),
        )
        .with_margin_right(8.)
        .finish();

        // Title
        let title = Text::new(
            "SSH extension couldn't be installed",
            appearance.ui_font_family(),
            font_size,
        )
        .with_color(fg_color)
        .finish();

        // Description
        let body = Text::new(
            "The binary could not be written or executed on the remote host. \
             This may be due to permission restrictions or missing dependencies. \
             While advanced features like file browsing and code review are currently \
             disabled, the rest of your Warpified experience is fully available.",
            appearance.ui_font_family(),
            small_font_size,
        )
        .soft_wrap(true)
        .with_color(muted_color)
        .finish();

        // Close (X) button
        let close_icon_color = muted_color;
        let close = Hoverable::new(self.close_mouse_state.clone(), move |_| {
            ConstrainedBox::new(Icon::X.to_warpui_icon(close_icon_color.into()).finish())
                .with_width(16.)
                .with_height(16.)
                .finish()
        })
        .with_cursor(Cursor::PointingHand)
        .on_click(|ctx, _, _| {
            ctx.dispatch_typed_action(SshRemoteServerFailedBannerAction::Dismiss);
        })
        .finish();

        let close_container = Container::new(close).with_uniform_padding(4.).finish();

        // Header row: [icon + title] ... [close]
        let header_row = Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(
                Shrinkable::new(
                    1.,
                    Flex::row()
                        .with_cross_axis_alignment(CrossAxisAlignment::Center)
                        .with_child(icon)
                        .with_child(Shrinkable::new(1., title).finish())
                        .finish(),
                )
                .finish(),
            )
            .with_child(close_container)
            .finish();

        // Body text indented past the icon to align with the title
        let body_container = Container::new(body)
            .with_margin_top(2.)
            .with_margin_left(24.)
            .finish();

        let content = Flex::column()
            .with_main_axis_size(MainAxisSize::Min)
            .with_child(header_row)
            .with_child(body_container)
            .finish();

        Container::new(content)
            .with_background(internal_colors::fg_overlay_1(theme))
            .with_uniform_padding(12.)
            .finish()
    }
}

impl TypedActionView for SshRemoteServerFailedBanner {
    type Action = SshRemoteServerFailedBannerAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            SshRemoteServerFailedBannerAction::Dismiss => {
                ctx.emit(SshRemoteServerFailedBannerEvent::Dismissed);
            }
        }
    }
}
