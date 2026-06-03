//! Commands to interact with available agents via the public API.

use warp_cli::agent::ListAgentSkillsArgs;
use warp_graphql::queries::get_oauth_connect_tx_status::OauthConnectTxStatus;
use warp_graphql::queries::user_repo_auth_status::UserRepoAuthStatusEnum;
use warpui::platform::TerminationMode;
use warpui::{AppContext, ModelContext, SingletonEntity};

use crate::ai::agent_sdk::oauth_flow::poll_oauth_until_terminal;
use crate::ai::cloud_environments::GithubRepo;
use crate::server::server_api::ai::AgentSkillItem;
use crate::server::server_api::ServerApiProvider;

const MAX_LINE_WIDTH: usize = 90;
const MAX_AUTH_ATTEMPTS: u32 = 8;

/// Singleton model that runs async work for agent CLI commands.
struct AgentConfigRunner;

/// List all available agent skills.
pub fn list_skills(ctx: &mut AppContext, args: ListAgentSkillsArgs) -> anyhow::Result<()> {
    let runner = ctx.add_singleton_model(|_ctx| AgentConfigRunner);
    runner.update(ctx, |runner, ctx| runner.list(args.repo.clone(), ctx))
}

/// Parse a repo spec string (owner/repo or GitHub URL) into a GithubRepo.
fn parse_repo_spec(spec: &str) -> anyhow::Result<GithubRepo> {
    let spec = spec.trim();

    // Try URL format: https://github.com/owner/repo or https://github.com/owner/repo.git
    if spec.starts_with("https://github.com/") || spec.starts_with("http://github.com/") {
        let path = spec
            .trim_start_matches("https://github.com/")
            .trim_start_matches("http://github.com/")
            .trim_end_matches(".git")
            .trim_end_matches('/');

        let parts: Vec<&str> = path.split('/').collect();
        if parts.len() >= 2 && !parts[0].is_empty() && !parts[1].is_empty() {
            return Ok(GithubRepo::new(parts[0].to_string(), parts[1].to_string()));
        }
    }

    // Try slug format: owner/repo
    let parts: Vec<&str> = spec.split('/').collect();
    if parts.len() == 2 && !parts[0].is_empty() && !parts[1].is_empty() {
        return Ok(GithubRepo::new(parts[0].to_string(), parts[1].to_string()));
    }

    Err(anyhow::anyhow!(i18n::t(
        "ai.agent_sdk.agent_config.invalid_repo_format"
    )
    .replace("{repo}", spec)))
}

impl AgentConfigRunner {
    fn list(&self, repo: Option<String>, ctx: &mut ModelContext<Self>) -> anyhow::Result<()> {
        // If a repo is specified, check auth first
        if let Some(ref repo_spec) = repo {
            let github_repo = parse_repo_spec(repo_spec)?;
            self.auth_then_list(vec![github_repo], 1, repo, ctx);
        } else {
            // No repo specified - just list from environments
            self.fetch_and_display_agents(repo, ctx);
        }
        Ok(())
    }

    /// Check GitHub auth for repos, then list agents.
    fn auth_then_list(
        &self,
        repos: Vec<GithubRepo>,
        attempt: u32,
        repo_spec: Option<String>,
        ctx: &mut ModelContext<Self>,
    ) {
        if attempt > MAX_AUTH_ATTEMPTS {
            ctx.terminate_app(
                TerminationMode::ForceTerminate,
                Some(Err(anyhow::anyhow!(i18n::t(
                    "ai.agent_sdk.agent_config.max_authorization_attempts_exceeded"
                )
                .replace("{count}", &MAX_AUTH_ATTEMPTS.to_string())))),
            );
            return;
        }

        let integrations_client = ServerApiProvider::handle(ctx)
            .as_ref(ctx)
            .get_integrations_client();

        let repo_tuples: Vec<(String, String)> = repos
            .iter()
            .map(|repo| (repo.owner.clone(), repo.repo.clone()))
            .collect();

        let auth_check_future = async move {
            integrations_client
                .check_user_repo_auth_status(repo_tuples)
                .await
        };

        ctx.spawn(auth_check_future, move |runner, auth_result, ctx| {
            match auth_result {
                Ok(response) => {
                    let mut has_blocking_private_issues = false;

                    for status in &response.statuses {
                        match status.status {
                            UserRepoAuthStatusEnum::Success => {}
                            UserRepoAuthStatusEnum::NoInstallationOrAccessForRepo => {
                                if !status.is_public {
                                    eprintln!(
                                        "{}",
                                        i18n::t(
                                            "ai.agent_sdk.agent_config.cannot_access_private_repo"
                                        )
                                        .replace("{owner}", &status.owner)
                                        .replace("{repo}", &status.repo),
                                    );
                                    has_blocking_private_issues = true;
                                }
                                // Public repos without auth are fine - no warning needed
                            }
                            UserRepoAuthStatusEnum::UserNotConnectedToGithub => {
                                eprintln!(
                                    "{}",
                                    i18n::t(
                                        "ai.agent_sdk.agent_config.user_not_connected_to_github"
                                    )
                                );
                                has_blocking_private_issues = true;
                                break;
                            }
                        }
                    }

                    if !has_blocking_private_issues {
                        // No blocking issues - proceed with listing
                        runner.fetch_and_display_agents(repo_spec, ctx);
                        return;
                    }

                    // Handle OAuth flow if server provides auth_url + tx_id
                    match (response.auth_url, response.tx_id) {
                        (Some(auth_url), Some(tx_id)) => {
                            println!(
                                "\n{}",
                                i18n::t(
                                    "ai.agent_sdk.agent_config.private_repo_authorization_required"
                                )
                            );
                            println!(
                                "{}\n",
                                i18n::t(
                                    "ai.agent_sdk.agent_config.opening_github_authorization"
                                )
                                .replace("{url}", &auth_url)
                            );
                            ctx.open_url(&auth_url);

                            let integrations_client = ServerApiProvider::handle(ctx)
                                .as_ref(ctx)
                                .get_integrations_client();
                            let tx_id = tx_id.into_inner();
                            let poll_future = poll_oauth_until_terminal(integrations_client, tx_id);

                            let next_attempt = attempt + 1;

                            ctx.spawn(poll_future, move |runner, poll_result, ctx| {
                                match poll_result {
                                    Ok(OauthConnectTxStatus::Completed) => {
                                        // OAuth completed, retry
                                        runner.auth_then_list(repos, next_attempt, repo_spec, ctx);
                                    }
                                    Ok(OauthConnectTxStatus::Failed) => {
                                        ctx.terminate_app(
                                            TerminationMode::ForceTerminate,
                                            Some(Err(anyhow::anyhow!(
                                                i18n::t(
                                                    "ai.agent_sdk.agent_config.github_authorization_failed"
                                                )
                                            ))),
                                        );
                                    }
                                    Ok(OauthConnectTxStatus::Expired) => {
                                        ctx.terminate_app(
                                            TerminationMode::ForceTerminate,
                                            Some(Err(anyhow::anyhow!(
                                                i18n::t(
                                                    "ai.agent_sdk.agent_config.github_authorization_expired"
                                                )
                                            ))),
                                        );
                                    }
                                    Ok(_) => {
                                        ctx.terminate_app(
                                            TerminationMode::ForceTerminate,
                                            Some(Err(anyhow::anyhow!(
                                                i18n::t(
                                                    "ai.agent_sdk.agent_config.unexpected_oauth_status"
                                                )
                                            ))),
                                        );
                                    }
                                    Err(err) => {
                                        ctx.terminate_app(
                                            TerminationMode::ForceTerminate,
                                            Some(Err(anyhow::anyhow!(
                                                i18n::t(
                                                    "ai.agent_sdk.agent_config.poll_oauth_status_error"
                                                )
                                                .replace("{error}", &err.to_string())
                                            ))),
                                        );
                                    }
                                }
                            });
                        }
                        (Some(auth_url), None) => {
                            println!(
                                "\n{}\n",
                                i18n::t("ai.agent_sdk.agent_config.authorize_access_here")
                                    .replace("{url}", &auth_url)
                            );
                            println!(
                                "{}",
                                i18n::t("ai.agent_sdk.agent_config.rerun_after_authorizing")
                            );
                            ctx.terminate_app(TerminationMode::ForceTerminate, None);
                        }
                        _ => {
                            ctx.terminate_app(
                                TerminationMode::ForceTerminate,
                                Some(Err(anyhow::anyhow!(
                                    i18n::t("ai.agent_sdk.agent_config.no_auth_flow_provided")
                                ))),
                            );
                        }
                    }
                }
                Err(e) => {
                    ctx.terminate_app(
                        TerminationMode::ForceTerminate,
                        Some(Err(e.context(i18n::t(
                            "ai.agent_sdk.agent_config.check_github_auth_status_failed",
                        )))),
                    );
                }
            }
        });
    }

    fn fetch_and_display_agents(&self, repo: Option<String>, ctx: &mut ModelContext<Self>) {
        let ai_client = ServerApiProvider::handle(ctx).as_ref(ctx).get_ai_client();

        if repo.is_some() {
            println!(
                "{}",
                i18n::t("ai.agent_sdk.agent_config.fetching_from_repository")
            );
        } else {
            println!(
                "{}",
                i18n::t("ai.agent_sdk.agent_config.fetching_from_environments")
            );
        }

        let list_future = async move { ai_client.list_skills(repo).await };

        ctx.spawn(list_future, |_, result, ctx| match result {
            Ok(agents) => {
                Self::print_agents_table(&agents);
                ctx.terminate_app(TerminationMode::ForceTerminate, None);
            }
            Err(err) => {
                super::report_fatal_error(err, ctx);
            }
        });
    }

    /// Print a list of agents in a card-style format.
    fn print_agents_table(agents: &[AgentSkillItem]) {
        if agents.is_empty() {
            println!("{}", i18n::t("ai.agent_sdk.agent_config.no_skills_found"));
            return;
        }

        if agents.len() == 1 {
            println!("\n{}", i18n::t("ai.agent_sdk.agent_config.heading.agent"));
        } else {
            println!(
                "\n{}",
                i18n::t("ai.agent_sdk.agent_config.heading.agents")
                    .replace("{count}", &agents.len().to_string())
            );
        }

        for agent in agents {
            println!("\n{}", agent.name);

            for variant in &agent.variants {
                let mut table = super::output::standard_table();

                // ID
                table.add_row(vec![
                    i18n::t("ai.agent_sdk.agent_config.field.id").replace("{id}", &variant.id)
                ]);

                // Description
                if !variant.description.is_empty() {
                    let description_label = i18n::t("ai.agent_sdk.agent_config.label.description");
                    let description_cell = super::text_layout::render_labeled_wrapped_field(
                        &description_label,
                        &variant.description,
                        MAX_LINE_WIDTH,
                    );
                    table.add_row(vec![description_cell]);
                }

                // Base prompt (truncated)
                if !variant.base_prompt.is_empty() {
                    let mut chars = variant.base_prompt.chars();
                    let truncated: String = chars.by_ref().take(100).collect();
                    let truncated_prompt = if chars.next().is_some() {
                        format!("{truncated}...")
                    } else {
                        truncated
                    };
                    let base_prompt_label = i18n::t("ai.agent_sdk.agent_config.label.base_prompt");
                    let prompt_cell = super::text_layout::render_labeled_wrapped_field(
                        &base_prompt_label,
                        &truncated_prompt,
                        MAX_LINE_WIDTH,
                    );
                    table.add_row(vec![prompt_cell]);
                }

                // Source
                table.add_row(vec![format!(
                    "{}",
                    i18n::t("ai.agent_sdk.agent_config.field.source")
                        .replace("{owner}", &variant.source.owner)
                        .replace("{name}", &variant.source.name)
                )]);

                // Environments
                if !variant.environments.is_empty() {
                    let env_entries: Vec<_> = variant
                        .environments
                        .iter()
                        .map(|e| format!("{} ({})", e.name, e.uid))
                        .collect();
                    table.add_row(vec![i18n::t(
                        "ai.agent_sdk.agent_config.field.environments",
                    )
                    .replace("{environments}", &env_entries.join(", "))]);
                }

                println!("{table}");
            }
        }
    }
}

impl warpui::Entity for AgentConfigRunner {
    type Event = ();
}

impl SingletonEntity for AgentConfigRunner {}
