//! [`InputHintLineView`] — the one-row hint line rendered below the TUI input box.
//!
//! Shows persistent mode hints (currently the shell-mode callout) and
//! short-lived transient notices via [`InputHintLineView::show_transient`], a
//! reusable pattern for surfacing errors or tips in this spot.

use std::time::Duration;

use warp::tui_export::BlocklistAIInputModel;
use warpui_core::elements::tui::{TuiElement, TuiText};
use warpui_core::r#async::Timer;
use warpui_core::{AppContext, Entity, ModelHandle, TuiView, TypedActionView, ViewContext};

use crate::tui_builder::TuiUiBuilder;

/// How long a transient hint stays visible before reverting to the persistent
/// content.
const TRANSIENT_HINT_DURATION: Duration = Duration::from_secs(3);

/// Callout shown while the input is in `!` shell mode.
const SHELL_MODE_HINT: &str = "shell mode · esc to exit";

/// The hint line below the input box: persistent content derived from current
/// state, temporarily replaced by transient notices.
pub(crate) struct InputHintLineView {
    /// Shared input-mode state, used to derive the persistent content.
    input_mode: ModelHandle<BlocklistAIInputModel>,
    /// The transient notice currently displayed, if any.
    transient: Option<String>,
    /// Incremented per transient notice so an expiring timer only clears the
    /// notice it was started for.
    transient_generation: u64,
}

impl Entity for InputHintLineView {
    type Event = ();
}

impl InputHintLineView {
    /// Creates the hint line, re-rendering whenever the input mode changes.
    pub(crate) fn new(
        input_mode: ModelHandle<BlocklistAIInputModel>,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        ctx.subscribe_to_model(&input_mode, |_, _, _, ctx| ctx.notify());
        Self {
            input_mode,
            transient: None,
            transient_generation: 0,
        }
    }

    /// Displays `text` for [`TRANSIENT_HINT_DURATION`], then reverts to the
    /// persistent content.
    pub(crate) fn show_transient(&mut self, text: String, ctx: &mut ViewContext<Self>) {
        self.transient = Some(text);
        self.transient_generation += 1;
        let generation = self.transient_generation;
        ctx.spawn(
            Timer::after(TRANSIENT_HINT_DURATION),
            move |view, _, ctx| {
                if view.transient_generation == generation {
                    view.transient = None;
                    ctx.notify();
                }
            },
        );
        ctx.notify();
    }

    fn is_shell_mode(&self, ctx: &AppContext) -> bool {
        let input_mode = self.input_mode.as_ref(ctx);
        input_mode.is_input_type_locked() && !input_mode.is_ai_input_enabled()
    }
}

impl TuiView for InputHintLineView {
    fn ui_name() -> &'static str {
        "InputHintLineView"
    }

    fn render(&self, ctx: &AppContext) -> Box<dyn TuiElement> {
        // Render a space when idle so the line always occupies one row and the
        // input box doesn't shift when hints appear.
        let text = if let Some(transient) = &self.transient {
            TuiText::new(transient.clone())
        } else if self.is_shell_mode(ctx) {
            TuiText::new(SHELL_MODE_HINT)
                .with_style(TuiUiBuilder::from_app(ctx).shell_mode_accent_style())
        } else {
            TuiText::new(" ")
        };
        Box::new(text.truncate())
    }
}

impl TypedActionView for InputHintLineView {
    type Action = ();
}
