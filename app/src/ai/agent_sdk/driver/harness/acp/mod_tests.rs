use std::fs;

use async_io::Async;
use base64::Engine as _;
use futures::io::{AsyncBufReadExt as _, AsyncRead, AsyncWrite, AsyncWriteExt as _, BufReader};
use serde_json::json;
use tempfile::TempDir;
use url::Url;
use warp_cli::agent::HarnessTransport;
use warpui::App;
use warpui::r#async::FutureExt as _;

use super::attachments::ResolvedFile;
use super::protocol::{ContentBlock, PromptCapabilities};
use super::resume::ResumeMiss;
use super::*;
use crate::ai::agent_sdk::driver::harness::{HarnessKind, harness_kind};
use crate::server::server_api::ServerApiProvider;
use crate::terminal::History;
use crate::terminal::model::block::BlockMetadata;
use crate::terminal::model::session::SessionInfo;
use crate::terminal::shell::ShellType;
use crate::terminal::view::Event;
use crate::test_util::terminal::{add_window_with_terminal, initialize_app_for_terminal_view};

/// Plays the agent's side of the handshake, answering `initialize` with `initialize_result`,
/// and returns the content of the first `session/prompt`.
async fn first_acp_prompt(
    stream: impl AsyncRead + AsyncWrite + Unpin,
    token: &str,
    initialize_result: Value,
) -> Value {
    let mut stream = BufReader::new(stream);
    stream
        .get_mut()
        .write_all(format!("{token}\n").as_bytes())
        .await
        .unwrap();
    let mut line = String::new();
    stream.read_line(&mut line).await.unwrap();
    assert_eq!(line.trim_end(), "ok");
    for (method, result) in [
        ("initialize", initialize_result),
        ("session/new", json!({"sessionId": "test-session"})),
    ] {
        line.clear();
        stream.read_line(&mut line).await.unwrap();
        let request: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(request["method"], method);
        let response = json!({"jsonrpc": "2.0", "id": request["id"], "result": result});
        stream
            .get_mut()
            .write_all(format!("{response}\n").as_bytes())
            .await
            .unwrap();
    }
    line.clear();
    stream.read_line(&mut line).await.unwrap();
    let request: Value = serde_json::from_str(&line).unwrap();
    assert_eq!(request["method"], "session/prompt");
    assert_eq!(request["params"]["sessionId"], "test-session");
    request["params"]["prompt"].clone()
}

#[test]
#[serial_test::serial]
fn initial_files_reach_the_acp_runner_without_changing_the_pty_prompt() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let temp = TempDir::new().unwrap();
        let file_path = temp.path().join("image.png");
        fs::write(&file_path, b"image bytes").unwrap();
        let server_api = app.read(|ctx| ServerApiProvider::as_ref(ctx).get());
        let setup_events = app.read(|ctx| {
            SetupClientEventReporter::noop(server_api.clone(), ctx.background_executor().clone())
        });
        let external_conversation = ChannelState::mock_server()
            .mock("POST", "/api/v1/harness-support/external-conversation")
            .with_body(r#"{"conversation_id":"test-conversation"}"#)
            .expect(3)
            .create();

        let file_uri = Url::from_file_path(&file_path).unwrap();
        let link_block = json!({
            "type": "resource_link",
            "uri": file_uri.as_str(),
            "name": "image.png",
        });
        let image_block = json!({
            "type": "image",
            "data": base64::engine::general_purpose::STANDARD.encode(b"image bytes"),
            "mimeType": "image/png",
        });
        let baseline_agent = json!({"protocolVersion": PROTOCOL_VERSION});
        let image_agent = json!({
            "protocolVersion": PROTOCOL_VERSION,
            "agentCapabilities": {"promptCapabilities": {"image": true}},
        });
        let cases = [
            (HarnessTransport::Acp, baseline_agent, link_block),
            (HarnessTransport::Acp, image_agent, image_block),
            (HarnessTransport::Pty, Value::Null, Value::Null),
        ];
        for (transport, initialize_result, expected_file_block) in cases {
            let terminal = add_window_with_terminal(&mut app, None);
            let session_id = terminal.read(&app, |terminal, _| {
                terminal
                    .model
                    .lock()
                    .block_list()
                    .active_block()
                    .session_id()
                    .unwrap()
            });
            terminal.update(&mut app, |terminal, ctx| {
                terminal
                    .model_event_dispatcher()
                    .update(ctx, |dispatcher, _| {
                        dispatcher.set_active_session_id(session_id);
                    });
                terminal.sessions_model().update(ctx, |sessions, ctx| {
                    sessions.initialize_bootstrapped_session(
                        SessionInfo::new_for_test()
                            .with_id(session_id)
                            .with_shell_type(ShellType::Zsh),
                        "test command".to_owned(),
                        Vec::new(),
                        None,
                        ctx,
                    );
                });
                terminal.input().update(ctx, |input, ctx| {
                    input.set_active_block_metadata(
                        BlockMetadata::new(Some(session_id), None),
                        false,
                        ctx,
                    );
                });
            });
            History::initialized_sessions(&mut History::handle(&app), &mut app, vec![session_id])
                .await;
            let (commands, command_rx) = async_channel::unbounded();
            app.update(|ctx| {
                ctx.subscribe_to_view(&terminal, move |_, event, _| {
                    if let Event::ExecuteCommand(event) = event {
                        commands.try_send(event.command.clone()).unwrap();
                    }
                });
            });
            let driver = app.add_model(|ctx| {
                let terminal_driver = TerminalDriver::create_from_existing_view(terminal, ctx);
                AgentDriver::new_for_test(temp.path().to_path_buf(), terminal_driver, ctx)
            });
            let foreground = driver.update(&mut app, |_, ctx| ctx.spawner());
            let terminal_driver = driver.read(&app, |driver, _| driver.terminal_driver.clone());
            let mut harness = harness_kind(Harness::Codex, transport).unwrap();
            harness.set_initial_attachments(vec![DownloadedAttachment {
                file_id: "attachment-id".to_owned(),
                file_name: "image.png".to_owned(),
                file_path: file_path.to_string_lossy().into_owned(),
            }]);
            let HarnessKind::ThirdParty(harness) = harness else {
                panic!("expected a third-party harness");
            };
            let old_codex_home = std::env::var_os("CODEX_HOME");
            unsafe { std::env::set_var("CODEX_HOME", temp.path().join("codex")) };
            let runner = harness.build_runner(
                "inspect these",
                None,
                None,
                Some("context"),
                None,
                temp.path(),
                temp.path(),
                None,
                server_api.clone(),
                terminal_driver,
                None,
                &HashMap::new(),
                &[],
                false,
                &HashMap::new(),
                &ResolvedMCPServers::default(),
                None,
            );
            match old_codex_home {
                Some(home) => unsafe { std::env::set_var("CODEX_HOME", home) },
                None => unsafe { std::env::remove_var("CODEX_HOME") },
            }
            let runner = runner.unwrap();
            let observe_prompt = async {
                let command = command_rx.recv().await.unwrap();
                match transport {
                    HarnessTransport::Acp => {
                        let launch_args =
                            shell_words::split(command.split_once("--launch-file ").unwrap().1)
                                .unwrap();
                        let launch: bridge::BridgeLaunch =
                            serde_json::from_slice(&fs::read(&launch_args[0]).unwrap()).unwrap();
                        let prompt = match launch.endpoint {
                            #[cfg(unix)]
                            bridge::BridgeEndpoint::Unix(path) => {
                                let stream = Async::<std::os::unix::net::UnixStream>::connect(path)
                                    .await
                                    .unwrap();
                                first_acp_prompt(stream, &launch.token, initialize_result).await
                            }
                            #[cfg(not(unix))]
                            bridge::BridgeEndpoint::Unix(_) => panic!("expected a TCP endpoint"),
                            bridge::BridgeEndpoint::Tcp(address) => {
                                let stream = Async::<std::net::TcpStream>::connect(address)
                                    .await
                                    .unwrap();
                                first_acp_prompt(stream, &launch.token, initialize_result).await
                            }
                        };
                        assert_eq!(
                            prompt,
                            json!([
                                {"type": "text", "text": "context\n\ninspect these"},
                                expected_file_block,
                            ])
                        );
                    }
                    HarnessTransport::Pty => {
                        let prompt_path = command.split_once("$(cat '").unwrap().1;
                        let prompt_path = prompt_path.strip_suffix("')\"").unwrap();
                        assert_eq!(
                            fs::read_to_string(prompt_path).unwrap(),
                            "context\n\ninspect these"
                        );
                    }
                }
            };
            async {
                let (started, ()) =
                    futures::join!(runner.start(&foreground, &setup_events), observe_prompt);
                started.unwrap();
            }
            .with_timeout(Duration::from_secs(10))
            .await
            .unwrap();
        }
        external_conversation.assert();
    });
}

#[test]
fn initial_prompt_links_files_after_server_composed_text() {
    let dir = std::env::temp_dir().join("acp-initial-attachments");
    let screenshot = dir.join("image #雪%.png");
    let notes = dir.join("notes.txt");
    let harness = AcpHarness::new(Harness::Claude).unwrap();
    let text = harness.initial_prompt_text(
        "inspect these",
        Some("system"),
        Some("recovery"),
        Some("context"),
        &SessionOrigin::New,
    );
    let files = [
        ResolvedFile {
            name: "image #雪%.png".to_owned(),
            path: screenshot.clone(),
            mime_type: "image/png".to_owned(),
            contents: None,
        },
        ResolvedFile {
            name: "notes.txt".to_owned(),
            path: notes.clone(),
            mime_type: "text/plain".to_owned(),
            contents: None,
        },
    ];

    let blocks =
        attachments::prompt_content(PromptCapabilities::default(), text, Vec::new(), &files);

    assert_eq!(blocks.len(), 3);
    let ContentBlock::ResourceLink { uri: image_uri, .. } = &blocks[1] else {
        panic!("expected an image resource link");
    };
    let ContentBlock::ResourceLink { uri: notes_uri, .. } = &blocks[2] else {
        panic!("expected a non-image resource link");
    };
    assert_eq!(
        serde_json::to_value(&blocks).unwrap(),
        json!([
            {"type": "text", "text": "recovery\n\ncontext\n\nsystem\n\ninspect these"},
            {"type": "resource_link", "uri": image_uri, "name": "image #雪%.png"},
            {"type": "resource_link", "uri": notes_uri, "name": "notes.txt"}
        ])
    );
    assert_eq!(
        Url::parse(image_uri).unwrap().to_file_path().unwrap(),
        screenshot
    );
    assert_eq!(
        Url::parse(notes_uri).unwrap().to_file_path().unwrap(),
        notes
    );
    assert!(image_uri.contains("image%20%23%E9%9B%AA%25.png"));
}

#[test]
fn initial_prompt_without_files_keeps_the_existing_text() {
    let harness = AcpHarness::new(Harness::Codex).unwrap();

    let text = harness.initial_prompt_text(
        "hello",
        Some("staged in config"),
        Some(" "),
        None,
        &SessionOrigin::New,
    );

    assert_eq!(text, "hello");
}

#[test]
fn a_resumed_session_is_not_sent_the_system_prompt_again() {
    let harness = AcpHarness::new(Harness::Claude).unwrap();

    let resumed = harness.initial_prompt_text(
        "inspect these",
        Some("system"),
        Some("recovery"),
        Some("context"),
        &SessionOrigin::Resumed,
    );
    let fresh = harness.initial_prompt_text(
        "inspect these",
        Some("system"),
        Some("recovery"),
        Some("context"),
        &SessionOrigin::ResumeMissed(ResumeMiss::NoTranscript),
    );

    assert_eq!(resumed, "recovery\n\ncontext\n\ninspect these");
    assert_eq!(fresh, "recovery\n\ncontext\n\nsystem\n\ninspect these");
}

const HELPER_COMMAND: &str = "/usr/local/bin/oz mcp-credential --server linear";

/// Two resolved servers, of which `linear` carries an expiring credential and has a helper.
fn servers_with_a_credential_helper() -> ResolvedMCPServers {
    let configs: HashMap<String, JSONMCPServer> = serde_json::from_value(json!({
        "linear": {
            "url": "https://app.warp.dev/mcp/integration-proxy/linear",
            "headers": {"Authorization": "Bearer proxy-token"}
        },
        "docs": {"url": "https://docs.example.test/mcp"}
    }))
    .unwrap();
    ResolvedMCPServers {
        configs,
        credential_helpers: HashMap::from([(
            "linear".to_owned(),
            McpCredentialHelper::for_test("Authorization", HELPER_COMMAND),
        )]),
        ..Default::default()
    }
}

fn agent_info(name: &str, version: &str) -> AgentInfo {
    AgentInfo {
        name: name.to_owned(),
        version: version.to_owned(),
    }
}

/// The names of `servers`, sorted.
fn server_names(servers: &[McpServer]) -> Vec<&str> {
    let mut names: Vec<_> = servers.iter().map(McpServer::name).collect();
    names.sort_unstable();
    names
}

#[test]
fn only_claude_adapters_known_to_forward_mcp_options_are_sent_them() {
    let forwards = |name, version| forwards_claude_mcp_options(Some(&agent_info(name, version)));

    assert!(forwards("@agentclientprotocol/claude-agent-acp", "0.93.0"));
    assert!(forwards("@agentclientprotocol/claude-agent-acp", "1.2.0"));
    assert!(!forwards("@agentclientprotocol/claude-agent-acp", "0.92.9"));
    assert!(!forwards(
        "@agentclientprotocol/claude-agent-acp",
        "nightly"
    ));
    assert!(!forwards("@zed-industries/claude-code-acp", "0.93.0"));
    assert!(!forwards("codex-acp", "2.6.0"));
    assert!(!forwards_claude_mcp_options(None));
}

/// A session request has no field for a helper, so a Claude adapter that forwards SDK options
/// gets the helper-backed server there and not in the request's list, where the adapter would
/// prefer the entry with the fixed credential.
#[test]
fn claude_gets_helper_backed_servers_as_options_when_the_adapter_forwards_them() {
    let resolved = servers_with_a_credential_helper();
    let servers = SessionMcpServers::new(Harness::Claude, &resolved).unwrap();

    let forwarding = agent_info("@agentclientprotocol/claude-agent-acp", "0.93.0");
    let (in_request, meta) = servers.for_agent(Some(&forwarding));
    assert_eq!(server_names(&in_request), ["docs"]);
    let meta = meta.expect("adapter options");
    let linear = &meta["claudeCode"]["options"]["mcpServers"]["linear"];
    assert_eq!(linear["type"], "http");
    assert_eq!(linear["headersHelper"], HELPER_COMMAND);
    assert_eq!(linear["headers"]["Authorization"], "Bearer proxy-token");

    // An adapter that is not known to forward options keeps the fixed credential.
    let (in_request, meta) = servers.for_agent(Some(&agent_info("claude-code-acp", "0.16.0")));
    assert_eq!(server_names(&in_request), ["docs", "linear"]);
    assert!(meta.is_none());
}

#[test]
fn claude_without_credential_helpers_sends_no_adapter_options() {
    let resolved = ResolvedMCPServers {
        credential_helpers: HashMap::new(),
        ..servers_with_a_credential_helper()
    };
    let servers = SessionMcpServers::new(Harness::Claude, &resolved).unwrap();

    let forwarding = agent_info("@agentclientprotocol/claude-agent-acp", "0.93.0");
    let (in_request, meta) = servers.for_agent(Some(&forwarding));
    assert_eq!(server_names(&in_request), ["docs", "linear"]);
    assert!(meta.is_none());
}

/// Codex runs its own config entry for a server with a helper, and would report a second entry
/// of the same name in the session request as a server it did not start.
#[test]
fn codex_is_not_sent_the_servers_its_config_runs_with_a_helper() {
    let resolved = servers_with_a_credential_helper();
    let servers = SessionMcpServers::new(Harness::Codex, &resolved).unwrap();

    let (in_request, meta) = servers.for_agent(Some(&agent_info("codex-acp", "2.6.0")));
    assert_eq!(server_names(&in_request), ["docs"]);
    assert!(meta.is_none());
}

#[test]
fn harnesses_without_a_helper_mechanism_keep_every_server_as_resolved() {
    let resolved = servers_with_a_credential_helper();

    let gemini = SessionMcpServers::new(Harness::Gemini, &resolved).unwrap();
    let (in_request, meta) = gemini.for_agent(None);
    assert_eq!(server_names(&in_request), ["docs", "linear"]);
    assert!(meta.is_none());

    // Pi's adapter reads its servers from its own config file.
    let pi = SessionMcpServers::new(Harness::Pi, &resolved).unwrap();
    let (in_request, meta) = pi.for_agent(None);
    assert!(in_request.is_empty());
    assert!(meta.is_none());
}
