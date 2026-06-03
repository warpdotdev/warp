use markdown_parser::{FormattedText, FormattedTextFragment, FormattedTextLine};
use warp_core::channel::ChannelState;
use warp_core::ui::theme::WarpTheme;
use warpui::elements::{
    Border, Container, CrossAxisAlignment, Flex, HighlightedHyperlink, Hoverable, Icon,
    MainAxisAlignment, MainAxisSize, MouseStateHandle, ParentElement,
};
use warpui::keymap::FixedBinding;
use warpui::platform::Cursor;
use warpui::ui_components::button::ButtonVariant;
use warpui::ui_components::components::{UiComponent, UiComponentStyles};
use warpui::{
    AppContext, BlurContext, Element, Entity, FocusContext, SingletonEntity, TypedActionView, View,
    ViewContext,
};

use crate::appearance::Appearance;
use crate::terminal::model::ansi::WarpificationUnavailableReason;
use crate::terminal::warpify;
use crate::terminal::warpify::render::{apply_spacing_styles, build_description_row};
use crate::terminal::warpify::settings::WarpifySettings;
use crate::ui_components::icons::Icon as UiIcon;

const SSH_GITHUB_ISSUE_URL: &str = "https://github.com/warpdotdev/Warp/issues/new?assignees=&labels=Bugs,SSH-tmux&projects=&template=03_ssh_tmux.yml";

fn get_ssh_github_issue_url(title: &str) -> String {
    let url = if let Some(version) = ChannelState::app_version() {
        format!("{SSH_GITHUB_ISSUE_URL}&warp-version={version}")
    } else {
        SSH_GITHUB_ISSUE_URL.to_string()
    };
    // prepend the title with "SSH tmux bug report: " and uri encode it
    let title = format!("SSH tmux bug report: {title:?}");
    let title = urlencoding::encode(&title);
    format!("{url}&title={title}")
}

impl WarpificationUnavailableReason {
    fn error_message(&self) -> String {
        match self {
            WarpificationUnavailableReason::TmuxNotInstalled { .. } => {
                i18n::t("terminal.ssh.error.tmux_not_installed")
            }
            WarpificationUnavailableReason::UnsupportedTmuxVersion { .. } => {
                i18n::t("terminal.ssh.error.unsupported_tmux_version")
            }
            WarpificationUnavailableReason::TmuxFailed => i18n::t("terminal.ssh.error.tmux_failed"),
            WarpificationUnavailableReason::Timeout { .. } => {
                i18n::t("terminal.ssh.error.warpify_timeout")
            }
            WarpificationUnavailableReason::UnsupportedShell { .. } => {
                i18n::t("terminal.ssh.error.unsupported_shell")
            }
            WarpificationUnavailableReason::TmuxInstallFailed { .. } => {
                i18n::t("terminal.ssh.error.tmux_install_failed")
            }
        }
    }

    fn error_title(&self) -> &'static str {
        match self {
            WarpificationUnavailableReason::TmuxNotInstalled { .. } => "tmux Not Installed",
            WarpificationUnavailableReason::UnsupportedTmuxVersion { .. } => {
                "Unsupported Tmux Version"
            }
            WarpificationUnavailableReason::TmuxFailed => "tmux Failed",
            WarpificationUnavailableReason::Timeout {
                is_tmux_install, ..
            } => {
                if *is_tmux_install {
                    "tmux Install Timeout"
                } else {
                    "SSH Warpify Timeout"
                }
            }
            WarpificationUnavailableReason::UnsupportedShell { .. } => "Unsupported Shell",
            WarpificationUnavailableReason::TmuxInstallFailed { .. } => "tmux Install Failed",
        }
    }
}

#[derive(Debug, Clone)]
pub enum SshErrorBlockEvent {
    ContinueWithoutWarpification,
    WarpifyWithoutTmux,
}

#[derive(Debug, Clone)]
pub enum SshErrorBlockAction {
    ContinueWithoutWarpification,
    WarpifyWithoutTmux,
    OpenUrl(String),
    AddSshHostToDenylist(String),
    Focus,
}

pub struct SshErrorBlock {
    error_reason: WarpificationUnavailableReason,
    ssh_host: Option<String>,
    warpify_without_tmux_button_mouse_state: MouseStateHandle,
    continue_button_mouse_state: MouseStateHandle,
    report_link_highlight_index: HighlightedHyperlink,
    never_warpify_mouse_state_handle: MouseStateHandle,
    block_mouse_state: MouseStateHandle,
    is_focused: bool,
}

pub fn init(app: &mut AppContext) {
    use warpui::keymap::macros::*;

    app.register_fixed_bindings([
        FixedBinding::new(
            "enter",
            SshErrorBlockAction::WarpifyWithoutTmux,
            id!(SshErrorBlock::ui_name()),
        ),
        FixedBinding::new(
            "escape",
            SshErrorBlockAction::ContinueWithoutWarpification,
            id!(SshErrorBlock::ui_name()),
        ),
        FixedBinding::new(
            "ctrl-c",
            SshErrorBlockAction::ContinueWithoutWarpification,
            id!(SshErrorBlock::ui_name()),
        ),
    ]);
}

impl SshErrorBlock {
    #[allow(clippy::new_without_default)]
    pub fn new(error_reason: WarpificationUnavailableReason, ssh_host: Option<String>) -> Self {
        Self {
            error_reason,
            ssh_host,
            warpify_without_tmux_button_mouse_state: Default::default(),
            continue_button_mouse_state: Default::default(),
            report_link_highlight_index: Default::default(),
            never_warpify_mouse_state_handle: Default::default(),
            block_mouse_state: Default::default(),
            is_focused: false,
        }
    }

    pub fn focus(&self, ctx: &mut ViewContext<Self>) {
        ctx.focus_self();
        ctx.notify();
    }

    fn should_show_report_to_warp_button(&self) -> bool {
        matches!(
            self.error_reason,
            WarpificationUnavailableReason::Timeout { .. }
                | WarpificationUnavailableReason::TmuxInstallFailed { .. }
        )
    }

    fn render_title_ui(
        &self,
        app: &AppContext,
        theme: &WarpTheme,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let header_contents = warpify::render::build_header_row(
            i18n::t("terminal.ssh.error.title"),
            Icon::new(UiIcon::AlertTriangle.into(), theme.ui_error_color()),
            theme,
            appearance,
        )
        .with_margin_right(8.)
        .finish();

        let right_hand_size = warpify::render::render_never_warpify_ssh_link(
            &self.ssh_host,
            app,
            appearance,
            self.never_warpify_mouse_state_handle.clone(),
            move |ctx, ssh_host| {
                ctx.dispatch_typed_action(SshErrorBlockAction::AddSshHostToDenylist(
                    ssh_host.to_owned(),
                ));
            },
        );

        let mut row = Flex::row()
            .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
            .with_cross_axis_alignment(CrossAxisAlignment::End)
            .with_main_axis_size(MainAxisSize::Max)
            .with_child(header_contents);

        if let Some(right_hand_size) = right_hand_size {
            row.add_child(right_hand_size);
        }

        warpify::render::apply_spacing_styles(Container::new(row.finish())).finish()
    }
}

impl Entity for SshErrorBlock {
    type Event = SshErrorBlockEvent;
}

pub const SSH_ERROR_BLOCK_VISIBLE_KEY: &str = "SshErrorBlockVisible";

impl View for SshErrorBlock {
    fn ui_name() -> &'static str {
        "SshErrorBlock"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();

        let mut content = Flex::column().with_cross_axis_alignment(CrossAxisAlignment::Stretch);

        content.add_child(self.render_title_ui(app, theme, appearance));

        let error_message = self.error_reason.error_message();
        content.add_child(warpify::render::description_row(
            &error_message,
            theme,
            appearance,
        ));

        let ui_builder = appearance.ui_builder();

        if self.should_show_report_to_warp_button() {
            let report_issue_text = build_description_row(
                FormattedText::new([FormattedTextLine::Line(vec![
                    FormattedTextFragment::plain_text(i18n::t(
                        "terminal.ssh.error.report_issue_prefix",
                    )),
                    FormattedTextFragment::hyperlink(
                        i18n::t("terminal.ssh.error.report_issue_link"),
                        get_ssh_github_issue_url(self.error_reason.error_title()),
                    ),
                    FormattedTextFragment::plain_text(i18n::t(
                        "terminal.ssh.error.report_issue_suffix",
                    )),
                ])]),
                theme,
                appearance,
                self.report_link_highlight_index.clone(),
            )
            .with_hyperlink_font_color(theme.accent().into())
            .register_default_click_handlers(|link, ctx, _| {
                ctx.dispatch_typed_action(SshErrorBlockAction::OpenUrl(link.url));
            })
            .finish();
            content.add_child(apply_spacing_styles(Container::new(report_issue_text)).finish());
        }

        let buttons = Flex::row()
            .with_main_axis_size(MainAxisSize::Min)
            .with_child(
                Container::new(
                    ui_builder
                        .button(
                            ButtonVariant::Accent,
                            self.warpify_without_tmux_button_mouse_state.clone(),
                        )
                        .with_centered_text_label(i18n::t(
                            "terminal.ssh.error.warpify_without_tmux",
                        ))
                        .with_style(UiComponentStyles {
                            font_size: Some(appearance.monospace_font_size()),
                            ..Default::default()
                        })
                        .build()
                        .with_cursor(Cursor::PointingHand)
                        .on_click(move |ctx, _, _| {
                            ctx.dispatch_typed_action(SshErrorBlockAction::WarpifyWithoutTmux)
                        })
                        .finish(),
                )
                .with_margin_right(8.)
                .finish(),
            )
            .with_child(
                ui_builder
                    .button(
                        ButtonVariant::Secondary,
                        self.continue_button_mouse_state.clone(),
                    )
                    .with_centered_text_label(i18n::t(
                        "terminal.ssh.error.continue_without_warpification",
                    ))
                    .with_style(UiComponentStyles {
                        font_size: Some(appearance.monospace_font_size()),
                        ..Default::default()
                    })
                    .build()
                    .with_cursor(Cursor::PointingHand)
                    .on_click(move |ctx, _, _| {
                        ctx.dispatch_typed_action(SshErrorBlockAction::ContinueWithoutWarpification)
                    })
                    .finish(),
            );

        content.add_child(
            Container::new(buttons.finish())
                .with_uniform_margin(20.)
                .finish(),
        );

        Hoverable::new(self.block_mouse_state.clone(), |_| {
            Container::new(content.finish())
                .with_padding_top(10.)
                .with_background(theme.foreground().with_opacity(10))
                .with_border(Border::top(1.).with_border_fill(theme.outline()))
                .finish()
        })
        .on_click(|ctx, _, _| {
            ctx.dispatch_typed_action(SshErrorBlockAction::Focus);
        })
        .finish()
    }

    fn on_focus(&mut self, focus_ctx: &FocusContext, ctx: &mut ViewContext<Self>) {
        if focus_ctx.is_self_focused() {
            self.is_focused = true;
            ctx.notify();
        }
    }

    fn on_blur(&mut self, blur_ctx: &BlurContext, ctx: &mut ViewContext<Self>) {
        if blur_ctx.is_self_blurred() {
            self.is_focused = false;
            ctx.notify();
        }
    }
}

impl TypedActionView for SshErrorBlock {
    type Action = SshErrorBlockAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            SshErrorBlockAction::WarpifyWithoutTmux => {
                ctx.emit(SshErrorBlockEvent::WarpifyWithoutTmux)
            }
            SshErrorBlockAction::ContinueWithoutWarpification => {
                ctx.emit(SshErrorBlockEvent::ContinueWithoutWarpification)
            }
            SshErrorBlockAction::OpenUrl(url) => {
                ctx.open_url(url);
            }
            SshErrorBlockAction::AddSshHostToDenylist(ssh_host) => {
                let settings = WarpifySettings::handle(ctx);
                settings.update(ctx, |warpify, ctx| {
                    warpify.denylist_ssh_host(ssh_host, ctx);
                });
                ctx.emit(SshErrorBlockEvent::ContinueWithoutWarpification);
                ctx.notify()
            }
            SshErrorBlockAction::Focus => {
                self.focus(ctx);
            }
        }
    }
}
