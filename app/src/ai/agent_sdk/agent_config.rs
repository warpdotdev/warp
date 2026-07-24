//! Commands to interact with available agents via the public API.

use warp_cli::agent::ListAgentSkillsArgs;
use warp_graphql::queries::get_oauth_connect_tx_status::OauthConnectTxStatus;
use warp_graphql::queries::user_repo_auth_status::UserRepoAuthStatusEnum;
use warpui::platform::TerminationMode;
use warpui::{AppContext, ModelContext, SingletonEntity};

use crate::ai::agent_sdk::oauth_flow::poll_oauth_until_terminal;
use crate::ai::cloud_environments::GithubRepo;
use crate::server::server_api::ServerApiProvider;
use crate::server::server_api::ai::AgentSkillItem;

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

    Err(anyhow::anyhow!(
        "Invalid repo format: '{}'. Expected 'owner/repo' or 'https://github.com/owner/repo'",
        spec
    ))
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
                Some(Err(anyhow::anyhow!(
                    "Exceeded maximum number of authorization attempts ({}). Please try again later.",
                    MAX_AUTH_ATTEMPTS
                ))),
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
                                        "Cannot access private repo {}/{}",
                                        status.owner, status.repo,
                                    );
                                    has_blocking_private_issues = true;
                                }
                                // Public repos without auth are fine - no warning needed
                            }
                            UserRepoAuthStatusEnum::UserNotConnectedToGithub => {
                                eprintln!("User not connected to GitHub");
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
                            println!("\nAuthorization required for private repository access.");
                            println!("Opening browser for GitHub authorization: {auth_url}\n");
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
                                                "GitHub authorization failed. Please try again."
                                            ))),
                                        );
                                    }
                                    Ok(OauthConnectTxStatus::Expired) => {
                                        ctx.terminate_app(
                                            TerminationMode::ForceTerminate,
                                            Some(Err(anyhow::anyhow!(
                                                "GitHub authorization expired. Please try again."
                                            ))),
                                        );
                                    }
                                    Ok(_) => {
                                        ctx.terminate_app(
                                            TerminationMode::ForceTerminate,
                                            Some(Err(anyhow::anyhow!(
                                                "Unexpected OAuth status"
                                            ))),
                                        );
                                    }
                                    Err(err) => {
                                        ctx.terminate_app(
                                            TerminationMode::ForceTerminate,
                                            Some(Err(anyhow::anyhow!(
                                                "Error polling OAuth status: {err}"
                                            ))),
                                        );
                                    }
                                }
                            });
                        }
                        (Some(auth_url), None) => {
                            println!("\nAuthorize access here: {auth_url}\n");
                            println!("After authorizing, please re-run this command.");
                            ctx.terminate_app(TerminationMode::ForceTerminate, None);
                        }
                        _ => {
                            ctx.terminate_app(
                                TerminationMode::ForceTerminate,
                                Some(Err(anyhow::anyhow!(
                                    "Cannot list agents: authorization required but no auth flow provided"
                                ))),
                            );
                        }
                    }
                }
                Err(e) => {
                    ctx.terminate_app(
                        TerminationMode::ForceTerminate,
                        Some(Err(e.context("Failed to check GitHub auth status"))),
                    );
                }
            }
        });
    }

    fn fetch_and_display_agents(&self, repo: Option<String>, ctx: &mut ModelContext<Self>) {
        let ai_client = ServerApiProvider::handle(ctx).as_ref(ctx).get_ai_client();

        if repo.is_some() {
            println!("Fetching agent skills from the specified repository...");
        } else {
            println!("Fetching agent skills from your Warp environments...");
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
            println!("No skills found.");
            return;
        }

        if agents.len() == 1 {
            println!("\nAgent:");
        } else {
            println!("\nAgents ({}):", agents.len());
        }

        for agent in agents {
            println!("\n{}", agent.name);

            for variant in &agent.variants {
                let mut table = super::output::standard_table();

                // ID
                table.add_row(vec![format!("ID: {}", variant.id)]);

                // Description
                if !variant.description.is_empty() {
                    let description_cell = super::text_layout::render_labeled_wrapped_field(
                        "Description",
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
                    let prompt_cell = super::text_layout::render_labeled_wrapped_field(
                        "Base Prompt",
                        &truncated_prompt,
                        MAX_LINE_WIDTH,
                    );
                    table.add_row(vec![prompt_cell]);
                }

                // Source
                table.add_row(vec![format!(
                    "Source: {}/{}",
                    variant.source.owner, variant.source.name
                )]);

                // Environments
                if !variant.environments.is_empty() {
                    let env_entries: Vec<_> = variant
                        .environments
                        .iter()
                        .map(|e| format!("{} ({})", e.name, e.uid))
                        .collect();
                    table.add_row(vec![format!("Environments: {}", env_entries.join(", "))]);
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
