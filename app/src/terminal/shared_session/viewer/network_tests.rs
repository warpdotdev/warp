use async_channel::Sender;
use async_io::Timer;
use instant::Instant;
use session_sharing_protocol::viewer::UpstreamMessage;
use std::{sync::Arc, time::Duration};

use parking_lot::FairMutex;

use warpui::{App, ModelHandle};

use crate::{
    terminal::{event_listener::ChannelEventListener, TerminalModel},
    test_util::{add_window_with_terminal, terminal::initialize_app_for_terminal_view},
};

use super::{Network, PtyBytesBatchStatus, Stage};

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
            ctx,
        )
    });

    network.update(app, |network, _| {
        network.stage = Stage::JoinedSuccessfully;
    });

    (network, write_to_pty_events_tx)
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
