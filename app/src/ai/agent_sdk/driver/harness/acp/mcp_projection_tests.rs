use serde_json::json;
use uuid::Uuid;
use warp_multi_agent_api as api;

use super::super::protocol::ToolCallFields;
use super::{KnownMcpServer, KnownMcpServers, outcome};

fn known(servers: &[(&str, Option<&str>)]) -> KnownMcpServers {
    KnownMcpServers::new(
        servers
            .iter()
            .enumerate()
            .map(|(index, (name, warp_id))| KnownMcpServer {
                name: (*name).to_owned(),
                installation_id: Uuid::from_u128(index as u128 + 1),
                warp_id: warp_id.map(str::to_owned),
            })
            .collect(),
    )
}

fn call(value: serde_json::Value) -> ToolCallFields {
    serde_json::from_value(value).expect("fixture should deserialize")
}

#[test]
fn servers_resolve_under_the_spellings_adapters_derive_from_their_names() {
    let known = known(&[("GitHub MCP", None), ("Team Sentry", None)]);
    for label in [
        "mcp.GitHub_MCP.create_issue",
        "mcp__github_mcp__create_issue",
        "mcp__GitHub-MCP__create_issue",
        // Pi's short prefix mode drops the `mcp` suffix.
        "github_create_issue",
        "GitHub_MCP_create_issue",
    ] {
        let resolved = known
            .recognize(&call(
                json!({ "toolCallId": "1", "title": label, "kind": "other" }),
            ))
            .unwrap_or_else(|| panic!("{label} should resolve"));
        assert_eq!(resolved.server.name, "GitHub MCP", "{label}");
        assert_eq!(resolved.tool, "create_issue", "{label}");
    }
    let sentry = known
        .recognize(&call(json!({
            "toolCallId": "1",
            "title": "Team_Sentry_list_issues",
            "kind": "other"
        })))
        .expect("resolves");
    assert_eq!(sentry.server.name, "Team Sentry");
    assert_eq!(sentry.tool, "list_issues");
}

#[test]
fn a_spelling_two_servers_share_attributes_to_neither() {
    let known = known(&[("foo", None), ("foo-mcp", None)]);
    // `foo-mcp` shortens to `foo`, which is also the first server's own name.
    assert!(
        known
            .recognize(&call(
                json!({ "toolCallId": "1", "title": "mcp__foo__bar" })
            ))
            .is_none()
    );
    // The unshortened spelling is still unambiguous.
    let resolved = known
        .recognize(&call(
            json!({ "toolCallId": "1", "title": "mcp__foo_mcp__bar" }),
        ))
        .expect("resolves");
    assert_eq!(resolved.server.name, "foo-mcp");
}

#[test]
fn the_longest_server_prefix_wins_for_direct_tools() {
    let known = known(&[("linear", None), ("linear_agent", None)]);
    let resolved = known
        .recognize(&call(json!({
            "toolCallId": "1",
            "title": "linear_agent_post_thought",
            "kind": "other"
        })))
        .expect("resolves");
    assert_eq!(resolved.server.name, "linear_agent");
    assert_eq!(resolved.tool, "post_thought");
}

#[test]
fn a_dotted_server_name_resolves_from_a_dotted_title() {
    let known = known(&[("acme.docs", None)]);
    let resolved = known
        .recognize(&call(json!({
            "toolCallId": "1",
            "title": "mcp.acme.docs.search",
            "kind": "execute"
        })))
        .expect("resolves");
    assert_eq!(resolved.server.name, "acme.docs");
    assert_eq!(resolved.tool, "search");
}

#[test]
fn identity_carries_the_name_and_the_warp_side_id() {
    let known = known(&[
        ("slack", Some("slack")),
        ("ops", Some("0c6d4d1e-3b8b-4d2f-9f0e-6b1c2b3d4e5f")),
        ("local", None),
    ]);
    let slack = known
        .recognize(&call(
            json!({ "toolCallId": "1", "title": "mcp__slack__read_thread" }),
        ))
        .expect("resolves");
    assert_eq!(
        slack.server.identity(),
        api::McpServerIdentity {
            display_name: "slack".to_owned(),
            integration: api::McpIntegration::Slack as i32,
            ..Default::default()
        }
    );
    let ops = known
        .recognize(&call(
            json!({ "toolCallId": "1", "title": "mcp__ops__deploy" }),
        ))
        .expect("resolves");
    assert_eq!(
        ops.server.identity(),
        api::McpServerIdentity {
            display_name: "ops".to_owned(),
            managed_server_uid: "0c6d4d1e-3b8b-4d2f-9f0e-6b1c2b3d4e5f".to_owned(),
            ..Default::default()
        }
    );
    let local = known
        .recognize(&call(
            json!({ "toolCallId": "1", "title": "mcp__local__ping" }),
        ))
        .expect("resolves");
    assert_eq!(
        local.server.identity(),
        api::McpServerIdentity {
            display_name: "local".to_owned(),
            ..Default::default()
        }
    );
}

#[test]
fn nothing_resolves_without_configured_servers_or_a_matching_shape() {
    let none = KnownMcpServers::default();
    assert!(
        none.recognize(&call(
            json!({ "toolCallId": "1", "title": "mcp__slack__read" })
        ))
        .is_none()
    );
    let known = known(&[("slack", None)]);
    for fixture in [
        json!({ "toolCallId": "1", "title": "Read file", "kind": "read" }),
        json!({ "toolCallId": "1", "title": "mcp__slack__", "kind": "other" }),
        json!({ "toolCallId": "1", "title": "mcp__slack", "kind": "other" }),
        json!({ "toolCallId": "1", "title": "mcp", "kind": "other", "rawInput": { "tool": "github_x" } }),
        json!({ "toolCallId": "1", "title": "slack_", "kind": "other" }),
        json!({ "toolCallId": "1", "title": "slack_cli --version", "kind": "execute" }),
    ] {
        assert!(
            known.recognize(&call(fixture.clone())).is_none(),
            "{fixture}"
        );
    }
}

#[test]
fn outcome_reads_results_errors_blocks_and_text() {
    assert_eq!(outcome(None), None);
    assert_eq!(
        outcome(Some(&json!({ "result": null, "error": null }))),
        None
    );
    assert_eq!(
        outcome(Some(&json!({
            "result": { "content": [{ "type": "text", "text": "a" }, { "type": "text", "text": "b" }] },
            "error": null
        }))),
        Some(Ok("a\nb".to_owned()))
    );
    assert_eq!(
        outcome(Some(
            &json!({ "result": null, "error": { "message": "boom" } })
        )),
        Some(Err("boom".to_owned()))
    );
    assert_eq!(
        outcome(Some(&json!({ "error": "boom" }))),
        Some(Err("\"boom\"".to_owned()))
    );
    assert_eq!(
        outcome(Some(&json!([{ "type": "text", "text": "posted" }]))),
        Some(Ok("posted".to_owned()))
    );
    assert_eq!(outcome(Some(&json!("plain"))), Some(Ok("plain".to_owned())));
    let image_only = json!({ "content": [{ "type": "image", "data": "..." }] });
    assert_eq!(
        outcome(Some(&image_only)),
        Some(Ok(serde_json::to_string_pretty(&image_only).unwrap()))
    );
}
