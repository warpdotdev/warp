use std::time::Duration;

use chrono::{TimeZone as _, Utc};
use futures::executor::block_on;

use super::*;
use crate::server::server_api::factory::{FactorySyncPlan, MockFactoryClient};

fn export_response(entries: &[(&str, &str)]) -> FactoryExportResponse {
    FactoryExportResponse {
        files: entries
            .iter()
            .map(|(path, content)| (path.to_string(), content.to_string()))
            .collect(),
    }
}

fn planned_change(path: &str, kind: &str, reason: &str) -> FactoryPlannedChange {
    FactoryPlannedChange {
        path: path.to_string(),
        kind: kind.to_string(),
        reason: reason.to_string(),
    }
}

fn resource_error(path: &str, line: Option<i64>, message: &str) -> FactoryResourceError {
    FactoryResourceError {
        resource_path: path.to_string(),
        line,
        message: message.to_string(),
    }
}

fn sync_summary(commit_sha: &str, status: FactorySyncState) -> FactorySyncSummary {
    FactorySyncSummary {
        commit_sha: commit_sha.to_string(),
        status,
        started_at: Utc.with_ymd_and_hms(2026, 7, 8, 5, 0, 0).unwrap(),
        finished_at: Some(Utc.with_ymd_and_hms(2026, 7, 8, 5, 0, 10).unwrap()),
        resource_errors: vec![],
        degraded_reasons: vec![],
    }
}

fn file_managed_status(latest_sync: Option<FactorySyncSummary>) -> FactorySyncStatusResponse {
    FactorySyncStatusResponse {
        management_mode: "file_managed".to_string(),
        source: Some(FactorySource {
            code_forge: "GITHUB".to_string(),
            repository: FactoryRepository {
                owner: "warpdotdev".to_string(),
                repo: "factory-config".to_string(),
            },
            r#ref: "main".to_string(),
            path: "factories/prod".to_string(),
        }),
        last_synced_commit: Some("aaa111".to_string()),
        latest_sync,
    }
}

fn render_status_text(status: &FactorySyncStatusResponse) -> String {
    let mut output = Vec::new();
    write_status(status, OutputFormat::Text, &mut output).expect("status renders");
    String::from_utf8(output).expect("output is valid utf-8")
}

fn render_plan_text(result: &FactorySyncDryRunResult) -> anyhow::Result<String> {
    let mut output = Vec::new();
    write_plan(result, OutputFormat::Text, &mut output)?;
    Ok(String::from_utf8(output).expect("output is valid utf-8"))
}

#[test]
fn plan_rendering_matches_golden() {
    let result = FactorySyncDryRunResult {
        commit_sha: "abc123".to_string(),
        plan: Some(FactorySyncPlan {
            creates: vec![
                planned_change("agents/reviewer.md", "Agent", "new resource"),
                planned_change("automations/nightly.md", "Automation", "new resource"),
            ],
            updates: vec![planned_change(
                "environments/prod.yaml",
                "Environment",
                "spec changed",
            )],
            deletes: vec![planned_change("runners/legacy.yaml", "Runner", "file removed")],
            no_ops: 3,
        }),
        resource_errors: vec![],
    };

    let rendered = render_plan_text(&result).unwrap();

    let golden = "\
Plan for commit abc123:

Create (2):
+ agents/reviewer.md (Agent): new resource
+ automations/nightly.md (Automation): new resource

Update (1):
~ environments/prod.yaml (Environment): spec changed

Delete (1):
- runners/legacy.yaml (Runner): file removed

3 resource(s) unchanged.
";
    assert_eq!(rendered, golden);
}

#[test]
fn empty_plan_renders_no_changes() {
    let result = FactorySyncDryRunResult {
        commit_sha: "abc123".to_string(),
        plan: Some(FactorySyncPlan {
            no_ops: 5,
            ..Default::default()
        }),
        resource_errors: vec![],
    };

    let rendered = render_plan_text(&result).unwrap();

    let golden = "\
Plan for commit abc123:

No changes.

5 resource(s) unchanged.
";
    assert_eq!(rendered, golden);
}

#[test]
fn plan_with_structural_errors_fails_with_file_and_line_details() {
    let result = FactorySyncDryRunResult {
        commit_sha: "abc123".to_string(),
        plan: None,
        resource_errors: vec![
            resource_error("environments/prod.yaml", Some(12), "invalid enum value"),
            resource_error("factory.yaml", None, "missing name"),
        ],
    };

    let err = render_plan_text(&result).unwrap_err();

    let message = err.to_string();
    assert!(message.contains("structural errors"), "got: {message}");
    assert!(
        message.contains("- environments/prod.yaml:12: invalid enum value"),
        "got: {message}"
    );
    assert!(message.contains("- factory.yaml: missing name"), "got: {message}");
}

#[test]
fn status_rendering_covers_successful_sync() {
    let status = file_managed_status(Some(sync_summary("aaa111", FactorySyncState::Success)));

    let rendered = render_status_text(&status);

    let golden = "\
Management mode: file_managed
Source: warpdotdev/factory-config @ main (GITHUB), path: factories/prod
Last synced commit: aaa111
Latest sync:
  Commit: aaa111
  Status: success
  Started: 2026-07-08T05:00:00+00:00
  Finished: 2026-07-08T05:00:10+00:00
";
    assert_eq!(rendered, golden);
}

#[test]
fn status_rendering_lists_failed_sync_errors() {
    let mut summary = sync_summary("bbb222", FactorySyncState::Failed);
    summary.resource_errors = vec![resource_error(
        "environments/prod.yaml",
        Some(12),
        "invalid enum value",
    )];
    let status = file_managed_status(Some(summary));

    let rendered = render_status_text(&status);

    assert!(rendered.contains("  Status: failed"), "got: {rendered}");
    assert!(rendered.contains("  Errors:"), "got: {rendered}");
    assert!(
        rendered.contains("  - environments/prod.yaml:12: invalid enum value"),
        "got: {rendered}"
    );
}

#[test]
fn status_rendering_lists_degraded_reasons() {
    let mut summary = sync_summary("ccc333", FactorySyncState::Partial);
    summary.degraded_reasons = vec!["missing managed secret DATADOG_API_KEY".to_string()];
    let status = file_managed_status(Some(summary));

    let rendered = render_status_text(&status);

    assert!(rendered.contains("  Status: partial"), "got: {rendered}");
    assert!(rendered.contains("  Degraded reasons:"), "got: {rendered}");
    assert!(
        rendered.contains("  - missing managed secret DATADOG_API_KEY"),
        "got: {rendered}"
    );
}

#[test]
fn status_rendering_covers_live_managed_never_synced() {
    let status = FactorySyncStatusResponse {
        management_mode: "live_managed".to_string(),
        source: None,
        last_synced_commit: None,
        latest_sync: None,
    };

    let rendered = render_status_text(&status);

    let golden = "\
Management mode: live_managed
Source: none
Last synced commit: none
Latest sync: never synced
";
    assert_eq!(rendered, golden);
}

#[test]
fn terminal_sync_requires_matching_commit_and_terminal_status() {
    let running = file_managed_status(Some(sync_summary("abc", FactorySyncState::Running)));
    assert!(terminal_sync_for_commit(&running, "abc").is_none());

    let other_commit = file_managed_status(Some(sync_summary("def", FactorySyncState::Success)));
    assert!(terminal_sync_for_commit(&other_commit, "abc").is_none());

    let never_synced = file_managed_status(None);
    assert!(terminal_sync_for_commit(&never_synced, "abc").is_none());

    let done = file_managed_status(Some(sync_summary("abc", FactorySyncState::Noop)));
    let summary = terminal_sync_for_commit(&done, "abc").expect("noop is terminal");
    assert_eq!(summary.status, FactorySyncState::Noop);
}

#[test]
fn apply_wait_outcome_fails_on_failed_sync() {
    let mut summary = sync_summary("abc", FactorySyncState::Failed);
    summary.resource_errors = vec![resource_error("factory.yaml", Some(3), "bad field")];

    let err = apply_wait_outcome(&summary).unwrap_err();

    let message = err.to_string();
    assert!(message.contains("Sync of commit abc failed."), "got: {message}");
    assert!(message.contains("- factory.yaml:3: bad field"), "got: {message}");
}

#[test]
fn apply_wait_outcome_reports_success_and_degraded_reasons() {
    let success = apply_wait_outcome(&sync_summary("abc", FactorySyncState::Success)).unwrap();
    assert_eq!(success, "Sync of commit abc succeeded.\n");

    let noop = apply_wait_outcome(&sync_summary("abc", FactorySyncState::Noop)).unwrap();
    assert!(noop.contains("no-op"), "got: {noop}");

    let mut partial_summary = sync_summary("abc", FactorySyncState::Partial);
    partial_summary.degraded_reasons = vec!["missing managed secret FOO".to_string()];
    let partial = apply_wait_outcome(&partial_summary).unwrap();
    assert!(partial.contains("degraded resources"), "got: {partial}");
    assert!(partial.contains("- missing managed secret FOO"), "got: {partial}");
}

#[test]
fn wait_returns_summary_once_sync_for_commit_is_terminal() {
    let mut client = MockFactoryClient::new();
    let mut polls = 0;
    client
        .expect_get_factory_sync_status()
        .times(3)
        .returning(move |_| {
            polls += 1;
            let summary = match polls {
                // Poll 1: our sync is still running. Poll 2: the ledger still shows an
                // older commit's sync. Poll 3: our sync reached a terminal state.
                1 => sync_summary("abc", FactorySyncState::Running),
                2 => sync_summary("older", FactorySyncState::Success),
                _ => sync_summary("abc", FactorySyncState::Failed),
            };
            Ok(file_managed_status(Some(summary)))
        });

    let summary = block_on(wait_for_factory_sync(
        &client,
        "fac-1",
        "abc",
        Duration::ZERO,
        10,
    ))
    .unwrap();

    assert_eq!(summary.commit_sha, "abc");
    assert_eq!(summary.status, FactorySyncState::Failed);
    assert!(apply_wait_outcome(&summary).is_err());
}

#[test]
fn wait_times_out_when_sync_never_becomes_terminal() {
    let mut client = MockFactoryClient::new();
    client
        .expect_get_factory_sync_status()
        .times(2)
        .returning(|_| {
            Ok(file_managed_status(Some(sync_summary(
                "abc",
                FactorySyncState::Running,
            ))))
        });

    let err = block_on(wait_for_factory_sync(
        &client,
        "fac-1",
        "abc",
        Duration::ZERO,
        2,
    ))
    .unwrap_err();

    assert!(err.to_string().contains("Timed out"), "got: {err}");
}

#[test]
fn exported_factory_name_reads_factory_yaml() {
    let export = export_response(&[(
        "factory.yaml",
        "kind: Factory\nschema_version: 1\nname: acme-factory\n",
    )]);

    assert_eq!(exported_factory_name(&export).unwrap(), "acme-factory");
}

#[test]
fn exported_factory_name_requires_usable_name() {
    let cases = [
        export_response(&[("agents/reviewer/agent.md", "body")]),
        export_response(&[("factory.yaml", "kind: Factory\n")]),
        export_response(&[("factory.yaml", "name: ../evil\n")]),
        export_response(&[("factory.yaml", "name: /evil\n")]),
        export_response(&[("factory.yaml", "name: a/b\n")]),
        export_response(&[("factory.yaml", "name: [unclosed\n")]),
    ];
    for export in cases {
        let err = exported_factory_name(&export).unwrap_err();
        assert!(err.to_string().contains("--out"), "got: {err}");
    }
}

#[test]
fn export_downloads_files_into_out_dir() {
    let mut client = MockFactoryClient::new();
    client.expect_export_factory().times(1).returning(|_| {
        Ok(export_response(&[
            (
                "factory.yaml",
                "kind: Factory\nschema_version: 1\nname: acme-factory\n",
            ),
            ("agents/reviewer/agent.md", "---\nkind: Agent\n---\nbody\n"),
        ]))
    });
    let temp = tempfile::tempdir().expect("tempdir");
    let out = temp.path().join("exported");

    let (dir, files) =
        block_on(export_factory_to_dir(&client, "fac-1", Some(out.clone()), false)).unwrap();

    assert_eq!(dir, out);
    assert_eq!(
        files,
        vec![
            "agents/reviewer/agent.md".to_string(),
            "factory.yaml".to_string(),
        ]
    );
    assert_eq!(
        std::fs::read_to_string(out.join("agents/reviewer/agent.md")).expect("agent.md exists"),
        "---\nkind: Agent\n---\nbody\n"
    );
}

#[test]
fn export_rejects_server_supplied_path_traversal() {
    let mut client = MockFactoryClient::new();
    client
        .expect_export_factory()
        .times(1)
        .returning(|_| Ok(export_response(&[("../escape.txt", "evil")])));
    let temp = tempfile::tempdir().expect("tempdir");
    let out = temp.path().join("exported");

    let err = block_on(export_factory_to_dir(&client, "fac-1", Some(out.clone()), false))
        .unwrap_err();

    assert!(err.to_string().contains("unsafe path"), "got: {err}");
    assert!(!out.exists());
    assert!(!temp.path().join("escape.txt").exists());
}

#[test]
fn init_scaffold_parses_as_yaml_with_expected_envelopes() {
    let files = warp_cli::factory::scaffold_files("acme-factory");

    let factory: serde_yaml::Value =
        serde_yaml::from_str(&files["factory.yaml"]).expect("factory.yaml is valid YAML");
    assert_eq!(factory["kind"].as_str(), Some("Factory"));
    assert_eq!(factory["schema_version"].as_i64(), Some(1));
    assert_eq!(factory["name"].as_str(), Some("acme-factory"));

    let secrets: serde_yaml::Value =
        serde_yaml::from_str(&files["secrets.yaml"]).expect("secrets.yaml is valid YAML");
    assert_eq!(secrets["kind"].as_str(), Some("SecretManifest"));
    assert_eq!(secrets["schema_version"].as_i64(), Some(1));
    assert!(
        secrets["secrets"]
            .as_sequence()
            .is_some_and(|secrets| secrets.is_empty()),
        "secrets is an empty list"
    );

    let agent = &files[warp_cli::factory::SCAFFOLD_AGENT_PATH];
    let (frontmatter, body) = agent
        .strip_prefix("---\n")
        .and_then(|rest| rest.split_once("\n---\n"))
        .expect("agent template has frontmatter");
    let frontmatter: serde_yaml::Value =
        serde_yaml::from_str(frontmatter).expect("agent frontmatter is valid YAML");
    assert_eq!(frontmatter["kind"].as_str(), Some("Agent"));
    assert_eq!(frontmatter["schema_version"].as_i64(), Some(1));
    assert!(
        frontmatter["description"]
            .as_str()
            .is_some_and(|description| !description.is_empty()),
        "agent description is set"
    );
    assert!(!body.trim().is_empty(), "agent template has a body");
}
