use std::collections::HashMap;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use warp_core::command::ExitCode;
use warp_multi_agent_api as api;

use crate::ai::agent::base_user_query::warp_client_origin;
use crate::ai::agent::task::TaskId;
use crate::ai::agent::{
    AIAgentActionResult, AIAgentActionResultType, AIAgentAttachment, AIAgentContext, AIAgentInput,
    BaseUserQuery, RunningCommand, TransferShellCommandControlToUserResult, UserQueryMode,
};
use crate::terminal::model::block::BlockId;

#[test]
fn git_context_converts_repository_and_pull_request_metadata() {
    let context = vec![
        AIAgentContext::Git {
            head: "abc123".to_string(),
            branch: Some("feature/repo-pr".to_string()),
        },
        AIAgentContext::Repository {
            name: "warp-internal".to_string(),
            owner: Some("warpdotdev".to_string()),
            host: Some("github.com".to_string()),
        },
        AIAgentContext::PullRequest {
            number: 42,
            state: "OPEN".to_string(),
            draft: true,
            base_branch: "main".to_string(),
            url: "https://github.com/warpdotdev/warp-internal/pull/42".to_string(),
        },
    ];

    let api_context = super::convert_context(&context);
    let git = api_context.git.expect("expected git context");
    assert_eq!(git.head, "abc123");
    assert_eq!(git.branch, "feature/repo-pr");

    let repository = git.repository.expect("expected repository context");
    assert_eq!(repository.name, "warp-internal");
    assert_eq!(repository.owner, "warpdotdev");
    assert_eq!(repository.host, "github.com");

    let pull_request = git.pull_request.expect("expected pull request context");
    assert_eq!(pull_request.number, 42);
    assert_eq!(
        pull_request.state,
        api::input_context::git::pull_request::State::OpenDraft as i32
    );
    assert_eq!(pull_request.base_branch, "main");
    assert_eq!(
        pull_request.url,
        "https://github.com/warpdotdev/warp-internal/pull/42"
    );
}

#[test]
fn git_context_skips_pull_request_metadata_with_invalid_number() {
    for number in [0, -1] {
        let context = vec![
            AIAgentContext::Git {
                head: "abc123".to_string(),
                branch: Some("feature/repo-pr".to_string()),
            },
            AIAgentContext::PullRequest {
                number,
                state: "OPEN".to_string(),
                draft: false,
                base_branch: "main".to_string(),
                url: "https://github.com/warpdotdev/warp-internal/pull/1".to_string(),
            },
        ];

        let api_context = super::convert_context(&context);
        let git = api_context.git.expect("expected git context");
        assert_eq!(git.head, "abc123");
        assert_eq!(git.branch, "feature/repo-pr");
        assert_eq!(git.pull_request, None);
    }
}

#[test]
fn git_context_skips_pull_request_metadata_with_unknown_state() {
    let context = vec![
        AIAgentContext::Git {
            head: "abc123".to_string(),
            branch: Some("feature/repo-pr".to_string()),
        },
        AIAgentContext::PullRequest {
            number: 42,
            state: "SOMETHING_ELSE".to_string(),
            draft: false,
            base_branch: "main".to_string(),
            url: "https://github.com/warpdotdev/warp-internal/pull/42".to_string(),
        },
    ];

    let api_context = super::convert_context(&context);
    let git = api_context.git.expect("expected git context");
    assert_eq!(git.pull_request, None);
}

#[test]
fn git_context_deserializes_legacy_string_pull_request_number() {
    let pull_request = serde_json::from_str::<AIAgentContext>(
        r#"{"PullRequest":{"number":"42","state":"OPEN","draft":false,"base_branch":"main"}}"#,
    )
    .expect("expected legacy serialized pull request context");

    let api_context = super::convert_context(&[pull_request]);
    let pull_request = api_context
        .git
        .expect("expected git context")
        .pull_request
        .expect("expected pull request context");
    assert_eq!(pull_request.number, 42);
}

#[test]
fn transfer_control_snapshot_result_converts_to_tool_call_result_input() {
    let block_id = BlockId::default();
    let input =
        api::request::input::user_inputs::user_input::Input::try_from(AIAgentActionResult {
            id: "tool_call".to_string().into(),
            task_id: TaskId::new("task".to_string()),
            result: AIAgentActionResultType::TransferShellCommandControlToUser(
                TransferShellCommandControlToUserResult::Snapshot {
                    block_id: block_id.clone(),
                    grid_contents: "snapshot".to_string(),
                    cursor: "<|cursor|>".to_string(),
                    is_alt_screen_active: false,
                    is_preempted: false,
                    activity: None,
                },
            ),
        })
        .unwrap();

    match input {
        api::request::input::user_inputs::user_input::Input::ToolCallResult(result) => {
            assert_eq!(result.tool_call_id, "tool_call");
            match result.result {
                Some(api::request::input::tool_call_result::Result::TransferShellCommandControlToUser(
                    api_result,
                )) => match api_result.result {
                    Some(
                        api::transfer_shell_command_control_to_user_result::Result::LongRunningCommandSnapshot(snapshot),
                    ) => {
                        assert_eq!(snapshot.command_id, block_id.to_string());
                        assert_eq!(snapshot.output, "snapshot");
                        assert_eq!(snapshot.cursor, "<|cursor|>");
                    }
                    other => panic!("Expected snapshot result, got {other:?}"),
                },
                other => panic!("Expected transfer-control tool call result, got {other:?}"),
            }
        }
        other => panic!("Expected tool-call-result input, got {other:?}"),
    }
}

#[test]
fn transfer_control_finished_result_converts_to_tool_call_result_input() {
    let block_id = BlockId::default();
    let start_ts = DateTime::from(Utc::now());
    let completed_ts = DateTime::from(Utc::now());
    let input =
        api::request::input::user_inputs::user_input::Input::try_from(AIAgentActionResult {
            id: "tool_call".to_string().into(),
            task_id: TaskId::new("task".to_string()),
            result: AIAgentActionResultType::TransferShellCommandControlToUser(
                TransferShellCommandControlToUserResult::CommandFinished {
                    block_id: block_id.clone(),
                    output: "done".to_string(),
                    exit_code: ExitCode::from(17),
                    start_ts: Some(start_ts),
                    completed_ts: Some(completed_ts),
                },
            ),
        })
        .unwrap();

    match input {
        api::request::input::user_inputs::user_input::Input::ToolCallResult(result) => {
            assert_eq!(result.tool_call_id, "tool_call");
            match result.result {
                Some(api::request::input::tool_call_result::Result::TransferShellCommandControlToUser(
                    api_result,
                )) => match api_result.result {
                    Some(
                        api::transfer_shell_command_control_to_user_result::Result::CommandFinished(finished),
                    ) => {
                        assert_eq!(finished.command_id, block_id.to_string());
                        assert_eq!(finished.output, "done");
                        assert_eq!(finished.exit_code, 17);
                        assert_eq!(finished.start_ts, Some(super::local_datetime_to_timestamp(start_ts)));
                        assert_eq!(finished.finish_ts, Some(super::local_datetime_to_timestamp(completed_ts)));
                    }
                    other => panic!("Expected command-finished result, got {other:?}"),
                },
                other => panic!("Expected transfer-control tool call result, got {other:?}"),
            }
        }
        other => panic!("Expected tool-call-result input, got {other:?}"),
    }
}

fn user_query_input(
    query: &str,
    base: Option<BaseUserQuery>,
    referenced_attachments: HashMap<String, AIAgentAttachment>,
) -> AIAgentInput {
    AIAgentInput::UserQuery {
        query: query.to_string(),
        context: Arc::new([]),
        static_query_type: None,
        referenced_attachments,
        user_query_mode: UserQueryMode::Normal,
        running_command: None,
        intended_agent: None,
        base,
    }
}

fn cli_user_query_input(
    query: &str,
    base: Option<BaseUserQuery>,
    intended_agent: Option<api::AgentType>,
) -> AIAgentInput {
    AIAgentInput::UserQuery {
        query: query.to_string(),
        context: Arc::new([]),
        static_query_type: None,
        referenced_attachments: HashMap::new(),
        user_query_mode: UserQueryMode::Normal,
        running_command: Some(RunningCommand {
            command: "cargo build".to_string(),
            block_id: BlockId::from("block-1".to_string()),
            grid_contents: "Compiling...".to_string(),
            cursor: String::new(),
            requested_command_id: None,
            is_alt_screen_active: false,
        }),
        intended_agent,
        base,
    }
}

fn converted_user_query(input: AIAgentInput) -> api::request::input::UserQuery {
    let Ok(api::request::input::user_inputs::user_input::Input::UserQuery(query)) =
        super::convert_input_to_user_input(input)
    else {
        panic!("expected a user query input");
    };
    query
}

fn converted_cli_user_query(input: AIAgentInput) -> api::request::input::UserQuery {
    let Ok(api::request::input::user_inputs::user_input::Input::CliAgentUserQuery(cli)) =
        super::convert_input_to_user_input(input)
    else {
        panic!("expected a CLI agent user query input");
    };
    cli.user_query.expect("CLI agent user query")
}

fn normal_mode() -> api::UserQueryMode {
    api::UserQueryMode { r#type: None }
}

fn plan_mode() -> api::UserQueryMode {
    api::UserQueryMode {
        r#type: Some(api::user_query_mode::Type::Plan(())),
    }
}

#[test]
fn local_user_query_converts_without_a_base() {
    let query = converted_user_query(user_query_input("hello", None, HashMap::new()));

    assert_eq!(query.query, "hello");
    assert_eq!(query.mode, Some(normal_mode()));
    assert_eq!(query.intended_agent, 0);
    // The fresh-local marker: warp-server resolves the author of a bare `WarpClient` origin
    // to the authenticated caller, and leaves an origin-less input alone.
    assert_eq!(query.origin, Some(warp_client_origin()));
    assert!(query.author.is_none());
    assert!(query.source_message.is_none());
}

#[test]
fn an_injected_query_without_an_origin_is_not_marked_fresh() {
    // The server (or an older relay) decided what this query carries; an absent origin is
    // theirs to leave absent, so the server does not attribute it to the sharer.
    let base = BaseUserQuery::from_proto(api::request::input::UserQuery {
        query: "forwarded text".to_string(),
        ..Default::default()
    });

    let query = converted_user_query(user_query_input(
        "forwarded text",
        Some(base),
        HashMap::new(),
    ));

    assert!(query.origin.is_none());
    assert!(query.author.is_none());
}

#[test]
fn injected_attribution_is_sent_as_is() {
    let author = api::QueryAuthor {
        principal: Some(api::query_author::Principal::User(api::WarpUser {
            uid: "external-author".to_string(),
            email: "author@example.com".to_string(),
            team_uid: "team".to_string(),
        })),
        resolution: api::IdentityResolution::ExternalAccountBinding.into(),
    };
    let origin = api::UserQueryOrigin {
        variant: Some(api::user_query_origin::Variant::ExternalPlatform(
            api::user_query_origin::ExternalPlatform {},
        )),
    };
    let source = api::ExternalMessage {
        body: "original message".to_string(),
        ..Default::default()
    };
    let base = BaseUserQuery::from_proto(api::request::input::UserQuery {
        origin: Some(origin.clone()),
        author: Some(author.clone()),
        source_message: Some(source.clone()),
        ..Default::default()
    });

    let query = converted_user_query(user_query_input(
        "rendered prompt",
        Some(base),
        HashMap::new(),
    ));

    assert_eq!(query.query, "rendered prompt");
    assert_eq!(query.origin, Some(origin));
    assert_eq!(query.author, Some(author));
    assert_eq!(query.source_message, Some(source));
}

#[test]
fn a_viewer_typed_query_carries_the_viewer_as_author() {
    let viewer = session_sharing_protocol::common::ProfileData {
        firebase_uid: "viewer".to_string(),
        email: Some("viewer@example.com".to_string()),
        ..Default::default()
    };

    let query = converted_user_query(user_query_input(
        "viewer text",
        Some(BaseUserQuery::for_viewer(Some(&viewer))),
        HashMap::new(),
    ));

    assert_eq!(
        query.query, "viewer text",
        "the prompt text fills the empty base query"
    );
    assert_eq!(query.origin, Some(warp_client_origin()));
    let Some(api::query_author::Principal::User(user)) = query.author.unwrap().principal else {
        panic!("expected the viewer as author");
    };
    assert_eq!(user.uid, "viewer");
    assert_eq!(user.email, "viewer@example.com");
    assert!(
        user.team_uid.is_empty(),
        "the sharer never claims a team for a viewer"
    );
}

#[test]
fn base_fields_this_client_does_not_model_pass_through() {
    let base = BaseUserQuery::from_proto(api::request::input::UserQuery {
        origin: Some(api::UserQueryOrigin::default()),
        author: Some(api::QueryAuthor::default()),
        source_message: Some(api::ExternalMessage {
            body: "raw comment".to_string(),
            ..Default::default()
        }),
        ..Default::default()
    });

    let query = converted_user_query(user_query_input(
        "from the client",
        Some(base),
        HashMap::new(),
    ));

    assert_eq!(query.query, "from the client");
    assert_eq!(query.mode, Some(normal_mode()));
    assert_eq!(query.origin, Some(api::UserQueryOrigin::default()));
    assert_eq!(query.author, Some(api::QueryAuthor::default()));
    assert_eq!(
        query.source_message.map(|message| message.body),
        Some("raw comment".to_string())
    );
}

#[test]
fn modeled_fields_are_written_over_the_base() {
    // The input's text, mode, and agent were seeded from the base when it was built, so they
    // are authoritative here even where they differ from what the base still carries.
    let base = BaseUserQuery::from_proto(api::request::input::UserQuery {
        query: "base text".to_string(),
        mode: Some(plan_mode()),
        intended_agent: api::AgentType::Cli.into(),
        origin: Some(api::UserQueryOrigin::default()),
        ..Default::default()
    });
    let input = AIAgentInput::UserQuery {
        query: "seeded text".to_string(),
        context: Arc::new([]),
        static_query_type: None,
        referenced_attachments: HashMap::new(),
        user_query_mode: UserQueryMode::Orchestrate,
        running_command: None,
        intended_agent: Some(api::AgentType::Primary),
        base: Some(base),
    };

    let query = converted_user_query(input);

    assert_eq!(query.query, "seeded text");
    assert_eq!(
        query.mode,
        Some(api::UserQueryMode {
            r#type: Some(api::user_query_mode::Type::Orchestrate(())),
        })
    );
    assert_eq!(query.intended_agent, i32::from(api::AgentType::Primary));
    assert_eq!(query.origin, Some(api::UserQueryOrigin::default()));
}

#[test]
fn client_resolved_attachments_are_added_without_overriding_base_ones() {
    let plain_text = |text: &str| api::Attachment {
        value: Some(api::attachment::Value::PlainText(text.to_string())),
    };
    let base = BaseUserQuery::from_proto(api::request::input::UserQuery {
        referenced_attachments: HashMap::from([(
            "notes.md".to_string(),
            plain_text("server copy"),
        )]),
        ..Default::default()
    });
    let client_attachments = HashMap::from([
        (
            "notes.md".to_string(),
            AIAgentAttachment::PlainText("client copy".to_string()),
        ),
        (
            "extra.txt".to_string(),
            AIAgentAttachment::PlainText("client only".to_string()),
        ),
    ]);

    let query = converted_user_query(user_query_input("prompt", Some(base), client_attachments));

    assert_eq!(query.referenced_attachments.len(), 2);
    assert_eq!(
        query.referenced_attachments["notes.md"],
        plain_text("server copy")
    );
    assert_eq!(
        query.referenced_attachments["extra.txt"],
        plain_text("client only")
    );
}

#[test]
fn cli_agent_user_query_carries_the_base_fields_and_defaults_to_the_cli_agent() {
    let base = BaseUserQuery::from_proto(api::request::input::UserQuery {
        source_message: Some(api::ExternalMessage {
            body: "raw comment".to_string(),
            ..Default::default()
        }),
        ..Default::default()
    });

    let query = converted_cli_user_query(cli_user_query_input("check the build", Some(base), None));

    assert_eq!(query.query, "check the build");
    assert_eq!(query.intended_agent, i32::from(api::AgentType::Cli));
    assert_eq!(
        query.source_message.map(|message| message.body),
        Some("raw comment".to_string())
    );
}

#[test]
fn cli_agent_user_query_keeps_a_seeded_intended_agent() {
    let base = BaseUserQuery::from_proto(api::request::input::UserQuery {
        intended_agent: api::AgentType::Primary.into(),
        ..Default::default()
    });

    let query = converted_cli_user_query(cli_user_query_input(
        "check the build",
        Some(base),
        Some(api::AgentType::Primary),
    ));

    assert_eq!(query.intended_agent, i32::from(api::AgentType::Primary));
}

#[test]
fn mcp_context_servers_carry_their_identity_alongside_the_installation_id() {
    use crate::ai::agent::{MCPContext, MCPServer};

    let server = |id: &str, name: &str, warp_id: &str| MCPServer {
        id: id.to_string(),
        name: name.to_string(),
        description: String::new(),
        warp_id: warp_id.to_string(),
        resources: vec![],
        tools: vec![],
    };
    #[allow(deprecated)]
    let context = MCPContext {
        resources: vec![],
        tools: vec![],
        servers: vec![
            server("3f6f2c1e-8b1a-4c2d-9e3f-0a1b2c3d4e5f", "linear", "linear"),
            server(
                "7c9e2d4a-0000-4000-8000-000000000001",
                "Team Sentry",
                "db4d553f-8172-4cad-8f48-bc53ba6f736a",
            ),
            server("7c9e2d4a-0000-4000-8000-000000000002", "local server", ""),
            server(
                "7c9e2d4a-0000-4000-8000-000000000003",
                "future integration",
                "not_yet_in_this_build",
            ),
        ],
    };

    let proto: api::request::McpContext = context.into();
    assert_eq!(proto.servers.len(), 4);

    // A well-known id becomes the enum; the installation id stays the key and
    // the display name is left to `name`.
    assert_eq!(proto.servers[0].id, "3f6f2c1e-8b1a-4c2d-9e3f-0a1b2c3d4e5f");
    let linear = proto.servers[0].identity.as_ref().expect("identity");
    assert_eq!(linear.integration(), api::McpIntegration::Linear);
    assert_eq!(linear.managed_server_uid, "");
    assert_eq!(linear.display_name, "");

    // A managed server uid goes in its own field.
    let sentry = proto.servers[1].identity.as_ref().expect("identity");
    assert_eq!(sentry.integration(), api::McpIntegration::Unspecified);
    assert_eq!(
        sentry.managed_server_uid,
        "db4d553f-8172-4cad-8f48-bc53ba6f736a"
    );

    // Local servers and ids this build does not know send no identity.
    for server in &proto.servers[2..] {
        assert!(server.identity.is_none(), "{}", server.name);
    }
}
