use warp::tui_export::ServerConversationToken;

use super::TuiArgs;

/// Parses a prompt and server conversation token.
#[test]
fn parses_prompt_and_conversation_id() {
    let server_conversation_token = ServerConversationToken::new("server-token".to_owned());
    assert_eq!(
        TuiArgs::parse([
            "--conversation-id".to_owned(),
            server_conversation_token.as_str().to_owned(),
            "--prompt".to_owned(),
            "hello".to_owned(),
        ],)
        .unwrap(),
        TuiArgs {
            prompt: Some("hello".to_owned()),
            server_conversation_token: Some(server_conversation_token),
        }
    );
}

/// Rejects flags whose values are missing.
#[test]
fn rejects_missing_argument_value() {
    let error = TuiArgs::parse(["--prompt".to_owned()]).unwrap_err();
    assert_eq!(error.to_string(), "--prompt requires a value");
}

/// Accepts opaque server conversation tokens.
#[test]
fn accepts_opaque_conversation_id() {
    let args = TuiArgs::parse([
        "--conversation-id".to_owned(),
        "not-a-local-uuid".to_owned(),
    ])
    .unwrap();
    assert_eq!(
        args.server_conversation_token
            .as_ref()
            .map(ServerConversationToken::as_str),
        Some("not-a-local-uuid")
    );
}

/// Rejects unsupported TUI frontend arguments.
#[test]
fn rejects_unknown_argument() {
    let error = TuiArgs::parse(["--unknown".to_owned()]).unwrap_err();
    assert_eq!(error.to_string(), "Unknown argument: --unknown");
}
