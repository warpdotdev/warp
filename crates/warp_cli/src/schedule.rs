use clap::{Args, Subcommand};

use crate::config_file::ConfigFileArgs;
use crate::environment::{EnvironmentCreateArgs, EnvironmentUpdateArgs};
use crate::mcp::MCPSpec;
use crate::model::ModelArgs;
use crate::scope::ObjectScope;
use crate::skill::SkillSpec;

/// `ScheduleCommand` has a slightly unusual definition because we allow `oz schedule` as
// a shorthand for `oz schedule create`.
#[derive(Debug, Clone, Args)]
#[clap(args_conflicts_with_subcommands = true)]
pub struct ScheduleCommand {
    #[clap(subcommand)]
    subcommand: Option<ScheduleSubcommand>,

    #[clap(flatten)]
    create: Option<CreateScheduleArgs>,
}

impl ScheduleCommand {
    /// Get the specific scheduling subcommand. Returns `None` if using the `oz schedule` creation shorthand.
    pub fn subcommand(&self) -> Option<&ScheduleSubcommand> {
        self.subcommand.as_ref()
    }

    /// Convert into the specific scheduling subcommand to run.
    pub fn into_subcommand(self) -> ScheduleSubcommand {
        if let Some(create) = self.create {
            ScheduleSubcommand::Create(create)
        } else if let Some(cmd) = self.subcommand {
            cmd
        } else {
            panic!("Either subcommand or create args are required");
        }
    }
}

/// Schedule-related subcommands.
#[derive(Debug, Clone, Subcommand)]
pub enum ScheduleSubcommand {
    /// Create a scheduled Oz agent.
    Create(CreateScheduleArgs),
    /// List scheduled Oz agents.
    List,
    /// Get a scheduled Oz agent's configuration.
    Get(GetScheduleArgs),
    /// Update a scheduled Oz agent.
    Update(UpdateScheduleArgs),
    /// Pause a scheduled Oz agent.
    ///
    /// A paused agent still exists, but will not run according to its schedule.
    Pause(PauseScheduleArgs),
    /// Unpause a scheduled Oz agent.
    ///
    /// The agent will resume executing on its previously-configured schedule.
    #[command(alias = "resume")]
    Unpause(UnpauseScheduleArgs),
    /// Delete a scheduled Oz agent.
    Delete(DeleteScheduleArgs),
}

#[derive(Debug, Clone, Args)]
#[command(
    group(
        clap::ArgGroup::new("prompt_group")
            .required(true)
            .multiple(true)
            .args(["prompt", "skill"])
    )
)]
pub struct CreateScheduleArgs {
    /// Name of the scheduled agent.
    #[arg(long = "name")]
    pub name: String,

    /// Cron schedule expression (e.g., "0 9 * * 1" for 9 AM every Monday).
    #[arg(long = "cron")]
    pub cron: String,

    #[command(flatten)]
    pub model: ModelArgs,

    #[command(flatten)]
    pub environment: EnvironmentCreateArgs,

    #[command(flatten)]
    pub config_file: ConfigFileArgs,

    #[command(flatten)]
    pub scope: ObjectScope,

    /// MCP servers to configure for this schedule.
    ///
    /// Can be specified as:
    /// - A path to a JSON file containing MCP configuration
    /// - Inline JSON with MCP server configuration
    ///
    /// Can be specified multiple times to include multiple servers.
    #[arg(long = "mcp", value_name = "SPEC")]
    pub mcp_specs: Vec<MCPSpec>,

    /// Prompt for what the scheduled agent should do.
    #[arg(long = "prompt", short = 'p')]
    pub prompt: Option<String>,

    /// Automate a skill to run on a schedule.
    ///
    /// Format: `repo:skill_name` or `org/repo:skill_name`
    ///
    /// Skills are searched in `.agents/skills/`, `.warp/skills/`, `.claude/skills/`, and `.codex/skills/` directories.
    /// The skill is resolved at runtime in the agent's cloud environment.
    ///
    /// When used with --prompt, the skill provides the base context and the prompt is the user task.
    /// This is useful for running recurring workflows like code reviews, dependency updates, or reports.
    #[arg(long = "skill", value_name = "SKILL")]
    pub skill: Option<SkillSpec>,

    /// Where this job should be hosted.
    ///
    /// Setting "warp" (or omitting this flag) runs it on Warp's infrastructure.
    /// Any other value is treated as a self-hosted job and the value will be matched
    /// with the self-hosted worker's name.
    #[arg(long = "host", value_name = "WORKER_ID")]
    pub worker_host: Option<String>,
}

#[derive(Debug, Clone, Args)]
pub struct PauseScheduleArgs {
    /// ID of the schedule to pause.
    pub schedule_id: String,
}

#[derive(Debug, Clone, Args)]
pub struct UnpauseScheduleArgs {
    /// ID of the schedule to unpause.
    pub schedule_id: String,
}

#[derive(Debug, Clone, Args)]
pub struct UpdateScheduleArgs {
    /// ID of the schedule to update.
    pub schedule_id: String,

    /// Update the scheduled agent name.
    #[arg(long = "name")]
    pub name: Option<String>,

    /// Update the cron schedule on which the agent is executed.
    #[arg(long = "cron")]
    pub cron: Option<String>,

    #[command(flatten)]
    pub model: ModelArgs,

    #[command(flatten)]
    pub environment: EnvironmentUpdateArgs,

    #[command(flatten)]
    pub config_file: ConfigFileArgs,

    /// MCP servers to configure for this schedule.
    ///
    /// Can be specified as:
    /// - A path to a JSON file containing MCP configuration
    /// - Inline JSON with MCP server configuration
    ///
    /// Can be specified multiple times to include multiple servers.
    #[arg(long = "mcp", value_name = "SPEC")]
    pub mcp_specs: Vec<MCPSpec>,

    /// Remove MCP servers from this schedule by server name.
    ///
    /// This removes the server entry whose key matches `SERVER_NAME`.
    #[arg(long = "remove-mcp", value_name = "SERVER_NAME")]
    pub remove_mcp: Vec<String>,

    /// Update the scheduled agent's prompt.
    #[arg(long = "prompt", short = 'p')]
    pub prompt: Option<String>,

    /// Update the skill used as the base prompt for the scheduled agent.
    ///
    /// Format: `skill_name`, `repo:skill_name`, or `org/repo:skill_name`
    ///
    /// Skills are searched in `.agents/skills/`, `.warp/skills/`, `.claude/skills/`, and `.codex/skills/` directories.
    /// The skill is resolved at runtime in the agent's cloud environment.
    #[arg(long = "skill", value_name = "SKILL", conflicts_with = "remove_skill")]
    pub skill: Option<SkillSpec>,

    /// Remove the skill from this scheduled agent.
    #[arg(long = "remove-skill", conflicts_with = "skill")]
    pub remove_skill: bool,

    /// Where this job should be hosted.
    ///
    /// Setting "warp" runs it on Warp's infrastructure.
    /// Any other value is treated as a self-hosted job and the value will be matched
    /// with the self-hosted worker's name.
    #[arg(long = "host", value_name = "WORKER_ID")]
    pub worker_host: Option<String>,
}

#[derive(Debug, Clone, Args)]
pub struct DeleteScheduleArgs {
    /// ID of the schedule to delete.
    pub schedule_id: String,
}

#[derive(Debug, Clone, Args)]
pub struct GetScheduleArgs {
    /// ID of the schedule to get.
    pub schedule_id: String,
}
