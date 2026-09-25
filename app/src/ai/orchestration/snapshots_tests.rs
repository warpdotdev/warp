use ai::agent::action::RunAgentsExecutionMode;
use warp_cli::agent::Harness;
use warpui::App;

use super::{
    AUTH_SECRET_INHERIT_LABEL, AuthSecretNamesInput, DEFAULT_MODEL_LABEL, HarnessEntryInput,
    ModelChoiceInput, OptionBadge, OptionFooter, OptionSourceStatus, build_api_key_snapshot,
    build_environment_snapshot, build_harness_snapshot, build_host_snapshot,
    build_non_oz_model_snapshot, build_oz_model_snapshot, build_runner_snapshot,
    environment_snapshot,
};
use crate::ai::cloud_environments::{
    AmbientAgentEnvironment, CloudAmbientAgentEnvironment, CloudAmbientAgentEnvironmentModel,
};
use crate::ai::local_harness_setup::LocalHarnessSetupState;
use crate::ai::orchestration::config_state::{AuthSecretSelection, OrchestrationConfigState};
use crate::cloud_object::model::persistence::CloudModel;
use crate::cloud_object::{CloudObjectMetadata, CloudObjectPermissions, Owner};
use crate::server::ids::{ServerId, SyncId};
use crate::workspaces::user_workspaces::TeamContextForOperation;

fn entry(harness: Harness, display_name: &str, enabled: bool) -> HarnessEntryInput {
    HarnessEntryInput {
        harness,
        display_name: display_name.to_string(),
        enabled,
    }
}

fn all_ready(_harness: Harness) -> LocalHarnessSetupState {
    LocalHarnessSetupState::Ready
}

fn environment(id: SyncId, name: &str, owner: Owner) -> CloudAmbientAgentEnvironment {
    let model = AmbientAgentEnvironment::new(
        name.to_string(),
        None,
        Vec::new(),
        "ubuntu:latest".to_string(),
        Vec::new(),
    );
    let mut permissions = CloudObjectPermissions::mock_personal();
    permissions.owner = owner;
    CloudAmbientAgentEnvironment::new(
        id,
        CloudAmbientAgentEnvironmentModel::new(model),
        CloudObjectMetadata::mock(),
        permissions,
    )
}

// ── Harness ─────────────────────────────────────────────────────────

#[test]
fn harness_snapshot_excludes_gemini_and_selects_initial() {
    let entries = vec![
        entry(Harness::Oz, "Warp", true),
        entry(Harness::Claude, "Claude Code", true),
        entry(Harness::Gemini, "Gemini", true),
    ];

    let snapshot = build_harness_snapshot(entries, "claude", None, false, &all_ready);

    let ids: Vec<&str> = snapshot.rows.iter().map(|r| r.id.as_str()).collect();
    assert!(!ids.contains(&"gemini"));
    assert_eq!(snapshot.selected_id.as_deref(), Some("claude"));
    assert_eq!(snapshot.status, OptionSourceStatus::Ready);
    assert!(snapshot.rows.iter().all(|r| r.harness.is_some()));
}

#[test]
fn harness_snapshot_filters_product_disabled_local_harness() {
    let entries = vec![
        entry(Harness::Oz, "Warp", true),
        entry(Harness::Codex, "Codex", true),
    ];

    // Local Codex is product-disabled (feature flag off in tests).
    let snapshot = build_harness_snapshot(entries, "oz", None, true, &all_ready);

    let ids: Vec<&str> = snapshot.rows.iter().map(|r| r.id.as_str()).collect();
    assert_eq!(ids, vec!["oz"]);
}

#[test]
fn harness_snapshot_keeps_cloud_opencode_selectable() {
    let entries = vec![
        entry(Harness::Oz, "Warp", true),
        entry(Harness::OpenCode, "OpenCode", true),
    ];

    let snapshot = build_harness_snapshot(entries, "oz", None, false, &all_ready);

    let opencode = snapshot
        .rows
        .iter()
        .find(|r| r.id == "opencode")
        .expect("OpenCode row present on Cloud");
    // The harness list doesn't disable OpenCode; the accept gate does.
    assert_eq!(opencode.disabled_reason, None);
}

#[test]
fn harness_snapshot_marks_missing_local_cli_disabled_and_sorts_last() {
    let entries = vec![
        entry(Harness::Claude, "Claude Code", true),
        entry(Harness::Oz, "Warp", true),
    ];
    let setup = |harness: Harness| match harness {
        Harness::Claude => LocalHarnessSetupState::MissingHarness {
            tooltip: "Install Claude Code to use this local harness.",
        },
        Harness::Oz | Harness::OpenCode | Harness::Gemini | Harness::Codex | Harness::Unknown => {
            LocalHarnessSetupState::Ready
        }
    };

    let snapshot = build_harness_snapshot(entries, "oz", None, true, &setup);

    let ids: Vec<&str> = snapshot.rows.iter().map(|r| r.id.as_str()).collect();
    assert_eq!(ids, vec!["oz", "claude"]);
    assert_eq!(
        snapshot.rows[1].disabled_reason.as_deref(),
        Some("Install Claude Code to use this local harness.")
    );
}

#[test]
fn harness_snapshot_marks_server_disabled_entries() {
    let entries = vec![
        entry(Harness::Oz, "Warp", true),
        entry(Harness::Claude, "Claude Code", false),
    ];

    let snapshot = build_harness_snapshot(entries, "oz", None, false, &all_ready);

    assert_eq!(
        snapshot.rows[1].disabled_reason.as_deref(),
        Some("Disabled by your administrator")
    );
}

#[test]
fn harness_snapshot_matches_selection_by_display_name_for_stale_cache() {
    // Stale cache: harness deserialized as Unknown but display_name intact.
    let entries = vec![entry(Harness::Unknown, "Claude Code", true)];

    let snapshot = build_harness_snapshot(
        entries,
        "claude",
        Some("Claude Code".to_string()),
        false,
        &all_ready,
    );

    assert_eq!(snapshot.selected_id.as_deref(), Some("claude"));
}

// ── Model ───────────────────────────────────────────────────────────

fn model(id: &str, label: &str) -> ModelChoiceInput {
    ModelChoiceInput {
        id: id.to_string(),
        label: label.to_string(),
        disabled_reason: None,
    }
}

#[test]
fn oz_model_snapshot_empty_catalog_reports_empty_status() {
    let snapshot = build_oz_model_snapshot(Vec::new(), "auto");
    assert!(matches!(snapshot.status, OptionSourceStatus::Empty { .. }));
}
/// Disabled model metadata remains available to every snapshot consumer.
#[test]
fn oz_model_snapshot_carries_disabled_reason() {
    let mut disabled_model = model("unavailable", "Unavailable");
    disabled_model.disabled_reason = Some("This model is unavailable.".to_string());

    let snapshot = build_oz_model_snapshot(vec![disabled_model], "");

    assert_eq!(
        snapshot.rows[0].disabled_reason.as_deref(),
        Some("This model is unavailable.")
    );
}

#[test]
fn non_oz_model_snapshot_puts_default_first_and_selects_server_model() {
    let snapshot = build_non_oz_model_snapshot(
        Some(vec![model("opus", "Opus"), model("sonnet", "Sonnet")]),
        "sonnet",
    );

    assert_eq!(snapshot.rows[0].label, DEFAULT_MODEL_LABEL);
    assert_eq!(snapshot.rows[0].id, "");
    assert_eq!(snapshot.selected_id.as_deref(), Some("sonnet"));
}

#[test]
fn non_oz_model_snapshot_falls_back_to_default_for_unknown_or_empty_id() {
    for initial in ["", "gone"] {
        let snapshot = build_non_oz_model_snapshot(Some(vec![model("opus", "Opus")]), initial);
        assert_eq!(snapshot.selected_id.as_deref(), Some(""));
    }
    // No server catalog at all: only the Default model row.
    let snapshot = build_non_oz_model_snapshot(None, "");
    assert_eq!(snapshot.rows.len(), 1);
    assert_eq!(snapshot.selected_id.as_deref(), Some(""));
}

// ── API key ─────────────────────────────────────────────────────────

#[test]
fn api_key_snapshot_lists_skip_then_names() {
    let snapshot = build_api_key_snapshot(
        AuthSecretNamesInput::Loaded(vec!["key-a".to_string(), "key-b".to_string()]),
        &AuthSecretSelection::Named("key-b".to_string()),
        true,
    );

    let labels: Vec<&str> = snapshot.rows.iter().map(|r| r.label.as_str()).collect();
    assert_eq!(labels, vec![AUTH_SECRET_INHERIT_LABEL, "key-a", "key-b"]);
    assert_eq!(snapshot.selected_id.as_deref(), Some("key-b"));
    assert_eq!(snapshot.status, OptionSourceStatus::Ready);
    assert_eq!(snapshot.footer, Some(OptionFooter::CreateNewAuthSecret));
}

#[test]
fn api_key_snapshot_keeps_named_selection_while_loading() {
    let snapshot = build_api_key_snapshot(
        AuthSecretNamesInput::NotLoaded,
        &AuthSecretSelection::Named("my-key".to_string()),
        true,
    );
    assert_eq!(snapshot.selected_id.as_deref(), Some("my-key"));
}

#[test]
fn api_key_snapshot_maps_inherit_and_unset_selection() {
    let inherit = build_api_key_snapshot(
        AuthSecretNamesInput::Loaded(vec![]),
        &AuthSecretSelection::Inherit,
        true,
    );
    assert_eq!(inherit.selected_id.as_deref(), Some(""));

    let unset = build_api_key_snapshot(
        AuthSecretNamesInput::Loaded(vec![]),
        &AuthSecretSelection::Unset,
        true,
    );
    assert_eq!(unset.selected_id, None);
}

// ── Host ────────────────────────────────────────────────────────────

#[test]
fn host_snapshot_orders_default_warp_connected_recent() {
    let snapshot = build_host_snapshot(
        Some("team-default".to_string()),
        Some("recent-host".to_string()),
        vec!["worker-1".to_string()],
        "warp",
    );

    let ids: Vec<&str> = snapshot.rows.iter().map(|r| r.id.as_str()).collect();
    assert_eq!(ids, vec!["team-default", "warp", "worker-1", "recent-host"]);
    assert_eq!(snapshot.rows[0].badge, Some(OptionBadge::Default));
    assert_eq!(snapshot.rows[2].badge, Some(OptionBadge::Connected));
    assert_eq!(snapshot.rows[3].badge, Some(OptionBadge::Recent));
    assert_eq!(snapshot.selected_id.as_deref(), Some("warp"));
    assert!(matches!(
        snapshot.footer,
        Some(OptionFooter::CustomText { .. })
    ));
}

#[test]
fn host_snapshot_dedupes_connected_and_recent_against_known_rows() {
    let snapshot = build_host_snapshot(
        Some("team-default".to_string()),
        Some("team-default".to_string()),
        vec!["warp".to_string(), "team-default".to_string()],
        "team-default",
    );

    let ids: Vec<&str> = snapshot.rows.iter().map(|r| r.id.as_str()).collect();
    assert_eq!(ids, vec!["team-default", "warp"]);
}

// ── Environment ─────────────────────────────────────────────────────

#[test]
fn environment_snapshot_puts_empty_option_first() {
    let snapshot = build_environment_snapshot(
        vec![
            ("env-a".to_string(), "Alpha".to_string()),
            ("env-b".to_string(), "Beta".to_string()),
        ],
        "env-b",
    );

    assert_eq!(snapshot.rows[0].id, "");
    assert_eq!(snapshot.rows[0].label, super::ORCHESTRATION_ENV_NONE_LABEL);
    assert_eq!(snapshot.selected_id.as_deref(), Some("env-b"));
}

#[test]
fn environment_snapshot_shows_personal_and_current_team_environments() {
    App::test((), |mut app| async move {
        let cloud_model = app.add_singleton_model(CloudModel::mock);
        let current_team_uid = ServerId::from(100);
        let other_team_uid = ServerId::from(200);
        let personal_id = SyncId::ServerId(ServerId::from(1));
        let current_team_id = SyncId::ServerId(ServerId::from(2));
        let other_team_id = SyncId::ServerId(ServerId::from(3));

        cloud_model.update(&mut app, |model, ctx| {
            model.create_object(
                personal_id,
                environment(personal_id, "Personal", Owner::mock_current_user()),
                ctx,
            );
            model.create_object(
                current_team_id,
                environment(
                    current_team_id,
                    "Current team",
                    Owner::Team {
                        team_uid: current_team_uid,
                    },
                ),
                ctx,
            );
            model.create_object(
                other_team_id,
                environment(
                    other_team_id,
                    "Other team",
                    Owner::Team {
                        team_uid: other_team_uid,
                    },
                ),
                ctx,
            );
        });

        let state = OrchestrationConfigState::from_run_agents_fields(
            None,
            None,
            &RunAgentsExecutionMode::Remote {
                environment_id: current_team_id.uid(),
                worker_host: "warp".to_string(),
                computer_use_enabled: false,
                runner_id: String::new(),
            },
        );
        let scope = TeamContextForOperation::new_for_test(current_team_uid);
        let snapshot = app.update(|ctx| environment_snapshot(&state, &scope, ctx));

        assert_eq!(
            snapshot
                .rows
                .iter()
                .map(|row| row.label.as_str())
                .collect::<Vec<_>>(),
            vec!["Empty environment", "Current team", "Personal"]
        );
        assert_eq!(snapshot.selected_id, Some(current_team_id.uid()));
    });
}

// ── Runner ──────────────────────────────────────────────────────

#[test]
fn runner_snapshot_puts_use_default_first_and_selects() {
    let snapshot = build_runner_snapshot(
        vec![
            ("r-a".to_string(), "Alpha".to_string()),
            ("r-b".to_string(), "Beta".to_string()),
        ],
        "r-b",
        false,
    );

    assert_eq!(snapshot.rows[0].id, "");
    assert_eq!(
        snapshot.rows[0].label,
        super::ORCHESTRATION_RUNNER_NONE_LABEL
    );
    assert_eq!(snapshot.selected_id.as_deref(), Some("r-b"));
    assert_eq!(snapshot.status, OptionSourceStatus::Ready);
}

#[test]
fn runner_snapshot_loading_reports_loading_status() {
    let snapshot = build_runner_snapshot(vec![], "", true);
    assert_eq!(snapshot.status, OptionSourceStatus::Loading);
    // Empty selection maps to the "use environment default" row.
    assert_eq!(snapshot.selected_id.as_deref(), Some(""));
}
