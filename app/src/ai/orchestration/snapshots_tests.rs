use warp_cli::agent::Harness;

use super::{
    build_api_key_snapshot, build_environment_snapshot, build_harness_snapshot,
    build_host_snapshot, build_non_oz_model_snapshot, build_oz_model_snapshot,
    AuthSecretNamesInput, HarnessEntryInput, ModelChoiceInput, OptionBadge, OptionFooter,
    OptionSourceStatus, AUTH_SECRET_INHERIT_LABEL, DEFAULT_MODEL_LABEL,
};
use crate::ai::local_harness_setup::LocalHarnessSetupState;
use crate::ai::orchestration::config_state::AuthSecretSelection;

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
    }
}

#[test]
fn oz_model_snapshot_selects_matching_id() {
    let snapshot = build_oz_model_snapshot(
        vec![model("auto", "Auto"), model("sonnet", "Sonnet")],
        "sonnet",
    );

    assert_eq!(snapshot.selected_id.as_deref(), Some("sonnet"));
    assert_eq!(snapshot.rows.len(), 2);
    assert_eq!(snapshot.status, OptionSourceStatus::Ready);
}

#[test]
fn oz_model_snapshot_with_unknown_id_has_no_selection() {
    let snapshot = build_oz_model_snapshot(vec![model("auto", "Auto")], "gone");
    assert_eq!(snapshot.selected_id, None);
}

#[test]
fn oz_model_snapshot_empty_catalog_reports_empty_status() {
    let snapshot = build_oz_model_snapshot(Vec::new(), "auto");
    assert!(matches!(snapshot.status, OptionSourceStatus::Empty { .. }));
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
fn api_key_snapshot_maps_fetch_states_to_statuses() {
    let loading = build_api_key_snapshot(
        AuthSecretNamesInput::NotLoaded,
        &AuthSecretSelection::Unset,
        true,
    );
    assert_eq!(loading.status, OptionSourceStatus::Loading);

    let failed = build_api_key_snapshot(
        AuthSecretNamesInput::Failed,
        &AuthSecretSelection::Unset,
        true,
    );
    assert!(matches!(failed.status, OptionSourceStatus::Failed { .. }));
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
fn environment_snapshot_selects_empty_option_for_empty_id() {
    let snapshot = build_environment_snapshot(Vec::new(), "");
    assert_eq!(snapshot.selected_id.as_deref(), Some(""));
}

#[test]
fn environment_snapshot_has_no_selection_for_missing_env() {
    let snapshot =
        build_environment_snapshot(vec![("env-a".to_string(), "Alpha".to_string())], "gone");
    assert_eq!(snapshot.selected_id, None);
}
