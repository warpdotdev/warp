//! The root view of the TUI agent surface.
//!
//! Owns the input buffer + caret and the focus state, reads the
//! [`TuiAgentBridge`] transcript, and composes the [`components`] into the full
//! layout: a header banner, a bordered transcript panel that fills the middle, a
//! focused input line, and a status bar. Keys are captured by a
//! [`components::key_capture`] wrapper that dispatches a [`TuiAgentAction`],
//! handled in [`TuiAgentView::handle_action`].
//!
//! `new(bridge, ctx)` is the contract used by [`crate::tui::bootstrap`] and is
//! kept stable.

use warpui::elements::{
    Container, CrossAxisAlignment, Expanded, Flex, MainAxisSize, ParentElement as _,
};
use warpui::{
    AppContext, BlurContext, Element, Entity, FocusContext, ModelHandle, SingletonEntity as _,
    TypedActionView, View, ViewContext,
};

use crate::appearance::Appearance;
use crate::tui::agent_bridge::TuiAgentBridge;
use crate::tui::components::{self, KeyPress};

/// Editing actions dispatched from the key-capture wrapper and applied to the
/// input buffer in [`TuiAgentView::handle_action`].
#[derive(Debug)]
pub enum TuiAgentAction {
    Insert(String),
    Backspace,
    Submit,
    Clear,
    MoveLeft,
    MoveRight,
    Home,
    End,
}

pub struct TuiAgentView {
    bridge: ModelHandle<TuiAgentBridge>,
    /// The text the user is composing.
    input: String,
    /// Caret position as a char index into `input`.
    caret: usize,
    focused: bool,
}

impl TuiAgentView {
    pub fn new(bridge: ModelHandle<TuiAgentBridge>, ctx: &mut ViewContext<Self>) -> Self {
        // Re-render whenever the bridge's transcript changes.
        ctx.subscribe_to_model(&bridge, |_view, _handle, _event, ctx| ctx.notify());
        Self {
            bridge,
            input: String::new(),
            caret: 0,
            focused: true,
        }
    }

    /// Byte offset of char index `char_idx`, clamped to the buffer end.
    fn byte_offset(&self, char_idx: usize) -> usize {
        self.input
            .char_indices()
            .nth(char_idx)
            .map(|(offset, _)| offset)
            .unwrap_or(self.input.len())
    }

    fn insert(&mut self, s: &str) {
        let offset = self.byte_offset(self.caret);
        self.input.insert_str(offset, s);
        self.caret += s.chars().count();
    }

    fn backspace(&mut self) {
        if self.caret == 0 {
            return;
        }
        let start = self.byte_offset(self.caret - 1);
        let end = self.byte_offset(self.caret);
        self.input.replace_range(start..end, "");
        self.caret -= 1;
    }

    fn submit(&mut self, ctx: &mut ViewContext<Self>) {
        let text = std::mem::take(&mut self.input);
        self.caret = 0;
        if !text.trim().is_empty() {
            self.bridge
                .update(ctx, |bridge, ctx| bridge.submit_user_input(text, ctx));
        }
        ctx.notify();
    }
}

impl Entity for TuiAgentView {
    type Event = ();
}

impl TypedActionView for TuiAgentView {
    type Action = TuiAgentAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            TuiAgentAction::Insert(s) => self.insert(s),
            TuiAgentAction::Backspace => self.backspace(),
            TuiAgentAction::Clear => {
                self.input.clear();
                self.caret = 0;
            }
            TuiAgentAction::MoveLeft => self.caret = self.caret.saturating_sub(1),
            TuiAgentAction::MoveRight => {
                self.caret = (self.caret + 1).min(self.input.chars().count());
            }
            TuiAgentAction::Home => self.caret = 0,
            TuiAgentAction::End => self.caret = self.input.chars().count(),
            TuiAgentAction::Submit => self.submit(ctx),
        }
        ctx.notify();
    }
}

impl View for TuiAgentView {
    fn ui_name() -> &'static str {
        "TuiAgentView"
    }

    fn on_focus(&mut self, focus_ctx: &FocusContext, ctx: &mut ViewContext<Self>) {
        if focus_ctx.is_self_focused() {
            self.focused = true;
            ctx.notify();
        }
    }

    fn on_blur(&mut self, blur_ctx: &BlurContext, ctx: &mut ViewContext<Self>) {
        if blur_ctx.is_self_blurred() {
            self.focused = false;
            ctx.notify();
        }
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let family = Appearance::as_ref(app).ui_font_family();

        let (transcript, status, streaming) = self.bridge.read(app, |bridge, _| {
            (
                components::transcript(bridge.entries(), family),
                bridge.status_line().to_owned(),
                bridge.is_streaming(),
            )
        });

        let body = Flex::column()
            .with_main_axis_size(MainAxisSize::Max)
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_child(components::header("Warp Agent", &status, streaming, family))
            .with_child(
                Expanded::new(1.0, components::panel("CONVERSATION", transcript, family)).finish(),
            )
            .with_child(components::input_line(
                &self.input,
                self.caret,
                self.focused,
                family,
            ))
            .with_child(components::status_bar(&status, streaming, family))
            .finish();

        let root = Container::new(body)
            .with_background_color(components::palette::BG)
            .finish();

        components::key_capture(root, |ctx, key| match key {
            KeyPress::Char(s) => ctx.dispatch_typed_action(TuiAgentAction::Insert(s.clone())),
            KeyPress::Backspace => ctx.dispatch_typed_action(TuiAgentAction::Backspace),
            KeyPress::Enter => ctx.dispatch_typed_action(TuiAgentAction::Submit),
            KeyPress::Escape => ctx.dispatch_typed_action(TuiAgentAction::Clear),
            KeyPress::Left => ctx.dispatch_typed_action(TuiAgentAction::MoveLeft),
            KeyPress::Right => ctx.dispatch_typed_action(TuiAgentAction::MoveRight),
            KeyPress::Home => ctx.dispatch_typed_action(TuiAgentAction::Home),
            KeyPress::End => ctx.dispatch_typed_action(TuiAgentAction::End),
        })
    }
}
