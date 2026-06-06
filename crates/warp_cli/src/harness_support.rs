use clap::{Args, Subcommand, ValueEnum};

/// Commands to support third-party agent harnesses running within Oz.
///
/// These commands are invoked by external agent harnesses (e.g. Claude Code)
/// during a cloud agent run to interact with Oz platform APIs.
#[derive(Debug, Clone, Args)]
pub struct HarnessSupportArgs {
    /// The run ID to associate with harness-support API calls.
    #[arg(long = "run-id", env = "OZ_RUN_ID")]
    pub run_id: String,

    #[command(subcommand)]
    pub command: HarnessSupportCommand,
}

#[derive(Debug, Clone, Subcommand)]
pub enum HarnessSupportCommand {
    /// Verify connectivity by fetching and displaying the current run.
    #[command(hide = true)]
    Ping,

    /// Report an artifact back to the Oz platform.
    ReportArtifact(ReportArtifactArgs),

    /// Send a progress notification to the task's originating platform (Slack, Linear, etc.).
    NotifyUser(NotifyUserArgs),

    /// Report task completion or failure, as well as a summary of the task.
    FinishTask(FinishTaskArgs),

    /// Report that the agent process is shutting down.
    ReportShutdown(ReportShutdownArgs),
}

#[derive(Debug, Clone, Args)]
pub struct ReportArtifactArgs {
    #[command(subcommand)]
    pub command: ReportArtifactCommand,
}

#[derive(Debug, Clone, Subcommand)]
pub enum ReportArtifactCommand {
    /// Report a pull request artifact.
    PullRequest(PullRequestArtifactArgs),
}

#[derive(Debug, Clone, Args)]
pub struct PullRequestArtifactArgs {
    /// URL of the pull request.
    #[arg(long)]
    pub url: String,

    /// Branch name associated with the pull request.
    #[arg(long)]
    pub branch: String,
}

#[derive(Debug, Clone, Args)]
pub struct NotifyUserArgs {
    /// The message to send as a progress update.
    #[arg(long)]
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum TaskStatus {
    Success,
    Failure,
}

#[derive(Debug, Clone, Args)]
pub struct FinishTaskArgs {
    /// Whether the task succeeded or failed.
    #[arg(long)]
    pub status: TaskStatus,

    /// A summary of the task outcome.
    #[arg(long)]
    pub summary: String,
}

#[derive(Debug, Clone, Args)]
pub struct ReportShutdownArgs {
    /// Error category for abnormal shutdown (e.g. "oom", "timeout").
    /// Omit for clean shutdown.
    #[arg(long)]
    pub error_category: Option<String>,

    /// Human-readable error message for abnormal shutdown.
    /// Omit for clean shutdown.
    #[arg(long)]
    pub error_message: Option<String>,
}
