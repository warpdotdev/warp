use std::sync::Arc;
use std::time::Duration;

use async_channel::Sender;
use async_io::Timer;
use futures_util::stream::AbortHandle;
use instant::Instant;
use parking_lot::FairMutex;
use session_sharing_protocol::common::{AgentAttachment, AgentPromptRequest, AgentPromptRequestId};
use session_sharing_protocol::viewer::UpstreamMessage;
use warpui::{App, ModelHandle};

use super::{Network, PtyBytesBatchStatus, ServerMessageSendOutcome, Stage, upstream_message_kind};
use crate::terminal::TerminalModel;
use crate::terminal::event_listener::ChannelEventListener;
use crate::terminal::shared_session::shared_handlers::RemoteUpdateGuard;
use crate::test_util::add_window_with_terminal;
use crate::test_util::terminal::initialize_app_for_terminal_view;

fn create_network(app: &mut App) -> (ModelHandle<Network>, Sender<Vec<u8>>) {
    initialize_app_for_terminal_view(app);
    let terminal_view = add_window_with_terminal(app, None).downgrade();
    let terminal_model = Arc::new(FairMutex::new(TerminalModel::mock(None, None)));
    let channel_event_proxy = ChannelEventListener::new_for_test();
    let (write_to_pty_events_tx, write_to_pty_events_rx) = async_channel::unbounded();

    let network = app.add_model(|ctx| {
        Network::new_for_test(
            channel_event_proxy,
            terminal_view,
            terminal_model,
            write_to_pty_events_rx,
            RemoteUpdateGuard::new(),
            ctx,
        )
    });

    network.update(app, |network, _| {
        network.stage = Stage::JoinedSuccessfully;
    });

    (network, write_to_pty_events_tx)
}

/// Sentinel strings that must never reach a log line. Each is distinctive enough that a substring
/// search cannot match it by accident.
const SENTINEL_PROMPT: &str = "ZZ-secret-prompt-body-ZZ";
const SENTINEL_FILE_NAME: &str = "ZZ-secret-attachment-name-ZZ";
const SENTINEL_ATTACHMENT_ID: &str = "ZZ-secret-attachment-id-ZZ";

fn agent_prompt_message(request_id: AgentPromptRequestId) -> UpstreamMessage {
    UpstreamMessage::SendAgentPrompt(AgentPromptRequest {
        id: request_id,
        server_conversation_token: None,
        prompt: SENTINEL_PROMPT.to_owned(),
        attachments: vec![
            AgentAttachment::PlainText {
                content: SENTINEL_PROMPT.to_owned(),
            },
            AgentAttachment::FileReference {
                attachment_id: SENTINEL_ATTACHMENT_ID.to_owned(),
                file_name: SENTINEL_FILE_NAME.to_owned(),
            },
        ],
    })
}

/// Builds each non-joined stage. `Reconnecting` needs a live abort handle, so it is constructed
/// from a throwaway abortable pair.
fn non_joined_stages() -> Vec<(&'static str, Stage)> {
    let (reconnecting_abort_handle, _registration) = AbortHandle::new_pair();
    vec![
        ("before_joined", Stage::BeforeJoined),
        (
            "reconnecting",
            Stage::Reconnecting {
                abort_handle: reconnecting_abort_handle,
            },
        ),
        ("finished", Stage::Finished),
    ]
}

#[test]
fn send_agent_prompt_request_is_undeliverable_in_every_non_joined_stage() {
    // The silent-drop bug: any stage other than `JoinedSuccessfully` discarded the prompt without
    // telling the caller. Each stage must now report `Undeliverable` and enqueue nothing.
    App::test((), |mut app| async move {
        let (network, _) = create_network(&mut app);
        let ws_proxy_rx = network.read(&app, |network, _ctx| network.ws_proxy_rx.clone());

        for (stage_name, stage) in non_joined_stages() {
            network.update(&mut app, |network, _| {
                network.stage = stage;
            });
            let outcome = network.update(&mut app, |network, _| {
                network.send_agent_prompt_request(
                    AgentPromptRequestId::new(),
                    None,
                    SENTINEL_PROMPT.to_owned(),
                    vec![],
                )
            });
            assert_eq!(
                outcome,
                ServerMessageSendOutcome::Undeliverable,
                "stage {stage_name} must report the prompt as undeliverable"
            );
            assert_eq!(
                ws_proxy_rx.len(),
                0,
                "stage {stage_name} must not enqueue anything on the proxy channel"
            );
        }
    });
}

#[test]
fn send_agent_prompt_request_is_undeliverable_when_the_proxy_channel_is_closed() {
    // A joined stage is not sufficient: the proxy channel closes when the websocket goes away, and
    // that rejection has to surface too.
    App::test((), |mut app| async move {
        let (network, _) = create_network(&mut app);
        network.update(&mut app, |network, _| {
            network.ws_proxy_tx.close();
        });

        let outcome = network.update(&mut app, |network, _| {
            network.send_agent_prompt_request(
                AgentPromptRequestId::new(),
                None,
                SENTINEL_PROMPT.to_owned(),
                vec![],
            )
        });

        assert_eq!(outcome, ServerMessageSendOutcome::Undeliverable);
    });
}

#[test]
fn send_agent_prompt_request_locally_queues_under_the_callers_request_id() {
    // The caller mints the ID so it can match the server's `AgentPromptRequestInFlight` echo to the
    // prompt it staged; the network must send under that exact ID rather than one of its own.
    App::test((), |mut app| async move {
        let (network, _) = create_network(&mut app);
        let ws_proxy_rx = network.read(&app, |network, _ctx| network.ws_proxy_rx.clone());
        let request_id = AgentPromptRequestId::new();

        let outcome = network.update(&mut app, |network, _| {
            network.send_agent_prompt_request(request_id.clone(), None, "hello".to_owned(), vec![])
        });

        assert_eq!(outcome, ServerMessageSendOutcome::LocallyQueued);
        let message = ws_proxy_rx.recv().await.expect("a message was enqueued");
        let UpstreamMessage::SendAgentPrompt(request) = message else {
            panic!("expected a SendAgentPrompt message");
        };
        assert_eq!(request.id, request_id);
        assert_eq!(request.prompt, "hello");
    });
}

#[test]
fn upstream_message_kind_is_a_static_label_not_a_rendering_of_the_message() {
    // `Debug`-formatting the message would nest the prompt and attachments into the log, so the
    // kind has to stay a fixed label that cannot vary with the payload.
    let empty = UpstreamMessage::SendAgentPrompt(AgentPromptRequest {
        id: AgentPromptRequestId::new(),
        server_conversation_token: None,
        prompt: String::new(),
        attachments: vec![],
    });
    let loaded = agent_prompt_message(AgentPromptRequestId::new());

    assert_eq!(upstream_message_kind(&empty), "SendAgentPrompt");
    assert_eq!(
        upstream_message_kind(&empty),
        upstream_message_kind(&loaded),
        "the label must not vary with the message's contents"
    );
}

#[test]
fn test_send_pty_write_event_advances_event_no() {
    App::test((), |mut app| async move {
        let (network, _) = create_network(&mut app);

        // Event number should start at 0.
        network.read(&app, |network, _ctx| {
            assert_eq!(network.write_to_pty_event_no.as_usize(), 0);
        });

        // Try to send a write to pty event message to the server.
        network.update(&mut app, |network, ctx| {
            let abort_handle = ctx.spawn_abortable(
                Timer::after(Duration::from_millis(1)),
                move |_, _, _| {},
                |_, _| {},
            );
            network.pty_bytes_batch_status = PtyBytesBatchStatus::Batching {
                accumulated: "a".into(),
                abort_handle,
            };
        });

        network.update(&mut app, |network, _| {
            network.send_write_to_pty();
        });

        // Event number is advanced to 1.
        network.read(&app, |network, _ctx| {
            assert_eq!(network.write_to_pty_event_no.as_usize(), 1);
        });
    });
}

#[test]
fn test_send_pty_write_event_while_batching() {
    App::test((), |mut app| async move {
        let (network, tx) = create_network(&mut app);
        let ws_proxy_rx = network.read(&app, |network, _ctx| network.ws_proxy_rx.clone());
        let init_time = Instant::now();

        // Reset batching status.
        network.update(&mut app, |network, _ctx| {
            network.pty_bytes_batch_status = PtyBytesBatchStatus::NotBatching {
                last_sent_at: init_time,
            };
        });

        // Try to send write to pty events.
        tx.try_send("a".into())
            .expect("Can send event over write_to_pty_tx");
        tx.try_send("b".into())
            .expect("Can send event over write_to_pty_tx");

        // Ensure the accumulated event is sent to the server, and the item in ws_proxy_tx is correct.
        let item = ws_proxy_rx.recv().await;
        assert!(
            matches!(item.unwrap(), UpstreamMessage::WriteToPty { bytes, .. } if bytes == b"ab")
        );

        // The batch status should be updated.
        network.read(&app, |network, _ctx| {
            assert!(matches!(network.pty_bytes_batch_status, PtyBytesBatchStatus::NotBatching { last_sent_at } if last_sent_at > init_time));
        });
    });
}

#[test]
fn test_send_pty_write_event_while_not_batching() {
    App::test((), |mut app| async move {
        let (network, _) = create_network(&mut app);
        let ws_proxy_rx = network.read(&app, |network, _ctx| network.ws_proxy_rx.clone());
        let init_time = Instant::now();

        // Set batch status to not batching.
        network.update(&mut app, |network, _ctx| {
            network.pty_bytes_batch_status = PtyBytesBatchStatus::NotBatching {
                last_sent_at: init_time,
            };
        });

        // Try to send write to pty message to server.
        network.update(&mut app, |network, _| {
            network.send_write_to_pty();
        });

        // Make sure we didn't try to send anything to the server.
        assert_eq!(ws_proxy_rx.len(), 0);

        // The batch status should be unchanged.
        network.read(&app, |network, _ctx| {
            assert!(matches!(network.pty_bytes_batch_status, PtyBytesBatchStatus::NotBatching { last_sent_at } if last_sent_at == init_time));
        });
    });
}
