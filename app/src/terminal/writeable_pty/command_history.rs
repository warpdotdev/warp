use std::sync::mpsc::SyncSender;
use std::sync::Arc;

use parking_lot::FairMutex;
use warpui::{AppContext, ModelHandle, SingletonEntity};

use crate::persistence::{ModelEvent, StartedCommandMetadata};
use crate::terminal::model::session::Sessions;
use crate::terminal::view::ExecuteCommandEvent;
use crate::terminal::{History, HistoryEntry, TerminalModel};

pub fn update_command_history(
    event: &ExecuteCommandEvent,
    model: &Arc<FairMutex<TerminalModel>>,
    model_event_sender: Option<&SyncSender<ModelEvent>>,
    sessions: &ModelHandle<Sessions>,
    ctx: &mut AppContext,
) {
    let model = model.lock();
    let active_block = model.block_list().active_block();
    let session_id = event.session_id;
    let Some(session) = sessions.as_ref(ctx).get(session_id) else {
        return;
    };

    let shell = session.shell();
    if !shell.should_add_command_to_history(&event.command) {
        return;
    }

    let is_agent_executed = event.source.is_ai_command();

    let session_ref = &*session;
    History::handle(ctx).update(ctx, move |history, _| {
        history.append_commands(
            session_id,
            vec![HistoryEntry::for_session_command(
                event.command.to_string(),
                active_block,
                session_ref,
                event.workflow_id.to_owned(),
                event.workflow_command.to_owned(),
                is_agent_executed,
            )],
        )
    });

    if let Some(sender) = model_event_sender {
        let sender_clone = sender.clone();
        let insert_command_event = ModelEvent::InsertCommand {
            metadata: StartedCommandMetadata {
                command: event.command.to_owned(),
                start_ts: active_block.start_ts().copied(),
                pwd: active_block.pwd().map(|pwd| pwd.to_owned()),
                shell: Some(session.shell().shell_type().name().to_owned()),
                username: Some(session.user().to_owned()),
                hostname: Some(session.hostname().to_owned()),
                session_id: Some(session_id),
                cloud_workflow_id: event.workflow_id.to_owned(),
                workflow_command: event.workflow_command.to_owned(),
                git_branch: active_block
                    .git_branch()
                    .map(|git_branch| git_branch.to_owned()),
                is_agent_executed,
            },
        };
        ctx.background_executor()
            .spawn(async move {
                // Sending over a sync sender can block the current thread, so we do this async.
                if let Err(e) = sender_clone.send(insert_command_event) {
                    // The persistence receiver is gone (e.g. on shutdown); expected teardown.
                    log::warn!("Error sending ModelEvent: {e}");
                }
            })
            .detach();
    }
}
