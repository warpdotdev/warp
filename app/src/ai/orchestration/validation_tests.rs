use ai::agent::action::RunAgentsExecutionMode;
use warpui::{App, AppContext, Entity};

use super::accept_disabled_reason_with_auth;
use crate::ai::orchestration::config_state::{AuthSecretSelection, OrchestrationConfigState};

/// Minimal entity used to borrow an `AppContext` inside `App::test`.
struct CtxProbe;

impl Entity for CtxProbe {
    type Event = ();
}

/// Runs `f` with a plain `AppContext` (no singletons registered).
fn with_app_ctx(f: impl FnOnce(&AppContext) + 'static) {
    App::test((), |mut app| async move {
        let probe = app.add_model(|_| CtxProbe);
        probe.update(&mut app, |_, ctx| f(ctx));
    });
}

fn state(
    harness: &str,
    mode: RunAgentsExecutionMode,
    auth: AuthSecretSelection,
) -> OrchestrationConfigState {
    let mut state =
        OrchestrationConfigState::from_run_agents_fields(Some("auto"), Some(harness), &mode);
    state.auth_secret_selection = auth;
    state
}

fn cloud() -> RunAgentsExecutionMode {
    RunAgentsExecutionMode::Remote {
        environment_id: "env-1".to_string(),
        worker_host: "warp".to_string(),
        computer_use_enabled: false,
    }
}

#[test]
fn accept_allowed_for_oz_local_and_cloud() {
    with_app_ctx(|ctx| {
        for mode in [RunAgentsExecutionMode::Local, cloud()] {
            let state = state("oz", mode, AuthSecretSelection::Unset);
            assert_eq!(accept_disabled_reason_with_auth(&state, ctx), None);
        }
    });
}

#[test]
fn accept_blocked_for_product_disabled_local_codex() {
    with_app_ctx(|ctx| {
        let state = state(
            "codex",
            RunAgentsExecutionMode::Local,
            AuthSecretSelection::Unset,
        );
        assert_eq!(
            accept_disabled_reason_with_auth(&state, ctx),
            Some("Local Codex child agents are temporarily disabled.".to_string())
        );
    });
}

#[test]
fn accept_blocked_for_opencode_cloud() {
    with_app_ctx(|ctx| {
        let state = state("opencode", cloud(), AuthSecretSelection::Unset);
        let reason = accept_disabled_reason_with_auth(&state, ctx)
            .expect("OpenCode + Cloud should block Accept");
        assert!(reason.contains("OpenCode"));
    });
}

#[test]
fn accept_blocked_for_cloud_harness_with_unset_auth_secret() {
    with_app_ctx(|ctx| {
        for harness in ["claude", "codex"] {
            for auth in [AuthSecretSelection::Unset, AuthSecretSelection::CreatingNew] {
                let state = state(harness, cloud(), auth);
                assert_eq!(
                    accept_disabled_reason_with_auth(&state, ctx),
                    Some("Select an API key for this harness to continue.".to_string()),
                    "Cloud + {harness} without an API key choice should block Accept"
                );
            }
        }
    });
}

#[test]
fn accept_allowed_for_cloud_harness_with_named_or_inherited_auth() {
    with_app_ctx(|ctx| {
        for harness in ["claude", "codex"] {
            for auth in [
                AuthSecretSelection::Named("my-key".to_string()),
                AuthSecretSelection::Inherit,
            ] {
                let state = state(harness, cloud(), auth);
                assert_eq!(accept_disabled_reason_with_auth(&state, ctx), None);
            }
        }
    });
}
