use super::{
    default_collapsible_state_for_orchestration_action, received_message_collapsible_id,
    CollapsibleElementState, CollapsibleExpansionState,
};
use crate::ai::agent::{AIAgentActionType, StartAgentExecutionMode};
use crate::ai::blocklist::action_model::{
    compose_run_agents_child_prompt, run_agents_to_start_agent_mode,
};
use crate::settings::AISettings;
use crate::test_util::settings::initialize_settings_for_tests;
use ai::agent::action::{RunAgentsAgentRunConfig, RunAgentsExecutionMode};
use ai::agent::action_result::StartAgentVersion;
use ai::skills::SkillReference;
use settings::Setting;
use std::path::PathBuf;
use warp_core::features::FeatureFlag;
use warpui::{App, SingletonEntity};

#[test]
fn reasoning_auto_collapses_when_user_has_not_manually_toggled() {
    App::test((), |mut app| async move {
        initialize_settings_for_tests(&mut app);
        let mut state = CollapsibleElementState::default();
        app.update(|ctx| {
            state.finish_reasoning(ctx);
        });

        assert!(matches!(
            state.expansion_state,
            CollapsibleExpansionState::Collapsed
        ));
    });
}

#[test]
fn collapsed_initializer_starts_collapsed() {
    let state = CollapsibleElementState::collapsed();

    assert!(matches!(
        state.expansion_state,
        CollapsibleExpansionState::Collapsed
    ));
}

#[test]
fn orchestration_send_message_starts_collapsed() {
    let state = default_collapsible_state_for_orchestration_action(
        &AIAgentActionType::SendMessageToAgent {
            addresses: vec!["child-agent".to_string()],
            subject: "Status".to_string(),
            message: "Body".to_string(),
        },
    )
    .expect("send-message actions should get a collapsible state");

    assert!(matches!(
        state.expansion_state,
        CollapsibleExpansionState::Collapsed
    ));
}

#[test]
fn orchestration_start_agent_keeps_expanded_default() {
    let state =
        default_collapsible_state_for_orchestration_action(&AIAgentActionType::StartAgent {
            version: StartAgentVersion::V1,
            name: "child-agent".to_string(),
            prompt: "Investigate".to_string(),
            execution_mode: StartAgentExecutionMode::local_harness("claude-code".to_string()),
            lifecycle_subscription: None,
        })
        .expect("start-agent actions should get a collapsible state");

    assert!(matches!(
        state.expansion_state,
        CollapsibleExpansionState::Expanded {
            is_finished: false,
            scroll_pinned_to_bottom: true
        }
    ));
}

#[test]
fn non_orchestration_actions_do_not_get_collapsible_state_defaults() {
    assert!(
        default_collapsible_state_for_orchestration_action(&AIAgentActionType::OpenCodeReview)
            .is_none()
    );
}

#[test]
fn always_show_thinking_stays_expanded_after_finish() {
    App::test((), |mut app| async move {
        initialize_settings_for_tests(&mut app);
        AISettings::handle(&app).update(&mut app, |settings, ctx| {
            settings
                .thinking_display_mode
                .set_value(crate::settings::ThinkingDisplayMode::AlwaysShow, ctx)
                .unwrap();
        });

        let mut state = CollapsibleElementState::default();
        app.update(|ctx| {
            state.finish_reasoning(ctx);
        });

        assert!(matches!(
            state.expansion_state,
            CollapsibleExpansionState::Expanded {
                is_finished: true,
                scroll_pinned_to_bottom: false
            }
        ));
    });
}

#[test]
fn manual_collapse_while_streaming_stays_collapsed_after_finish() {
    App::test((), |mut app| async move {
        initialize_settings_for_tests(&mut app);
        let mut state = CollapsibleElementState::default();

        state.toggle_expansion();
        app.update(|ctx| {
            state.finish_reasoning(ctx);
        });

        assert!(matches!(
            state.expansion_state,
            CollapsibleExpansionState::Collapsed
        ));
    });
}

#[test]
fn manual_reexpand_while_streaming_stays_expanded_after_finish() {
    App::test((), |mut app| async move {
        initialize_settings_for_tests(&mut app);
        let mut state = CollapsibleElementState::default();

        state.toggle_expansion();
        state.toggle_expansion();
        app.update(|ctx| {
            state.finish_reasoning(ctx);
        });

        assert!(matches!(
            state.expansion_state,
            CollapsibleExpansionState::Expanded {
                is_finished: true,
                scroll_pinned_to_bottom: false
            }
        ));
    });
}

#[test]
fn received_message_collapsible_id_prefixes_row_ids() {
    let first = received_message_collapsible_id("message-1");
    let second = received_message_collapsible_id("message-2");

    assert_eq!(&*first, "received-message:message-1");
    assert_eq!(&*second, "received-message:message-2");
    assert_ne!(first, second);
}

#[test]
fn compose_child_prompt_concatenates_when_both_non_empty() {
    let composed = compose_run_agents_child_prompt("base", "do X");
    assert_eq!(composed, "base\n\ndo X");
}

#[test]
fn compose_child_prompt_uses_base_only_when_per_agent_empty() {
    let composed = compose_run_agents_child_prompt("base", "");
    assert_eq!(composed, "base");
}

#[test]
fn compose_child_prompt_uses_per_agent_only_when_base_empty() {
    let composed = compose_run_agents_child_prompt("", "do X");
    assert_eq!(composed, "do X");
}

#[test]
fn compose_child_prompt_returns_empty_when_both_empty() {
    let composed = compose_run_agents_child_prompt("", "");
    assert_eq!(composed, "");
}

#[test]
fn compose_child_prompt_treats_whitespace_only_base_as_empty() {
    let composed = compose_run_agents_child_prompt("   \n", "do X");
    assert_eq!(composed, "do X");
}

fn agent_cfg() -> RunAgentsAgentRunConfig {
    RunAgentsAgentRunConfig {
        name: "child".to_string(),
        prompt: "do X".to_string(),
        title: "Child".to_string(),
    }
}

#[test]
fn remote_arm_propagates_skills_into_skill_references() {
    let skills = vec![
        SkillReference::BundledSkillId("writing-pr-descriptions".to_string()),
        SkillReference::Path(PathBuf::from("/tmp/skill/SKILL.md")),
    ];
    let mode = run_agents_to_start_agent_mode(
        &RunAgentsExecutionMode::Remote {
            environment_id: "env-1".to_string(),
            worker_host: "warp".to_string(),
            computer_use_enabled: true,
        },
        "oz",
        "auto",
        &skills,
        None,
        &agent_cfg(),
    )
    .expect("Remote+oz must convert");
    let StartAgentExecutionMode::Remote {
        skill_references,
        environment_id,
        worker_host,
        harness_type,
        model_id,
        computer_use_enabled,
        title,
        auth_secret_name,
    } = mode
    else {
        panic!("expected Remote start-agent mode");
    };
    assert_eq!(skill_references, skills);
    assert_eq!(environment_id, "env-1");
    assert_eq!(worker_host, "warp");
    assert_eq!(harness_type, "oz");
    assert_eq!(model_id, "auto");
    assert!(computer_use_enabled);
    assert_eq!(title, "Child");
    assert_eq!(auth_secret_name, None);
}

#[test]
fn remote_arm_with_empty_skills_propagates_empty_vec() {
    let mode = run_agents_to_start_agent_mode(
        &RunAgentsExecutionMode::Remote {
            environment_id: "env-1".to_string(),
            worker_host: "warp".to_string(),
            computer_use_enabled: false,
        },
        "claude",
        "auto",
        &[],
        None,
        &agent_cfg(),
    )
    .expect("Remote+claude must convert");
    let StartAgentExecutionMode::Remote {
        skill_references, ..
    } = mode
    else {
        panic!("expected Remote start-agent mode");
    };
    assert!(skill_references.is_empty());
}

#[test]
fn remote_arm_rejects_opencode() {
    let err = run_agents_to_start_agent_mode(
        &RunAgentsExecutionMode::Remote {
            environment_id: "env-1".to_string(),
            worker_host: "warp".to_string(),
            computer_use_enabled: false,
        },
        "opencode",
        "auto",
        &[],
        None,
        &agent_cfg(),
    )
    .expect_err("Remote+opencode must be rejected");
    assert!(err.to_lowercase().contains("opencode"));
}

#[test]
fn local_arm_rejects_disabled_claude() {
    let err = run_agents_to_start_agent_mode(
        &RunAgentsExecutionMode::Local,
        "claude",
        "auto",
        &[],
        None,
        &agent_cfg(),
    )
    .expect_err("Local+claude must be rejected while disabled");
    assert_eq!(
        err,
        "Local Claude Code child agents are temporarily disabled."
    );
}

#[test]
fn local_arm_allows_claude_when_feature_enabled() {
    let _local_harnesses = FeatureFlag::LocalClaudeCodexChildHarnesses.override_enabled(true);

    let mode = run_agents_to_start_agent_mode(
        &RunAgentsExecutionMode::Local,
        "claude",
        "auto",
        &[],
        None,
        &agent_cfg(),
    )
    .expect("Local+claude should convert when feature is enabled");
    assert!(matches!(
        mode,
        StartAgentExecutionMode::Local {
            harness_type: Some(ref harness_type),
            model_id: Some(ref model_id),
        } if harness_type == "claude" && model_id == "auto"
    ));
}

#[test]
fn remote_arm_propagates_claude_auth_secret_into_mode() {
    let mode = run_agents_to_start_agent_mode(
        &RunAgentsExecutionMode::Remote {
            environment_id: "env-1".to_string(),
            worker_host: "warp".to_string(),
            computer_use_enabled: false,
        },
        "claude",
        "auto",
        &[],
        Some("my-claude-key"),
        &agent_cfg(),
    )
    .expect("Remote+claude must convert");
    let StartAgentExecutionMode::Remote {
        auth_secret_name, ..
    } = mode
    else {
        panic!("expected Remote start-agent mode");
    };
    assert_eq!(auth_secret_name.as_deref(), Some("my-claude-key"));
}

#[test]
fn remote_arm_filters_whitespace_auth_secret_name_to_none() {
    let mode = run_agents_to_start_agent_mode(
        &RunAgentsExecutionMode::Remote {
            environment_id: "env-1".to_string(),
            worker_host: "warp".to_string(),
            computer_use_enabled: false,
        },
        "codex",
        "auto",
        &[],
        Some("   "),
        &agent_cfg(),
    )
    .expect("Remote+codex must convert");
    let StartAgentExecutionMode::Remote {
        auth_secret_name, ..
    } = mode
    else {
        panic!("expected Remote start-agent mode");
    };
    assert_eq!(auth_secret_name, None);
}

#[test]
fn local_arm_ignores_auth_secret_name() {
    let _local_harnesses = FeatureFlag::LocalClaudeCodexChildHarnesses.override_enabled(true);
    let mode = run_agents_to_start_agent_mode(
        &RunAgentsExecutionMode::Local,
        "claude",
        "auto",
        &[],
        Some("my-claude-key"),
        &agent_cfg(),
    )
    .expect("Local+claude should convert when feature is enabled");
    // Local children don't carry an auth_secret_name field.
    assert!(matches!(mode, StartAgentExecutionMode::Local { .. }));
}

#[test]
fn should_show_agent_mode_ask_user_question_speedbump_defaults_to_true() {
    App::test((), |mut app| async move {
        initialize_settings_for_tests(&mut app);
        AISettings::handle(&app).read(&app, |settings, _ctx| {
            assert!(*settings.should_show_agent_mode_ask_user_question_speedbump);
        });
    });
}

#[test]
fn should_show_agent_mode_ask_user_question_speedbump_round_trips_to_false() {
    App::test((), |mut app| async move {
        initialize_settings_for_tests(&mut app);
        AISettings::handle(&app).update(&mut app, |settings, ctx| {
            settings
                .should_show_agent_mode_ask_user_question_speedbump
                .set_value(false, ctx)
                .unwrap();
        });
        AISettings::handle(&app).read(&app, |settings, _ctx| {
            assert!(!*settings.should_show_agent_mode_ask_user_question_speedbump);
        });
    });
}
