use std::collections::HashSet;

use comfy_table::Cell;
use inquire::{error::InquireError, Confirm, Select};
use serde::Serialize;
use warp_cli::{
    agent::OutputFormat,
    environment::{EnvironmentCommand, ImageCommand},
    scope::ObjectScope,
    GlobalOptions,
};
use warpui::r#async::FutureExt;
use warpui::{AppContext, ModelContext, SingletonEntity};

use crate::ai::agent_sdk::output::{self, TableFormat};
use crate::localization;

use crate::ai::agent_sdk::driver::WARP_DRIVE_SYNC_TIMEOUT;
use crate::ai::agent_sdk::oauth_flow::poll_oauth_until_terminal;
use crate::ai::cloud_environments::{
    AmbientAgentEnvironment, BaseImage, CloudAmbientAgentEnvironment,
    CloudAmbientAgentEnvironmentModel, GithubRepo,
};
use crate::auth::UserUid;
use crate::cloud_object::model::generic_string_model::GenericStringObjectId;
use crate::cloud_object::CloudObject;
use crate::server::cloud_objects::update_manager::{
    ObjectOperation, OperationSuccessType, UpdateManager, UpdateManagerEvent,
};
use crate::server::ids::{ClientId, ServerId, SyncId};
use crate::server::server_api::ServerApiProvider;
use crate::util::time_format::format_approx_duration_from_now_utc;
use crate::workspaces::user_profiles::UserProfiles;
use crate::CloudObjectTypeAndId;
use cynic::QueryBuilder;
use warp_graphql::queries::get_oauth_connect_tx_status::OauthConnectTxStatus;
use warp_graphql::queries::list_warp_dev_images::{
    ListWarpDevImages, ListWarpDevImagesResult, ListWarpDevImagesVariables,
};
use warp_graphql::queries::user_repo_auth_status::UserRepoAuthStatusEnum;
use warp_localization::LocaleId;

const WARP_DEV_ENVIRONMENTS_REPO: &str = "https://github.com/warpdotdev/warp-dev-environments";

fn text(app: &AppContext, key: &str) -> String {
    localization::text_for_app(app, key)
}

fn text_for_locale(locale: LocaleId, key: &str) -> String {
    localization::text_for_locale(locale, key)
}

fn text_with_args(app: &AppContext, key: &str, args: &[(&str, &str)]) -> String {
    localization::text_for_app_with_args(app, key, args)
}

/// Parse repo strings in the format "owner/repo" into GithubRepo objects.
fn parse_repos(repo_strings: Vec<String>) -> anyhow::Result<Vec<GithubRepo>> {
    repo_strings
        .into_iter()
        .map(|r| {
            let parts: Vec<&str> = r.split('/').collect();
            if parts.len() != 2 {
                return Err(anyhow::anyhow!(
                    "Invalid repo format: '{}'. Expected format: 'owner/repo'",
                    r
                ));
            }
            Ok(GithubRepo::new(parts[0].to_string(), parts[1].to_string()))
        })
        .collect()
}

/// Handle environment-related CLI commands.
pub fn run(
    ctx: &mut AppContext,
    global_options: GlobalOptions,
    command: EnvironmentCommand,
) -> anyhow::Result<()> {
    let runner = ctx.add_singleton_model(|_ctx| EnvironmentCommandRunner);
    match command {
        EnvironmentCommand::List => {
            runner.update(ctx, |runner, ctx| runner.list(global_options, ctx));
            Ok(())
        }
        EnvironmentCommand::Create {
            name,
            description,
            docker_image,
            repo,
            setup_command,
            scope,
        } => {
            let repos = parse_repos(repo)?;

            runner.update(ctx, |runner, ctx| {
                runner.create(
                    name,
                    description,
                    docker_image,
                    repos,
                    setup_command,
                    scope,
                    ctx,
                )
            });
            Ok(())
        }
        EnvironmentCommand::Delete { id, force } => {
            runner.update(ctx, |runner, ctx| runner.delete(id, force, ctx));
            Ok(())
        }
        EnvironmentCommand::Update {
            id,
            name,
            description,
            remove_description,
            docker_image,
            repo,
            setup_command,
            remove_repo,
            remove_setup_command,
            force,
        } => {
            let repos = parse_repos(repo)?;
            let remove_repos = parse_repos(remove_repo)?;

            runner.update(ctx, |runner, ctx| {
                runner.update_environment(
                    id,
                    name,
                    description,
                    remove_description,
                    docker_image,
                    repos,
                    setup_command,
                    remove_repos,
                    remove_setup_command,
                    force,
                    ctx,
                )
            });
            Ok(())
        }
        EnvironmentCommand::Get { id } => {
            runner.update(ctx, |runner, ctx| runner.get(id, ctx));
            Ok(())
        }
        EnvironmentCommand::Image(image_cmd) => match image_cmd {
            ImageCommand::List => {
                runner.update(ctx, |runner, ctx| runner.list_images(global_options, ctx));
                Ok(())
            }
        },
    }
}

/// Singleton model for running async work as part of environment CLI commands.
struct EnvironmentCommandRunner;

impl EnvironmentCommandRunner {
    fn list_images(&self, global_options: GlobalOptions, ctx: &mut ModelContext<Self>) {
        let server_api = ServerApiProvider::as_ref(ctx).get();

        let operation = ListWarpDevImages::build(ListWarpDevImagesVariables {});
        let fetch_images = async move { server_api.send_graphql_request(operation, None).await };

        ctx.spawn(fetch_images, move |_, result, ctx| match result {
            Ok(response) => match response.list_warp_dev_images {
                ListWarpDevImagesResult::ListWarpDevImagesOutput(output) => {
                    let image_infos: Vec<_> = output
                        .images
                        .into_iter()
                        .map(|img| ImageInfo {
                            image: img.image,
                            repository: img.repository,
                            tag: img.tag,
                        })
                        .collect();

                    if matches!(
                        global_options.output_format,
                        OutputFormat::Text | OutputFormat::Pretty
                    ) {
                        println!(
                            "{}\n",
                            text_with_args(
                                ctx,
                                "agent_sdk.environment.image_list_info",
                                &[("url", WARP_DEV_ENVIRONMENTS_REPO)]
                            )
                        );
                    }
                    output::print_list_for_app(image_infos, global_options.output_format, ctx);
                    ctx.terminate_app(warpui::platform::TerminationMode::ForceTerminate, None);
                }
                ListWarpDevImagesResult::UserFacingError(_) | ListWarpDevImagesResult::Unknown => {
                    super::report_fatal_error(
                        anyhow::anyhow!(text(ctx, "agent_sdk.environment.error.fetch_images")),
                        ctx,
                    );
                }
            },
            Err(err) => {
                super::report_fatal_error(
                    anyhow::anyhow!(text_with_args(
                        ctx,
                        "agent_sdk.environment.error.fetch_images_with_error",
                        &[("error", &err.to_string())]
                    )),
                    ctx,
                );
            }
        });
    }

    fn list(&self, global_options: GlobalOptions, ctx: &mut ModelContext<Self>) {
        let initial_sync = UpdateManager::as_ref(ctx)
            .initial_load_complete()
            .with_timeout(WARP_DRIVE_SYNC_TIMEOUT);

        ctx.spawn(initial_sync, move |_, result, ctx| {
            if result.is_err() {
                super::report_fatal_error(
                    anyhow::anyhow!(text(ctx, "agent_sdk.common.error.warp_drive_sync_timeout")),
                    ctx,
                );
                return;
            }

            let environments = CloudAmbientAgentEnvironment::get_all(ctx);

            let environment_infos: Vec<_> = environments
                .iter()
                .map(|environment| {
                    let name = environment.model().string_model.name.clone();
                    let description = environment.model().string_model.description.clone();
                    let base_image = environment.model().string_model.base_image.clone();
                    let github_repos = environment.model().string_model.github_repos.clone();
                    let setup_commands = environment.model().string_model.setup_commands.clone();

                    let creator_email = environment
                        .metadata()
                        .creator_uid
                        .as_ref()
                        .and_then(|uid| {
                            UserProfiles::as_ref(ctx)
                                .profile_for_uid(UserUid::new(uid))
                                .map(|profile| profile.email.clone())
                        })
                        .unwrap_or_else(|| {
                            crate::localization::text_for_app(ctx, "agent_sdk.common.value.unknown")
                        });

                    let last_edited_utc = environment.metadata().revision.as_ref().map(|r| r.utc());

                    let last_edited = last_edited_utc
                        .map(format_approx_duration_from_now_utc)
                        .unwrap_or_else(|| {
                            crate::localization::text_for_app(ctx, "agent_sdk.common.value.unknown")
                        });

                    let scope_display =
                        super::common::format_owner_for_app(&environment.permissions().owner, ctx);

                    let id = match environment.sync_id() {
                        SyncId::ServerId(server_id) => server_id.to_string(),
                        SyncId::ClientId(_) => crate::localization::text_for_app(
                            ctx,
                            "agent_sdk.common.value.unsynced",
                        ),
                    };

                    EnvironmentInfo {
                        id,
                        name,
                        description,
                        base_image,
                        github_repos,
                        setup_commands,
                        creator_email,
                        last_edited,
                        last_edited_utc,
                        scope: scope_display.to_string(),
                    }
                })
                .collect();

            output::print_list_for_app(environment_infos, global_options.output_format, ctx);

            ctx.terminate_app(warpui::platform::TerminationMode::ForceTerminate, None);
        });
    }

    fn get(&mut self, id: String, ctx: &mut ModelContext<Self>) {
        let initial_sync = UpdateManager::as_ref(ctx)
            .initial_load_complete()
            .with_timeout(WARP_DRIVE_SYNC_TIMEOUT);

        ctx.spawn(initial_sync, move |_, result, ctx| {
            if result.is_err() {
                super::report_fatal_error(
                    anyhow::anyhow!(text(ctx, "agent_sdk.common.error.warp_drive_sync_timeout")),
                    ctx,
                );
                return;
            }

            // Get the ServerId and check if the environment exists
            let server_id = match ServerId::try_from(id.as_str()) {
                Ok(sid) => sid,
                Err(_) => {
                    let error = anyhow::anyhow!(text_with_args(
                        ctx,
                        "agent_sdk.environment.error.not_found",
                        &[("id", &id)]
                    ));
                    ctx.terminate_app(
                        warpui::platform::TerminationMode::ForceTerminate,
                        Some(Err(error)),
                    );
                    return;
                }
            };
            let sync_id = SyncId::ServerId(server_id);
            let environment = CloudAmbientAgentEnvironment::get_by_id(&sync_id, ctx);

            if let Some(environment) = environment {
                Self::print_environment_details(&environment.model().string_model, ctx);
                ctx.terminate_app(warpui::platform::TerminationMode::ForceTerminate, None);
            } else {
                let error = anyhow::anyhow!(text_with_args(
                    ctx,
                    "agent_sdk.environment.error.not_found",
                    &[("id", &id)]
                ));
                ctx.terminate_app(
                    warpui::platform::TerminationMode::ForceTerminate,
                    Some(Err(error)),
                );
            }
        });
    }

    fn print_environment_details(env: &AmbientAgentEnvironment, ctx: &AppContext) {
        println!(
            "{}",
            text_with_args(
                ctx,
                "agent_sdk.environment.detail.name",
                &[("name", &env.name)]
            )
        );
        if let Some(desc) = &env.description {
            println!(
                "{}",
                text_with_args(
                    ctx,
                    "agent_sdk.environment.detail.description",
                    &[("description", desc)]
                )
            );
        }
        match &env.base_image {
            BaseImage::DockerImage(img) => {
                println!(
                    "{}",
                    text_with_args(
                        ctx,
                        "agent_sdk.environment.detail.docker_image",
                        &[("image", img)]
                    )
                );
            }
        }
        if env.github_repos.is_empty() {
            println!(
                "{}",
                text(ctx, "agent_sdk.environment.detail.repositories_none")
            );
        } else {
            println!("{}", text(ctx, "agent_sdk.environment.detail.repositories"));
            for repo in &env.github_repos {
                println!("  - {}/{}", repo.owner, repo.repo);
            }
        }
        if env.setup_commands.is_empty() {
            println!(
                "{}",
                text(ctx, "agent_sdk.environment.detail.setup_commands_none")
            );
        } else {
            println!(
                "{}",
                text(ctx, "agent_sdk.environment.detail.setup_commands")
            );
            for (i, cmd) in env.setup_commands.iter().enumerate() {
                println!("  {}. {}", i + 1, cmd);
            }
        }
    }

    /// Handle inquire errors, returning true if the error was handled (and caller should return early).
    fn handle_inquire_error(err: InquireError, ctx: &mut ModelContext<Self>) -> bool {
        match err {
            InquireError::OperationCanceled | InquireError::OperationInterrupted => {
                eprintln!(
                    "{}",
                    text(ctx, "agent_sdk.environment.output.creation_canceled")
                );
                ctx.terminate_app(warpui::platform::TerminationMode::ForceTerminate, None);
                true
            }
            _ => false,
        }
    }

    /// Fetch images from server and prompt user to select one. Calls continuation with selected image.
    fn prompt_for_docker_image<F>(continuation: F, ctx: &mut ModelContext<Self>)
    where
        F: FnOnce(String, &mut ModelContext<Self>) + Send + 'static,
    {
        let server_api = ServerApiProvider::as_ref(ctx).get();
        let operation = ListWarpDevImages::build(ListWarpDevImagesVariables {});
        let fetch_images = async move { server_api.send_graphql_request(operation, None).await };

        ctx.spawn(fetch_images, move |_, result, ctx| match result {
            Ok(response) => match response.list_warp_dev_images {
                ListWarpDevImagesResult::ListWarpDevImagesOutput(output) => {
                    if output.images.is_empty() {
                        super::report_fatal_error(
                            anyhow::anyhow!(text(ctx, "agent_sdk.environment.error.no_images")),
                            ctx,
                        );
                        return;
                    }

                    println!("{}", text(ctx, "agent_sdk.environment.no_image_provided"));
                    println!(
                        "{}",
                        text(ctx, "agent_sdk.environment.image_info")
                            .replace("{url}", WARP_DEV_ENVIRONMENTS_REPO)
                    );

                    let mut image_choices: Vec<String> =
                        output.images.into_iter().map(|img| img.image).collect();
                    let custom_image_option = text(ctx, "agent_sdk.environment.custom_image");
                    image_choices.push(custom_image_option.clone());

                    let select_prompt = text(ctx, "agent_sdk.environment.select_base_image");
                    let selected_image = match Select::new(&select_prompt, image_choices).prompt() {
                        Ok(image) => image,
                        Err(err) => {
                            if !Self::handle_inquire_error(err, ctx) {
                                super::report_fatal_error(
                                    anyhow::anyhow!(text(
                                        ctx,
                                        "agent_sdk.environment.error.selecting_image",
                                    )),
                                    ctx,
                                );
                            }
                            return;
                        }
                    };

                    let final_image = if selected_image == custom_image_option {
                        let custom_prompt = text(ctx, "agent_sdk.environment.enter_custom_image");
                        match inquire::Text::new(&custom_prompt).prompt() {
                            Ok(custom) => custom,
                            Err(err) => {
                                if !Self::handle_inquire_error(err, ctx) {
                                    super::report_fatal_error(
                                        anyhow::anyhow!(text(
                                            ctx,
                                            "agent_sdk.environment.error.entering_custom_image",
                                        )),
                                        ctx,
                                    );
                                }
                                return;
                            }
                        }
                    } else {
                        selected_image
                    };

                    continuation(final_image, ctx);
                }
                ListWarpDevImagesResult::UserFacingError(_) | ListWarpDevImagesResult::Unknown => {
                    super::report_fatal_error(
                        anyhow::anyhow!(text(ctx, "agent_sdk.environment.error.fetch_base_images")),
                        ctx,
                    );
                }
            },
            Err(err) => {
                super::report_fatal_error(
                    anyhow::anyhow!(text_with_args(
                        ctx,
                        "agent_sdk.environment.error.fetch_images_with_error",
                        &[("error", &err.to_string())]
                    )),
                    ctx,
                );
            }
        });
    }

    #[allow(clippy::too_many_arguments)]
    fn create(
        &mut self,
        name: String,
        description: Option<String>,
        docker_image: Option<String>,
        github_repos: Vec<GithubRepo>,
        setup_commands: Vec<String>,
        scope: warp_cli::scope::ObjectScope,
        ctx: &mut ModelContext<Self>,
    ) {
        if let Some(image) = docker_image {
            Self::create_with_image(
                name,
                description,
                image,
                github_repos,
                setup_commands,
                scope,
                ctx,
            );
        } else {
            Self::prompt_for_docker_image(
                move |image, ctx| {
                    Self::create_with_image(
                        name,
                        description,
                        image,
                        github_repos,
                        setup_commands,
                        scope,
                        ctx,
                    );
                },
                ctx,
            );
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn create_with_image(
        name: String,
        description: Option<String>,
        docker_image: String,
        github_repos: Vec<GithubRepo>,
        setup_commands: Vec<String>,
        scope: warp_cli::scope::ObjectScope,
        ctx: &mut ModelContext<Self>,
    ) {
        let initial_sync = UpdateManager::as_ref(ctx)
            .initial_load_complete()
            .with_timeout(WARP_DRIVE_SYNC_TIMEOUT);

        ctx.spawn(initial_sync, move |_, result, ctx| {
            if result.is_err() {
                super::report_fatal_error(
                    anyhow::anyhow!(text(ctx, "agent_sdk.common.error.warp_drive_sync_timeout")),
                    ctx,
                );
                return;
            }

            // Start the iterative auth + create flow.
            Self::auth_repos_then_execute(
                github_repos.clone(),
                1,
                "create",
                move |ctx| {
                    Self::create_environment_after_auth_check(
                        name,
                        description,
                        github_repos,
                        docker_image,
                        setup_commands,
                        scope,
                        ctx,
                    );
                },
                ctx,
            );
        });
    }

    /// Generic auth flow that checks repo authorization and handles OAuth.
    /// Takes a closure that will be called after successful auth.
    fn auth_repos_then_execute<F>(
        repos: Vec<GithubRepo>,
        attempt: u32,
        operation_name: &'static str,
        on_success: F,
        ctx: &mut ModelContext<Self>,
    ) where
        F: FnOnce(&mut ModelContext<Self>) + Send + 'static,
    {
        const MAX_AUTH_ATTEMPTS: u32 = 8;

        if repos.is_empty() {
            on_success(ctx);
            return;
        }

        if attempt > MAX_AUTH_ATTEMPTS {
            ctx.terminate_app(
                warpui::platform::TerminationMode::ForceTerminate,
                Some(Err(anyhow::anyhow!(
                    "Exceeded maximum number of authorization attempts ({}). Please try again later.",
                    MAX_AUTH_ATTEMPTS
                ))),
            );
            return;
        }

        // Get IntegrationsClient for auth checks and polling
        let integrations_client = ServerApiProvider::as_ref(ctx).get_integrations_client();

        let repo_tuples: Vec<(String, String)> = repos
            .iter()
            .map(|repo| (repo.owner.clone(), repo.repo.clone()))
            .collect();

        let auth_check_future = async move {
            integrations_client
                .check_user_repo_auth_status(repo_tuples)
                .await
        };

        ctx.spawn(auth_check_future, move |_, auth_result, ctx| {
            match auth_result {
                Ok(response) => {
                    let mut has_blocking_private_issues = false;
                    let mut has_public_auth_gaps = false;
                    let mut private_repo_owners = HashSet::new();

                    for status in &response.statuses {
                        match status.status {
                            UserRepoAuthStatusEnum::Success => {
                                if !status.is_public {
                                    private_repo_owners.insert(status.owner.clone());
                                }
                            }
                            UserRepoAuthStatusEnum::NoInstallationOrAccessForRepo => {
                                if !status.is_public {
                                    eprintln!(
                                        "{}",
                                        text_with_args(
                                            ctx,
                                            "agent_sdk.environment.auth.cannot_access_private_repo",
                                            &[
                                                ("owner", &status.owner),
                                                ("repo", &status.repo),
                                            ],
                                        )
                                    );
                                    has_blocking_private_issues = true;
                                    private_repo_owners.insert(status.owner.clone());
                                } else {
                                    has_public_auth_gaps = true;
                                }
                            }
                            UserRepoAuthStatusEnum::UserNotConnectedToGithub => {
                                eprintln!(
                                    "{}",
                                    text(ctx, "agent_sdk.environment.auth.github_not_connected")
                                );
                                has_blocking_private_issues = true;
                                break;
                            }
                        }
                    }

                    // Check that all private repos have the same owner
                    if private_repo_owners.len() > 1 {
                        let owners_str = private_repo_owners.into_iter().collect::<Vec<_>>().join(", ");
                        let error = text_with_args(
                            ctx,
                            "agent_sdk.environment.error.private_repo_multiple_owners",
                            &[("owners", &owners_str)],
                        );
                        ctx.terminate_app(
                            warpui::platform::TerminationMode::ForceTerminate,
                            Some(Err(anyhow::anyhow!(error))),
                        );
                        return;
                    }

                    if !has_blocking_private_issues {
                        // No blocking issues with private repos.
                        // Public repos without auth can proceed with warnings.
                        if has_public_auth_gaps {
                            for status in &response.statuses {
                                if status.is_public
                                    && matches!(
                                        status.status,
                                        UserRepoAuthStatusEnum::NoInstallationOrAccessForRepo
                                    )
                                {
                                    eprintln!(
                                        "{}",
                                        text_with_args(
                                            ctx,
                                            "agent_sdk.environment.auth.public_repo_without_auth",
                                            &[
                                                ("owner", &status.owner),
                                                ("repo", &status.repo),
                                            ],
                                        )
                                    );
                                }
                            }
                            if let Some(auth_url) = response.auth_url {
                                println!(
                                    "\n{}\n",
                                    text_with_args(
                                        ctx,
                                        "agent_sdk.environment.auth.authorize_access_here",
                                        &[("auth_url", &auth_url)],
                                    )
                                );
                            }
                        }

                        // Proceed with the operation after successful auth
                        on_success(ctx);
                        return;
                    }

                    // We have blocking issues with private repos.
                    // Handle OAuth flow if server provides auth_url + tx_id.
                    match (response.auth_url, response.tx_id) {
                        (Some(auth_url), Some(tx_id)) => {
                            // Open URL and poll for OAuth completion.
                            println!(
                                "\n{}",
                                text(ctx, "agent_sdk.environment.auth.private_repo_required")
                            );
                            println!(
                                "{}\n",
                                text_with_args(
                                    ctx,
                                    "agent_sdk.environment.auth.opening_browser",
                                    &[("auth_url", &auth_url)],
                                )
                            );
                            ctx.open_url(&auth_url);

                            let integrations_client = ServerApiProvider::as_ref(ctx)
                                .get_integrations_client();
                            let tx_id = tx_id.into_inner();
                            let poll_future = poll_oauth_until_terminal(
                                integrations_client,
                                tx_id,
                                text(ctx, "agent_sdk.oauth.waiting_for_authorization"),
                                text(ctx, "agent_sdk.oauth.error.timeout"),
                            );

                            let next_attempt = attempt + 1;

                            ctx.spawn(
                                poll_future,
                                move |_, poll_result, ctx| {
                                    match poll_result {
                                        Ok(OauthConnectTxStatus::Completed) => {
                                            // OAuth completed, retry auth check and operation.
                                            Self::auth_repos_then_execute(repos, next_attempt, operation_name, on_success, ctx);
                                        }
                                        Ok(OauthConnectTxStatus::Failed) => {
                                            let error = text(
                                                ctx,
                                                "agent_sdk.environment.error.github_authorization_failed",
                                            );
                                            ctx.terminate_app(
                                                warpui::platform::TerminationMode::ForceTerminate,
                                                Some(Err(anyhow::anyhow!(error))),
                                            );
                                        }
                                        Ok(OauthConnectTxStatus::Expired) => {
                                            let error = text(
                                                ctx,
                                                "agent_sdk.environment.error.github_authorization_expired",
                                            );
                                            ctx.terminate_app(
                                                warpui::platform::TerminationMode::ForceTerminate,
                                                Some(Err(anyhow::anyhow!(error))),
                                            );
                                        }
                                        Ok(OauthConnectTxStatus::Pending)
                                        | Ok(OauthConnectTxStatus::InProgress) => {
                                            // Should not be returned by poll_oauth_until_terminal.
                                            let error = text(
                                                ctx,
                                                "agent_sdk.environment.error.oauth_non_terminal",
                                            );
                                            ctx.terminate_app(
                                                warpui::platform::TerminationMode::ForceTerminate,
                                                Some(Err(anyhow::anyhow!(error))),
                                            );
                                        }
                                        Err(err) => {
                                            let error = text_with_args(
                                                ctx,
                                                "agent_sdk.environment.error.oauth_poll",
                                                &[("error", &err.to_string())],
                                            );
                                            ctx.terminate_app(
                                                warpui::platform::TerminationMode::ForceTerminate,
                                                Some(Err(anyhow::anyhow!(error))),
                                            );
                                        }
                                    }
                                },
                            );
                        }
                        (Some(auth_url), None) => {
                            // Legacy flow: no txId, print URL and exit.
                            println!(
                                "\n{}\n",
                                text_with_args(
                                    ctx,
                                    "agent_sdk.environment.auth.authorize_access_here",
                                    &[("auth_url", &auth_url)],
                                )
                            );
                            println!(
                                "{}",
                                text(ctx, "agent_sdk.environment.auth.rerun_after_authorizing")
                            );
                            ctx.terminate_app(
                                warpui::platform::TerminationMode::ForceTerminate,
                                None,
                            );
                        }
                        (None, Some(_)) => {
                            // Server returned txId without authUrl - unexpected.
                            let error =
                                text(ctx, "agent_sdk.environment.error.oauth_missing_auth_url");
                            ctx.terminate_app(
                                warpui::platform::TerminationMode::ForceTerminate,
                                Some(Err(anyhow::anyhow!(error))),
                            );
                        }
                        (None, None) => {
                            // No auth URL or txId provided, but we have auth issues.
                            let error = text_with_args(
                                ctx,
                                "agent_sdk.environment.error.auth_flow_missing",
                                &[("operation", operation_name)],
                            );
                            ctx.terminate_app(
                                warpui::platform::TerminationMode::ForceTerminate,
                                Some(Err(anyhow::anyhow!(error))),
                            );
                        }
                    }
                }
                Err(e) => {
                    let error = text(ctx, "agent_sdk.environment.error.github_auth_status");
                    ctx.terminate_app(
                        warpui::platform::TerminationMode::ForceTerminate,
                        Some(Err(e.context(error))),
                    );
                }
            }
        });
    }

    // Helper function to create environment after successful auth check
    fn create_environment_after_auth_check(
        name: String,
        description: Option<String>,
        github_repos: Vec<GithubRepo>,
        docker_image: String,
        setup_commands: Vec<String>,
        scope: ObjectScope,
        ctx: &mut ModelContext<Self>,
    ) {
        let environment = AmbientAgentEnvironment::new(
            name,
            description,
            github_repos,
            docker_image,
            setup_commands,
        );
        let client_id = ClientId::default();

        let owner = match super::common::resolve_owner(scope.team, scope.personal, ctx) {
            Ok(owner) => owner,
            Err(e) => {
                super::report_fatal_error(e, ctx);
                return;
            }
        };

        // Create on the server
        UpdateManager::handle(ctx).update(ctx, |update_manager, ctx| {
            update_manager.create_ambient_agent_environment(environment, client_id, owner, ctx);
        });

        // Await creation on the server, then return.
        // We should subscribe to the UpdateManager here because we want to wait
        // for our environment to be assigned a ServerId. Environments are not
        // usable without first being synced.
        ctx.subscribe_to_model(&UpdateManager::handle(ctx), move |_, event, ctx| {
            if let UpdateManagerEvent::ObjectOperationComplete { result } = event {
                if matches!(result.operation, ObjectOperation::Create { .. })
                    && matches!(result.success_type, OperationSuccessType::Success)
                    && result.client_id == Some(client_id)
                {
                    let server_id = result.server_id.unwrap();
                    println!(
                        "{}",
                        text_with_args(
                            ctx,
                            "agent_sdk.environment.output.created",
                            &[("server_id", &server_id.to_string())],
                        )
                    );
                    ctx.terminate_app(warpui::platform::TerminationMode::ForceTerminate, None);
                }
            }
        });
    }

    // Before doing an action like `update` or `delete`, use this function to check whether
    // they are currently being used in any integrations -- if they are, ask the user to confirm
    // before running the supplied `on_confirm` function
    fn confirm_if_integrations_using_environment<F>(
        id: String,
        action: &'static str,
        on_confirm: F,
        ctx: &mut ModelContext<Self>,
    ) where
        F: FnOnce(&mut ModelContext<Self>) + Send + 'static,
    {
        let integrations_client = ServerApiProvider::as_ref(ctx).get_integrations_client();

        let check_integrations_future = async move {
            integrations_client
                .get_integrations_using_environment(id)
                .await
        };

        ctx.spawn(
            check_integrations_future,
            move |_, result, ctx| match result {
                Ok(output) => {
                    if !output.provider_names.is_empty() {
                        let integration_list = output.provider_names.join(", ");
                        let prompt_message = text_with_args(
                            ctx,
                            "agent_sdk.environment.confirm.used_by_integrations",
                            &[("integration_list", &integration_list), ("action", action)],
                        );

                        let confirmation =
                            Confirm::new(&prompt_message).with_default(false).prompt();

                        match confirmation {
                            Ok(true) => on_confirm(ctx),
                            Ok(false)
                            | Err(
                                InquireError::OperationCanceled
                                | InquireError::OperationInterrupted,
                            ) => {
                                println!(
                                    "{}",
                                    text_with_args(
                                        ctx,
                                        "agent_sdk.environment.output.action_canceled",
                                        &[("action", action)],
                                    )
                                );
                                ctx.terminate_app(
                                    warpui::platform::TerminationMode::ForceTerminate,
                                    None,
                                );
                            }
                            Err(err) => {
                                let error = text_with_args(
                                    ctx,
                                    "agent_sdk.environment.error.prompt_confirmation",
                                    &[("error", &err.to_string())],
                                );
                                ctx.terminate_app(
                                    warpui::platform::TerminationMode::ForceTerminate,
                                    Some(Err(anyhow::anyhow!(error))),
                                );
                            }
                        }
                    } else {
                        on_confirm(ctx);
                    }
                }
                Err(_) => {
                    let error = text_with_args(
                        ctx,
                        "agent_sdk.environment.error.integration_usage_unknown",
                        &[("action", action)],
                    );
                    ctx.terminate_app(
                        warpui::platform::TerminationMode::ForceTerminate,
                        Some(Err(anyhow::anyhow!(error))),
                    );
                }
            },
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn update_environment(
        &mut self,
        id: String,
        name: Option<String>,
        description: Option<String>,
        remove_description: bool,
        docker_image: Option<String>,
        add_repos: Vec<GithubRepo>,
        add_setup_commands: Vec<String>,
        remove_repos: Vec<GithubRepo>,
        remove_setup_commands: Vec<String>,
        force: bool,
        ctx: &mut ModelContext<Self>,
    ) {
        let initial_sync = UpdateManager::as_ref(ctx)
            .initial_load_complete()
            .with_timeout(WARP_DRIVE_SYNC_TIMEOUT);

        ctx.spawn(initial_sync, move |_, result, ctx| {
            if result.is_err() {
                super::report_fatal_error(
                    anyhow::anyhow!(text(ctx, "agent_sdk.common.error.warp_drive_sync_timeout")),
                    ctx,
                );
                return;
            }

            // Get the ServerId and check if the environment exists
            let server_id = match ServerId::try_from(id.as_str()) {
                Ok(sid) => sid,
                Err(_) => {
                    let error = anyhow::anyhow!(text_with_args(
                        ctx,
                        "agent_sdk.environment.error.not_found",
                        &[("id", &id)]
                    ));
                    ctx.terminate_app(
                        warpui::platform::TerminationMode::ForceTerminate,
                        Some(Err(error)),
                    );
                    return;
                }
            };
            let sync_id = SyncId::ServerId(server_id);
            let environment = CloudAmbientAgentEnvironment::get_by_id(&sync_id, ctx);
            let Some(environment) = environment else {
                let error = anyhow::anyhow!(text_with_args(
                    ctx,
                    "agent_sdk.environment.error.not_found",
                    &[("id", &id)]
                ));
                ctx.terminate_app(
                    warpui::platform::TerminationMode::ForceTerminate,
                    Some(Err(error)),
                );
                return;
            };

            // Set up the update environment callback, to be run after we've
            // confirmed with the user and checked on auth.
            let environment_clone = environment.clone();
            let repos_clone = add_repos.clone();
            let execute_update = move |ctx: &mut ModelContext<Self>| {
                Self::update_environment_after_auth_check(
                    &environment_clone,
                    server_id,
                    name,
                    description,
                    remove_description,
                    docker_image,
                    repos_clone,
                    add_setup_commands,
                    remove_repos,
                    remove_setup_commands,
                    ctx,
                );
            };

            // Set up the auth check, to be run before updating but after we've
            // confirmed with the user if the operation should occur.
            let auth_repos_before_update = move |ctx: &mut ModelContext<Self>| {
                Self::auth_repos_then_execute(add_repos, 1, "update", execute_update, ctx);
            };

            // Check if any integrations are using this environment
            if force {
                auth_repos_before_update(ctx);
            } else {
                Self::confirm_if_integrations_using_environment(
                    id,
                    "update",
                    auth_repos_before_update,
                    ctx,
                );
            }
        });
    }

    // Helper function to update environment after successful auth check
    #[allow(clippy::too_many_arguments)]
    fn update_environment_after_auth_check(
        environment: &CloudAmbientAgentEnvironment,
        server_id: ServerId,
        name: Option<String>,
        description: Option<String>,
        remove_description: bool,
        docker_image: Option<String>,
        add_repos: Vec<GithubRepo>,
        add_setup_commands: Vec<String>,
        remove_repos: Vec<GithubRepo>,
        remove_setup_commands: Vec<String>,
        ctx: &mut ModelContext<Self>,
    ) {
        let mut updated_env = environment.model().string_model.clone();

        // Update whatever fields we have been provided
        if let Some(new_name) = name {
            updated_env.name = new_name;
        }

        if remove_description {
            updated_env.description = None;
        } else if let Some(new_description) = description {
            updated_env.description = Some(new_description);
        }

        if let Some(new_docker_image) = docker_image {
            updated_env.base_image = BaseImage::DockerImage(new_docker_image);
        }

        for repo in add_repos {
            if !updated_env.github_repos.contains(&repo) {
                updated_env.github_repos.push(repo);
            }
        }

        for repo in &remove_repos {
            if let Some(pos) = updated_env.github_repos.iter().position(|r| r == repo) {
                updated_env.github_repos.remove(pos);
            } else {
                eprintln!(
                    "Warning: repository {}/{} not found in environment, skipping removal",
                    repo.owner, repo.repo
                );
            }
        }

        for cmd in add_setup_commands {
            updated_env.setup_commands.push(cmd);
        }

        for cmd in &remove_setup_commands {
            if let Some(pos) = updated_env.setup_commands.iter().position(|c| c == cmd) {
                updated_env.setup_commands.remove(pos);
            } else {
                eprintln!(
                    "Warning: setup command '{cmd}' not found in environment, skipping removal"
                );
            }
        }

        // Update the environment via UpdateManager
        let revision = environment.metadata.revision.clone();
        UpdateManager::handle(ctx).update(ctx, |update_manager, ctx| {
            update_manager
                .update_object::<GenericStringObjectId, CloudAmbientAgentEnvironmentModel>(
                    CloudAmbientAgentEnvironmentModel::new(updated_env.clone()),
                    environment.sync_id(),
                    revision,
                    ctx,
                );
        });

        // Subscribe to UpdateManager to wait for the update to complete
        ctx.subscribe_to_model(&UpdateManager::handle(ctx), move |_, event, ctx| {
            if let UpdateManagerEvent::ObjectOperationComplete { result } = event {
                if matches!(result.operation, ObjectOperation::Update)
                    && result.server_id == Some(server_id)
                {
                    match result.success_type {
                        OperationSuccessType::Success => {
                            println!("{}", text(ctx, "agent_sdk.environment.output.updated"));
                            Self::print_environment_details(&updated_env, ctx);
                            ctx.terminate_app(
                                warpui::platform::TerminationMode::ForceTerminate,
                                None,
                            );
                        }
                        _ => {
                            super::report_fatal_error(
                                anyhow::anyhow!(text(
                                    ctx,
                                    "agent_sdk.environment.error.update_failed"
                                )),
                                ctx,
                            );
                        }
                    }
                }
            }
        });
    }

    fn delete(&mut self, id: String, force: bool, ctx: &mut ModelContext<Self>) {
        let initial_sync = UpdateManager::as_ref(ctx)
            .initial_load_complete()
            .with_timeout(WARP_DRIVE_SYNC_TIMEOUT);

        ctx.spawn(initial_sync, move |_, result, ctx| {
            if result.is_err() {
                super::report_fatal_error(
                    anyhow::anyhow!(text(ctx, "agent_sdk.common.error.warp_drive_sync_timeout")),
                    ctx,
                );
                return;
            }

            // Get the ServerId and check if the environment exists
            let server_id = match ServerId::try_from(id.as_str()) {
                Ok(sid) => sid,
                Err(_) => {
                    let error = anyhow::anyhow!(text_with_args(
                        ctx,
                        "agent_sdk.environment.error.not_found",
                        &[("id", &id)]
                    ));
                    ctx.terminate_app(
                        warpui::platform::TerminationMode::ForceTerminate,
                        Some(Err(error)),
                    );
                    return;
                }
            };
            let sync_id = SyncId::ServerId(server_id);
            let environment = CloudAmbientAgentEnvironment::get_by_id(&sync_id, ctx);
            let Some(environment) = environment else {
                let error = anyhow::anyhow!(text_with_args(
                    ctx,
                    "agent_sdk.environment.error.not_found",
                    &[("id", &id)]
                ));
                ctx.terminate_app(
                    warpui::platform::TerminationMode::ForceTerminate,
                    Some(Err(error)),
                );
                return;
            };
            let type_and_id = environment.cloud_object_type_and_id();

            // Check if any integrations are using this environment
            if force {
                Self::execute_delete(type_and_id, ctx);
            } else {
                Self::confirm_if_integrations_using_environment(
                    id,
                    "delete",
                    move |ctx| {
                        Self::execute_delete(type_and_id, ctx);
                    },
                    ctx,
                );
            }
        });
    }

    fn execute_delete(type_and_id: CloudObjectTypeAndId, ctx: &mut ModelContext<Self>) {
        UpdateManager::handle(ctx).update(ctx, |update_manager, ctx| {
            update_manager.delete_object_by_user(type_and_id, ctx);
        });

        // Listen to the UpdateManager for a completed object deletion
        ctx.subscribe_to_model(&UpdateManager::handle(ctx), move |_, event, ctx| {
            if let UpdateManagerEvent::ObjectOperationComplete { result } = event {
                if matches!(result.operation, ObjectOperation::Delete { .. }) {
                    match result.success_type {
                        OperationSuccessType::Success => {
                            println!("{}", text(ctx, "agent_sdk.environment.output.deleted"));
                            ctx.terminate_app(
                                warpui::platform::TerminationMode::ForceTerminate,
                                None,
                            );
                        }
                        _ => {
                            super::report_fatal_error(
                                anyhow::anyhow!(text(
                                    ctx,
                                    "agent_sdk.environment.error.delete_failed"
                                )),
                                ctx,
                            );
                        }
                    }
                }
            }
        });
    }
}

impl warpui::Entity for EnvironmentCommandRunner {
    type Event = ();
}
impl SingletonEntity for EnvironmentCommandRunner {}

/// Environment information that's shown in the `list` command.
#[derive(Serialize)]
struct EnvironmentInfo {
    id: String,
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    base_image: BaseImage,
    github_repos: Vec<GithubRepo>,
    setup_commands: Vec<String>,
    creator_email: String,
    #[serde(skip_serializing)]
    last_edited: String,
    #[serde(rename = "last_edited")]
    last_edited_utc: Option<chrono::DateTime<chrono::Utc>>,
    scope: String,
}

impl TableFormat for EnvironmentInfo {
    fn header() -> Vec<Cell> {
        vec![
            Cell::new(text_for_locale(
                LocaleId::EnUs,
                "agent_sdk.environment.table.id",
            )),
            Cell::new(text_for_locale(
                LocaleId::EnUs,
                "agent_sdk.environment.table.name",
            )),
            Cell::new(text_for_locale(
                LocaleId::EnUs,
                "agent_sdk.environment.table.description",
            )),
            Cell::new(text_for_locale(
                LocaleId::EnUs,
                "agent_sdk.environment.table.base_image",
            )),
            Cell::new(text_for_locale(
                LocaleId::EnUs,
                "agent_sdk.environment.table.git_repos",
            )),
            Cell::new(text_for_locale(
                LocaleId::EnUs,
                "agent_sdk.environment.table.setup_commands",
            )),
            Cell::new(text_for_locale(
                LocaleId::EnUs,
                "agent_sdk.environment.table.creator",
            )),
            Cell::new(text_for_locale(
                LocaleId::EnUs,
                "agent_sdk.environment.table.last_edited",
            )),
            Cell::new(text_for_locale(
                LocaleId::EnUs,
                "agent_sdk.environment.table.scope",
            )),
        ]
    }

    fn header_for_app(app: &AppContext) -> Vec<Cell> {
        vec![
            Cell::new(text(app, "agent_sdk.environment.table.id")),
            Cell::new(text(app, "agent_sdk.environment.table.name")),
            Cell::new(text(app, "agent_sdk.environment.table.description")),
            Cell::new(text(app, "agent_sdk.environment.table.base_image")),
            Cell::new(text(app, "agent_sdk.environment.table.git_repos")),
            Cell::new(text(app, "agent_sdk.environment.table.setup_commands")),
            Cell::new(text(app, "agent_sdk.environment.table.creator")),
            Cell::new(text(app, "agent_sdk.environment.table.last_edited")),
            Cell::new(text(app, "agent_sdk.environment.table.scope")),
        ]
    }

    fn row(&self) -> Vec<Cell> {
        let github_repos_display = self
            .github_repos
            .iter()
            .map(|repo| repo.to_string())
            .collect::<Vec<_>>()
            .join(", ");
        let setup_commands_display = self.setup_commands.join("\n");
        let description_display = self.description.as_deref().unwrap_or("");

        vec![
            Cell::new(&self.id),
            Cell::new(&self.name),
            Cell::new(description_display),
            Cell::new(self.base_image.to_string()),
            Cell::new(github_repos_display),
            Cell::new(setup_commands_display),
            Cell::new(&self.creator_email),
            Cell::new(&self.last_edited),
            Cell::new(&self.scope),
        ]
    }

    fn row_for_app(&self, app: &AppContext) -> Vec<Cell> {
        let github_repos_display = if self.github_repos.is_empty() {
            text(app, "agent_sdk.common.value.none")
        } else {
            self.github_repos
                .iter()
                .map(|repo| repo.to_string())
                .collect::<Vec<_>>()
                .join(", ")
        };
        let setup_commands_display = if self.setup_commands.is_empty() {
            text(app, "agent_sdk.common.value.none")
        } else {
            self.setup_commands.join("\n")
        };
        let description_display = self
            .description
            .clone()
            .unwrap_or_else(|| text(app, "agent_sdk.common.value.none"));

        vec![
            Cell::new(&self.id),
            Cell::new(&self.name),
            Cell::new(description_display),
            Cell::new(self.base_image.to_string()),
            Cell::new(github_repos_display),
            Cell::new(setup_commands_display),
            Cell::new(&self.creator_email),
            Cell::new(&self.last_edited),
            Cell::new(&self.scope),
        ]
    }
}

/// Image information that's shown in the `image list` command.
#[derive(Serialize)]
struct ImageInfo {
    image: String,
    repository: String,
    tag: String,
}

impl TableFormat for ImageInfo {
    fn header() -> Vec<Cell> {
        vec![
            Cell::new(text_for_locale(
                LocaleId::EnUs,
                "agent_sdk.environment.image_table.image",
            )),
            Cell::new(text_for_locale(
                LocaleId::EnUs,
                "agent_sdk.environment.image_table.repository",
            )),
            Cell::new(text_for_locale(
                LocaleId::EnUs,
                "agent_sdk.environment.image_table.tag",
            )),
        ]
    }

    fn header_for_app(app: &AppContext) -> Vec<Cell> {
        vec![
            Cell::new(text(app, "agent_sdk.environment.image_table.image")),
            Cell::new(text(app, "agent_sdk.environment.image_table.repository")),
            Cell::new(text(app, "agent_sdk.environment.image_table.tag")),
        ]
    }

    fn row(&self) -> Vec<Cell> {
        vec![
            Cell::new(&self.image),
            Cell::new(&self.repository),
            Cell::new(&self.tag),
        ]
    }
}
