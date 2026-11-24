use std::sync::Arc;
use std::time::Duration;

use async_channel::{Receiver, Sender};
use session_sharing_protocol::common::ParticipantId;
use warpui::r#async::Timer;
use warpui::r#async::executor::Background;

use crate::ai::agent::BaseUserQuery;
use crate::ai::ambient_agents::AmbientAgentTaskId;
use crate::ai::blocklist::ExternalHarnessPrompt;
use crate::server::server_api::ServerApi;
use crate::server::server_api::ai::UndeliveredAgentMessage;

const RETRY_DELAY: Duration = Duration::from_secs(1);

pub(super) fn connect(
    server: Arc<ServerApi>,
    task_id: AmbientAgentTaskId,
    incoming: Receiver<ExternalHarnessPrompt>,
    opening_context: Option<String>,
    background: &Arc<Background>,
) -> (Receiver<ExternalHarnessPrompt>, Receiver<()>) {
    // The turn driver consumes ordinary followups and hydrated inbox batches from one prompt stream.
    let (prompts, receiver) = async_channel::unbounded();
    // Startup must not idle-exit before the first inbox lookup is empty or has queued a prompt.
    let (ready, startup_ready) = async_channel::bounded(1);
    // Wake queries carry no content; one pending signal covers any number of new messages.
    let (wakes, wake_receiver) = async_channel::bounded(1);
    let _ = wakes.try_send(());
    let forwarded = prompts.clone();
    background
        .spawn(async move {
            while let Ok(prompt) = incoming.recv().await {
                if prompt
                    .base
                    .as_ref()
                    .is_some_and(BaseUserQuery::is_agent_message_wake)
                {
                    let _ = wakes.try_send(());
                } else if forwarded.send(prompt).await.is_err() {
                    break;
                }
            }
        })
        .detach();
    background
        .spawn(drain(
            server,
            task_id,
            wake_receiver,
            prompts,
            opening_context,
            ready,
        ))
        .detach();
    (receiver, startup_ready)
}

async fn drain(
    server: Arc<ServerApi>,
    task_id: AmbientAgentTaskId,
    wakes: Receiver<()>,
    prompts: Sender<ExternalHarnessPrompt>,
    mut opening_context: Option<String>,
    startup_ready: Sender<()>,
) {
    while wakes.recv().await.is_ok() {
        loop {
            if prompts.is_closed() {
                return;
            }
            let messages = match server
                .list_undelivered_agent_messages_for_task(&task_id)
                .await
            {
                Ok(messages) if messages.is_empty() => {
                    log::info!("Agent inbox drain found no undelivered messages");
                    let _ = startup_ready.try_send(());
                    break;
                }
                Ok(messages) => messages,
                Err(error) => {
                    log::warn!("Agent inbox fetch failed; retrying: {error:#}");
                    Timer::after(RETRY_DELAY).await;
                    continue;
                }
            };
            let text = format_agent_messages(&messages);
            let text = match &opening_context {
                Some(context) if !context.is_empty() => format!("{context}\n\n{text}"),
                Some(_) | None => text,
            };
            let (accepted, receipt) = async_channel::bounded(1);
            let prompt = ExternalHarnessPrompt {
                text,
                attachments: Vec::new(),
                participant_id: ParticipantId::new(),
                base: Some(BaseUserQuery::unattributed("agent_inbox_delivery")),
                accepted: Some(accepted),
            };
            if prompts.send(prompt).await.is_err() {
                return;
            }
            let _ = startup_ready.try_send(());
            // ACP has no prompt registration ACK. A successful response, or an injected steer,
            // proves acceptance; a dropped unsent prompt closes the receipt without accepting it.
            if receipt.recv().await.is_err() {
                Timer::after(RETRY_DELAY).await;
                continue;
            }
            opening_context = None;
            for message in messages {
                loop {
                    match server
                        .mark_message_delivered_for_task(&task_id, &message.message_id)
                        .await
                    {
                        Ok(()) => break,
                        Err(error) => {
                            log::warn!("Agent inbox acknowledgement failed; retrying: {error:#}");
                            if prompts.is_closed() {
                                return;
                            }
                            Timer::after(RETRY_DELAY).await;
                        }
                    }
                }
            }
        }
    }
}

fn format_agent_messages(messages: &[UndeliveredAgentMessage]) -> String {
    let messages = serde_json::to_string(messages).expect("agent messages serialize as JSON");
    format!(
        "You have received new agent messages:\n\
         [Agent messages]\n\
         {messages}\n\
         [/Agent messages]"
    )
}

#[cfg(test)]
#[path = "inbox_tests.rs"]
mod tests;
