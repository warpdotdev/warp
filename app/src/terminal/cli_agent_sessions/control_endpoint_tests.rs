use std::sync::Arc;
#[cfg(unix)]
use std::time::Duration;

use parking_lot::Mutex;
#[cfg(unix)]
use tokio::io::{AsyncBufReadExt as _, BufReader, Lines};
#[cfg(unix)]
use tokio::net::UnixStream;
use warpui::EntityId;
#[cfg(unix)]
use warpui::r#async::executor::Background;

use super::*;
use crate::ai::blocklist::{InputConfig, InputType};
use crate::terminal::CLIAgent;
use crate::terminal::cli_agent_sessions::CLIAgentInputEntrypoint;

const ACTIVE_JSON: &str = r#"{"v":1,"event":"rich_input","active":true}"#;
const INACTIVE_JSON: &str = r#"{"v":1,"event":"rich_input","active":false}"#;

fn test_publisher() -> (CLIAgentControlPublisher, async_channel::Receiver<Arc<str>>) {
    let (tx, rx) = async_channel::bounded(CLIENT_QUEUE_CAPACITY);
    let state = Arc::new(Mutex::new(EndpointState {
        rich_input_active: false,
        clients: vec![tx],
        closed: false,
    }));
    (CLIAgentControlPublisher { state }, rx)
}

fn open_state() -> CLIAgentInputState {
    CLIAgentInputState::Open {
        entrypoint: CLIAgentInputEntrypoint::CtrlG,
        previous_input_config: InputConfig {
            input_type: InputType::Shell,
            is_locked: false,
        },
        previous_was_lock_set_with_empty_buffer: false,
    }
}

fn input_session_changed(
    terminal_view_id: EntityId,
    previous_input_state: CLIAgentInputState,
    new_input_state: CLIAgentInputState,
) -> CLIAgentSessionsModelEvent {
    CLIAgentSessionsModelEvent::InputSessionChanged {
        terminal_view_id,
        agent: CLIAgent::Claude,
        previous_input_state,
        new_input_state,
    }
}

#[cfg(unix)]
fn test_background() -> Arc<Background> {
    Arc::new(Background::new(1, |_| "cli-agent-control-test".to_owned()))
}

#[cfg(unix)]
async fn connect(endpoint: &CLIAgentControlEndpoint) -> Lines<BufReader<UnixStream>> {
    let stream = UnixStream::connect(endpoint.address()).await.unwrap();
    BufReader::new(stream).lines()
}

/// Reads the next control event, or `None` once the endpoint closed the connection.
#[cfg(unix)]
async fn next_line(lines: &mut Lines<BufReader<UnixStream>>) -> Option<String> {
    tokio::time::timeout(Duration::from_secs(5), lines.next_line())
        .await
        .expect("timed out waiting for a control event")
        .unwrap()
}

#[test]
fn rich_input_line_is_versioned_json_line() {
    assert_eq!(&*rich_input_line(true), format!("{ACTIVE_JSON}\n"));
    assert_eq!(&*rich_input_line(false), format!("{INACTIVE_JSON}\n"));
}

#[test]
fn publisher_sends_one_line_per_transition() {
    let (publisher, rx) = test_publisher();

    publisher.set_rich_input_active(false);
    assert!(rx.try_recv().is_err(), "closed -> closed publishes nothing");

    publisher.set_rich_input_active(true);
    publisher.set_rich_input_active(true);
    assert_eq!(&*rx.try_recv().unwrap(), format!("{ACTIVE_JSON}\n"));
    assert!(rx.try_recv().is_err(), "open -> open publishes nothing");

    publisher.set_rich_input_active(false);
    assert_eq!(&*rx.try_recv().unwrap(), format!("{INACTIVE_JSON}\n"));
    assert!(rx.try_recv().is_err());
}

#[test]
fn observe_maps_input_session_transitions() {
    let (publisher, rx) = test_publisher();
    let terminal_view_id = EntityId::new();

    publisher.observe(&input_session_changed(
        terminal_view_id,
        CLIAgentInputState::Closed,
        open_state(),
    ));
    assert_eq!(&*rx.try_recv().unwrap(), format!("{ACTIVE_JSON}\n"));

    publisher.observe(&input_session_changed(
        terminal_view_id,
        open_state(),
        open_state(),
    ));
    assert!(rx.try_recv().is_err(), "open -> open publishes nothing");

    publisher.observe(&input_session_changed(
        terminal_view_id,
        open_state(),
        CLIAgentInputState::Closed,
    ));
    assert_eq!(&*rx.try_recv().unwrap(), format!("{INACTIVE_JSON}\n"));
    assert!(rx.try_recv().is_err());
}

#[test]
fn observe_ended_while_active_publishes_inactive() {
    let (publisher, rx) = test_publisher();
    let terminal_view_id = EntityId::new();

    publisher.observe(&input_session_changed(
        terminal_view_id,
        CLIAgentInputState::Closed,
        open_state(),
    ));
    assert_eq!(&*rx.try_recv().unwrap(), format!("{ACTIVE_JSON}\n"));

    publisher.observe(&CLIAgentSessionsModelEvent::Ended {
        terminal_view_id,
        agent: CLIAgent::Claude,
    });
    assert_eq!(&*rx.try_recv().unwrap(), format!("{INACTIVE_JSON}\n"));
    assert!(rx.try_recv().is_err());
}

#[cfg(unix)]
#[test]
fn bound_unix_socket_is_owner_only_and_short() {
    use std::os::unix::fs::PermissionsExt as _;

    let endpoint = CLIAgentControlEndpoint::bind(test_background()).unwrap();
    let mode = std::fs::metadata(endpoint.address())
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(mode & 0o777, 0o600);
    #[cfg(target_os = "macos")]
    assert!(endpoint.address().len() < 104, "{:?}", endpoint.address());
}

#[cfg(unix)]
#[tokio::test]
async fn new_client_receives_current_state_on_connect() {
    let endpoint = CLIAgentControlEndpoint::bind(test_background()).unwrap();

    let mut connected_while_closed = connect(&endpoint).await;
    assert_eq!(
        next_line(&mut connected_while_closed).await.as_deref(),
        Some(INACTIVE_JSON)
    );

    endpoint.publisher().set_rich_input_active(true);
    let mut connected_while_open = connect(&endpoint).await;
    assert_eq!(
        next_line(&mut connected_while_open).await.as_deref(),
        Some(ACTIVE_JSON)
    );
    assert_eq!(
        next_line(&mut connected_while_closed).await.as_deref(),
        Some(ACTIVE_JSON)
    );
}

#[cfg(unix)]
#[tokio::test]
async fn broadcast_survives_client_disconnect_and_ends_on_drop() {
    let endpoint = CLIAgentControlEndpoint::bind(test_background()).unwrap();
    let publisher = endpoint.publisher();

    let mut first = connect(&endpoint).await;
    let mut second = connect(&endpoint).await;
    assert_eq!(next_line(&mut first).await.as_deref(), Some(INACTIVE_JSON));
    assert_eq!(next_line(&mut second).await.as_deref(), Some(INACTIVE_JSON));

    publisher.set_rich_input_active(true);
    assert_eq!(next_line(&mut first).await.as_deref(), Some(ACTIVE_JSON));
    assert_eq!(next_line(&mut second).await.as_deref(), Some(ACTIVE_JSON));

    drop(first);
    publisher.set_rich_input_active(false);
    assert_eq!(next_line(&mut second).await.as_deref(), Some(INACTIVE_JSON));

    drop(endpoint);
    assert_eq!(next_line(&mut second).await, None);
}
