use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use async_trait::async_trait;
use futures::channel::oneshot;
use tempfile::TempDir;
use warpui::r#async::FutureExt as _;
use warpui::{App, ModelSpawner, SingletonEntity as _};

use super::{HarnessPersistence, HarnessRunner, PersistenceOutcome, SavePoint};
use crate::ai::agent_sdk::driver::terminal::{CommandHandle, TerminalDriver};
use crate::ai::agent_sdk::driver::{AgentDriver, AgentDriverError, IdleTimeoutSender};
use crate::ai::agent_sdk::setup_observability::SetupClientEventReporter;
use crate::terminal::CLIAgent;
use crate::terminal::cli_agent_sessions::event::parse_event;
use crate::terminal::cli_agent_sessions::{
    CLIAgentInputState, CLIAgentSession, CLIAgentSessionContext, CLIAgentSessionStatus,
    CLIAgentSessionsModel,
};
use crate::test_util::terminal::{add_window_with_terminal, initialize_app_for_terminal_view};

#[derive(Debug, PartialEq)]
enum Request {
    Refresh,
    Save(SavePoint),
}

struct RecordingRunner {
    requests: async_channel::Sender<Request>,
    persistence: HarnessPersistence,
}

#[cfg_attr(not(target_family = "wasm"), async_trait)]
#[cfg_attr(target_family = "wasm", async_trait(?Send))]
impl HarnessRunner for RecordingRunner {
    fn harness_name(&self) -> &str {
        "recording"
    }

    async fn start(
        &self,
        _: &ModelSpawner<AgentDriver>,
        _: &SetupClientEventReporter,
    ) -> Result<CommandHandle, AgentDriverError> {
        unreachable!()
    }

    async fn exit(&self, _: &ModelSpawner<AgentDriver>) -> Result<()> {
        unreachable!()
    }

    fn persistence(&self) -> &HarnessPersistence {
        &self.persistence
    }

    async fn save_conversation(
        &self,
        _: SavePoint,
        _: &ModelSpawner<AgentDriver>,
    ) -> PersistenceOutcome {
        unreachable!()
    }

    async fn enqueue_save(
        self: Arc<Self>,
        point: SavePoint,
        _: &ModelSpawner<AgentDriver>,
    ) -> Result<()> {
        self.requests.send(Request::Save(point)).await?;
        Ok(())
    }

    async fn handle_session_update(&self, _: &ModelSpawner<AgentDriver>) -> Result<()> {
        self.requests.send(Request::Refresh).await?;
        Ok(())
    }
}

#[test]
fn cli_activity_refreshes_metadata_without_saving_until_turn_completion() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);
        let terminal_view_id = terminal.id();
        let temp = TempDir::new().unwrap();
        let (record, requests) = async_channel::unbounded();
        let (exit, _exited) = oneshot::channel();
        let driver = app.add_model(|ctx| {
            let terminal_driver = TerminalDriver::create_from_existing_view(terminal, ctx);
            let mut driver =
                AgentDriver::new_for_test(temp.path().to_path_buf(), terminal_driver, ctx);
            driver.harness = Some(Arc::new(RecordingRunner {
                requests: record,
                persistence: HarnessPersistence::default(),
            }));
            driver.subscribe_to_cli_agent_session_events(IdleTimeoutSender::new(exit), ctx);
            driver
        });
        let sessions = CLIAgentSessionsModel::handle(&app);
        sessions.update(&mut app, |model, ctx| {
            model.set_session(
                terminal_view_id,
                CLIAgentSession {
                    agent: CLIAgent::Claude,
                    status: CLIAgentSessionStatus::InProgress,
                    session_context: CLIAgentSessionContext::default(),
                    input_state: CLIAgentInputState::Closed,
                    should_auto_toggle_input: false,
                    listener: None,
                    plugin_version: None,
                    remote_host: None,
                    draft_text: None,
                    custom_command_prefix: None,
                    received_rich_notification: false,
                },
                ctx,
            );
        });

        let emit = |body, app: &mut App| {
            let event = parse_event(Some("warp://cli-agent"), body).unwrap();
            sessions.update(app, |model, ctx| {
                model.update_from_event(terminal_view_id, &event, ctx);
            });
        };
        emit(
            r#"{"agent":"claude","event":"session_start","session_id":"native"}"#,
            &mut app,
        );
        assert_eq!(requests.recv().await.unwrap(), Request::Refresh);
        emit(
            r#"{"agent":"claude","event":"prompt_submit","query":"work"}"#,
            &mut app,
        );
        assert_eq!(requests.recv().await.unwrap(), Request::Refresh);
        emit(
            r#"{"agent":"claude","event":"permission_request"}"#,
            &mut app,
        );
        emit(r#"{"agent":"claude","event":"tool_complete"}"#, &mut app);
        assert_eq!(requests.recv().await.unwrap(), Request::Refresh);
        assert!(requests.is_empty());
        assert_eq!(
            sessions.read(&app, |model, _| model
                .session(terminal_view_id)
                .unwrap()
                .session_context
                .session_id
                .clone()),
            Some("native".to_owned()),
        );

        emit(r#"{"agent":"claude","event":"stop"}"#, &mut app);
        assert_eq!(
            requests
                .recv()
                .with_timeout(Duration::from_secs(5))
                .await
                .unwrap()
                .unwrap(),
            Request::Save(SavePoint::PostTurn),
        );
        drop(driver);
    });
}
