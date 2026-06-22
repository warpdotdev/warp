use futures::future;
use warp_cli::integration::{CreateIntegrationArgs, IntegrationCommand, UpdateIntegrationArgs};
use warp_cli::provider::ProviderType;
use warp_cli::GlobalOptions;
use warp_graphql::mutations::create_simple_integration::CreateSimpleIntegrationOutput;
use warp_graphql::queries::get_oauth_connect_tx_status::OauthConnectTxStatus;
use warp_graphql::queries::get_simple_integrations::SimpleIntegrationsOutput;
use warpui::platform::TerminationMode;
use warpui::{AppContext, ModelContext, SingletonEntity};

use super::common::{EnvironmentChoice, ResolveConfigurationError};
use super::integration_output;
use super::oauth_flow::poll_oauth_until_terminal;
use crate::server::server_api::ServerApiProvider;

pub fn run(
    ctx: &mut AppContext,
    global_options: GlobalOptions,
    command: IntegrationCommand,
) -> anyhow::Result<()> {
    let runner = ctx.add_singleton_model(|_ctx| IntegrationCommandRunner);
    match command {
        IntegrationCommand::Create(args) => {
            runner.update(ctx, |runner, ctx| runner.create(args, ctx));
        }
        IntegrationCommand::Update(args) => {
            runner.update(ctx, |runner, ctx| runner.update(args, ctx));
        }
        IntegrationCommand::List => {
            runner.update(ctx, |runner, ctx| runner.list(global_options, ctx));
        }
    }
    Ok(())
}

struct IntegrationCommandRunner;

impl IntegrationCommandRunner {
    fn list(&self, global_options: GlobalOptions, ctx: &mut ModelContext<Self>) {
        // Hardcoded set of providers that this client knows how to render.
        let providers = vec![ProviderType::Linear, ProviderType::Slack];
        let provider_slugs: Vec<String> = providers.into_iter().map(|p| p.slug()).collect();

        let integrations_client = ServerApiProvider::as_ref(ctx).get_integrations_client();

        let list_future = async move {
            integrations_client
                .list_simple_integrations(provider_slugs)
                .await
        };

        ctx.spawn(
            list_future,
            move |_, result: anyhow::Result<SimpleIntegrationsOutput>, ctx| match result {
                Ok(output) => {
                    integration_output::print_integrations(&output, global_options.output_format);
                    ctx.terminate_app(TerminationMode::ForceTerminate, None);
                }
                Err(err) => {
                    ctx.terminate_app(TerminationMode::ForceTerminate, Some(Err(err)));
                }
            },
        );
    }

    fn create(&self, args: CreateIntegrationArgs, ctx: &mut ModelContext<Self>) {
        let refresh_future = super::common::refresh_workspace_metadata(ctx);
        let warp_drive_sync_future = super::common::refresh_warp_drive(ctx);
        let setup_future = future::try_join(refresh_future, warp_drive_sync_future);

        ctx.spawn(setup_future, move |runner, setup_result, ctx| {
            if let Err(err) = setup_result {
                ctx.terminate_app(TerminationMode::ForceTerminate, Some(Err(err)));
                return;
            }

            let loaded_file = match args.config_file.file.as_deref() {
                Some(path) => match super::config_file::load_config_file(path) {
                    Ok(file) => Some(file),
                    Err(err) => {
                        ctx.terminate_app(TerminationMode::ForceTerminate, Some(Err(err)));
                        return;
                    }
                },
                None => None,
            };

            let integration_type = args.provider.slug();
            let enabled = true;
            let is_update = false;

            let cli_mcp_servers =
                match super::mcp_config::build_mcp_servers_from_specs(&args.mcp_specs) {
                    Ok(mcp_servers) => mcp_servers,
                    Err(err) => {
                        ctx.terminate_app(TerminationMode::ForceTerminate, Some(Err(err)));
                        return;
                    }
                };

            let mut merged_config = super::config_file::merge_with_precedence(
                loaded_file.as_ref(),
                crate::ai::ambient_agents::AgentConfigSnapshot {
                    name: None,
                    environment_id: args.environment.environment.clone(),
                    model_id: args.model.model.clone(),
                    base_prompt: args.prompt.clone(),
                    mcp_servers: cli_mcp_servers,
                    profile_id: None,
                    worker_host: args.worker_host.clone(),
                    skill_spec: None,
                    // TODO(QUALITY-295): Support computer use flag in integrations.
                    computer_use_enabled: None,
                    // TODO(REMOTE-1134): Support harness selection for integrations.
                    harness: None,
                    harness_auth_secrets: None,
                },
            );

            // We must wait until after workspace metadata is refreshed to check available LLMs.
            let model_id = match merged_config
                .model_id
                .as_deref()
                .map(|model_id| super::common::validate_agent_mode_base_model_id(model_id, ctx))
                .transpose()
            {
                Ok(model_id) => model_id.map(|model_id| model_id.to_string()),
                Err(err) => {
                    ctx.terminate_app(TerminationMode::ForceTerminate, Some(Err(err)));
                    return;
                }
            };

            let base_prompt = merged_config.base_prompt.take();
            let worker_host = merged_config.worker_host.take();

            let mcp_servers_json = match merged_config.mcp_servers.take() {
                Some(map) => match serde_json::to_string(&map) {
                    Ok(json) => Some(json),
                    Err(err) => {
                        ctx.terminate_app(TerminationMode::ForceTerminate, Some(Err(err.into())));
                        return;
                    }
                },
                None => None,
            };

            //If the user didn't explicitly request no environment, load environment from the config
            let mut environment_args = args.environment;
            if environment_args.environment.is_none() && !environment_args.no_environment {
                environment_args.environment = merged_config.environment_id.take();
            }

            let environment_uid = match EnvironmentChoice::resolve_for_create(environment_args, ctx)
            {
                Ok(EnvironmentChoice::None) => {
                    eprintln!("Creating integration without an environment.");
                    None
                }
                Ok(EnvironmentChoice::Environment { id, .. }) => {
                    eprintln!("Creating integration with environment {id}.");
                    Some(id)
                }
                Err(ResolveConfigurationError::Canceled) => {
                    eprintln!("Integration creation canceled.");
                    ctx.terminate_app(TerminationMode::ForceTerminate, None);
                    return;
                }
                Err(err) => {
                    super::report_fatal_error(anyhow::anyhow!(err), ctx);
                    return;
                }
            };

            runner.start_create_or_update_flow(
                ctx,
                integration_type,
                environment_uid,
                base_prompt,
                model_id,
                mcp_servers_json,
                None,
                worker_host,
                enabled,
                is_update,
                1,
            );
        });
    }

    #[allow(clippy::too_many_arguments)]
    fn start_create_or_update_flow(
        &self,
        ctx: &mut ModelContext<Self>,
        integration_type: String,
        environment_uid: Option<String>,
        base_prompt: Option<String>,
        model_id: Option<String>,
        mcp_servers_json: Option<String>,
        remove_mcp_server_names: Option<Vec<String>>,
        worker_host: Option<String>,
        enabled: bool,
        is_update: bool,
        attempt: u32,
    ) {
        const MAX_CREATE_ATTEMPTS: u32 = 8;
        let action = if is_update { "update" } else { "creation" };

        if attempt > MAX_CREATE_ATTEMPTS {
            ctx.terminate_app(
                TerminationMode::ForceTerminate,
                Some(Err(anyhow::anyhow!(
                    "Exceeded maximum number of integration creation attempts ({}). Retry.",
                    MAX_CREATE_ATTEMPTS
                ))),
            );
            return;
        }

        let integrations_client = ServerApiProvider::as_ref(ctx).get_integrations_client();

        let future_integration_type = integration_type.clone();
        let future_environment_uid = environment_uid.clone();
        let future_base_prompt = base_prompt.clone();
        let future_model_id = model_id.clone();
        let future_mcp_servers_json = mcp_servers_json.clone();
        let future_remove_mcp_server_names = remove_mcp_server_names.clone();
        let future_worker_host = worker_host.clone();
        let future_is_update = is_update;

        let create_future = async move {
            integrations_client
                .create_or_update_simple_integration(
                    future_integration_type,
                    future_is_update,
                    future_environment_uid,
                    future_base_prompt,
                    future_model_id,
                    future_mcp_servers_json,
                    future_remove_mcp_server_names,
                    future_worker_host,
                    enabled,
                )
                .await
        };

        ctx.spawn(
            create_future,
            move |_runner, result: anyhow::Result<CreateSimpleIntegrationOutput>, ctx| {
                match result {
                    Ok(output) => {
                        println!("{}", output.message);

                        let auth_url = output.auth_url;
                        let tx_id = output.tx_id;

                        match (auth_url, tx_id) {
                            (Some(auth_url), Some(tx_id)) => {
                                // We have another auth step: open URL and poll txId.
                                println!("Authorize the provider here: {auth_url}\n");
                                ctx.open_url(&auth_url);

                                let integrations_client = ServerApiProvider::as_ref(ctx)
                                    .get_integrations_client();
                                let tx_id = tx_id.into_inner();

                                let poll_future =
                                    poll_oauth_until_terminal(integrations_client, tx_id);

                                let next_integration_type = integration_type.clone();
                                let next_environment_uid = environment_uid.clone();
                                let next_base_prompt = base_prompt.clone();
                                let next_model_id = model_id.clone();
                                let next_mcp_servers_json = mcp_servers_json.clone();
                                let next_remove_mcp_server_names = remove_mcp_server_names.clone();
                                let next_worker_host = worker_host.clone();
                                let next_enabled = enabled;
                                let next_is_update = is_update;
                                let next_attempt = attempt + 1;

                                ctx.spawn(
                                    poll_future,
                                    move |runner, poll_result, ctx| {
                                        match poll_result {
                                            Ok(OauthConnectTxStatus::Completed) => {
                                                // Inner loop done; try create or update again (outer loop).
                                                // This may happen multiple times if the user needs to authorize multiple services.
                                                runner.start_create_or_update_flow(
                                                    ctx,
                                                    next_integration_type,
                                                    next_environment_uid,
                                                    next_base_prompt,
                                                    next_model_id,
                                                    next_mcp_servers_json,
                                                    next_remove_mcp_server_names,
                                                    next_worker_host,
                                                    next_enabled,
                                                    next_is_update,
                                                    next_attempt,
                                                );
                                            }
                                            Ok(OauthConnectTxStatus::Failed) => {
                                                ctx.terminate_app(
                                                    TerminationMode::ForceTerminate,
                                                    Some(Err(anyhow::anyhow!("OAuth authorization failed."))),
                                                );
                                            }
                                            Ok(OauthConnectTxStatus::Expired) => {
                                                ctx.terminate_app(
                                                    TerminationMode::ForceTerminate,
                                                    Some(Err(anyhow::anyhow!("OAuth authorization expired."))),
                                                );
                                            }
                                            Ok(OauthConnectTxStatus::Pending)
                                            | Ok(OauthConnectTxStatus::InProgress) => {
                                                // Should not be returned by poll_oauth_until_terminal.
                                                ctx.terminate_app(
                                                    TerminationMode::ForceTerminate,
                                                    Some(Err(anyhow::anyhow!("Unexpected non-terminal OAuth status returned"))),
                                                );
                                            }
                                            Err(err) => {
                                                ctx.terminate_app(
                                                    TerminationMode::ForceTerminate,
                                                    Some(Err(anyhow::anyhow!("Error polling OAuth status: {err}"))),
                                                );
                                            }
                                        }
                                    },
                                );
                            }
                            (Some(auth_url), None) => {
                                println!("Authorize the provider here: {auth_url}\n");
                                ctx.open_url(&auth_url);
                                println!(
                                    "After authorizing, re-run the command to continue the integration {action} process.",
                                );
                                ctx.terminate_app(
                                    TerminationMode::ForceTerminate,
                                    None,
                                );
                            }
                            (None, Some(_)) => {
                                ctx.terminate_app(
                                    TerminationMode::ForceTerminate,
                                    Some(Err(anyhow::anyhow!("Server did not return an authURL for the integration creation process."))),
                                );
                            }
                            (None, None) => {
                                // No more auth steps; finalize.
                                if output.success {
                                    ctx.terminate_app(
                                        TerminationMode::ForceTerminate,
                                        None,
                                    );
                                } else {
                                    ctx.terminate_app(
                                        TerminationMode::ForceTerminate,
                                        Some(Err(anyhow::anyhow!("Integration creation reported failure: {}", output.message))),
                                    );
                                }
                            }
                        }
                    }
                    Err(err) => {
                        ctx.terminate_app(
                            TerminationMode::ForceTerminate,
                            Some(Err(err)),
                        );
                    }
                }
            },
        );
    }

    fn update(&self, args: UpdateIntegrationArgs, ctx: &mut ModelContext<Self>) {
        let refresh_future = super::common::refresh_workspace_metadata(ctx);
        let warp_drive_sync_future = super::common::refresh_warp_drive(ctx);
        let setup_future = future::try_join(refresh_future, warp_drive_sync_future);

        ctx.spawn(setup_future, move |runner, setup_result, ctx| {
            if let Err(err) = setup_result {
                ctx.terminate_app(TerminationMode::ForceTerminate, Some(Err(err)));
                return;
            }

            let loaded_file = match args.config_file.file.as_deref() {
                Some(path) => match super::config_file::load_config_file(path) {
                    Ok(file) => Some(file),
                    Err(err) => {
                        ctx.terminate_app(TerminationMode::ForceTerminate, Some(Err(err)));
                        return;
                    }
                },
                None => None,
            };

            let remove_mcp = args.remove_mcp.clone();

            let integration_type = args.provider.slug();
            let enabled = true;
            let is_update = true;

            let cli_mcp_servers =
                match super::mcp_config::build_mcp_servers_from_specs(&args.mcp_specs) {
                    Ok(mcp_servers) => mcp_servers,
                    Err(err) => {
                        ctx.terminate_app(TerminationMode::ForceTerminate, Some(Err(err)));
                        return;
                    }
                };

            let mut merged_config = super::config_file::merge_with_precedence(
                loaded_file.as_ref(),
                crate::ai::ambient_agents::AgentConfigSnapshot {
                    name: None,
                    environment_id: args.environment.environment.clone(),
                    model_id: args.model.model.clone(),
                    base_prompt: args.prompt.clone(),
                    mcp_servers: cli_mcp_servers,
                    profile_id: None,
                    worker_host: args.worker_host.clone(),
                    skill_spec: None,
                    // TODO(QUALITY-295): Support computer use flag in integrations.
                    computer_use_enabled: None,
                    // TODO(REMOTE-1134): Support harness selection for integrations.
                    harness: None,
                    harness_auth_secrets: None,
                },
            );

            // We must wait until after workspace metadata is refreshed to check available LLMs.
            let model_id = match merged_config
                .model_id
                .as_deref()
                .map(|model_id| super::common::validate_agent_mode_base_model_id(model_id, ctx))
                .transpose()
            {
                Ok(model_id) => model_id.map(|model_id| model_id.to_string()),
                Err(err) => {
                    ctx.terminate_app(TerminationMode::ForceTerminate, Some(Err(err)));
                    return;
                }
            };

            let base_prompt = merged_config.base_prompt.take();
            let worker_host = merged_config.worker_host.take();

            // MCP update semantics are patch-only:
            // - `mcp_servers_json` adds/overwrites MCP servers.
            // - `remove_mcp_server_names` removes MCP servers.
            // If both are present, removals win by filtering removed names out of the JSON payload.
            let mcp_servers_json = match merged_config.mcp_servers.take() {
                Some(mut map) => {
                    for name in &remove_mcp {
                        map.remove(name);
                    }

                    if map.is_empty() {
                        None
                    } else {
                        match serde_json::to_string(&map) {
                            Ok(json) => Some(json),
                            Err(err) => {
                                ctx.terminate_app(
                                    TerminationMode::ForceTerminate,
                                    Some(Err(err.into())),
                                );
                                return;
                            }
                        }
                    }
                }
                None => None,
            };

            let remove_mcp_server_names = if args.remove_mcp.is_empty() {
                None
            } else {
                Some(args.remove_mcp)
            };

            if args.environment.remove_environment {
                // Explicitly requested to update without an environment.
                runner.start_create_or_update_flow(
                    ctx,
                    integration_type,
                    Some(String::new()),
                    base_prompt,
                    model_id,
                    mcp_servers_json,
                    remove_mcp_server_names,
                    worker_host,
                    enabled,
                    is_update,
                    1,
                );
                return;
            }

            let environment_uid = merged_config.environment_id.take();

            runner.start_create_or_update_flow(
                ctx,
                integration_type,
                environment_uid,
                base_prompt,
                model_id,
                mcp_servers_json,
                remove_mcp_server_names,
                worker_host,
                enabled,
                is_update,
                1,
            );
        });
    }
}

impl warpui::Entity for IntegrationCommandRunner {
    type Event = ();
}
impl SingletonEntity for IntegrationCommandRunner {}
