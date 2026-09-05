use ai::agent::action::{ReadFilesRequest, RunAgentsExecutionMode, RunAgentsRequest};

use super::*;

#[test]
fn oz_hooks_adapters_cover_side_effect_categories_without_sensitive_content() {
    let redactor = HookRedactor::new(["secret".into()]);
    let cases = [
        (
            AIAgentActionType::RequestCommandOutput {
                command: "printf secret".into(),
                is_read_only: Some(false),
                is_risky: Some(true),
                wait_until_completion: true,
                uses_pager: Some(false),
                rationale: Some("secret rationale".into()),
                citations: vec![],
            },
            "run_shell_command",
            "secret",
        ),
        (
            AIAgentActionType::ReadFiles(ReadFilesRequest { locations: vec![] }),
            "read_files",
            "file contents",
        ),
        (
            AIAgentActionType::CallMCPTool {
                server_id: None,
                name: "safe-tool-name".into(),
                input: serde_json::json!({"token": "secret"}),
            },
            "call_mcp_tool",
            "secret",
        ),
        (
            AIAgentActionType::RunAgents(RunAgentsRequest {
                summary: "safe summary".into(),
                base_prompt: "secret child prompt".into(),
                skills: vec![],
                model_id: "model".into(),
                harness_type: "oz".into(),
                execution_mode: RunAgentsExecutionMode::Local,
                agent_run_configs: vec![],
                plan_id: String::new(),
                harness_auth_secret_name: Some("secret-name".into()),
            }),
            "run_agents",
            "secret",
        ),
    ];

    for (action, expected_name, prohibited) in cases {
        let (name, payload) = local_action_payload(&action, &redactor);
        let serialized = serde_json::to_string(&payload).unwrap();
        assert_eq!(name, expected_name);
        assert!(!serialized.contains(prohibited));
        assert!(!super::super::redaction::contains_prohibited_payload_key(
            &serde_json::to_value(payload).unwrap()
        ));
    }
}
