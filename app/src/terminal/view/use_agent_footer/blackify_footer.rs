use std::sync::Arc;

use parking_lot::FairMutex;
use black_ui::elements::{
    ChildView, Container, CrossAxisAlignment, Expanded, Flex, MainAxisSize, ParentElement,
};
use black_ui::prelude::Empty;
use black_ui::{AppContext, Element, Entity, TypedActionView, View, ViewContext, ViewHandle};

use super::{AgentFooterButtonTheme, USE_AGENT_KEYSTROKE};
use crate::terminal::view::block_banner::WarpificationMode;
use crate::terminal::view::{TerminalModel, PADDING_LEFT};
use crate::ui_components::icons::Icon;
use crate::view_components::action_button::{
    ActionButton, ButtonSize, KeystrokeSource, TooltipAlignment,
};

/// Footer view rendered for detected subshell/SSH commands, offering both
/// "Blackify" and "Use agent" buttons in a horizontal row.
pub(super) struct BlackifyFooterView {
    terminal_model: Arc<FairMutex<TerminalModel>>,
    blackify_button: ViewHandle<ActionButton>,
    use_agent_button: ViewHandle<ActionButton>,
    dismiss_button: ViewHandle<ActionButton>,
    mode: Option<WarpificationMode>,
}

impl BlackifyFooterView {
    pub fn new(terminal_model: Arc<FairMutex<TerminalModel>>, ctx: &mut ViewContext<Self>) -> Self {
        let button_size = ButtonSize::XSmall;

        let blackify_button = ctx.add_typed_action_view(|_ctx| {
            ActionButton::new("Blackify subshell", AgentFooterButtonTheme::new(None))
                .with_icon(Icon::Warp)
                .with_size(button_size)
                .with_tooltip("Enable Black shell integration in this session")
                .with_tooltip_alignment(TooltipAlignment::Left)
                .on_click(|ctx| {
                    ctx.dispatch_typed_action(BlackifyFooterViewAction::Blackify);
                })
        });

        let use_agent_button = ctx.add_typed_action_view(|ctx| {
            ActionButton::new("Use agent", AgentFooterButtonTheme::new(None))
                .with_icon(Icon::Oz)
                .with_keybinding(KeystrokeSource::Fixed(USE_AGENT_KEYSTROKE.clone()), ctx)
                .with_size(button_size)
                .with_tooltip("Ask the Black agent to assist")
                .with_tooltip_alignment(TooltipAlignment::Left)
                .on_click(|ctx| {
                    ctx.dispatch_typed_action(BlackifyFooterViewAction::UseAgent);
                })
        });

        let dismiss_button = ctx.add_typed_action_view(|_ctx| {
            ActionButton::new("Dismiss", AgentFooterButtonTheme::new(None))
                .with_size(button_size)
                .on_click(|ctx| {
                    ctx.dispatch_typed_action(BlackifyFooterViewAction::Dismiss);
                })
        });

        Self {
            terminal_model,
            blackify_button,
            use_agent_button,
            dismiss_button,
            mode: None,
        }
    }

    /// Updates the blackify button label, keybinding, and stores the current blackification mode.
    pub fn set_mode(&mut self, mode: WarpificationMode, ctx: &mut ViewContext<Self>) {
        let (label, binding_name) = match mode {
            WarpificationMode::Ssh { .. } => {
                ("Blackify SSH session", "terminal:blackify_ssh_session")
            }
            WarpificationMode::Subshell { .. } => ("Blackify subshell", "terminal:blackify_subshell"),
        };
        self.blackify_button.update(ctx, |button, ctx| {
            button.set_label(label, ctx);
            button.set_keybinding(Some(KeystrokeSource::Binding(binding_name)), ctx);
        });
        self.mode = Some(mode);
        ctx.notify();
    }

    /// Returns the current blackification mode, if set.
    pub fn mode(&self) -> Option<&WarpificationMode> {
        self.mode.as_ref()
    }

    /// Clears the blackification mode.
    pub fn clear_mode(&mut self, ctx: &mut ViewContext<Self>) {
        self.mode = None;
        self.blackify_button.update(ctx, |button, ctx| {
            button.set_keybinding(None, ctx);
        });
        ctx.notify();
    }
}

#[derive(Debug, Clone)]
pub enum BlackifyFooterViewAction {
    Blackify,
    UseAgent,
    Dismiss,
}

pub enum BlackifyFooterViewEvent {
    Blackify { mode: WarpificationMode },
    UseAgent,
    Dismiss,
}

impl Entity for BlackifyFooterView {
    type Event = BlackifyFooterViewEvent;
}

impl View for BlackifyFooterView {
    fn ui_name() -> &'static str {
        "BlackifyFooterView"
    }

    fn render(&self, _app: &AppContext) -> Box<dyn Element> {
        let terminal_model = self.terminal_model.lock();

        let button_row = Flex::row()
            .with_spacing(4.)
            .with_main_axis_size(MainAxisSize::Max)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(ChildView::new(&self.blackify_button).finish())
            .with_child(ChildView::new(&self.use_agent_button).finish())
            .with_child(Expanded::new(1., Empty::new().finish()).finish())
            .with_child(ChildView::new(&self.dismiss_button).finish());

        let mut container = Container::new(button_row.finish())
            .with_horizontal_padding(*PADDING_LEFT)
            .with_vertical_padding(4.);

        if terminal_model.is_alt_screen_active() {
            if let Some(bg_color) = terminal_model.alt_screen().inferred_bg_color() {
                container = container.with_background(bg_color);
            }
        }

        container.finish()
    }
}

impl TypedActionView for BlackifyFooterView {
    type Action = BlackifyFooterViewAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            BlackifyFooterViewAction::Blackify => {
                if let Some(mode) = self.mode.clone() {
                    self.clear_mode(ctx);
                    ctx.emit(BlackifyFooterViewEvent::Blackify { mode });
                }
            }
            BlackifyFooterViewAction::UseAgent => {
                self.clear_mode(ctx);
                ctx.emit(BlackifyFooterViewEvent::UseAgent);
            }
            BlackifyFooterViewAction::Dismiss => {
                self.clear_mode(ctx);
                ctx.emit(BlackifyFooterViewEvent::Dismiss);
            }
        }
    }
}
