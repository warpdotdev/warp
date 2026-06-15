//! Warp's ACP connection wrapper.
//!
//! Based on `acpx` but auto-approves `session/request_permission` so local agents
//! can run tools without blocking on a client that doesn't implement the UI yet.

use std::{cell::RefCell, io::ErrorKind, rc::Rc};

use acpx::{Error, Result, RuntimeContext};
use agent_client_protocol::{self as acp, Agent as _};
use async_process::{Child, Command, Stdio};
use futures::channel::{mpsc, oneshot};

#[derive(Clone, Debug, Default)]
struct SessionUpdateBroadcaster {
    subscribers: Rc<RefCell<Vec<mpsc::UnboundedSender<acp::SessionNotification>>>>,
}

impl SessionUpdateBroadcaster {
    fn subscribe(&self) -> mpsc::UnboundedReceiver<acp::SessionNotification> {
        let (tx, rx) = mpsc::unbounded();
        self.subscribers.borrow_mut().push(tx);
        rx
    }

    fn publish(&self, notification: &acp::SessionNotification) {
        let mut subscribers = self.subscribers.borrow_mut();
        subscribers.retain(|subscriber| subscriber.unbounded_send(notification.clone()).is_ok());
    }
}

#[derive(Clone, Debug)]
struct ConnectionClient {
    session_updates: SessionUpdateBroadcaster,
}

impl ConnectionClient {
    fn new(session_updates: SessionUpdateBroadcaster) -> Self {
        Self { session_updates }
    }
}

#[async_trait::async_trait(?Send)]
impl acp::Client for ConnectionClient {
    async fn request_permission(
        &self,
        args: acp::RequestPermissionRequest,
    ) -> acp::Result<acp::RequestPermissionResponse> {
        let Some(option) = preferred_permission_option(&args.options) else {
            return Err(acp::Error::internal_error());
        };

        Ok(acp::RequestPermissionResponse::new(
            acp::RequestPermissionOutcome::Selected(acp::SelectedPermissionOutcome::new(
                option.option_id.clone(),
            )),
        ))
    }

    async fn session_notification(&self, args: acp::SessionNotification) -> acp::Result<()> {
        self.session_updates.publish(&args);
        Ok(())
    }

    async fn ext_method(&self, _args: acp::ExtRequest) -> acp::Result<acp::ExtResponse> {
        Err(acp::Error::method_not_found())
    }

    async fn ext_notification(&self, _args: acp::ExtNotification) -> acp::Result<()> {
        Err(acp::Error::method_not_found())
    }
}

fn preferred_permission_option(
    options: &[acp::PermissionOption],
) -> Option<&acp::PermissionOption> {
    options
        .iter()
        .find(|option| option.kind == acp::PermissionOptionKind::AllowAlways)
        .or_else(|| {
            options
                .iter()
                .find(|option| option.kind == acp::PermissionOptionKind::AllowOnce)
        })
        .or_else(|| options.first())
}

struct ConnectionState {
    connection: Option<Rc<acp::ClientSideConnection>>,
    child: Option<Child>,
    io_task: Option<oneshot::Receiver<Result<()>>>,
}

/// A connected ACP client bound to a local subprocess.
pub(crate) struct Connection {
    session_updates: SessionUpdateBroadcaster,
    state: Rc<RefCell<ConnectionState>>,
}

impl Connection {
    pub(crate) fn spawn(command: &mut Command, runtime: &RuntimeContext) -> Result<Self> {
        command.stdin(Stdio::piped());
        command.stdout(Stdio::piped());
        command.kill_on_drop(true);

        let mut child = command
            .spawn()
            .map_err(|source| Error::SpawnProcess { source })?;
        let outgoing = child.stdin.take().ok_or(Error::MissingChildStdin)?;
        let incoming = child.stdout.take().ok_or(Error::MissingChildStdout)?;
        let session_updates = SessionUpdateBroadcaster::default();
        let client = ConnectionClient::new(session_updates.clone());
        let runtime_for_sdk = runtime.clone();
        let (connection, io_task) =
            acp::ClientSideConnection::new(client, outgoing, incoming, move |task| {
                runtime_for_sdk.spawn_local(task);
            });
        let connection = Rc::new(connection);
        let (io_task_tx, io_task_rx) = oneshot::channel();

        runtime.spawn(async move {
            let _ = io_task_tx.send(io_task.await.map_err(Error::from));
        });

        Ok(Self {
            session_updates,
            state: Rc::new(RefCell::new(ConnectionState {
                connection: Some(connection),
                child: Some(child),
                io_task: Some(io_task_rx),
            })),
        })
    }

    pub(crate) fn subscribe_session_updates(
        &self,
    ) -> mpsc::UnboundedReceiver<acp::SessionNotification> {
        self.session_updates.subscribe()
    }

    pub(crate) async fn close(&self) -> Result<()> {
        let (connection, mut child, io_task) = {
            let mut state = self.state.borrow_mut();
            let Some(connection) = state.connection.take() else {
                return Ok(());
            };

            (connection, state.child.take(), state.io_task.take())
        };

        drop(connection);

        if let Some(child) = child.as_mut() {
            match child.kill() {
                Ok(()) => {}
                Err(source) if source.kind() == ErrorKind::InvalidInput => {}
                Err(source) => return Err(Error::KillProcess { source }),
            }
        }

        if let Some(mut child) = child {
            child
                .status()
                .await
                .map_err(|source| Error::WaitForProcess { source })?;
        }

        if let Some(io_task) = io_task {
            let _ = io_task.await;
        }

        Ok(())
    }

    pub(crate) async fn initialize(
        &self,
        args: acp::InitializeRequest,
    ) -> Result<acp::InitializeResponse> {
        self.connection()?.initialize(args).await.map_err(Error::from)
    }

    pub(crate) async fn authenticate(
        &self,
        args: acp::AuthenticateRequest,
    ) -> Result<acp::AuthenticateResponse> {
        self.connection()?.authenticate(args).await.map_err(Error::from)
    }

    pub(crate) async fn new_session(
        &self,
        args: acp::NewSessionRequest,
    ) -> Result<acp::NewSessionResponse> {
        self.connection()?.new_session(args).await.map_err(Error::from)
    }

    pub(crate) async fn set_session_mode(
        &self,
        args: acp::SetSessionModeRequest,
    ) -> Result<acp::SetSessionModeResponse> {
        self.connection()?
            .set_session_mode(args)
            .await
            .map_err(Error::from)
    }

    pub(crate) async fn prompt(&self, args: acp::PromptRequest) -> Result<acp::PromptResponse> {
        self.connection()?.prompt(args).await.map_err(Error::from)
    }

    pub(crate) async fn set_session_config_option(
        &self,
        args: acp::SetSessionConfigOptionRequest,
    ) -> Result<acp::SetSessionConfigOptionResponse> {
        self.connection()?
            .set_session_config_option(args)
            .await
            .map_err(Error::from)
    }

    pub(crate) async fn ext_method(&self, args: acp::ExtRequest) -> Result<acp::ExtResponse> {
        self.connection()?.ext_method(args).await.map_err(Error::from)
    }

    fn connection(&self) -> Result<Rc<acp::ClientSideConnection>> {
        self.state.borrow().connection.clone().ok_or(Error::Closed)
    }
}

impl Drop for Connection {
    fn drop(&mut self) {
        let (connection, child, io_task) = {
            let mut state = self.state.borrow_mut();
            (
                state.connection.take(),
                state.child.take(),
                state.io_task.take(),
            )
        };

        drop(connection);
        drop(io_task);
        drop(child);
    }
}
