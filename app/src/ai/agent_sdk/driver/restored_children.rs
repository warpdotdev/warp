use super::*;
use crate::ai::ambient_agents::discovery::{is_native_local_run, list_local_subtree};
use crate::ai::blocklist::orchestration_event_streamer::{
    LocalChildRestorationTarget, OrchestrationEventStreamer,
};
use crate::server::retry_strategies::is_transient_http_error;

impl AgentDriver {
    pub(super) async fn prepare_local_subtree(
        foreground: &ModelSpawner<Self>,
    ) -> Result<(), AgentDriverError> {
        let Some((parent_id, parent_conversation_id, terminal_id, group, client, env, working_dir)) =
            foreground
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
                        me.resolved_env_vars.clone(),
                        me.harness_working_dir.clone(),
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
        let pane_group_id = group.id();
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
        let mut conversations = HashMap::from([(parent_id, parent_conversation_id)]);
        let mut local_descendant_conversation_ids = Vec::new();
        for batch in batches {
            let parent_id = batch.parent_task_id;
            let parent_conversation_id = conversations[&parent_id];
            for task in batch
                .children
                .iter()
                .filter(|task| is_native_local_run(task))
            {
                let task = task.clone();
                let task_id = task.task_id;
                let group = group.clone();
                let env = env.clone();
                let (child_id, terminal) = foreground
                    .spawn(move |_, ctx| {
                        let mut env_vars = (*env).clone();
                        env_vars.extend(task_env_vars(
                            Some(&task.task_id),
                            Some(&parent_id.to_string()),
                            Harness::Oz,
                        ));
                        group.update(ctx, |group, ctx| {
                            group.prepare_restored_local_child(
                                parent_conversation_id,
                                terminal_id,
                                &task,
                                env_vars,
                                ctx,
                            )
                        })
                    })
                    .await?;
                conversations.insert(task_id, child_id);
                local_descendant_conversation_ids.push(child_id);
                let result = match terminal {
                    Ok(None) => continue,
                    Ok(Some(terminal)) => {
                        let terminal_for_bootstrap = terminal.clone();
                        let (driver, bootstrap) = foreground
                            .spawn(move |_, ctx| {
                                let driver = terminal::TerminalDriver::create_from_existing_view(
                                    terminal_for_bootstrap,
                                    ctx,
                                );
                                let bootstrap = driver.update(ctx, |driver, _| {
                                    driver.wait_for_session_bootstrapped()
                                });
                                (driver, bootstrap)
                            })
                            .await?;
                        match bootstrap.await {
                            Ok(()) => {
                                let directory = working_dir.to_string_lossy().into_owned();
                                let cd = foreground
                                    .spawn(move |_, ctx| {
                                        driver.update(ctx, |driver, ctx| {
                                            driver.cd_silent(&directory, ctx)
                                        })
                                    })
                                    .await?;
                                match cd.await {
                                    Ok(output)
                                        if output
                                            .exit_code
                                            .is_some_and(|code| code.was_successful()) =>
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
                        OrchestrationEventStreamer::handle(ctx).update(ctx, |streamer, ctx| {
                            match result {
                                Ok(terminal) => streamer.register_restored_local_child(
                                    child_id,
                                    LocalChildRestorationTarget {
                                        pane_group_id,
                                        terminal_id: terminal.id(),
                                    },
                                    ctx,
                                ),
                                Err(error) => streamer.fail_local_child_restoration(
                                    child_id,
                                    format!("{error:#}"),
                                    ctx,
                                ),
                            }
                        });
                    })
                    .await?;
            }
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
}
