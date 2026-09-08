//! View-supplied policy for input-mode decisions.
//!
//! [`BlocklistAIInputModel`](super::BlocklistAIInputModel) is shared between
//! frontends (the GUI terminal input and the TUI prompt input), but several of
//! its decisions depend on view concepts the model cannot know about — e.g.
//! whether the surface distinguishes "fullscreen agent view" from "top-level
//! terminal", or whether locking the input to AI is allowed outside a
//! conversation. Each frontend supplies those answers via [`InputModePolicy`],
//! mirroring how [`ConversationSelection`](super::conversation_selection::ConversationSelection)
//! injects per-view selection semantics.
//!
//! The GUI implementation lives in
//! `super::agent_view::GuiInputModePolicy`; the TUI implementation lives in
//! `crates/warp_tui/src/input_mode_policy.rs`.

use std::rc::Rc;

use warpui::AppContext;

use super::conversation_selection::ConversationSelectionEvent;
use super::input_model::{InputConfig, InputTypeAutoDetectionSource};
use crate::settings::AISettingsChangedEvent;

/// A config write produced by an [`InputModePolicy`] decision. The fields
/// mirror exactly what the previously-inlined GUI decision code passed to the
/// model's internal setter: the config, the decision source recorded with it,
/// and (for one agent-view entry path) a brief autodetection suppression so
/// the applied config isn't immediately overridden by a keystroke-driven
/// detection pass.
pub struct PolicyConfigUpdate {
    /// The config to apply.
    pub config: InputConfig,
    /// The decision source recorded alongside the config.
    pub decision_source: Option<InputTypeAutoDetectionSource>,
    /// Whether to briefly suppress autodetection before applying.
    pub temporarily_disable_autodetection: bool,
}

impl PolicyConfigUpdate {
    /// An update with no decision source and no autodetection suppression.
    pub fn new(config: InputConfig) -> Self {
        Self {
            config,
            decision_source: None,
            temporarily_disable_autodetection: false,
        }
    }

    /// An update recorded with `decision_source`, without autodetection
    /// suppression.
    pub fn with_source(config: InputConfig, decision_source: InputTypeAutoDetectionSource) -> Self {
        Self {
            config,
            decision_source: Some(decision_source),
            temporarily_disable_autodetection: false,
        }
    }
}

/// Per-view policy consulted by [`BlocklistAIInputModel`](super::BlocklistAIInputModel)
/// for decisions it cannot make view-agnostically: lock gating, the
/// autodetection setting for the surface's current context, and reactive
/// config transitions driven by conversation-selection and settings events.
///
/// The reactive hooks receive the raw event and decide the config to apply,
/// so view-specific event payloads (fullscreen vs. inline, entry origins)
/// stay a concern of the implementing view.
pub trait InputModePolicy: 'static {
    /// The config the surface starts with.
    fn initial_config(&self, app: &AppContext) -> InputConfig;

    /// Whether the input may currently be locked to AI. When this returns
    /// `false`, `{AI, locked}` config writes are rejected.
    fn allows_locked_ai_input(&self, app: &AppContext) -> bool;

    /// Whether NL autodetection is enabled for the surface's current context.
    /// This is the raw setting lookup; the model layers its own view-agnostic
    /// guards (agent-in-control, pending attachments) on top.
    fn is_autodetection_enabled(&self, app: &AppContext) -> bool;

    /// The config to apply in response to a conversation-selection event, or
    /// `None` to leave the config unchanged.
    fn config_on_conversation_selection_changed(
        &self,
        event: &ConversationSelectionEvent,
        current: InputConfig,
        app: &AppContext,
    ) -> Option<PolicyConfigUpdate>;

    /// The config to apply when AI settings change, or `None` to leave the
    /// config unchanged. `is_autodetection_enabled_for_current_context` is the
    /// model's guarded autodetection state (agent-in-control and attachment
    /// checks layered over [`Self::is_autodetection_enabled`]). Computing it
    /// takes the terminal-model lock, so the model only computes it for
    /// `AIAutoDetectionEnabled` events; for all other events it is `false`.
    fn config_on_ai_settings_changed(
        &self,
        event: &AISettingsChangedEvent,
        current: InputConfig,
        is_autodetection_enabled_for_current_context: bool,
        app: &AppContext,
    ) -> Option<PolicyConfigUpdate>;
}

/// Shared handle to a view-supplied [`InputModePolicy`].
pub type InputModePolicyHandle = Rc<dyn InputModePolicy>;
