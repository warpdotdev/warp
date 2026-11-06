use std::path::PathBuf;

use serde_json::json;
use warp_util::path::EscapeChar;
use warpui::{App, SingletonEntity};

use super::super::protocol::ToolCallFields;
use super::{PolicyDecision, PolicyRequest};
use crate::ai::blocklist::BlocklistAIPermissions;
use crate::settings::AgentModeCommandExecutionPredicate;
use crate::test_util::terminal::{add_window_with_terminal, initialize_app_for_terminal_view};

fn tool_call(value: serde_json::Value) -> ToolCallFields {
    serde_json::from_value(value).expect("fixture should deserialize")
}

#[test]
fn permission_requests_are_reduced_to_what_policy_cares_about() {
    assert_eq!(
        PolicyRequest::for_tool_call(&tool_call(json!({
            "toolCallId": "c1",
            "kind": "execute",
            "rawInput": { "command": "rm -rf build" }
        }))),
        Some(PolicyRequest::Execute {
            command: "rm -rf build".to_owned()
        })
    );
    assert_eq!(
        PolicyRequest::for_tool_call(&tool_call(json!({
            "toolCallId": "c2",
            "kind": "edit",
            "locations": [{ "path": "src/a.rs", "line": 3 }],
            "content": [{ "type": "diff", "path": "src/b.rs", "newText": "x" }],
            "rawInput": { "file_path": "src/c.rs" }
        }))),
        Some(PolicyRequest::WriteFiles {
            paths: vec![
                PathBuf::from("src/a.rs"),
                PathBuf::from("src/b.rs"),
                PathBuf::from("src/c.rs"),
            ]
        })
    );
    assert_eq!(
        PolicyRequest::for_tool_call(&tool_call(json!({
            "toolCallId": "c3",
            "kind": "read",
            "rawInput": { "path": "README.md" }
        }))),
        None,
        "reads have no hard refusal in Warp's policy"
    );
    assert_eq!(
        PolicyRequest::for_tool_call(&tool_call(json!({ "toolCallId": "c4" }))),
        None,
        "a bare tool call id carries nothing to evaluate"
    );
}

#[test]
fn denylisted_commands_and_protected_writes_are_refused_everything_else_allowed() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);
        BlocklistAIPermissions::handle(&app).update(&mut app, |permissions, ctx| {
            permissions
                .add_command_to_autoexecution_denylist(
                    AgentModeCommandExecutionPredicate::new_regex("rm .*").unwrap(),
                    ctx,
                )
                .unwrap();
        });
        terminal.update(&mut app, |terminal, ctx| {
            let terminal_view_id = ctx.view_id();
            terminal.ai_controller().update(ctx, |controller, ctx| {
                controller.bind_native_prompt_conversation(None, ctx);
                let evaluate = |request: PolicyRequest, ctx: &_| {
                    request.evaluate(controller, terminal_view_id, EscapeChar::Backslash, ctx)
                };

                assert!(matches!(
                    evaluate(
                        PolicyRequest::Execute {
                            command: "rm -rf /tmp/x".to_owned()
                        },
                        ctx
                    ),
                    PolicyDecision::Deny { .. }
                ));
                assert_eq!(
                    evaluate(
                        PolicyRequest::Execute {
                            command: "cargo test".to_owned()
                        },
                        ctx
                    ),
                    PolicyDecision::Allow,
                    "a command the user would merely be asked about runs unattended"
                );
                assert!(matches!(
                    evaluate(PolicyRequest::write("/repo/.warp/.mcp.json"), ctx),
                    PolicyDecision::Deny { .. }
                ));
                assert_eq!(
                    evaluate(PolicyRequest::write("/repo/src/main.rs"), ctx),
                    PolicyDecision::Allow
                );
            });
        });
    });
}
