use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::SystemTime;

use ::preview_proxy::protocol::{self, ControlPlane, Error, Session, StartRequest};
use anyhow::Context as _;
use tokio_util::sync::CancellationToken;
use warp_server_client::HttpStatusError;

use crate::ai::ambient_agents::AmbientAgentTaskId;
use crate::server::server_api::ai::AIClient;

pub(super) struct Manager {
    cancel: CancellationToken,
    thread: Option<JoinHandle<()>>,
}

impl Manager {
    pub(super) fn start(
        task_id: AmbientAgentTaskId,
        client: Arc<dyn AIClient>,
        sandbox_deadline: Option<SystemTime>,
    ) -> anyhow::Result<Self> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .context("Failed to create preview proxy runtime")?;
        let cancel = CancellationToken::new();
        let task_cancel = cancel.clone();
        let thread = std::thread::Builder::new()
            .name("preview-proxy".into())
            .spawn(move || {
                runtime.block_on(async move {
                    let deadline = sandbox_deadline.map(|deadline| {
                        tokio::time::Instant::now()
                            + deadline
                                .duration_since(SystemTime::now())
                                .unwrap_or_default()
                    });
                    let control = Client { task_id, client };
                    let manager = protocol::run(
                        control,
                        uuid::Uuid::new_v4().to_string(),
                        format!(
                            "{}{}",
                            uuid::Uuid::new_v4().simple(),
                            uuid::Uuid::new_v4().simple()
                        ),
                        deadline,
                        task_cancel.clone(),
                    );
                    tokio::pin!(manager);
                    match deadline {
                        Some(deadline) => tokio::select! {
                            _ = &mut manager => {},
                            _ = tokio::time::sleep_until(deadline) => {
                                task_cancel.cancel();
                                manager.await;
                            }
                        },
                        None => manager.await,
                    }
                });
            })
            .context("Failed to start preview proxy thread")?;
        Ok(Self {
            cancel,
            thread: Some(thread),
        })
    }
}

impl Drop for Manager {
    fn drop(&mut self) {
        self.cancel.cancel();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

struct Client {
    task_id: AmbientAgentTaskId,
    client: Arc<dyn AIClient>,
}

fn classify(error: anyhow::Error) -> Error {
    if error
        .downcast_ref::<HttpStatusError>()
        .is_some_and(|error| matches!(error.status, 401 | 403 | 404 | 410))
    {
        Error::Fenced
    } else {
        Error::Transport
    }
}

impl ControlPlane for Client {
    async fn start(&self, request: &StartRequest) -> Result<Session, Error> {
        self.client
            .start_preview_proxy(&self.task_id, request)
            .await
            .map_err(classify)
    }
}
