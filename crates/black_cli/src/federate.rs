use clap::{Args, Subcommand};

/// Federated authentication between Oz and cloud providers.
///
/// Oz supports OIDC federation to allow agents to securely authenticate to other systems
/// using short-lived credentials.
#[derive(Debug, Clone, Subcommand)]
pub enum FederateCommand {
    /// Issue an identity token for the current Oz agent. This can only be called within a running Oz agent session.
    IssueToken(IssueTokenArgs),
    /// Issue an identity token for the current Oz agent, in the format expected by Google Cloud's
    /// [executable-sourced credentials](https://docs.cloud.google.com/iam/docs/workload-identity-federation-with-other-providers#executable-sourced-credentials)
    /// mechanism.
    #[command(hide = true)]
    IssueGcpToken(IssueGcpTokenArgs),
}

#[derive(Debug, Clone, Args)]
#[command(name = "issue-token")]
pub struct IssueTokenArgs {
    /// The run ID to issue the token for.
    #[arg(long = "run-id")]
    pub run_id: String,

    /// The audience claim for the identity token.
    #[arg(long = "audience")]
    pub audience: String,

    /// Requested token lifetime (e.g. "1h", "30m").
    #[arg(long = "duration", default_value = "1h")]
    pub duration: humantime::Duration,

    /// Controls how the OIDC token subject is formatted.
    ///
    /// The template consists of a list of claims, which are joined together to
    /// form the subject. The default subject template is the principal, such as
    /// `user:user-id`.
    ///
    /// Supported components are:
    /// - principal (`user:my-user-id`)
    /// - scoped_principal (`principal:my-team-id/user:my-user-id`)
    /// - email (`email:user@warp.dev`)
    /// - teams (`teams:my-team-id`)
    /// - environment (`environment:my-environment-id`)
    /// - agent_name (`agent_name:my-agent`)
    /// - skill_spec (`skill_spec:warpdotdev/repo_path_to_skill`)
    /// - run_id (`run_id:abc123`)
    /// - host (`host:my-worker-id`)
    #[arg(long = "subject-template", num_args = 1..)]
    pub subject_template: Option<Vec<String>>,
}

#[derive(Debug, Clone, Args)]
#[command(name = "issue-gcp-token")]
pub struct IssueGcpTokenArgs {
    /// The run ID to issue the token for.
    #[arg(long = "run-id")]
    pub run_id: String,

    /// Requested token lifetime (e.g. "1h", "30m").
    #[arg(long = "duration", default_value = "1h")]
    pub duration: humantime::Duration,

    /// The audience for the token request.
    #[arg(long, env = "GOOGLE_EXTERNAL_ACCOUNT_AUDIENCE")]
    pub audience: String,

    /// The requested token type (e.g. "urn:ietf:params:oauth:token-type:id_token").
    #[arg(long, env = "GOOGLE_EXTERNAL_ACCOUNT_TOKEN_TYPE")]
    pub token_type: String,

    /// Optional path to write the token output for caching.
    #[arg(long, env = "GOOGLE_EXTERNAL_ACCOUNT_OUTPUT_FILE")]
    pub output_file: Option<String>,
}
