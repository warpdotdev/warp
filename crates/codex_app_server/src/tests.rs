use serde_json::json;

use crate::{
    AccountStatus, AccountType, LoginChallenge, Notification, ServerRequest, ServerRequestResponse,
    deny_server_request,
};

#[test]
fn parses_chatgpt_account_without_exposing_tokens() {
    let status = AccountStatus::from_value(json!({
        "account": {
            "type": "chatgpt",
            "email": "coder@example.com",
            "planType": "pro",
        },
        "requiresOpenaiAuth": true,
    }))
    .unwrap();

    let account = status.account.unwrap();
    assert_eq!(account.account_type, AccountType::ChatGpt);
    assert_eq!(account.email.as_deref(), Some("coder@example.com"));
    assert_eq!(account.plan_type.as_deref(), Some("pro"));
    assert!(status.requires_openai_auth);
}

#[test]
fn parses_browser_and_device_login_challenges() {
    let browser = LoginChallenge::from_value(json!({
        "type": "chatgpt",
        "loginId": "browser-id",
        "authUrl": "https://example.com/login",
    }))
    .unwrap();
    assert!(matches!(
        browser,
        LoginChallenge::Browser { login_id, auth_url }
            if login_id == "browser-id" && auth_url == "https://example.com/login"
    ));

    let device = LoginChallenge::from_value(json!({
        "type": "chatgptDeviceCode",
        "loginId": "device-id",
        "verificationUrl": "https://example.com/device",
        "userCode": "ABCD-EFGH",
    }))
    .unwrap();
    assert!(matches!(
        device,
        LoginChallenge::DeviceCode { login_id, verification_url, user_code }
            if login_id == "device-id"
                && verification_url == "https://example.com/device"
                && user_code == "ABCD-EFGH"
    ));
}

#[test]
fn exposes_streaming_agent_message_deltas() {
    let notification = Notification {
        method: "item/agentMessage/delta".to_owned(),
        params: json!({ "delta": "hello" }),
    };
    assert_eq!(notification.agent_message_delta(), Some("hello"));
}

#[test]
fn extracts_completed_messages_and_tool_output() {
    let completed = Notification {
        method: "item/completed".to_owned(),
        params: json!({
            "item": {
                "type": "agentMessage",
                "id": "message-1",
                "text": "complete answer",
            },
        }),
    };
    assert_eq!(completed.completed_agent_message(), Some("complete answer"));

    let command_output = Notification {
        method: "item/commandExecution/outputDelta".to_owned(),
        params: json!({ "delta": "test output\n" }),
    };
    assert_eq!(command_output.command_output_delta(), Some("test output\n"));
}

#[test]
fn default_server_request_handler_denies_mutating_approvals() {
    for method in [
        "item/commandExecution/requestApproval",
        "item/fileChange/requestApproval",
    ] {
        let response = deny_server_request(&ServerRequest {
            id: json!(1),
            method: method.to_owned(),
            params: json!({}),
        });
        assert_eq!(
            response,
            ServerRequestResponse::Result(json!({ "decision": "decline" }))
        );
    }
}

#[test]
#[ignore = "requires a recent Codex CLI and an authenticated account"]
fn real_codex_app_server_smoke() {
    futures_lite::future::block_on(async {
        let mut client = crate::Client::spawn(crate::ClientOptions::default())
            .await
            .expect("start codex app-server");
        let account = client.account(false).await.expect("read Codex account");
        assert!(account.account.is_some(), "Codex must be signed in");

        let mut options =
            crate::ThreadOptions::new(std::env::current_dir().expect("resolve current directory"));
        options.approval_policy = crate::ApprovalPolicy::Never;
        options.sandbox = crate::SandboxMode::ReadOnly;
        options.thread_source = "warp-live-smoke-test".to_owned();
        let thread_id = client
            .start_thread(&options)
            .await
            .expect("start read-only Codex thread");

        let mut answer = String::new();
        let result = client
            .run_turn(
                &thread_id,
                "Reply with exactly WARP_CODEX_SMOKE_OK and nothing else.",
                |notification| {
                    if let Some(delta) = notification.agent_message_delta() {
                        answer.push_str(delta);
                    }
                },
                crate::deny_server_request,
            )
            .await
            .expect("run Codex turn");
        assert_eq!(result.status, "completed");
        assert!(
            answer.contains("WARP_CODEX_SMOKE_OK"),
            "unexpected Codex response: {answer:?}"
        );
    });
}
