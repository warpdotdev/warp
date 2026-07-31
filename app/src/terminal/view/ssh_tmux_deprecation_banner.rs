//! One-time inline banner shown to users who had previously opted into the now-deprecated
//! tmux-based SSH warpification flow. It explains that tmux SSH warpification has been turned
//! off in favor of Warp's SSH extension (remote server) and links to the docs.
//!
//! The banner is shown at most once per affected user: it is gated on the
//! `ssh_tmux_deprecation_notice_pending` setting, which is set by a one-time migration and
//! cleared by the parent [`crate::terminal::TerminalView`] once the banner is shown.

use warp_core::ui::theme::color::internal_colors;
use warpui::elements::{
    Align, ConstrainedBox, Container, CrossAxisAlignment, Flex, Hoverable, MainAxisAlignment,
    MainAxisSize, MouseStateHandle, ParentElement, Shrinkable, Text,
};
use warpui::platform::Cursor;
use warpui::ui_components::components::{UiComponent, UiComponentStyles};
use warpui::{AppContext, Element, Entity, SingletonEntity, TypedActionView, View, ViewContext};

use crate::Appearance;
use crate::terminal::model::session::SessionId;
use crate::terminal::warpify::render::SSH_DOCS_URL;
use crate::ui_components::icons::Icon;

const BANNER_TITLE: &str = "Tmux SSH warpification has been deprecated";

const BANNER_BODY: &str = "Warp now connects to remote sessions using the SSH extension, which is \
    more robust than the tmux-based flow. The tmux option has been removed.";

const LEARN_MORE_LABEL: &str = "Learn more";

#[derive(Clone, Debug)]
pub enum SshTmuxDeprecationBannerAction {
    Dismiss,
    LearnMore,
}

#[derive(Clone, Debug)]
pub enum SshTmuxDeprecationBannerEvent {
    Dismissed,
}

pub struct SshTmuxDeprecationBanner {
    session_id: SessionId,
    learn_more_mouse_state: MouseStateHandle,
    close_mouse_state: MouseStateHandle,
}

impl SshTmuxDeprecationBanner {
    pub fn new(session_id: SessionId) -> Self {
        Self {
            session_id,
            learn_more_mouse_state: MouseStateHandle::default(),
            close_mouse_state: MouseStateHandle::default(),
        }
    }

    pub fn session_id(&self) -> SessionId {
        self.session_id
    }
}

impl Entity for SshTmuxDeprecationBanner {
    type Event = SshTmuxDeprecationBannerEvent;
}

impl View for SshTmuxDeprecationBanner {
    fn ui_name() -> &'static str {
        "SshTmuxDeprecationBanner"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        let fg_color = theme.foreground().into_solid();
        let muted_color = internal_colors::neutral_5(theme);
        let accent_color = theme.accent().into_solid();
        let font_size = appearance.monospace_font_size();
        let small_font_size = font_size - 2.;

        // Warp icon to match the other warpification blocks.
        let icon = Container::new(
            ConstrainedBox::new(Icon::Warp.to_warpui_icon(fg_color.into()).finish())
                .with_width(16.)
                .with_height(16.)
                .finish(),
        )
        .with_margin_right(8.)
        .finish();

        let title = Text::new(
            BANNER_TITLE.to_string(),
            appearance.ui_font_family(),
            font_size,
        )
        .with_color(fg_color)
        .finish();

        let body = Text::new(
            BANNER_BODY.to_string(),
            appearance.ui_font_family(),
            small_font_size,
        )
        .soft_wrap(true)
        .with_color(muted_color)
        .finish();

        let learn_more = appearance
            .ui_builder()
            .link(
                LEARN_MORE_LABEL.into(),
                None,
                Some(Box::new(|ctx| {
                    ctx.dispatch_typed_action(SshTmuxDeprecationBannerAction::LearnMore);
                })),
                self.learn_more_mouse_state.clone(),
            )
            .soft_wrap(false)
            .with_style(UiComponentStyles {
                font_size: Some(small_font_size),
                font_family_id: Some(appearance.ui_font_family()),
                font_color: Some(accent_color),
                ..Default::default()
            })
            .build()
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
            ctx.dispatch_typed_action(SshTmuxDeprecationBannerAction::Dismiss);
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

        // Body text + learn more link, indented past the icon to align with the title.
        let body_container = Container::new(body)
            .with_margin_top(2.)
            .with_margin_left(24.)
            .finish();

        // Wrap the link in a left-aligned `Align` so its hover/underline region hugs the
        // link text instead of stretching to the full banner width (the parent column uses
        // `CrossAxisAlignment::Stretch`).
        let learn_more_container = Container::new(Align::new(learn_more).left().finish())
            .with_margin_top(4.)
            .with_margin_left(24.)
            .finish();

        let content = Flex::column()
            .with_main_axis_size(MainAxisSize::Min)
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_child(header_row)
            .with_child(body_container)
            .with_child(learn_more_container)
            .finish();

        Container::new(content)
            .with_background(internal_colors::fg_overlay_1(theme))
            .with_uniform_padding(12.)
            .finish()
    }
}

impl TypedActionView for SshTmuxDeprecationBanner {
    type Action = SshTmuxDeprecationBannerAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            SshTmuxDeprecationBannerAction::Dismiss => {
                ctx.emit(SshTmuxDeprecationBannerEvent::Dismissed);
            }
            SshTmuxDeprecationBannerAction::LearnMore => {
                ctx.open_url(SSH_DOCS_URL);
                ctx.emit(SshTmuxDeprecationBannerEvent::Dismissed);
            }
        }
    }
}
