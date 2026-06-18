//! The headless `warp-tui` front-end: a real (headless) Warp app whose root
//! window is a [`RootTuiView`] rendered through the `tui`-gated WarpUI backend.
//!
//! `RootTuiView` composes two child views — a [`TuiTranscriptView`] filling the
//! space above a bottom-anchored single-row [`TuiInputView`] — and routes the
//! input's submission events into the transcript. No agent harness is wired up
//! yet; submitting a prompt only appends it to the local transcript. [`init`] is
//! called from `run_internal` once the headless app is up (see
//! [`crate::run_tui`]). Ctrl-C quit is handled by the runtime's input loop.

mod input_view;
mod transcript_view;

use input_view::{InputEvent, TuiInputView};
use transcript_view::TuiTranscriptView;
use warpui_core::elements::tui::{TuiChildView, TuiColumn, TuiConstrainedBox, TuiElement};
use warpui_core::platform::{TerminationMode, WindowStyle};
use warpui_core::runtime::{spawn_tui_driver, TuiDriverHandle};
use warpui_core::{
    AddWindowOptions, AppContext, Entity, SingletonEntity, TuiView, TypedActionView, ViewContext,
    ViewHandle,
};

/// The bottom input frame's height: one text row inside a single-cell rounded
/// border (top + bottom), i.e. three rows total.
const INPUT_ROWS: u16 = 3;

/// The root TUI view: a transcript that grows upward above a fixed,
/// bottom-anchored input. It owns both child views and forwards the input's
/// submissions into the transcript.
struct RootTuiView {
    transcript: ViewHandle<TuiTranscriptView>,
    input: ViewHandle<TuiInputView>,
}

impl RootTuiView {
    fn new(ctx: &mut ViewContext<Self>) -> Self {
        // The transcript has no typed actions, so a plain TUI view suffices; the
        // input dispatches editing actions, so it must be a typed-action view.
        let transcript = ctx.add_tui_view(|_| TuiTranscriptView::default());
        let input = ctx.add_typed_action_tui_view(|_| TuiInputView::default());

        // On submission, append the text to the transcript. Routing through the
        // root (rather than wiring the transcript directly to the input) keeps
        // the view-ownership boundaries explicit and proves child-view
        // communication.
        ctx.subscribe_to_view(&input, |root, _input, event, ctx| match event {
            InputEvent::Submitted(text) => {
                let text = text.clone();
                root.transcript
                    .update(ctx, |transcript, ctx| transcript.append(text, ctx));
            }
        });

        ctx.focus(&input);

        Self { transcript, input }
    }
}

impl Entity for RootTuiView {
    type Event = ();
}

impl TuiView for RootTuiView {
    fn ui_name() -> &'static str {
        "RootTuiView"
    }

    fn render(&self, ctx: &AppContext) -> Box<dyn TuiElement> {
        let transcript = TuiChildView::new(&self.transcript, ctx);
        let input = TuiChildView::new(&self.input, ctx);

        // The transcript fills the space above the fixed-height input row.
        let column = TuiColumn::new()
            .flex_child(transcript)
            .child(TuiConstrainedBox::new(input).with_max_rows(INPUT_ROWS));

        Box::new(column)
    }
}

impl TypedActionView for RootTuiView {
    // The root handles no typed actions itself: editing actions are handled by
    // the input view, and Ctrl-C quit is handled by the runtime input loop.
    type Action = ();
}

/// Holds the live TUI session for the app's lifetime; dropping it on app
/// teardown restores the terminal.
struct TuiSession {
    _handle: TuiDriverHandle,
}

impl Entity for TuiSession {
    type Event = ();
}

impl SingletonEntity for TuiSession {}

/// Creates the TUI root window and starts the headless draw + input driver.
/// Registered as a singleton so the session lives for the app's lifetime.
pub fn init(ctx: &mut AppContext) {
    let (window_id, root) = ctx.add_tui_window(
        AddWindowOptions {
            window_style: WindowStyle::NotStealFocus,
            ..Default::default()
        },
        RootTuiView::new,
    );

    match spawn_tui_driver(ctx, window_id, root) {
        Ok(handle) => {
            ctx.add_singleton_model(|_| TuiSession { _handle: handle });
        }
        Err(error) => {
            log::error!("failed to start the TUI driver: {error}");
            // Not in the alternate screen yet (entering it is what failed), so
            // print to stderr too — otherwise the process just exits instantly
            // with the reason buried in the log file.
            eprintln!(
                "warp-tui: could not start the terminal UI: {error}\n\
                 Run it directly in an interactive terminal (a real TTY), not piped or backgrounded."
            );
            ctx.terminate_app(TerminationMode::ForceTerminate, None);
        }
    }
}

#[cfg(test)]
#[path = "tui_tests.rs"]
mod tests;
