use clap::ValueEnum as _;
use warp_cli::skill::SkillSpec;
use warp_graphql::queries::execution_config::{
    AwsBedrockInferenceClientConfig, ExecutionConfiguration,
};

use super::*;
use crate::ai::agent_sdk::driver::agent_setup::configured_skills::resolve_configured_skills;
use crate::ai::agent_sdk::{
    AgentDriverRunner, FACTORY_ID_ENV, RepositoryContract, build_execution_task_and_options,
    factory_repository_contract,
};
use crate::ai::mcp::TemplatableMCPServerInstallation;
use crate::server::server_api::ServerApi;
use crate::terminal::view::load_ai_conversation::{
    RestoreConversationEntryBehavior, RestoredAIConversation,
};

struct AssignmentInstallationContext {
    team_scope: HeadlessTeamScope,
    share_requests: Vec<ShareRequest>,
    bedrock: Option<AwsBedrockInferenceClientConfig>,
    specs: Vec<SkillSpec>,
    setup_events: SetupClientEventReporter,
    server_api: Arc<ServerApi>,
    working_dir: PathBuf,
    assigned_mode: GitHubCredentialMode,
    use_factory_repositories: bool,
    retained_mcp: Vec<TemplatableMCPServerInstallation>,
}

fn validate_assignment(config: &ExecutionConfiguration) -> anyhow::Result<()> {
    anyhow::ensure!(
        FeatureFlag::FactoryDeferredRepositories.is_enabled()
            || config
                .deferred_repositories
                .as_ref()
                .is_none_or(Vec::is_empty),
        "Pre-warmed assignment has deferred repositories this client cannot prepare"
    );
    Ok(())
}

fn assignment_harness_transport(
    runtime_env: &HashMap<String, String>,
) -> anyhow::Result<HarnessTransport> {
    runtime_env
        .get("WARP_HARNESS_TRANSPORT")
        .map(|value| HarnessTransport::from_str(value, false).map_err(anyhow::Error::msg))
        .transpose()
        .map(|transport| transport.unwrap_or_default())
}

impl AgentDriver {
    pub(crate) async fn report_prewarmed_lifecycle_failure(
        spawner: &ModelSpawner<Self>,
        message: String,
    ) {
        if let Ok(Some((task_id, client))) = spawner
            .spawn(|me, ctx| {
                me.task_id
                    .map(|task_id| (task_id, ServerApiProvider::as_ref(ctx).get_ai_client()))
            })
            .await
        {
            report_driver_error(
                task_id,
                &AgentDriverError::ConfigBuildFailed(anyhow!(message)),
                &client,
            )
            .await;
        }
    }

    /// Restarts the agent's shell so it inherits the environment resolved for the assignment.
    async fn replace_shell_with_resolved_env(
        spawner: &ModelSpawner<Self>,
    ) -> Result<(), AgentDriverError> {
        spawner
            .spawn(move |me, ctx| {
                let working_dir = me.harness_working_dir.clone();
                let env_vars = HashMap::clone(&me.resolved_env_vars);
                me.terminal_driver.update(ctx, |driver, ctx| {
                    driver.replace_shell(working_dir, env_vars, ctx)
                })
            })
            .await?
            .await
            .map_err(|error| AgentDriverError::BootstrapFailed { error })
    }

    /// Installs a committed assignment on this standby driver and returns the task to run.
    ///
    /// The server commits the run before delivering it, so the sandbox cannot hand it back and
    /// every failure here must be reported against the run. That report needs the run's own
    /// credential, so the credential is installed before anything else can fail.
    pub(crate) async fn prepare_prewarmed_assignment(
        spawner: &ModelSpawner<Self>,
        config: ExecutionConfiguration,
        runtime_env: HashMap<String, String>,
        api_key: String,
        expires_at: DateTime<Utc>,
    ) -> Result<Task, AgentDriverError> {
        let task_id = config
            .task_id
            .inner()
            .parse::<AmbientAgentTaskId>()
            .map_err(|error| AgentDriverError::ConfigBuildFailed(anyhow!(error)))?;
        let run_key = api_key.clone();
        let ai_client = spawner
            .spawn(move |_, ctx| {
                AuthStateProvider::as_ref(ctx)
                    .get()
                    .set_credentials(Some(Credentials::ApiKey {
                        key: run_key,
                        owner_type: None,
                    }));
                ServerApiProvider::as_ref(ctx).get_ai_client().clone()
            })
            .await?;
        AgentDriverRunner::set_ambient_agent_task_id(spawner, Some(task_id)).await?;
        spawner
            .spawn(move |me, _| me.task_id = Some(task_id))
            .await?;
        let result = Self::install_assignment(
            spawner,
            task_id,
            config,
            runtime_env,
            api_key,
            expires_at,
            &ai_client,
        )
        .await;
        if let Err(error) = &result {
            report_driver_error(task_id, error, &ai_client).await;
        }
        result
    }

    async fn install_assignment(
        spawner: &ModelSpawner<Self>,
        task_id: AmbientAgentTaskId,
        mut config: ExecutionConfiguration,
        mut runtime_env: HashMap<String, String>,
        api_key: String,
        expires_at: DateTime<Utc>,
        ai_client: &Arc<dyn AIClient>,
    ) -> Result<Task, AgentDriverError> {
        #[cfg(not(target_family = "wasm"))]
        {
            runtime_env = spawner
                .spawn(move |_, ctx| {
                    let client = ServerApiProvider::as_ref(ctx).get_managed_secrets_client();
                    crate::tracing::assign_run_credentials(&mut runtime_env, client, ctx);
                    runtime_env
                })
                .await?;
        }
        #[cfg(target_family = "wasm")]
        runtime_env.remove("WARP_CLOUD_AGENT_OTLP_TOKEN");
        let assigned_mode = git_credentials::credential_mode(
            runtime_env
                .get("WARP_GITHUB_CREDENTIAL_MODE")
                .map(String::as_str),
        )
        .map_err(AgentDriverError::ConfigBuildFailed)?;
        let (prepared_mode, standby_owner) = spawner
            .spawn(|me, _| (me.github_credential_mode, me.credential_store_owner.clone()))
            .await?;
        if assigned_mode != prepared_mode {
            return Err(AgentDriverError::ConfigBuildFailed(anyhow!(
                "Assigned GitHub credential mode does not match preparation"
            )));
        }
        validate_assignment(&config).map_err(AgentDriverError::ConfigBuildFailed)?;
        let retained_mcp = Self::stop_prewarmed_mcp_servers(spawner, HashMap::new()).await?;
        if let Some(owner) = standby_owner {
            git_credentials::close_restricted_credentials(&owner)
                .map_err(AgentDriverError::ConfigBuildFailed)?;
        }
        if assigned_mode == GitHubCredentialMode::Legacy {
            git_credentials::clear_standby_credentials()
                .map_err(AgentDriverError::ConfigBuildFailed)?;
        } else {
            git_credentials::clear_standby_provider_credentials()
                .map_err(AgentDriverError::ConfigBuildFailed)?;
        }
        let standby_providers = spawner
            .spawn(|me, _| std::mem::take(&mut me.cloud_providers))
            .await?;
        for provider in standby_providers {
            provider
                .cleanup()
                .await
                .map_err(AgentDriverError::CloudProviderSetupFailed)?;
        }
        validate_assignment(&config).map_err(AgentDriverError::ConfigBuildFailed)?;
        let team_scope = execution_config::team_scope(config.team_id.as_ref())
            .map_err(AgentDriverError::ConfigBuildFailed)?;
        let share_requests =
            execution_config::sharing_acls(std::mem::take(&mut config.session_sharing_acls))
                .map_err(AgentDriverError::ConfigBuildFailed)?;
        let bedrock = config
            .inference_providers
            .take()
            .and_then(|providers| providers.aws_bedrock);
        let specs = config
            .skills
            .iter()
            .map(|skill| {
                skill
                    .spec
                    .parse::<SkillSpec>()
                    .map_err(AgentDriverError::SkillResolutionFailed)
            })
            .collect::<Result<Vec<_>, _>>()?;
        runtime_env.insert("WARP_API_KEY".into(), api_key);
        runtime_env.insert(
            "WARP_SANDBOX_DEADLINE".into(),
            expires_at.timestamp().to_string(),
        );
        let use_factory_repositories = factory_repository_contract(
            Some(task_id),
            FeatureFlag::FactoryDeferredRepositories.is_enabled(),
            runtime_env.get(FACTORY_ID_ENV),
        );
        let reporter_client = ai_client.clone();
        let (server_api, working_dir, setup_events) = spawner
            .spawn(move |me, ctx| {
                let reporter = SetupClientEventReporter::new(
                    task_id,
                    reporter_client,
                    ctx.background_executor(),
                );
                (
                    ServerApiProvider::as_ref(ctx).get(),
                    me.working_dir.clone(),
                    reporter,
                )
            })
            .await?;
        let repositories = spawner
            .spawn(|me, _| {
                let Some(WorkspaceState::Prepared(prepared)) = me.workspace.as_ref() else {
                    return Err(AgentDriverError::InvalidRuntimeState);
                };
                Ok(prepared
                    .repositories
                    .iter()
                    .map(|repo| {
                        (
                            me.working_dir.join(&repo.repo),
                            repo.code_forge.map(|forge| forge.host()).unwrap_or(""),
                        )
                    })
                    .collect::<Vec<_>>())
            })
            .await??;
        if assigned_mode == GitHubCredentialMode::RestrictedInstallation {
            git_credentials::restricted::install_assignment_credentials(
                &task_id.to_string(),
                ai_client,
                use_factory_repositories,
                &repositories,
            )
            .await
            .map_err(AgentDriverError::GitCredentialsFetchFailed)?;
        } else {
            let credentials = AgentDriverRunner::fetch_task_git_credentials(
                task_id.to_string(),
                ai_client.clone(),
                use_factory_repositories,
            )
            .await
            .map_err(AgentDriverError::GitCredentialsFetchFailed)?;
            git_credentials::configure_assignment_git_credentials(&credentials, &repositories)
                .map_err(AgentDriverError::ConfigBuildFailed)?;
        }
        let installation = Self::finish_assignment(
            spawner,
            task_id,
            config,
            runtime_env,
            expires_at,
            ai_client,
            AssignmentInstallationContext {
                team_scope,
                share_requests,
                bedrock,
                specs,
                setup_events,
                server_api,
                working_dir,
                assigned_mode,
                use_factory_repositories,
                retained_mcp,
            },
        );
        if assigned_mode == GitHubCredentialMode::RestrictedInstallation {
            let owner = ManagedStoreOwner::Task(task_id.to_string());
            git_credentials::restricted::with_expiry(&owner, installation)
                .await
                .map_err(AgentDriverError::ConfigBuildFailed)?
        } else {
            installation.await
        }
    }

    async fn finish_assignment(
        spawner: &ModelSpawner<Self>,
        task_id: AmbientAgentTaskId,
        config: ExecutionConfiguration,
        runtime_env: HashMap<String, String>,
        expires_at: DateTime<Utc>,
        ai_client: &Arc<dyn AIClient>,
        installation: AssignmentInstallationContext,
    ) -> Result<Task, AgentDriverError> {
        let AssignmentInstallationContext {
            team_scope,
            share_requests,
            bedrock,
            specs,
            setup_events,
            server_api,
            working_dir,
            assigned_mode,
            use_factory_repositories,
            retained_mcp,
        } = installation;
        AgentDriverRunner::refresh_team_metadata(spawner, true).await?;
        let (first_skill, discovery_dirs) =
            resolve_configured_skills(spawner, specs, false, &working_dir, &setup_events).await?;
        let harness_transport = assignment_harness_transport(&runtime_env)
            .map_err(AgentDriverError::ConfigBuildFailed)?;
        let (mut options, mut task, conversation_id, providers) = spawner
            .spawn(move |me, ctx| {
                let (options, task, conversation_id, providers) = build_execution_task_and_options(
                    None,
                    harness_transport,
                    config,
                    RepositoryContract {
                        supports_deferred_repositories: FeatureFlag::FactoryDeferredRepositories
                            .is_enabled(),
                        use_factory_repositories,
                    },
                    working_dir,
                    first_skill,
                    discovery_dirs,
                    ctx,
                )?;
                let Some(WorkspaceState::Prepared(prepared)) = me.workspace.as_ref() else {
                    anyhow::bail!("Standby resources were not prepared");
                };
                anyhow::ensure!(
                    prepared.harness == options.selected_harness
                        && prepared.configuration.same_preparation(&options.workspace),
                    "Assignment workspace does not match prepared resources"
                );
                Ok::<_, anyhow::Error>((options, task, conversation_id, providers))
            })
            .await?
            .map_err(AgentDriverError::ConfigBuildFailed)?;
        if FeatureFlag::OzIdentityFederation.is_enabled() {
            options.cloud_providers =
                cloud_provider::load_providers(&providers, Some(&task_id.to_string()))
                    .map_err(AgentDriverError::CloudProviderSetupFailed)?;
        }
        options.team_scope = Some(team_scope);
        options.github_credential_mode = assigned_mode;
        // MCP servers get only the runtime environment, because in a normal run they inherit the
        // worker environment that it replaces and never the terminal's managed secrets.
        let mut mcp_environment = runtime_env.clone();
        let metadata = runtime_env.clone();
        spawner
            .spawn(move |me, _| {
                if let Some(vars) = me.setup_vars.as_mut() {
                    vars.write_assignment_metadata(&metadata).map_err(|_| {
                        AgentDriverError::EnvironmentSetupFailed(
                            "Could not write assignment metadata to .warp.vars".into(),
                        )
                    })?;
                }
                Ok::<_, AgentDriverError>(())
            })
            .await??;
        // The sandbox's runtime environment is the terminal environment for this assignment, so
        // the shared resolver layers managed secrets, task variables, and harness variables
        // underneath it exactly as it does for a normal run.
        options.environment_variables = runtime_env;
        let task_secrets = spawner
            .spawn(move |_, ctx| {
                ManagedSecretManager::as_ref(ctx).get_task_secrets(task_id.to_string())
            })
            .await?
            .await
            .map_err(AgentDriverError::SecretsFetchFailed)?;
        let attachments = if FeatureFlag::AmbientAgentsImageUpload.is_enabled() {
            ai_client.get_task_attachments(task_id.to_string()).await
        } else {
            Ok(Vec::new())
        };
        AgentDriverRunner::prepare_execution_task_data(
            server_api,
            ai_client,
            task_id,
            task_secrets,
            attachments,
            &mut options,
            &mut task,
        )
        .await?;
        // A run requested over ACP whose sandbox has no ACP adapter is driven over a PTY
        // instead, before the resume path and the run configuration read the transport.
        let transport_fallback = super::harness::apply_transport_fallback(
            &mut task.harness,
            &mut options.harness_transport,
        )?;
        if let Some(reason) = transport_fallback {
            let harness_name = task.harness.harness().to_string();
            log::info!(
                "No ACP adapter for the {harness_name} harness in this sandbox ({reason}); driving it over a PTY"
            );
            let _ = spawner
                .spawn(move |_, ctx| {
                    use warp_core::telemetry::TelemetryEvent as _;
                    let event =
                        super::harness::ThirdPartyHarnessTelemetryEvent::TransportFallback {
                            harness: harness_name,
                            reason: reason.as_str(),
                        };
                    crate::send_telemetry_from_app_ctx!(event, ctx);
                })
                .await;
        }
        options.harness_transport_fallback = transport_fallback;
        if let HarnessKind::ThirdParty(harness) = &task.harness {
            harness.validate()?;
        }
        // A harness whose transport is fixed (Pi) sends no report: the server rejects it.
        if task.harness.reports_transport() {
            let transport = options.harness_transport;
            spawner
                .spawn(move |_, ctx| {
                    crate::ai::agent_sdk::transport_report::report_transport_in_background(
                        &ctx.background_executor(),
                        ServerApiProvider::as_ref(ctx).get_harness_support_client(),
                        transport,
                        transport_fallback.map(super::harness::TransportFallbackReason::as_str),
                    );
                })
                .await?;
        }
        if let Some(conversation_id) = conversation_id {
            options.resume = setup_events
                .record_result(
                    SetupStep::ConversationResumeLoading,
                    AgentDriverRunner::load_conversation_information(
                        spawner,
                        conversation_id,
                        &task.harness,
                    ),
                )
                .await?;
        }
        #[cfg(not(target_family = "wasm"))]
        if let Some(bedrock) = bedrock {
            let region = bedrock.region.ok_or_else(|| {
                AgentDriverError::AwsBedrockCredentialsFailed("Bedrock region is required".into())
            })?;
            crate::ai::agent_sdk::configure_bedrock_oidc_credentials(
                spawner,
                &mut options,
                bedrock.role_arn,
                region,
            )
            .await?;
        }
        #[cfg(target_family = "wasm")]
        if bedrock.is_some() {
            return Err(AgentDriverError::InvalidRuntimeState);
        }
        let (terminal_inputs, run_config) = split_run_options(options)?;
        let should_share = terminal_inputs.should_share;
        let mcp_environment = spawner
            .spawn(move |me, ctx| {
                me.apply_prewarmed_run_configuration(terminal_inputs, run_config, ctx)?;
                me.merge_assignment_setup_vars(&mut mcp_environment);
                me.sandbox_deadline = Some(expires_at.into());
                Ok::<_, AgentDriverError>(mcp_environment)
            })
            .await??;
        Self::replace_shell_with_resolved_env(spawner).await?;
        if matches!(task.harness, HarnessKind::Oz) {
            Self::stop_prewarmed_mcp_servers(spawner, mcp_environment).await?;
            let retained_mcp = retained_mcp
                .into_iter()
                .filter(|installation| installation.warp_id().is_none())
                .collect::<Vec<_>>();
            let mcp_restart = Self::start_retained_mcp_servers(spawner, retained_mcp).await;
            Self::handle_mcp_startup_result(mcp_restart, spawner).await?;
            let managed_client = spawner
                .spawn(|_, ctx| ServerApiProvider::as_ref(ctx).get_managed_mcp_client())
                .await?;
            let startup = mcp_startup::start_task_and_profile_mcp_servers(
                &task.mcp_specs,
                managed_client,
                mcp_startup::McpStartupMode::Run,
                Some(task_id),
                spawner,
            )
            .await;
            Self::handle_mcp_startup_result(startup, spawner).await?;
        }
        spawner
            .spawn(move |me, ctx| {
                let scope = me
                    .team_scope
                    .as_ref()
                    .ok_or(AgentDriverError::InvalidRuntimeState)?;
                me.terminal_driver.update(ctx, |driver, ctx| {
                    driver.bind_prewarmed_task(
                        terminal::PrewarmedTaskBinding {
                            task_id,
                            working_dir: me.working_dir.clone(),
                            team_scope: scope,
                            share_requests,
                            should_share,
                            restored_conversation_id: me.restored_conversation_id,
                        },
                        ctx,
                    )
                })
            })
            .await??;
        spawner
            .spawn(|me, ctx| {
                me.terminal_driver
                    .update(ctx, |driver, _| driver.wait_for_session_shared())
            })
            .await?
            .await?;
        Ok(task)
    }

    fn apply_prewarmed_run_configuration(
        &mut self,
        terminal_inputs: TerminalLaunchInputs,
        config: RunConfiguration,
        ctx: &mut ModelContext<Self>,
    ) -> Result<(), AgentDriverError> {
        if let Some(restoration) = terminal_inputs.conversation_restoration {
            let use_live_appearance = restoration.should_use_live_appearance();
            let ConversationRestorationInNewPaneType::Historical { conversation, .. } = restoration
            else {
                return Err(AgentDriverError::InvalidRuntimeState);
            };
            self.terminal_driver.update(ctx, |driver, ctx| {
                driver.with_terminal_view(ctx, |terminal, ctx| {
                    terminal.restore_conversation_after_view_creation(
                        RestoredAIConversation::new(conversation),
                        use_live_appearance,
                        RestoreConversationEntryBehavior::EnterRestoredConversation,
                        ctx,
                    );
                });
            });
        }
        self.apply_run_configuration(config, ctx);
        self.bind_run_conversation(
            terminal_inputs.selected_harness,
            terminal_inputs.harness_transport,
            terminal_inputs.should_share,
            ctx,
        );
        Ok(())
    }

    pub(crate) async fn run_prewarmed_assignment(
        spawner: &ModelSpawner<Self>,
        task: Task,
    ) -> Result<(), AgentDriverError> {
        spawner.spawn(move |me, ctx| me.run(task, ctx)).await?.await
    }
}

#[cfg(test)]
#[path = "prewarmed_tests.rs"]
mod tests;
