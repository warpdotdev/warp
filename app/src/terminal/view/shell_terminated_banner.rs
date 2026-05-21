use std::{borrow::Cow, cell::RefCell};

use warp_core::ui::{
    appearance::Appearance,
    builder::UiBuilder,
    theme::{color::internal_colors, WarpTheme},
};
use warpui::{
    clipboard::ClipboardContent,
    elements::*,
    text_layout::ClipConfig,
    ui_components::{button::ButtonVariant, components::UiComponent as _},
    Entity, SingletonEntity as _, TypedActionView, View, ViewContext,
};

use crate::{terminal::model::terminal_model::ExitReason, ui_components};

const FILE_ISSUE_TEXT: &str = "File issue";
const MORE_INFO_TEXT: &str = "More info";

/// A banner to display when the shell process terminates.
///
/// This can be a simple informational banner or one giving information about
/// an unexpected error.
pub struct ShellTerminatedBanner {
    termination_type: TerminationType,
    handles: RefCell<Vec<MouseStateHandle>>,
}

impl ShellTerminatedBanner {
    pub fn new(termination_type: TerminationType, ctx: &mut ViewContext<Self>) -> Self {
        let appearance = Appearance::as_ref(ctx);

        let mut handles = vec![];
        let _ = termination_type.buttons(appearance, &mut handles, ctx);

        Self {
            termination_type,
            handles: RefCell::new(handles),
        }
    }
}

impl Entity for ShellTerminatedBanner {
    type Event = ();
}

impl View for ShellTerminatedBanner {
    fn ui_name() -> &'static str {
        "ShellTerminatedBanner"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();

        let banner = Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
            .with_cross_axis_alignment(CrossAxisAlignment::Center);

        let mut text_column = Flex::column()
            .with_main_axis_alignment(MainAxisAlignment::Start)
            .with_child(self.termination_type.text(appearance));

        if let Some(subtext) = self.termination_type.subtext(appearance) {
            text_column.add_child(subtext);
        }

        let text_column = text_column.finish();

        let left = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Start)
            .with_child(self.termination_type.icon(appearance))
            .with_child(Shrinkable::new(1., text_column).finish())
            .finish();

        let mut handles = self.handles.borrow_mut();
        let buttons = self
            .termination_type
            .buttons(appearance, &mut handles, app)
            .into_iter()
            .map(|button| Container::new(button).with_margin_left(8.).finish());

        let right = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_children(buttons)
            .finish();

        Container::new(
            banner
                .with_child(Shrinkable::new(1., left).finish())
                .with_child(right)
                .finish(),
        )
        .with_background(theme.ansi_fg_red())
        .with_uniform_padding(12.)
        .finish()
    }
}

#[derive(Debug)]
pub enum Action {
    OpenUrl(String),
    CopyPtySpawnError(String),
}

impl TypedActionView for ShellTerminatedBanner {
    type Action = Action;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            Action::OpenUrl(url) => {
                ctx.open_url(url);
            }
            Action::CopyPtySpawnError(error_str) => {
                ctx.clipboard()
                    .write(ClipboardContent::plain_text(error_str.to_owned()));
            }
        }
    }
}

pub enum TerminationType {
    /// The shell process terminated normally.
    ///
    /// TODO(vorporeal): Use this instead of the old inline banner.  Requires
    /// updating styling to support this properly.
    #[allow(dead_code)]
    Normal,
    /// The PTY failed to spawn and the shell process never started.
    #[cfg_attr(target_family = "wasm", allow(dead_code))]
    PtySpawnFailure { pty_spawn_error: anyhow::Error },
    /// The shell process terminated before we were able to bootstrap.
    Premature {
        shell_detail: String,
        reason: ExitReason,
    },
}

impl TerminationType {
    fn icon(&self, appearance: &Appearance) -> Box<dyn Element> {
        const ICON_SIZE: f32 = 16.;

        let icon_type = match self {
            TerminationType::Normal => ui_components::icons::Icon::Info,
            TerminationType::Premature { .. } | TerminationType::PtySpawnFailure { .. } => {
                ui_components::icons::Icon::Warning
            }
        };

        Container::new(
            ConstrainedBox::new(
                icon_type
                    .to_warpui_icon(appearance.theme().background())
                    .finish(),
            )
            .with_width(ICON_SIZE)
            .with_height(ICON_SIZE)
            .finish(),
        )
        .with_margin_right(8.)
        .finish()
    }

    fn text(&self, appearance: &Appearance) -> Box<dyn Element> {
        let text = match self {
            TerminationType::Normal => "Shell process exited",
            TerminationType::PtySpawnFailure { .. } => "Shell process could not start!",
            TerminationType::Premature { .. } => "Shell process exited prematurely!",
        };

        Text::new(text, appearance.ui_font_family(), 14.)
            .with_color(appearance.theme().background().into_solid())
            .with_clip(ClipConfig::end())
            .finish()
    }

    fn subtext(&self, appearance: &Appearance) -> Option<Box<dyn Element>> {
        let text: Cow<str> = match self {
            TerminationType::Normal => return None,
            TerminationType::PtySpawnFailure { pty_spawn_error } => {
                format!("{pty_spawn_error:#}").into()
            }
            TerminationType::Premature { shell_detail, .. } => format!(
                "Something went wrong while starting {shell_detail} and Warpifying it, causing the \
                process to terminate. Warpify script output is displayed here, which may point at \
                a cause."
            )
            .into(),
        };

        let text = Text::new(text, appearance.ui_font_family(), 12.)
            .with_color(internal_colors::neutral_2(appearance.theme()))
            .soft_wrap(true)
            .finish();

        Some(Container::new(text).with_margin_top(2.).finish())
    }

    fn buttons(
        &self,
        appearance: &Appearance,
        handles: &mut Vec<MouseStateHandle>,
        app: &AppContext,
    ) -> Vec<Box<dyn Element>> {
        match self {
            TerminationType::Normal => vec![],
            TerminationType::Premature { .. } => {
                let ui_builder = inverted_color_ui_builder(appearance);

                handles.resize_with(2, MouseStateHandle::default);
                vec![
                    ui_builder
                        .button(ButtonVariant::Text, handles[0].clone())
                        .with_text_label(FILE_ISSUE_TEXT.to_string())
                        .build()
                        .on_click(|ctx, _, _| {
                            ctx.dispatch_typed_action(Action::OpenUrl(
                                "https://github.com/warpdotdev/Warp/issues/new/choose".to_string(),
                            ));
                        })
                        .finish(),
                    ui_builder
                        .button(ButtonVariant::Outlined, handles[1].clone())
                        .with_text_label(MORE_INFO_TEXT.to_string())
                        .build()
                        .on_click(|ctx, _, _| {
                            ctx.dispatch_typed_action(Action::OpenUrl(
                                "https://docs.warp.dev/support-and-community/troubleshooting-and-support/known-issues#debugging".to_string(),
                            ));
                        })
                        .finish(),
                ]
            }
            TerminationType::PtySpawnFailure { pty_spawn_error } => {
                let ui_builder = inverted_color_ui_builder(appearance);

                handles.resize_with(3, MouseStateHandle::default);
                let error_str = format!("{pty_spawn_error:#}");
                vec![
                    ui_builder
                        .button(ButtonVariant::Text, handles[0].clone())
                        .with_text_label(crate::i18n::tr_static(app, "Copy error").to_string())
                        .build()
                        .on_click(move |evt_ctx, _ctx, _position| {
                            evt_ctx.dispatch_typed_action(Action::CopyPtySpawnError(
                                error_str.clone(),
                            ));
                        })
                        .finish(),
                    ui_builder
                        .button(ButtonVariant::Text, handles[1].clone())
                        .with_text_label(FILE_ISSUE_TEXT.to_string())
                        .build()
                        .on_click(|ctx, _, _| {
                            ctx.dispatch_typed_action(Action::OpenUrl(
                                "https://github.com/warpdotdev/Warp/issues/new/choose".to_string(),
                            ));
                        })
                        .finish(),
                    ui_builder
                        .button(ButtonVariant::Outlined, handles[2].clone())
                        .with_text_label(MORE_INFO_TEXT.to_string())
                        .build()
                        .on_click(|ctx, _, _| {
                            ctx.dispatch_typed_action(Action::OpenUrl(
                                "https://docs.warp.dev/support-and-community/troubleshooting-and-support/known-issues#debugging".to_string(),
                            ));
                        })
                        .finish(),
                ]
            }
        }
    }
}

fn inverted_color_ui_builder(appearance: &Appearance) -> UiBuilder {
    let theme = appearance.theme();
    let theme = WarpTheme::new(
        theme.foreground(),
        theme.background().into_solid(),
        theme.background(),
        None,
        None,
        theme.terminal_colors().clone(),
        None,
        None,
    );

    UiBuilder::new(
        theme,
        appearance.ui_font_family(),
        appearance.ui_font_size(),
        14.,
        appearance.line_height_ratio(),
    )
}
