//! Agent SDK entry points for invoking Agent-related functionality from the app.
//! For now this provides a simple runner that echoes the received command.

use std::fmt::Write;
use std::path::Path;

use crate::ai::agent_sdk::driver::harness::{harness_kind, HarnessKind};
use crate::ai::agent_sdk::driver::{AgentDriverOptions, AgentRunPrompt, Task};
use crate::ai::agent_sdk::mcp_config::build_mcp_servers_from_specs;
use anyhow::Context;
use warp_cli::{agent::AgentCommand, CliCommand, GlobalOptions};
use warp_core::features::FeatureFlag;
use warp_logging::log_file_path;
use warpui::{platform::TerminationMode, AppContext, ModelSpawner};

use crate::ai::ambient_agents::AgentConfigSnapshot;
use driver::AgentDriverError;

use crate::ai::skills::{resolve_skill_spec, ResolveSkillError, ResolvedSkill};

pub(crate) use driver::harness::{validate_cli_installed, ClaudeHarness, ThirdPartyHarness};
pub use driver::AgentDriver;
use warp_cli::agent::{Harness, Prompt, RunAgentArgs};

mod common;
pub(crate) mod config_file;
pub(crate) mod driver;
mod mcp;
mod mcp_config;
mod model;
pub mod output;
mod profiles;
mod provider;

/// Run a Warp CLI command.
pub fn run(
    ctx: &mut AppContext,
    command: CliCommand,
    global_options: GlobalOptions,
) -> anyhow::Result<()> {
    launch_command(ctx, command, global_options)
}

/// Dispatch a CLI command to its handler.
fn dispatch_command(
    ctx: &mut AppContext,
    command: CliCommand,
    global_options: GlobalOptions,
) -> anyhow::Result<()> {
    match command {
        CliCommand::Agent(agent_cmd) => run_agent(ctx, global_options, agent_cmd),
        CliCommand::MCP(mcp_cmd) => mcp::run(ctx, global_options, mcp_cmd),
        CliCommand::Model(model_cmd) => model::run(ctx, global_options, model_cmd),
        CliCommand::Provider(provider_cmd) => {
            if !FeatureFlag::ProviderCommand.is_enabled() {
                return Err(anyhow::anyhow!("invalid value 'provider'"));
            }
            provider::run(ctx, global_options, provider_cmd)
        }
        CliCommand::Secret(_) => Err(anyhow::anyhow!("invalid value 'secret'")),
    }
}

fn format_skill_resolution_error(err: ResolveSkillError) -> String {
    match err {
        ResolveSkillError::NotFound { skill } => {
            format!("Skill '{skill}' not found")
        }
        ResolveSkillError::RepoNotFound { repo } => {
            format!("Repository '{repo}' not found")
        }
        ResolveSkillError::Ambiguous { skill, candidates } => {
            let mut msg = format!(
                "Skill '{skill}' is ambiguous; specify as repo:skill_name\n\nCandidates:\n"
            );
            for path in candidates {
                msg.push_str(&format!("- {}\n", path.display()));
            }
            msg
        }
        ResolveSkillError::OrgMismatch {
            repo,
            expected,
            found,
        } => {
            format!("Repository '{repo}' found but belongs to org '{found}', expected '{expected}'")
        }
        ResolveSkillError::ParseFailed { path, message } => {
            format!("Failed to parse skill file {}: {message}", path.display())
        }
    }
}

/// Run the agent with the provided command.
fn run_agent(
    ctx: &mut AppContext,
    global_options: GlobalOptions,
    command: AgentCommand,
) -> anyhow::Result<()> {
    match command {
        AgentCommand::Run(args) => {
            if args.bedrock_inference_role.is_some() {
                return Err(anyhow::anyhow!(
                    "hosted OIDC role authentication is unavailable in this build"
                ));
            }

            // Start the agent driver runner, which will handle the rest of the setup steps
            // (managing both sync and async steps) as well as triggering the driver.
            let runner = ctx.add_singleton_model(|_| AgentDriverRunner);
            runner.update(ctx, move |_, ctx| {
                let spawner = ctx.spawner();
                ctx.spawn(
                    AgentDriverRunner::setup_and_run_driver(spawner, *args),
                    |_, result, _ctx| {
                        if let Err(e) = result {
                            report_fatal_error(e.into(), _ctx);
                        }
                    },
                );
            });

            Ok(())
        }
        AgentCommand::Profile(sub) => profiles::run(ctx, global_options, sub),
        AgentCommand::List(_) => Err(anyhow::anyhow!("invalid value 'list'")),
    }
}

/// Build the merged agent configuration from all sources and the Task for the driver.
/// Merge precedence: file < CLI < skill
fn build_merged_config_and_task(
    args: &RunAgentArgs,
    resolved_skill: &Option<ResolvedSkill>,
    prompt: &Option<Prompt>,
    ctx: &mut AppContext,
) -> anyhow::Result<(AgentConfigSnapshot, Task)> {
    let loaded_file = match args.config_file.file.as_deref() {
        Some(path) => Some(config_file::load_config_file(path)?),
        None => None,
    };

    let cli_mcp_servers = build_mcp_servers_from_specs(&args.all_mcp_specs())?;

    // Merge precedence: file < CLI < skill
    let file_merged = config_file::merge_with_precedence(loaded_file.as_ref(), Default::default());

    // Skill provides base_prompt and optionally name
    let (skill_name, runtime_base_prompt) = match resolved_skill {
        Some(skill) => (Some(skill.name.clone()), Some(skill.instructions.clone())),
        None => (None, None),
    };

    let mut merged_config = AgentConfigSnapshot {
        // CLI name > skill name > file name
        name: args.name.clone().or(skill_name).or(file_merged.name),
        environment_id: file_merged.environment_id,
        model_id: args.model.model.clone().or(file_merged.model_id),
        // Skill base_prompt takes precedence over file base_prompt
        base_prompt: runtime_base_prompt.clone().or(file_merged.base_prompt),
        mcp_servers: config_file::merge_mcp_servers(file_merged.mcp_servers, cli_mcp_servers),
        profile_id: args.profile.clone(),
        worker_host: file_merged.worker_host,
        skill_spec: file_merged.skill_spec,
        computer_use_enabled: args
            .computer_use
            .computer_use_override()
            .or(file_merged.computer_use_enabled),
        harness: None,
        harness_auth_secrets: None,
    };

    let model_override = merged_config
        .model_id
        .as_deref()
        .map(|model_id| common::validate_agent_mode_base_model_id(model_id, ctx))
        .transpose()?;

    // Keep the task config snapshot aligned with the effective model selection.
    merged_config.model_id = model_override.clone().map(|id| id.to_string());

    // Combine base_prompt with user prompt locally.
    let local_prompt = match (merged_config.base_prompt.as_deref(), prompt) {
        (Some(base_prompt), Some(Prompt::PlainText(user_prompt))) => {
            Prompt::PlainText(format!("{base_prompt}\n\n{user_prompt}"))
        }
        (Some(base_prompt), None) => {
            // Skill-only invocation: use skill instructions as the prompt
            Prompt::PlainText(base_prompt.to_string())
        }
        (_, Some(p)) => p.clone(),
        (None, None) => {
            return Err(anyhow::anyhow!(AgentDriverError::InvalidRuntimeState));
        }
    };

    let selected_harness: Harness = args.harness.map(Into::into).ok_or_else(|| {
        anyhow::anyhow!(AgentDriverError::HarnessSetupFailed {
            harness: "local".to_string(),
            reason: "Select a local CLI runner with --runner claude or --runner gemini."
                .to_string(),
        })
    })?;

    let task = Task {
        prompt: AgentRunPrompt::Local(resolve_prompt(&local_prompt, ctx)?),
        harness: harness_kind(selected_harness)?,
    };

    Ok((merged_config, task))
}

/// Resolve a `Prompt` to a plain string.
fn resolve_prompt(prompt: &Prompt, _ctx: &AppContext) -> Result<String, AgentDriverError> {
    match prompt {
        Prompt::PlainText(prompt_str) => Ok(prompt_str.to_string()),
        Prompt::SavedPrompt(workflow_id) => {
            Err(AgentDriverError::AIWorkflowNotFound(workflow_id.to_owned()))
        }
    }
}

/// Singleton model that provides a ModelContext for spawning async operations
/// when starting the agent driver. This is needed because conversation fetching
/// requires spawning an async task, which requires a ModelContext.
struct AgentDriverRunner;

impl warpui::Entity for AgentDriverRunner {
    type Event = ();
}

impl warpui::SingletonEntity for AgentDriverRunner {}

impl AgentDriverRunner {
    async fn setup_and_run_driver(
        foreground: ModelSpawner<Self>,
        args: RunAgentArgs,
    ) -> Result<(), AgentDriverError> {
        let result: Result<(), AgentDriverError> = async {
            let (driver_options, task) =
                Self::build_driver_options_and_task(&foreground, args).await?;

            match &task.harness {
                HarnessKind::Unsupported(harness) => {
                    return Err(AgentDriverError::HarnessSetupFailed {
                        harness: harness.to_string(),
                        reason: format!(
                            "The {harness} harness is only supported for local child agent launches."
                        ),
                    });
                }
                HarnessKind::ThirdParty(_) => {}
            }

            // Validate that the third-party harness is installed and authed.
            if let HarnessKind::ThirdParty(harness) = &task.harness {
                harness.validate()?;
            }

            // Run the driver
            foreground
                .spawn(move |_, ctx| {
                    Self::create_and_run_driver(ctx, driver_options, task);
                })
                .await?;

            Ok(())
        }
        .await;
        result
    }

    /// Resolve the skill spec from args, if one was provided.
    ///
    /// In sandboxed mode with a fully-qualified spec (org + repo), the repo is
    /// cloned first since it may not exist locally. Otherwise we resolve directly
    /// against the local filesystem.
    async fn resolve_skill(
        foreground: &ModelSpawner<Self>,
        args: &RunAgentArgs,
        working_dir: &Path,
    ) -> Result<Option<ResolvedSkill>, AgentDriverError> {
        let Some(skill_spec) = args.skill.clone() else {
            return Ok(None);
        };

        let working_dir_buf = working_dir.to_path_buf();
        let skill = foreground
            .spawn(move |_, ctx| resolve_skill_spec(&skill_spec, &working_dir_buf, ctx))
            .await?
            .map_err(|err| {
                AgentDriverError::SkillResolutionFailed(format_skill_resolution_error(err))
            })?;
        log::debug!(
            "Resolved skill '{}' from {}",
            skill.name,
            skill.skill_path.display()
        );
        Ok(Some(skill))
    }

    /// Build the AgentDriverOptions and Task for a local runner.
    async fn build_driver_options_and_task(
        foreground: &ModelSpawner<Self>,
        args: RunAgentArgs,
    ) -> Result<(AgentDriverOptions, Task), AgentDriverError> {
        // Get the working directory
        let working_dir = match args.cwd.as_ref() {
            Some(dir) => dunce::canonicalize(dir)
                .with_context(|| format!("Unable to resolve {}", dir.display())),
            None => std::env::current_dir().context("Unable to determine working directory"),
        }
        .map_err(AgentDriverError::ConfigBuildFailed)?;

        // Resolve the skill, if we have one
        let resolved_skill = Self::resolve_skill(foreground, &args, &working_dir).await?;

        let prompt = args.prompt_arg.to_prompt();

        // Build the AgentConfigSnapshot, Task, and AgentDriverOptions
        let prompt_clone = prompt.clone();
        let (_merged_config, task, driver_options) = foreground
            .spawn(move |_, ctx| -> anyhow::Result<_> {
                let (merged_config, task) =
                    build_merged_config_and_task(&args, &resolved_skill, &prompt_clone, ctx)?;

                let driver_options = driver::AgentDriverOptions {
                    working_dir: working_dir.clone(),
                    idle_on_complete: args.idle_on_complete.map(|d| d.into()),
                    secrets: Default::default(),
                };

                Ok((merged_config, task, driver_options))
            })
            .await?
            .map_err(AgentDriverError::ConfigBuildFailed)?;

        Ok((driver_options, task))
    }

    /// Create the AgentDriver and start running the task.
    fn create_and_run_driver(
        ctx: &mut AppContext,
        driver_options: driver::AgentDriverOptions,
        task: driver::Task,
    ) {
        let driver = ctx.add_singleton_model(|ctx| {
            AgentDriver::new(driver_options, ctx).expect("Could not initialize driver")
        });

        driver.update(ctx, |driver, ctx| {
            let agent_future = driver.run(task, ctx);

            ctx.spawn(agent_future, |_, result, ctx| match result {
                Ok(()) => {
                    ctx.terminate_app(TerminationMode::ForceTerminate, None);
                }
                Err(err) => {
                    report_fatal_error(err.into(), ctx);
                }
            });
        });
    }
}

/// Launch a CLI command. Hosted Warp server authentication is not part of Warper's
/// app-side CLI dispatch path.
fn launch_command(
    ctx: &mut AppContext,
    command: CliCommand,
    global_options: GlobalOptions,
) -> anyhow::Result<()> {
    dispatch_command(ctx, command, global_options)
}

/// Report a fatal error and terminate the app.
fn report_fatal_error(err: anyhow::Error, ctx: &mut AppContext) {
    let mut message = err.to_string();
    for cause in err.chain().skip(1) {
        let _ = write!(&mut message, "\n=> {cause}");
    }
    {
        if let Ok(path) = log_file_path() {
            let _ = write!(
                message,
                "\n\nFor more information, check Warper logs at {}",
                path.display()
            );
        }
    }

    let error = anyhow::anyhow!(message);
    ctx.terminate_app(TerminationMode::ForceTerminate, Some(Err(error)));
}
