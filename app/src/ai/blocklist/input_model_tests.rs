//! Unit tests for [`BlocklistAIInputModel`] input handling.
//!
//! Covers [`resolve_history_match`], which pins down the NLD history-match
//! decision matrix between command history and agent prompt history, and the
//! policy-driven mechanism where the initial config, the locked-AI gate, and
//! the reactive subscriptions all defer to the injected [`InputModePolicy`].
//!
//! For [`resolve_history_match`], each [`HistoryMatch`] argument models one
//! history source: `NoMatch` means the source had no close match, `MatchedAt`
//! carries the matched entry's timestamp, and `MatchedWithoutTimestamp` is a
//! match with no timestamp (command-history-file entries may have no timestamp;
//! agent prompt entries always carry one).

use std::rc::Rc;
use std::sync::Arc;

use chrono::Duration;
use parking_lot::FairMutex;
use settings::Setting as _;
use warpui::r#async::executor::Background;
use warpui::{App, AppContext, EntityId, ModelHandle, SingletonEntity};

use super::*;
use crate::ai::agent::conversation::AIConversationId;
use crate::ai::blocklist::BlocklistAIContextModel;
use crate::ai::blocklist::agent_view::AgentViewEntryOrigin;
use crate::ai::blocklist::conversation_selection::{
    ConversationSelection, ConversationSelectionEvent, ConversationSelectionHandle,
    MockConversationSelection,
};
use crate::ai::blocklist::input_mode_policy::{InputModePolicy, PolicyConfigUpdate};
use crate::settings::{AISettings, AISettingsChangedEvent};
use crate::terminal::cli_agent_sessions::CLIAgentSessionsModel;
use crate::terminal::color::{self, Colors};
use crate::terminal::event_listener::ChannelEventListener;
use crate::terminal::model::TerminalModel;
use crate::terminal::model::test_utils::block_size;
use crate::test_util::settings::initialize_history_persistence_for_tests;

/// Returns a timestamp and a strictly-later timestamp, for ordering assertions.
fn earlier_and_later() -> (DateTime<Local>, DateTime<Local>) {
    let earlier = Local::now();
    let later = earlier + Duration::seconds(1);
    (earlier, later)
}

const HISTORY_MATCH_AI: Option<(InputType, InputTypeAutoDetectionSource)> =
    Some((InputType::AI, InputTypeAutoDetectionSource::HistoryMatch));
const HISTORY_MATCH_SHELL: Option<(InputType, InputTypeAutoDetectionSource)> =
    Some((InputType::Shell, InputTypeAutoDetectionSource::HistoryMatch));

#[test]
fn no_match_from_either_source_is_not_history_match() {
    // Neither command nor prompt history matched: the caller must fall through
    // to the classifier, so we cannot report a `HistoryMatch` decision.
    assert_eq!(
        resolve_history_match(HistoryMatch::NoMatch, HistoryMatch::NoMatch),
        None,
    );
}

#[test]
fn prompt_only_match_locks_to_ai_history_match() {
    let (_, prompt_ts) = earlier_and_later();
    assert_eq!(
        resolve_history_match(HistoryMatch::NoMatch, HistoryMatch::MatchedAt(prompt_ts)),
        HISTORY_MATCH_AI,
    );
}

#[test]
fn command_only_match_locks_to_shell_history_match() {
    let (command_ts, _) = earlier_and_later();
    assert_eq!(
        resolve_history_match(HistoryMatch::MatchedAt(command_ts), HistoryMatch::NoMatch),
        HISTORY_MATCH_SHELL,
    );
}

#[test]
fn command_only_match_without_timestamp_locks_to_shell_history_match() {
    // History-file commands can match without carrying a timestamp.
    assert_eq!(
        resolve_history_match(HistoryMatch::MatchedWithoutTimestamp, HistoryMatch::NoMatch),
        HISTORY_MATCH_SHELL,
    );
}

#[test]
fn both_match_prompt_newer_locks_to_ai() {
    let (command_ts, prompt_ts) = earlier_and_later();
    assert_eq!(
        resolve_history_match(
            HistoryMatch::MatchedAt(command_ts),
            HistoryMatch::MatchedAt(prompt_ts),
        ),
        HISTORY_MATCH_AI,
    );
}

#[test]
fn both_match_command_newer_locks_to_shell() {
    let (prompt_ts, command_ts) = earlier_and_later();
    assert_eq!(
        resolve_history_match(
            HistoryMatch::MatchedAt(command_ts),
            HistoryMatch::MatchedAt(prompt_ts),
        ),
        HISTORY_MATCH_SHELL,
    );
}

#[test]
fn both_match_equal_timestamps_prefer_shell() {
    // The newer-wins check is strict, so a tie cannot prove the prompt is more
    // recent and we preserve the Shell short-circuit.
    let ts = Local::now();
    assert_eq!(
        resolve_history_match(HistoryMatch::MatchedAt(ts), HistoryMatch::MatchedAt(ts)),
        HISTORY_MATCH_SHELL,
    );
}

#[test]
fn both_match_command_without_timestamp_locks_to_ai() {
    // A timestamped prompt match beats a command match with no timestamp
    // (e.g. a shell history-file entry): the prompt is the only entry whose
    // recency we can establish, so it is treated as more recent.
    let (_, prompt_ts) = earlier_and_later();
    assert_eq!(
        resolve_history_match(
            HistoryMatch::MatchedWithoutTimestamp,
            HistoryMatch::MatchedAt(prompt_ts),
        ),
        HISTORY_MATCH_AI,
    );
}

#[test]
fn both_match_prompt_without_timestamp_prefer_shell() {
    // Without a prompt timestamp we cannot prove the prompt is newer, so we
    // preserve the Shell short-circuit (prompt entries always carry a timestamp
    // in practice; this pins the defensive fallback).
    let (command_ts, _) = earlier_and_later();
    assert_eq!(
        resolve_history_match(
            HistoryMatch::MatchedAt(command_ts),
            HistoryMatch::MatchedWithoutTimestamp,
        ),
        HISTORY_MATCH_SHELL,
    );
    assert_eq!(
        resolve_history_match(
            HistoryMatch::MatchedWithoutTimestamp,
            HistoryMatch::MatchedWithoutTimestamp,
        ),
        HISTORY_MATCH_SHELL,
    );
}

const AI_LOCKED: InputConfig = InputConfig {
    input_type: InputType::AI,
    is_locked: true,
};
const SHELL_LOCKED: InputConfig = InputConfig {
    input_type: InputType::Shell,
    is_locked: true,
};
const SHELL_UNLOCKED: InputConfig = InputConfig {
    input_type: InputType::Shell,
    is_locked: false,
};

/// Configurable [`InputModePolicy`] stub.
struct StubPolicy {
    initial: InputConfig,
    allows_locked_ai: bool,
    on_conversation_activated: Option<InputConfig>,
    on_conversation_deactivated: Option<InputConfig>,
    on_settings_changed: Option<InputConfig>,
}

impl StubPolicy {
    /// A policy with `initial` config that permits locked AI and never reacts.
    fn inert(initial: InputConfig) -> Self {
        Self {
            initial,
            allows_locked_ai: true,
            on_conversation_activated: None,
            on_conversation_deactivated: None,
            on_settings_changed: None,
        }
    }
}

impl InputModePolicy for StubPolicy {
    fn initial_config(&self, _app: &AppContext) -> InputConfig {
        self.initial
    }

    fn allows_locked_ai_input(&self, _app: &AppContext) -> bool {
        self.allows_locked_ai
    }

    fn is_autodetection_enabled(&self, _app: &AppContext) -> bool {
        false
    }

    fn config_on_conversation_selection_changed(
        &self,
        event: &ConversationSelectionEvent,
        _current: InputConfig,
        _app: &AppContext,
    ) -> Option<PolicyConfigUpdate> {
        match event {
            ConversationSelectionEvent::Changed => None,
            ConversationSelectionEvent::Activated { .. } => {
                self.on_conversation_activated.map(PolicyConfigUpdate::new)
            }
            ConversationSelectionEvent::Deactivated { .. } => self
                .on_conversation_deactivated
                .map(PolicyConfigUpdate::new),
        }
    }

    fn config_on_ai_settings_changed(
        &self,
        _event: &AISettingsChangedEvent,
        _current: InputConfig,
        _is_autodetection_enabled_for_current_context: bool,
        _app: &AppContext,
    ) -> Option<PolicyConfigUpdate> {
        self.on_settings_changed.map(PolicyConfigUpdate::new)
    }
}

/// Builds an input model driven by `policy`, returning the conversation
/// selection handle so tests can emit selection events.
fn build_input_model(
    app: &mut App,
    policy: StubPolicy,
) -> (
    ModelHandle<BlocklistAIInputModel>,
    ConversationSelectionHandle,
) {
    initialize_history_persistence_for_tests(app);
    app.add_singleton_model(|_| CLIAgentSessionsModel::new());

    let terminal_model = Arc::new(FairMutex::new(TerminalModel::new_for_test(
        block_size(),
        color::List::from(&Colors::default()),
        ChannelEventListener::new_for_test(),
        Arc::new(Background::default()),
        false, /* should_show_bootstrap_block */
        None,  /* restored_blocks */
        false, /* honor_ps1 */
        false, /* is_inverted */
        None,  /* session_startup_path */
    )));
    let terminal_surface_id = EntityId::new();
    let conversation_selection =
        app.add_model(|_| Box::new(MockConversationSelection) as Box<dyn ConversationSelection>);
    let context_model = app.add_model(|_| {
        BlocklistAIContextModel::new_for_test(
            terminal_model.clone(),
            terminal_surface_id,
            conversation_selection.clone(),
        )
    });
    let input_model = app.add_model(|ctx| {
        BlocklistAIInputModel::new(
            terminal_model,
            conversation_selection.clone(),
            context_model,
            Rc::new(policy),
            terminal_surface_id,
            ctx,
        )
    });
    (input_model, conversation_selection)
}

#[test]
fn initial_config_comes_from_policy() {
    App::test((), |mut app| async move {
        // A locked-AI initial config sticks — a TUI-style policy is not subject
        // to any GUI gating.
        let (input_model, _) = build_input_model(&mut app, StubPolicy::inert(AI_LOCKED));
        input_model.read(&app, |model, _| {
            assert_eq!(model.input_config(), AI_LOCKED);
        });
    });
}

#[test]
fn locked_ai_write_requires_policy_permission() {
    App::test((), |mut app| async move {
        let policy = StubPolicy {
            allows_locked_ai: false,
            ..StubPolicy::inert(SHELL_UNLOCKED)
        };
        let (input_model, _) = build_input_model(&mut app, policy);

        // Rejected: the policy forbids locking to AI.
        input_model.update(&mut app, |model, ctx| {
            model.set_input_config(AI_LOCKED, true, None, ctx);
        });
        input_model.read(&app, |model, _| {
            assert_eq!(model.input_config(), SHELL_UNLOCKED);
        });

        // Locked shell (and unlocked AI) writes are not gated.
        input_model.update(&mut app, |model, ctx| {
            model.set_input_config(SHELL_LOCKED, true, None, ctx);
        });
        input_model.read(&app, |model, _| {
            assert_eq!(model.input_config(), SHELL_LOCKED);
        });
    });
}

#[test]
fn conversation_events_with_inert_policy_leave_config_unchanged() {
    App::test((), |mut app| async move {
        let (input_model, conversation_selection) =
            build_input_model(&mut app, StubPolicy::inert(AI_LOCKED));

        conversation_selection.update(&mut app, |_, ctx| {
            ctx.emit(ConversationSelectionEvent::Activated {
                is_fullscreen: true,
                origin: AgentViewEntryOrigin::Cli,
            });
            ctx.emit(ConversationSelectionEvent::Deactivated {
                conversation_id: AIConversationId::new(),
                final_exchange_count: 0,
                is_exit_before_new_entrance: false,
            });
        });

        input_model.read(&app, |model, _| {
            assert_eq!(model.input_config(), AI_LOCKED);
        });
    });
}

#[test]
fn conversation_events_apply_policy_updates() {
    App::test((), |mut app| async move {
        let policy = StubPolicy {
            on_conversation_activated: Some(SHELL_LOCKED),
            on_conversation_deactivated: Some(AI_LOCKED),
            ..StubPolicy::inert(SHELL_UNLOCKED)
        };
        let (input_model, conversation_selection) = build_input_model(&mut app, policy);

        conversation_selection.update(&mut app, |_, ctx| {
            ctx.emit(ConversationSelectionEvent::Activated {
                is_fullscreen: false,
                origin: AgentViewEntryOrigin::Cli,
            });
        });
        input_model.read(&app, |model, _| {
            assert_eq!(model.input_config(), SHELL_LOCKED);
        });

        conversation_selection.update(&mut app, |_, ctx| {
            ctx.emit(ConversationSelectionEvent::Deactivated {
                conversation_id: AIConversationId::new(),
                final_exchange_count: 0,
                is_exit_before_new_entrance: false,
            });
        });
        input_model.read(&app, |model, _| {
            assert_eq!(model.input_config(), AI_LOCKED);
        });
    });
}

#[test]
fn settings_change_applies_policy_update() {
    App::test((), |mut app| async move {
        let policy = StubPolicy {
            on_settings_changed: Some(SHELL_LOCKED),
            ..StubPolicy::inert(AI_LOCKED)
        };
        let (input_model, _) = build_input_model(&mut app, policy);

        AISettings::handle(&app).update(&mut app, |settings, ctx| {
            settings
                .ai_autodetection_enabled_internal
                .set_value(false, ctx)
                .unwrap();
        });

        input_model.read(&app, |model, _| {
            assert_eq!(model.input_config(), SHELL_LOCKED);
        });
    });
}
