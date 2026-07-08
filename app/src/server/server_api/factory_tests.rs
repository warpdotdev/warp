use chrono::{TimeZone as _, Utc};
use futures::executor::block_on;
use serde_json::json;
use warp_core::channel::ChannelState;

use super::*;

fn factory_response_body(uid: &str) -> String {
    json!({
        "uid": uid,
        "team_uid": "team-1",
        "name": "Prod Factory",
        "description": null,
        "code_forge": "GITHUB",
        "repositories": [{ "owner": "warpdotdev", "repo": "factory-config" }],
        "default_environment": "env-1",
        "default_model": "model-1",
        "management_mode": "file_managed",
        "source": {
            "code_forge": "GITHUB",
            "repository": { "owner": "warpdotdev", "repo": "factory-config" },
            "ref": "main",
            "path": "factories/prod",
        },
        "created_at": "2026-07-01T00:00:00Z",
        "updated_at": "2026-07-01T00:00:00Z",
    })
    .to_string()
}

#[test]
fn build_factory_urls_encode_the_uid() {
    assert_eq!(build_factory_source_url("fac-1"), "factory/fac-1/source");
    assert_eq!(
        build_factory_sync_status_url("fac-1"),
        "factory/fac-1/sync-status"
    );
    assert_eq!(build_factory_sync_url("fac-1"), "factory/fac-1/sync");
    assert_eq!(build_factory_export_url("fac-1"), "factory/fac-1/export");
    assert_eq!(build_factory_source_url("fac/1"), "factory/fac%2F1/source");
}

#[test]
fn link_factory_source_posts_full_source_request() {
    // The mock only responds when method, path, and body all match, so a
    // successful deserialization asserts the request shape end to end.
    let _request = {
        let mut server = ChannelState::mock_server();
        server
            .mock("POST", "/api/v1/factory/fac-link-full/source")
            .match_body(mockito::Matcher::Json(json!({
                "code_forge": "GITHUB",
                "repository": { "owner": "warpdotdev", "repo": "factory-config" },
                "ref": "main",
                "path": "factories/prod",
            })))
            .with_status(200)
            .with_body(factory_response_body("fac-link-full"))
            .create()
    };
    let api = ServerApi::new_for_test();

    let request = FactorySourceRequest {
        code_forge: GITHUB_CODE_FORGE.to_string(),
        repository: FactoryRepository {
            owner: "warpdotdev".to_string(),
            repo: "factory-config".to_string(),
        },
        r#ref: Some("main".to_string()),
        path: Some("factories/prod".to_string()),
    };
    let factory = block_on(api.link_factory_source("fac-link-full", request)).unwrap();

    assert_eq!(factory.uid, "fac-link-full");
    assert_eq!(factory.management_mode.as_deref(), Some("file_managed"));
    let source = factory.source.expect("source is present");
    assert_eq!(source.r#ref, "main");
    assert_eq!(source.path, "factories/prod");
}

#[test]
fn link_factory_source_omits_default_ref_and_path() {
    let _request = {
        let mut server = ChannelState::mock_server();
        server
            .mock("POST", "/api/v1/factory/fac-link-defaults/source")
            .match_body(mockito::Matcher::Json(json!({
                "code_forge": "GITHUB",
                "repository": { "owner": "warpdotdev", "repo": "factory-config" },
            })))
            .with_status(200)
            .with_body(factory_response_body("fac-link-defaults"))
            .create()
    };
    let api = ServerApi::new_for_test();

    let request = FactorySourceRequest {
        code_forge: GITHUB_CODE_FORGE.to_string(),
        repository: FactoryRepository {
            owner: "warpdotdev".to_string(),
            repo: "factory-config".to_string(),
        },
        r#ref: None,
        path: None,
    };
    let factory = block_on(api.link_factory_source("fac-link-defaults", request)).unwrap();

    assert_eq!(factory.uid, "fac-link-defaults");
}

#[test]
fn unlink_factory_source_sends_delete() {
    let _request = {
        let mut server = ChannelState::mock_server();
        server
            .mock("DELETE", "/api/v1/factory/fac-unlink/source")
            .with_status(200)
            .with_body(factory_response_body("fac-unlink"))
            .create()
    };
    let api = ServerApi::new_for_test();

    block_on(api.unlink_factory_source("fac-unlink")).unwrap();
}

#[test]
fn get_factory_sync_status_deserializes_ledger_state() {
    let _request = {
        let mut server = ChannelState::mock_server();
        server
            .mock("GET", "/api/v1/factory/fac-status/sync-status")
            .with_status(200)
            .with_body(
                json!({
                    "management_mode": "file_managed",
                    "source": {
                        "code_forge": "GITHUB",
                        "repository": { "owner": "warpdotdev", "repo": "factory-config" },
                        "ref": "main",
                        "path": "",
                    },
                    "last_synced_commit": "aaa111",
                    "latest_sync": {
                        "commit_sha": "bbb222",
                        "status": "failed",
                        "started_at": "2026-07-08T05:00:00Z",
                        "finished_at": "2026-07-08T05:00:10Z",
                        "resource_errors": [
                            { "resource_path": "environments/prod.yaml", "line": 12, "message": "invalid enum value" },
                        ],
                        "degraded_reasons": [],
                    },
                })
                .to_string(),
            )
            .create()
    };
    let api = ServerApi::new_for_test();

    let status = block_on(api.get_factory_sync_status("fac-status")).unwrap();

    assert_eq!(status.management_mode, "file_managed");
    assert_eq!(status.last_synced_commit.as_deref(), Some("aaa111"));
    let latest = status.latest_sync.expect("latest sync is present");
    assert_eq!(latest.commit_sha, "bbb222");
    assert_eq!(latest.status, FactorySyncState::Failed);
    assert!(latest.status.is_terminal());
    assert_eq!(
        latest.started_at,
        Utc.with_ymd_and_hms(2026, 7, 8, 5, 0, 0).unwrap()
    );
    assert_eq!(
        latest.resource_errors,
        vec![FactoryResourceError {
            resource_path: "environments/prod.yaml".to_string(),
            line: Some(12),
            message: "invalid enum value".to_string(),
        }]
    );
}

#[test]
fn get_factory_sync_status_tolerates_never_synced_factories() {
    let _request = {
        let mut server = ChannelState::mock_server();
        server
            .mock("GET", "/api/v1/factory/fac-never-synced/sync-status")
            .with_status(200)
            .with_body(json!({ "management_mode": "live_managed" }).to_string())
            .create()
    };
    let api = ServerApi::new_for_test();

    let status = block_on(api.get_factory_sync_status("fac-never-synced")).unwrap();

    assert_eq!(status.management_mode, "live_managed");
    assert!(status.source.is_none());
    assert!(status.last_synced_commit.is_none());
    assert!(status.latest_sync.is_none());
}

#[test]
fn sync_factory_dry_run_posts_dry_run_body_and_parses_plan() {
    let _request = {
        let mut server = ChannelState::mock_server();
        server
            .mock("POST", "/api/v1/factory/fac-plan/sync")
            .match_body(mockito::Matcher::Json(json!({ "dry_run": true })))
            .with_status(200)
            .with_body(
                json!({
                    "dry_run": true,
                    "commit_sha": "ccc333",
                    "plan": {
                        "creates": [
                            { "path": "agents/reviewer.md", "kind": "Agent", "reason": "new resource" },
                        ],
                        "updates": [],
                        "deletes": [],
                        "no_ops": 3,
                    },
                    "resource_errors": [],
                })
                .to_string(),
            )
            .create()
    };
    let api = ServerApi::new_for_test();

    let result = block_on(api.sync_factory_dry_run("fac-plan", None)).unwrap();

    assert_eq!(result.commit_sha, "ccc333");
    let plan = result.plan.expect("plan is present");
    assert_eq!(plan.no_ops, 3);
    assert_eq!(plan.creates.len(), 1);
    assert_eq!(plan.creates[0].path, "agents/reviewer.md");
    assert_eq!(plan.creates[0].kind, "Agent");
}

#[test]
fn sync_factory_dry_run_includes_requested_sha() {
    let _request = {
        let mut server = ChannelState::mock_server();
        server
            .mock("POST", "/api/v1/factory/fac-plan-sha/sync")
            .match_body(mockito::Matcher::Json(json!({
                "sha": "ddd444",
                "dry_run": true,
            })))
            .with_status(200)
            .with_body(
                json!({
                    "dry_run": true,
                    "commit_sha": "ddd444",
                    "plan": { "creates": [], "updates": [], "deletes": [], "no_ops": 0 },
                    "resource_errors": [],
                })
                .to_string(),
            )
            .create()
    };
    let api = ServerApi::new_for_test();

    let result =
        block_on(api.sync_factory_dry_run("fac-plan-sha", Some("ddd444".to_string()))).unwrap();

    assert_eq!(result.commit_sha, "ddd444");
}

#[test]
fn sync_factory_posts_sha_and_parses_202_accepted_body() {
    let _request = {
        let mut server = ChannelState::mock_server();
        server
            .mock("POST", "/api/v1/factory/fac-apply/sync")
            .match_body(mockito::Matcher::Json(json!({ "sha": "eee555" })))
            .with_status(202)
            .with_body(json!({ "commit_sha": "eee555" }).to_string())
            .create()
    };
    let api = ServerApi::new_for_test();

    let accepted = block_on(api.sync_factory("fac-apply", Some("eee555".to_string()))).unwrap();

    assert_eq!(accepted.commit_sha, "eee555");
}

#[test]
fn sync_factory_without_sha_sends_empty_body() {
    let _request = {
        let mut server = ChannelState::mock_server();
        server
            .mock("POST", "/api/v1/factory/fac-apply-head/sync")
            .match_body(mockito::Matcher::Json(json!({})))
            .with_status(202)
            .with_body(json!({ "commit_sha": "fff666" }).to_string())
            .create()
    };
    let api = ServerApi::new_for_test();

    let accepted = block_on(api.sync_factory("fac-apply-head", None)).unwrap();

    assert_eq!(accepted.commit_sha, "fff666");
}

#[test]
fn export_factory_deserializes_file_map() {
    let _request = {
        let mut server = ChannelState::mock_server();
        server
            .mock("GET", "/api/v1/factory/fac-export/export")
            .with_status(200)
            .with_body(
                json!({
                    "files": {
                        "factory.yaml": "kind: Factory\nname: acme-factory\n",
                        "agents/reviewer/agent.md": "---\nkind: Agent\n---\nbody\n",
                    },
                })
                .to_string(),
            )
            .create()
    };
    let api = ServerApi::new_for_test();

    let export = block_on(api.export_factory("fac-export")).unwrap();

    assert_eq!(export.files.len(), 2);
    assert_eq!(
        export.files["factory.yaml"],
        "kind: Factory\nname: acme-factory\n"
    );
    assert_eq!(
        export.files["agents/reviewer/agent.md"],
        "---\nkind: Agent\n---\nbody\n"
    );
}

#[test]
fn sync_request_serialization_omits_unset_fields() {
    let empty = FactorySyncRequest {
        sha: None,
        dry_run: None,
    };
    assert_eq!(serde_json::to_value(&empty).unwrap(), json!({}));

    let dry_run = FactorySyncRequest {
        sha: Some("abc".to_string()),
        dry_run: Some(true),
    };
    assert_eq!(
        serde_json::to_value(&dry_run).unwrap(),
        json!({ "sha": "abc", "dry_run": true })
    );
}
