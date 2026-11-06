//! Fixtures for the standby control-stream tests: a fake sandbox client, a controllable clock,
//! a scripted server, and the harness that drives `run_lifecycle`.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::TimeDelta;
use futures::channel::oneshot;
use futures::executor::block_on;
use futures::{FutureExt as _, StreamExt as _, future, sink};
use warpui::r#async::FutureExt as _;

use super::super::*;

/// A recorded call against the fake client.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Call {
    Status(PrewarmedSandboxState),
    Connected,
    Install,
    Run,
}

#[derive(Default)]
pub(super) struct FakeClientState {
    pub calls: Vec<Call>,
    pub fail_status: bool,
    pub status_release: Option<oneshot::Receiver<()>>,
}

#[derive(Default)]
pub(super) struct FakeClient {
    pub state: Mutex<FakeClientState>,
    pub events: Option<async_channel::Sender<Call>>,
}

impl FakeClient {
    pub fn calls(&self) -> Vec<Call> {
        self.state.lock().expect("uncontended").calls.clone()
    }
}

#[async_trait]
impl PrewarmedSandboxClient for FakeClient {
    async fn fetch_configuration(
        &self,
        sandbox_id: &str,
    ) -> anyhow::Result<PrewarmedSandboxConfiguration> {
        panic!("Unexpected configuration fetch for {sandbox_id}")
    }

    async fn report_status(
        &self,
        sandbox_id: &str,
        generation: &str,
        state: PrewarmedSandboxState,
    ) -> anyhow::Result<()> {
        assert_eq!(sandbox_id, "sandbox");
        assert_eq!(generation, "abc123");
        let (fail_status, status_release) = {
            let mut client = self.state.lock().expect("uncontended");
            client.calls.push(Call::Status(state));
            if let Some(events) = &self.events {
                events.try_send(Call::Status(state)).unwrap();
            }
            (client.fail_status, client.status_release.take())
        };
        if let Some(release) = status_release {
            release.await.unwrap();
        }
        if fail_status {
            anyhow::bail!("workload token acquisition failed");
        }
        Ok(())
    }

    async fn connect_control_stream(
        &self,
        sandbox_id: &str,
        _generation: &str,
    ) -> anyhow::Result<crate::server::server_api::prewarmed_sandbox::PrewarmedSandboxControlStream>
    {
        panic!("Unexpected control stream connection for {sandbox_id}")
    }
}

struct ClockState {
    now: DateTime<Utc>,
    timers: Vec<(DateTime<Utc>, oneshot::Sender<()>)>,
}

#[derive(Clone)]
pub(super) struct TestClock(Arc<Mutex<ClockState>>);

impl TestClock {
    pub fn new() -> Self {
        Self(Arc::new(Mutex::new(ClockState {
            now: Utc::now(),
            timers: Vec::new(),
        })))
    }

    pub fn now(&self) -> DateTime<Utc> {
        self.0.lock().unwrap().now
    }

    pub fn clock(&self) -> StandbyClock {
        let now = self.clone();
        let sleep = self.clone();
        StandbyClock {
            now: Arc::new(move || now.now()),
            sleep: Arc::new(move |duration| {
                let (tx, rx) = oneshot::channel();
                if duration.is_zero() {
                    tx.send(()).unwrap();
                } else {
                    let mut state = sleep.0.lock().unwrap();
                    let deadline = state.now + TimeDelta::from_std(duration).unwrap();
                    state.timers.push((deadline, tx));
                }
                async move {
                    let _ = rx.await;
                }
                .boxed()
            }),
        }
    }

    pub async fn advance(&self, seconds: i64) {
        yield_once().await;
        {
            let mut state = self.0.lock().unwrap();
            state.now += TimeDelta::seconds(seconds);
            let now = state.now;
            let timers = std::mem::take(&mut state.timers);
            for (deadline, timer) in timers {
                if deadline <= now {
                    let _ = timer.send(());
                } else if !timer.is_canceled() {
                    state.timers.push((deadline, timer));
                }
            }
        }
        yield_once().await;
    }
}

pub(super) async fn yield_once() {
    let mut yielded = false;
    future::poll_fn(|ctx| {
        if yielded {
            return std::task::Poll::Ready(());
        }
        yielded = true;
        ctx.waker().wake_by_ref();
        std::task::Poll::Pending
    })
    .await
}

pub(super) struct Hooks {
    pub calls: Mutex<Vec<Call>>,
    pub events: (async_channel::Sender<Call>, async_channel::Receiver<Call>),
    pub install_gate: Mutex<Option<oneshot::Receiver<()>>>,
}

impl Default for Hooks {
    fn default() -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            events: async_channel::unbounded(),
            install_gate: Mutex::new(None),
        }
    }
}

impl Hooks {
    pub fn record(&self, call: Call) {
        self.calls.lock().unwrap().push(call.clone());
        self.events.0.try_send(call).unwrap();
    }

    pub async fn expect(&self, call: Call) {
        assert_eq!(self.events.1.recv().await.unwrap(), call);
    }

    pub fn count(&self, call: Call) -> usize {
        self.calls
            .lock()
            .unwrap()
            .iter()
            .filter(|entry| **entry == call)
            .count()
    }
}

pub(super) struct TestServer {
    pub events: async_channel::Sender<PrewarmedSandboxControlEvent>,
    pub heartbeats: async_channel::Receiver<String>,
}

impl TestServer {
    pub fn send(&self, event: PrewarmedSandboxControlEvent) {
        self.events.try_send(event).unwrap();
    }

    pub async fn expect_heartbeat(&self, generation: &str) {
        assert_eq!(self.heartbeats.recv().await.unwrap(), generation);
    }
}

pub(super) fn connection() -> (ConnectionAttempt, TestServer) {
    let (event_tx, event_rx) = async_channel::unbounded();
    let (heartbeat_tx, heartbeat_rx) = async_channel::unbounded();
    let heartbeats = sink::unfold(heartbeat_tx, |tx, generation: String| async move {
        tx.send(generation).await.unwrap();
        Ok::<_, anyhow::Error>(tx)
    });
    (
        ConnectionAttempt::Connected(ControlConnection {
            heartbeats: Box::pin(heartbeats),
            events: event_rx.map(Ok).boxed(),
        }),
        TestServer {
            events: event_tx,
            heartbeats: heartbeat_rx,
        },
    )
}

pub(super) fn execution_config() -> ExecutionConfiguration {
    ExecutionConfiguration {
        task_id: "run-id".into(),
        execution_id: "42".into(),
        conversation_id: None,
        parent_run_id: None,
        team_id: None,
        harness: warp_graphql::ai::AgentHarness::Oz,
        model_id: None,
        reasoning_level: None,
        profile_id: None,
        mcp_servers_json: "{}".into(),
        skills: Vec::new(),
        factory_skill_dirs: Vec::new(),
        computer_use_enabled: false,
        computer_use_model_id: None,
        inference_providers: None,
        repositories: Vec::new(),
        deferred_repositories: Some(Vec::new()),
        setup_commands: Vec::new(),
        providers: None,
        session_sharing_acls: Vec::new(),
        skip_initial_turn: false,
        idle_on_complete_seconds: None,
        idle_on_fail_seconds: None,
        snapshot_disabled: true,
    }
}

pub(super) fn assignment(execution_id: &str, clock: &TestClock) -> PrewarmedSandboxControlEvent {
    PrewarmedSandboxControlEvent::Assignment {
        generation: "abc123".into(),
        execution_config: Box::new(ExecutionConfiguration {
            execution_id: execution_id.into(),
            ..execution_config()
        }),
        runtime_env: Default::default(),
        api_key: "test-run-key".into(),
        expires_at: clock.now() + TimeDelta::minutes(10),
    }
}

pub(super) async fn drive(
    preparation: impl Future<Output = anyhow::Result<usize>>,
    connections: async_channel::Receiver<ConnectionAttempt>,
    clock: &TestClock,
    hooks: &Hooks,
    standby_seconds: i64,
) -> anyhow::Result<()> {
    run_lifecycle(
        "abc123",
        clock.now() + TimeDelta::seconds(standby_seconds),
        preparation,
        || {
            async {
                let attempt = connections
                    .recv()
                    .await
                    .unwrap_or(ConnectionAttempt::Unavailable);
                if matches!(attempt, ConnectionAttempt::Connected(_)) {
                    hooks.record(Call::Connected);
                }
                attempt
            }
            .boxed()
        },
        |prepared, _assignment| {
            async move {
                assert_eq!(prepared, 7);
                hooks.record(Call::Install);
                let gate = hooks.install_gate.lock().unwrap().take();
                if let Some(gate) = gate {
                    gate.await.unwrap();
                }
                Ok(())
            }
            .boxed()
        },
        |()| {
            async move {
                hooks.record(Call::Run);
                Ok(())
            }
            .boxed()
        },
        &clock.clock(),
    )
    .await
}

pub(super) fn exercise(
    lifecycle: impl Future<Output = anyhow::Result<()>>,
    actions: impl Future<Output = ()>,
) -> anyhow::Result<()> {
    block_on(async {
        future::join(lifecycle, actions)
            .with_timeout(Duration::from_secs(5))
            .await
            .expect("lifecycle test stalled")
            .0
    })
}
