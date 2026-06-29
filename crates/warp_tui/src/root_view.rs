//! [`RootTuiView`]: the root view of the `warp-tui` front-end.
//!
//! The view gates on login state (observed from [`warp::TuiLoginModel`]):
//! - while the user is signing in, it shows a centered placeholder (and the
//!   device-login URL/code once known, since the alt screen hides stdout);
//! - once authenticated, it renders the input box — matching the Figma: a single
//!   bordered [`TuiInputView`] docked at the bottom of the screen. The input box
//!   grows with newlines up to six visual rows and then scrolls (behavior
//!   provided by [`TuiInputView`] itself).

use warp::editor::CodeEditorModel;
use warp::{TuiLoginModel, TuiLoginPhase};
use warpui_core::elements::tui::{
    TuiChildView, TuiColumn, TuiConstrainedBox, TuiContainer, TuiElement,
};
use warpui_core::{
    keymap, AppContext, Entity, EntityId, SingletonEntity, TuiView, TypedActionView, ViewContext,
    ViewHandle,
};

use crate::input::TuiInputView;
use crate::ui::{login_failed, login_placeholder};

/// Width used to construct the editor model before the first layout pass pushes
/// the real terminal width onto it.
const INITIAL_TERMINAL_WIDTH: u16 = 80;

/// The input box grows up to this many text rows (matching [`TuiInputView`]'s own
/// cap) before scrolling; the bordered container adds one row above and below.
const MAX_INPUT_TEXT_ROWS: u16 = 6;

/// Rows the box's border occupies (top + bottom).
const BORDER_ROWS: u16 = 2;

/// The root view: a login placeholder until the user is authenticated, then a
/// single bordered input box docked at the bottom of the screen. Owns the
/// editor-backed [`TuiInputView`] as a child view.
pub struct RootTuiView {
    input: ViewHandle<TuiInputView>,
}

impl RootTuiView {
    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        let model = ctx.add_model(|ctx| CodeEditorModel::new_tui(INITIAL_TERMINAL_WIDTH, ctx));
        let input = ctx.add_typed_action_tui_view(move |ctx| TuiInputView::new(model, ctx));
        Self { input }
    }

    /// The bordered input box, capped so it never grows past its six text rows
    /// plus the one-cell border on each side, with a flex spacer above docking
    /// it to the bottom.
    fn render_input(&self) -> Box<dyn TuiElement> {
        let input_box =
            TuiConstrainedBox::new(TuiContainer::new(TuiChildView::new(&self.input)).with_border())
                .with_max_rows(MAX_INPUT_TEXT_ROWS + BORDER_ROWS);

        Box::new(
            TuiColumn::new()
                .flex_child(TuiColumn::new())
                .child(input_box),
        )
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
        // Reading the login phase here registers this view as a dependency, so a
        // phase change (`notify`) re-renders and the TUI driver repaints.
        match TuiLoginModel::as_ref(ctx).phase() {
            TuiLoginPhase::LoggedIn => self.render_input(),
            TuiLoginPhase::AwaitingLogin {
                verification_uri,
                user_code,
            } => login_placeholder(verification_uri.as_deref(), user_code.as_deref()),
            TuiLoginPhase::Failed { message } => login_failed(message),
        }
    }

    fn child_view_ids(&self, ctx: &AppContext) -> Vec<EntityId> {
        // The input view only participates while it is actually shown, so
        // keystrokes never reach it during the login placeholder.
        match TuiLoginModel::as_ref(ctx).phase() {
            TuiLoginPhase::LoggedIn => vec![self.input.id()],
            TuiLoginPhase::AwaitingLogin { .. } | TuiLoginPhase::Failed { .. } => Vec::new(),
        }
    }

    fn keymap_context(&self, _ctx: &AppContext) -> keymap::Context {
        // Propagate focus context into the input view so keystrokes reach it.
        let mut context = keymap::Context::default();
        context.set.insert("RootTuiView");
        context
    }
}

impl TypedActionView for RootTuiView {
    // The root handles no typed actions itself; editing actions are handled by
    // the input view, and Ctrl-C quit is handled by the TUI driver.
    type Action = ();
}
