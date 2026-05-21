use pathfinder_color::ColorU;
use warpui::{
    elements::{
        Align, ConstrainedBox, Container, CrossAxisAlignment, Flex, HighlightedHyperlink,
        MouseStateHandle, ParentElement, Shrinkable,
    },
    fonts::Weight,
    keymap::Keystroke,
    ui_components::{
        button::ButtonVariant,
        components::{Coords, UiComponent, UiComponentStyles},
    },
    AppContext, Element,
};

use crate::{
    appearance::Appearance,
    terminal::{
        ssh::warpify::warpify_description,
        view::{RememberForWarpification, TerminalAction},
    },
    themes::theme::Fill,
    ui_components::blended_colors,
};

use super::{render_block_banner, BLOCK_BANNER_DESCRIPTION_MAX_HEIGHT};

const CLOSE_BUTTON_DIAMETER: f32 = 20.0;
const STANDARD_PADDING: f32 = 8.0;

#[derive(Clone)]
pub enum WarpificationMode {
    Ssh {
        command: String,
        host: Option<String>,
        hyperlink_index: HighlightedHyperlink,
    },
    Subshell {
        command: String,
    },
}

impl WarpificationMode {
    pub fn ssh(command: String, host: Option<String>) -> Self {
        Self::Ssh {
            command,
            host,
            hyperlink_index: Default::default(),
        }
    }

    pub fn has_host(&self) -> bool {
        matches!(self, Self::Ssh { host: Some(_), .. })
    }

    pub fn subshell(command: String) -> Self {
        Self::Subshell { command }
    }
}

impl WarpificationMode {
    pub fn is_ssh(&self) -> bool {
        matches!(self, Self::Ssh { .. })
    }
}

pub struct WarpifyBannerState {
    pub mode: WarpificationMode,
    pub height: f32,
    pub accept_button_mouse_state: MouseStateHandle,
    pub dont_ask_button_mouse_state: MouseStateHandle,
    pub dismiss_button_mouse_state: MouseStateHandle,

    /// This keybinding gets rendered in the Warpification banner, but we can't look it up
    /// during render as a &mut AppContext is not available then. This needs to get
    /// looked up during action handling and cached here.
    pub initialize_warpify_keybinding: Option<Keystroke>,
    pub hover_state: MouseStateHandle,
}

impl WarpifyBannerState {
    pub fn new(mode: WarpificationMode, initialize_warpify_keybinding: Option<Keystroke>) -> Self {
        Self {
            mode,
            height: 0.0,
            initialize_warpify_keybinding,
            accept_button_mouse_state: Default::default(),
            dont_ask_button_mouse_state: Default::default(),
            dismiss_button_mouse_state: Default::default(),
            hover_state: Default::default(),
        }
    }

    pub fn is_ssh(&self) -> bool {
        self.mode.is_ssh()
    }

    pub fn title(&self) -> &str {
        match &self.mode {
            WarpificationMode::Ssh { .. } => "Warpify SSH session",
            WarpificationMode::Subshell { .. } => "Warpify subshell",
        }
    }

    pub fn action(&self) -> TerminalAction {
        match &self.mode {
            WarpificationMode::Ssh { .. } => TerminalAction::WarpifySSHSession,
            WarpificationMode::Subshell { .. } => TerminalAction::TriggerSubshellBootstrap,
        }
    }

    fn remember_for_warpification(&self, should_remember: bool) -> RememberForWarpification {
        match &self.mode {
            WarpificationMode::Ssh { command, host, .. } => {
                let Some(host) = host else {
                    if should_remember {
                        return RememberForWarpification::RememberSubshellCommand(
                            command.to_owned(),
                        );
                    }
                    return RememberForWarpification::DoNotRememberSSHHost;
                };
                if should_remember {
                    RememberForWarpification::RememberSSHHost(host.to_owned())
                } else {
                    RememberForWarpification::DoNotRememberSSHHost
                }
            }
            WarpificationMode::Subshell { command } => {
                if should_remember {
                    RememberForWarpification::RememberSubshellCommand(command.to_owned())
                } else {
                    RememberForWarpification::DoNotRememberSubshellCommand
                }
            }
        }
    }
}

/// This banner is shown when the user runs a command which is recognized as a subshell-compatible
/// command. It asks if they want to bootstrap a subshell and, if so, whether we should ask again
/// next time they run the same command.
pub fn render_warpification_banner(
    state: &WarpifyBannerState,
    appearance: &Appearance,
    app: &AppContext,
) -> Box<dyn Element> {
    let yes_button = render_yes_button(
        state,
        &state.initialize_warpify_keybinding,
        &state.accept_button_mouse_state,
        appearance,
    );

    let remember = state.remember_for_warpification(true);
    let dont_ask_button = Container::new(
        appearance
            .ui_builder()
            .button(
                ButtonVariant::Text,
                state.dont_ask_button_mouse_state.clone(),
            )
            .with_text_label(crate::i18n::tr_static(app, "Do not show again").to_owned())
            .build()
            .on_click(move |ctx, _, _| {
                ctx.dispatch_typed_action(TerminalAction::DismissWarpifyBanner(
                    remember.to_owned(),
                ));
            })
            .finish(),
    )
    .with_margin_right(16.)
    .finish();

    let do_not_remember = state.remember_for_warpification(false);
    let close_button = appearance
        .ui_builder()
        .close_button(
            CLOSE_BUTTON_DIAMETER,
            state.dismiss_button_mouse_state.clone(),
        )
        .build()
        .on_click(move |ctx, _, _| {
            ctx.dispatch_typed_action(TerminalAction::DismissWarpifyBanner(
                do_not_remember.to_owned(),
            ));
        })
        .finish();

    let mut col = Flex::column()
        .with_child(
            Flex::row()
                .with_child(Align::new(yes_button).finish())
                .with_child(
                    Shrinkable::new(1., Align::new(dont_ask_button).right().finish()).finish(),
                )
                .with_child(Align::new(close_button).finish())
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .finish(),
        )
        .with_cross_axis_alignment(CrossAxisAlignment::Start);

    render_block_banner(
        |hover_state| {
            if let WarpificationMode::Ssh {
                hyperlink_index, ..
            } = &state.mode
            {
                let description = Container::new(warpify_description(app, hyperlink_index))
                    .with_uniform_margin(STANDARD_PADDING)
                    .with_margin_top(4.)
                    .finish();
                let description = if hover_state.is_hovered() {
                    description
                } else {
                    ConstrainedBox::new(description)
                        .with_max_height(2. * BLOCK_BANNER_DESCRIPTION_MAX_HEIGHT)
                        .finish()
                };
                col.add_child(description);
            }
            col.finish()
        },
        state.hover_state.clone(),
        appearance.theme(),
    )
}

fn render_yes_button(
    state: &WarpifyBannerState,
    initialize_warpification_keybinding: &Option<Keystroke>,
    mouse_state: &MouseStateHandle,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let yes_button = match initialize_warpification_keybinding {
        Some(keystroke) => appearance
            .ui_builder()
            .keyboard_shortcut_button(state.title().to_owned(), keystroke, mouse_state.clone())
            .with_style(UiComponentStyles {
                height: Some(36.),
                padding: Some(Coords {
                    top: 0.,
                    bottom: 0.,
                    left: STANDARD_PADDING,
                    right: STANDARD_PADDING,
                }),
                ..Default::default()
            }),
        None => appearance
            .ui_builder()
            .button(ButtonVariant::Basic, mouse_state.clone())
            .with_text_label(state.title().to_owned())
            .with_style(UiComponentStyles {
                background: Some(Fill::Solid(ColorU::transparent_black()).into()),
                height: Some(36.),
                font_size: Some(appearance.ui_font_size() + 2.),
                font_weight: Some(Weight::Bold),
                font_color: Some(blended_colors::text_main(
                    appearance.theme(),
                    appearance.theme().background(),
                )),
                border_color: Some(appearance.theme().surface_3().into()),
                border_width: Some(1.),
                padding: Some(Coords::uniform(STANDARD_PADDING)),
                ..Default::default()
            })
            .with_hovered_styles(UiComponentStyles {
                background: Some(appearance.theme().surface_3().into()),
                border_color: Some(blended_colors::accent(appearance.theme()).into()),
                ..Default::default()
            }),
    };
    let action = state.action();
    yes_button
        .build()
        .on_click(move |ctx, _, _| ctx.dispatch_typed_action(action.to_owned()))
        .finish()
}
