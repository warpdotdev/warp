use std::future::Future;

use futures::{StreamExt, stream};
use warpui::ViewHandle;

use super::*;
use crate::ai::ambient_agents::AmbientAgentTask;
use crate::ai::ambient_agents::discovery::{
    ChildRunBatch, is_native_local_run, list_local_subtree,
};
use crate::ai::blocklist::orchestration_event_streamer::{
    LocalChildRestorationTarget, OrchestrationEventStreamer,
};
use crate::pane_group::PaneGroup;
use crate::server::retry_strategies::is_transient_http_error;

const LOCAL_CHILD_SETUP_CONCURRENCY: usize = 4;

async fn prepare_local_subtree_conversations<F, Fut>(
    root_task_id: AmbientAgentTaskId,
    root_conversation_id: AIConversationId,
    batches: &[ChildRunBatch],
    mut prepare_child: F,
) -> Result<HashMap<AmbientAgentTaskId, AIConversationId>, AgentDriverError>
where
    F: FnMut(AmbientAgentTaskId, AIConversationId, AmbientAgentTask) -> Fut,
    Fut: Future<Output = Result<AIConversationId, AgentDriverError>>,
{
    let mut conversations = HashMap::from([(root_task_id, root_conversation_id)]);
    for batch in batches {
        let parent_conversation_id = conversations[&batch.parent_task_id];
        // Descendant shells inherit their parent's startup directory, so finish each parent batch.
        let children = batch
            .children
            .iter()
            .filter(|task| is_native_local_run(task))
            .cloned()
            .collect::<Vec<_>>();
        let results = stream::iter(children)
            .map(|task| {
                let task_id = task.task_id;
                let setup = prepare_child(batch.parent_task_id, parent_conversation_id, task);
                async move { setup.await.map(|child_id| (task_id, child_id)) }
            })
            .buffer_unordered(LOCAL_CHILD_SETUP_CONCURRENCY)
            .collect::<Vec<_>>()
            .await;
        for result in results {
            let (task_id, child_id) = result?;
            conversations.insert(task_id, child_id);
        }
    }
    Ok(conversations)
}

impl AgentDriver {
    pub(super) async fn prepare_local_subtree(
        foreground: &ModelSpawner<Self>,
    ) -> Result<(), AgentDriverError> {
        let Some((parent_id, parent_conversation_id, terminal_id, group, client)) = foreground
            .spawn(|me, ctx| {
                if !FeatureFlag::RestoreLocalSubtree.is_enabled() {
                    return None;
                }
                let parent_id = me.task_id?;
                let conversation_id = me.restored_conversation_id?;
                let terminal = me.terminal_driver.as_ref(ctx);
                Some((
                    parent_id,
                    conversation_id,
                    terminal.terminal_view().id(),
                    terminal.pane_group().cloned(),
                    ServerApiProvider::as_ref(ctx).get_ai_client(),
                ))
            })
            .await?
        else {
            foreground
                .spawn(|me, ctx| {
                    if let Some(group) = me.terminal_driver.as_ref(ctx).pane_group().cloned() {
                        group.update(ctx, |group, ctx| group.release_passive_child_seeding(ctx));
                    }
                })
                .await?;
            return Ok(());
        };
        let group = group.ok_or(AgentDriverError::TerminalUnavailable)?;
        let mut attempt = 0;
        let batches = loop {
            match list_local_subtree(client.as_ref(), parent_id).await {
                Ok(batches) => break batches,
                Err(error) if is_transient_http_error(&error) && attempt < 4 => {
                    attempt += 1;
                    log::warn!(
                        "Child discovery for {parent_id} failed (attempt {attempt}): {error:#}"
                    );
                    Timer::after(Duration::from_secs(attempt)).await;
                }
                Err(error) => {
                    return Err(AgentDriverError::EnvironmentSetupFailed(format!(
                        "Restoring children of {parent_id}: {error:#}"
                    )));
                }
            }
        };
        let conversations = prepare_local_subtree_conversations(
            parent_id,
            parent_conversation_id,
            &batches,
            |parent_id, parent_conversation_id, task| {
                Self::prepare_local_child(
                    foreground,
                    &group,
                    terminal_id,
                    parent_id,
                    parent_conversation_id,
                    task,
                )
            },
        )
        .await?;
        let local_descendant_conversation_ids = batches
            .iter()
            .flat_map(|batch| &batch.children)
            .filter(|task| is_native_local_run(task))
            .map(|task| conversations[&task.task_id])
            .collect::<Vec<_>>();
        for batch in batches {
            let parent_conversation_id = conversations[&batch.parent_task_id];
            foreground
                .spawn({
                    let group = group.clone();
                    move |_, ctx| {
                        OrchestrationEventStreamer::handle(ctx).update(ctx, |streamer, ctx| {
                            for task in &batch.children {
                                streamer.register_watched_run_id(
                                    parent_conversation_id,
                                    task.task_id.to_string(),
                                    ctx,
                                );
                            }
                        });
                        group.update(ctx, |group, ctx| {
                            group.seed_passive_children(
                                parent_conversation_id,
                                terminal_id,
                                &batch.children,
                                ctx,
                            )
                        });
                    }
                })
                .await?;
        }
        foreground
            .spawn(move |_, ctx| {
                OrchestrationEventStreamer::handle(ctx).update(ctx, |streamer, ctx| {
                    streamer.release_local_subtree_startup(&local_descendant_conversation_ids, ctx);
                });
                group.update(ctx, |group, ctx| group.release_passive_child_seeding(ctx));
            })
            .await?;
        Ok(())
    }

    async fn prepare_local_child(
        foreground: &ModelSpawner<Self>,
        group: &ViewHandle<PaneGroup>,
        terminal_id: EntityId,
        parent_id: AmbientAgentTaskId,
        parent_conversation_id: AIConversationId,
        task: AmbientAgentTask,
    ) -> Result<AIConversationId, AgentDriverError> {
        let pane_group_id = group.id();
        let (child_id, terminal, directory) = foreground
            .spawn({
                let group = group.clone();
                move |me, ctx| {
                    let mut env_vars = (*me.resolved_env_vars).clone();
                    env_vars.extend(task_env_vars(
                        Some(&task.task_id),
                        Some(&parent_id.to_string()),
                        Harness::Oz,
                    ));
                    let (child_id, terminal) = group.update(ctx, |group, ctx| {
                        group.prepare_restored_local_child(
                            parent_conversation_id,
                            terminal_id,
                            &task,
                            env_vars,
                            ctx,
                        )
                    });
                    (
                        child_id,
                        terminal,
                        me.harness_working_dir.to_string_lossy().into_owned(),
                    )
                }
            })
            .await?;
        let result = match terminal {
            Ok(None) => return Ok(child_id),
            Ok(Some(terminal)) => {
                let terminal_for_bootstrap = terminal.clone();
                let (driver, bootstrap) = foreground
                    .spawn(move |_, ctx| {
                        let driver = terminal::TerminalDriver::create_from_existing_view(
                            terminal_for_bootstrap,
                            ctx,
                        );
                        let bootstrap =
                            driver.update(ctx, |driver, _| driver.wait_for_session_bootstrapped());
                        (driver, bootstrap)
                    })
                    .await?;
                match bootstrap.await {
                    Ok(()) => {
                        let cd = foreground
                            .spawn(move |_, ctx| {
                                driver.update(ctx, |driver, ctx| driver.cd_silent(&directory, ctx))
                            })
                            .await?;
                        match cd.await {
                            Ok(output)
                                if output.exit_code.is_some_and(|code| code.was_successful()) =>
                            {
                                Ok(terminal)
                            }
                            Ok(output) => Err(anyhow!(
                                "owner shell directory setup failed: {:?}",
                                output.exit_code
                            )),
                            Err(error) => Err(anyhow!(error)),
                        }
                    }
                    Err(error) => Err(anyhow!("owner shell bootstrap: {error}")),
                }
            }
            Err(error) => Err(error),
        };
        foreground
            .spawn(move |_, ctx| {
                OrchestrationEventStreamer::handle(ctx).update(ctx, |streamer, ctx| match result {
                    Ok(terminal) => streamer.register_restored_local_child(
                        child_id,
                        LocalChildRestorationTarget {
                            pane_group_id,
                            terminal_id: terminal.id(),
                        },
                        ctx,
                    ),
                    Err(error) => {
                        streamer.fail_local_child_restoration(child_id, format!("{error:#}"), ctx)
                    }
                });
            })
            .await?;
        Ok(child_id)
    }
}

#[cfg(test)]
#[path = "restored_children_tests.rs"]
mod tests;
