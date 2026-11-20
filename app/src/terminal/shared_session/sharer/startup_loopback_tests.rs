use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use byte_unit::Byte;
use instant::Instant;
use session_sharing_protocol::common::{
    ParticipantId, Scrollback, SessionId, UniversalDeveloperInputContext, WindowSize,
};
use session_sharing_protocol::sharer::{
    DownstreamMessage, FailedToInitializeSessionReason, Lifetime, ReconnectToken,
    SessionEndedReason, UpstreamMessage,
};
use warpui::r#async::FutureExt as _;
use warpui::{App, ModelHandle};
use websocket::tungstenite;

use super::{Network, NetworkEvent, Stage, StartupConfig, StartupRetryState, startup_max_attempts};
use crate::editor::ReplicaId;
use crate::terminal::shared_session::SharedSessionSource;
use crate::test_util::assert_eventually;

#[derive(Default)]
struct Shutdown {
    stopped: Mutex<bool>,
    changed: Condvar,
}

impl Shutdown {
    fn wait_until(&self, deadline: Instant) -> bool {
        let mut stopped = self.stopped.lock().unwrap();
        while !*stopped {
            let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
                return true;
            };
            stopped = self.changed.wait_timeout(stopped, remaining).unwrap().0;
        }
        false
    }

    fn stop(&self) {
        *self.stopped.lock().unwrap() = true;
        self.changed.notify_all();
    }
}

#[derive(Clone, Debug, Default)]
struct ServerObservations {
    connections: Vec<Duration>,
    initializations: Vec<Duration>,
}

struct LoopbackServer {
    address: SocketAddr,
    started: Instant,
    observations: Arc<Mutex<ServerObservations>>,
    shutdown: Arc<Shutdown>,
    thread: Option<JoinHandle<()>>,
}

impl LoopbackServer {
    fn start(handshake_delay: Duration, initialization_delay: Duration) -> Self {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let address = listener.local_addr().unwrap();
        let started = Instant::now();
        let observations = Arc::new(Mutex::new(ServerObservations::default()));
        let shutdown = Arc::new(Shutdown::default());
        let server_observations = observations.clone();
        let server_shutdown = shutdown.clone();
        let thread = thread::spawn(move || {
            let mut connections = Vec::new();
            while !*server_shutdown.stopped.lock().unwrap() {
                let (stream, _) = listener.accept().unwrap();
                if *server_shutdown.stopped.lock().unwrap() {
                    break;
                }
                server_observations
                    .lock()
                    .unwrap()
                    .connections
                    .push(started.elapsed());
                let observations = server_observations.clone();
                let shutdown = server_shutdown.clone();
                connections.push(thread::spawn(move || {
                    serve_connection(
                        stream,
                        started,
                        handshake_delay,
                        initialization_delay,
                        observations,
                        shutdown,
                    );
                }));
            }
            for connection in connections {
                connection.join().unwrap();
            }
        });
        Self {
            address,
            started,
            observations,
            shutdown,
            thread: Some(thread),
        }
    }
}

impl Drop for LoopbackServer {
    fn drop(&mut self) {
        self.shutdown.stop();
        // Wake the blocking accept before joining all owned connection threads.
        let _ = TcpStream::connect_timeout(&self.address, Duration::from_secs(1));
        self.thread.take().unwrap().join().unwrap();
    }
}

fn serve_connection(
    stream: TcpStream,
    started: Instant,
    handshake_delay: Duration,
    initialization_delay: Duration,
    observations: Arc<Mutex<ServerObservations>>,
    shutdown: Arc<Shutdown>,
) {
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    stream
        .set_write_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    if !shutdown.wait_until(started + handshake_delay) {
        return;
    }
    let Ok(mut socket) = tungstenite::accept(stream) else {
        return;
    };
    let Ok(message) = socket.read() else {
        return;
    };
    let initialize = UpstreamMessage::from_json(message.to_text().unwrap()).unwrap();
    let UpstreamMessage::Initialize(payload) = initialize else {
        panic!("Expected the real startup path to send Initialize");
    };
    assert!(payload.user_id.access_token.is_none());
    assert_eq!(payload.source_task_id.as_deref(), Some("loopback-startup"));
    observations
        .lock()
        .unwrap()
        .initializations
        .push(started.elapsed());
    if !shutdown.wait_until(started + initialization_delay) {
        return;
    }
    let response = DownstreamMessage::SessionInitialized {
        session_id: SessionId::new(),
        session_secret: Default::default(),
        reconnect_token: ReconnectToken::new(),
        sharer_id: ParticipantId::new(),
        sharer_firebase_uid: "loopback-user".to_string(),
    };
    if socket
        .send(tungstenite::Message::Text(response.to_json().unwrap()))
        .is_ok()
    {
        shutdown.wait_until(started + Duration::from_secs(30));
    }
}

#[derive(Debug)]
struct StartupObservation {
    result: Result<(), FailedToInitializeSessionReason>,
    elapsed: Duration,
    server: ServerObservations,
}

#[test]
fn cancel_pending_startup_aborts_timer_and_transport() {
    App::test((), |mut app| async move {
        let server = LoopbackServer::start(Duration::from_secs(60), Duration::ZERO);
        let (network, result_rx) = start_network(&server, &mut app);
        assert_eventually!(
            server.observations.lock().unwrap().connections.len() == 1,
            "Loopback connection should begin before cancellation"
        );
        let (timer, transport) = network.read(&app, |network, _| {
            let Stage::BeforeStarted { startup_retry } = &network.stage else {
                panic!("Expected pending startup");
            };
            (
                startup_retry.timeout_abort_handle.as_ref().unwrap().clone(),
                startup_retry
                    .transport_abort_handle
                    .as_ref()
                    .unwrap()
                    .clone(),
            )
        });

        network.update(&mut app, |network, _| {
            network.end_session(SessionEndedReason::EndedBySharer)
        });

        assert!(timer.is_aborted());
        assert!(transport.is_aborted());
        assert!(
            result_rx
                .recv()
                .with_timeout(Duration::from_millis(100))
                .await
                .is_err()
        );
        assert_eq!(server.observations.lock().unwrap().connections.len(), 1);
        network.read(&app, |network, _| {
            assert!(matches!(network.stage, Stage::Finished))
        });
    });
}

#[test]
fn dropping_pending_startup_aborts_timer_and_transport() {
    App::test((), |mut app| async move {
        let server = LoopbackServer::start(Duration::from_secs(60), Duration::ZERO);
        let (network, _) = start_network(&server, &mut app);
        assert_eventually!(
            server.observations.lock().unwrap().connections.len() == 1,
            "Loopback connection should begin before dropping the network"
        );
        let (timer, transport) = network.read(&app, |network, _| {
            let Stage::BeforeStarted { startup_retry } = &network.stage else {
                panic!("Expected pending startup");
            };
            (
                startup_retry.timeout_abort_handle.as_ref().unwrap().clone(),
                startup_retry
                    .transport_abort_handle
                    .as_ref()
                    .unwrap()
                    .clone(),
            )
        });

        drop(network);
        app.update(|_| {});

        assert_eventually!(
            timer.is_aborted() && transport.is_aborted(),
            "Dropping the network should abort its startup work"
        );
        assert_eq!(server.observations.lock().unwrap().connections.len(), 1);
    });
}

fn start_network(
    server: &LoopbackServer,
    app: &mut App,
) -> (
    ModelHandle<Network>,
    async_channel::Receiver<Result<(), FailedToInitializeSessionReason>>,
) {
    let network = super::tests::create_network(app, false).0;
    let (result_tx, result_rx) = async_channel::bounded(1);
    app.update(|ctx| {
        ctx.subscribe_to_model(&network, move |_, event, _| match event {
            NetworkEvent::SharedSessionCreatedSuccessfully { .. } => {
                result_tx.try_send(Ok(())).unwrap();
            }
            NetworkEvent::FailedToCreateSharedSession { reason, .. } => {
                result_tx.try_send(Err(reason.clone())).unwrap();
            }
            _ => {}
        });
    });
    network.update(app, |network, ctx| {
        let source = SharedSessionSource::ambient_agent(Some("loopback-startup".to_string()));
        network.stage = Stage::BeforeStarted {
            startup_retry: StartupRetryState::new(startup_max_attempts(&source)),
        };
        network.source = source;
        network.max_session_size = Byte::from_u64(200 * 1024 * 1024);
        network.test_max_websocket_message_bytes = Some(200 * 1024 * 1024);
        network.startup_config = Some(StartupConfig {
            endpoint: format!("ws://{}/sessions/create", server.address),
            scrollback: Scrollback {
                blocks: vec![],
                is_alt_screen_active: false,
            },
            window_size: WindowSize::default(),
            init_block_id: network.next_buffer_seq_no.0.clone(),
            input_replica_id: ReplicaId::new("loopback-replica"),
            universal_developer_input_context: UniversalDeveloperInputContext::default(),
            lifetime: Lifetime::default(),
            selected_model_id: "loopback-model".to_string(),
            share_with_team_uid: None,
        });
        network.start_create_session_attempt(ctx);
    });
    (network, result_rx)
}

async fn observe_startup(
    handshake_delay: Duration,
    initialization_delay: Duration,
    app: &mut App,
) -> StartupObservation {
    let server = LoopbackServer::start(handshake_delay, initialization_delay);
    let (network, result_rx) = start_network(&server, app);
    let result = result_rx
        .recv()
        .with_timeout(Duration::from_secs(50))
        .await
        .expect("Real startup timers should resolve the scenario within 50 seconds")
        .unwrap();
    let elapsed = server.started.elapsed();
    let observations = server.observations.lock().unwrap().clone();
    network.update(app, |network, _| network.close_without_reconnection());
    drop(server);
    StartupObservation {
        result,
        elapsed,
        server: observations,
    }
}

#[test]
#[ignore = "manual loopback verification with real startup timers"]
fn immediate_handshake_starts_session() {
    App::test((), |mut app| async move {
        let observation = observe_startup(Duration::ZERO, Duration::ZERO, &mut app).await;
        eprintln!("immediate_handshake: {observation:?}");
        assert!(observation.result.is_ok());
        assert_eq!(observation.server.connections.len(), 1);
        assert_eq!(observation.server.initializations.len(), 1);
        assert!(observation.elapsed < Duration::from_secs(5));
    });
}

#[test]
#[ignore = "manual loopback verification with the real startup deadline"]
fn stalled_initialization_exhausts_the_overall_deadline_without_retry() {
    App::test((), |mut app| async move {
        let observation = observe_startup(Duration::ZERO, Duration::from_secs(60), &mut app).await;
        eprintln!("stalled_initialization: {observation:?}");
        let reason = observation.result.unwrap_err();
        assert!(matches!(
            &reason,
            FailedToInitializeSessionReason::InternalServerError { details }
                if details == "Timed out creating shared session"
        ));
        assert_eq!(observation.server.connections.len(), 1);
        assert_eq!(observation.server.initializations.len(), 1);
        assert!(observation.elapsed >= Duration::from_secs(44));
        assert!(observation.elapsed < Duration::from_secs(50));
    });
}

#[test]
#[ignore = "manual loopback verification with real startup timers"]
fn handshake_ready_after_eight_seconds_succeeds() {
    App::test((), |mut app| async move {
        let observation = observe_startup(Duration::from_secs(8), Duration::ZERO, &mut app).await;
        eprintln!("handshake_ready_after_eight_seconds: {observation:?}");
        assert!(observation.result.is_ok());
        assert_eq!(observation.server.connections.len(), 1);
        assert_eq!(observation.server.initializations.len(), 1);
        assert!(observation.elapsed >= Duration::from_secs(8));
        assert!(observation.elapsed < Duration::from_secs(13));
    });
}

#[test]
#[ignore = "manual loopback verification with real startup timers"]
fn handshake_ready_after_twenty_seconds_recovers_on_retry() {
    App::test((), |mut app| async move {
        let observation = observe_startup(Duration::from_secs(20), Duration::ZERO, &mut app).await;
        eprintln!("handshake_ready_after_twenty_seconds: {observation:?}");
        assert!(observation.result.is_ok());
        assert_eq!(observation.server.connections.len(), 2);
        assert_eq!(observation.server.initializations.len(), 1);
        assert!(observation.elapsed >= Duration::from_secs(20));
        assert!(observation.elapsed < Duration::from_secs(25));
    });
}

#[test]
#[ignore = "manual loopback verification with real startup timers"]
fn initialization_ready_after_twenty_seconds_does_not_create_duplicate_sessions() {
    App::test((), |mut app| async move {
        let observation = observe_startup(Duration::ZERO, Duration::from_secs(20), &mut app).await;
        eprintln!("initialization_ready_after_twenty_seconds: {observation:?}");
        assert!(observation.result.is_ok());
        assert_eq!(observation.server.connections.len(), 1);
        assert_eq!(observation.server.initializations.len(), 1);
        assert!(observation.elapsed >= Duration::from_secs(20));
        assert!(observation.elapsed < Duration::from_secs(25));
    });
}

#[test]
#[ignore = "manual loopback verification with the real startup deadline"]
fn unavailable_handshake_exhausts_the_overall_deadline() {
    App::test((), |mut app| async move {
        let observation = observe_startup(Duration::from_secs(60), Duration::ZERO, &mut app).await;
        eprintln!("unavailable_handshake: {observation:?}");
        let reason = observation.result.unwrap_err();
        assert!(matches!(
            &reason,
            FailedToInitializeSessionReason::InternalServerError { details }
                if details == "Timed out creating shared session"
        ));
        assert_eq!(observation.server.connections.len(), 3);
        assert_eq!(observation.server.initializations.len(), 0);
        assert!(observation.elapsed >= Duration::from_secs(44));
        assert!(observation.elapsed < Duration::from_secs(50));
    });
}
