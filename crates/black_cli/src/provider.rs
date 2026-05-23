use clap::{ArgGroup, Args, Subcommand, ValueEnum};

/// Provider-related subcommands.
#[derive(Debug, Clone, Subcommand)]
pub enum ProviderCommand {
    Setup(SetupArgs),
    List,
}

// If we want these at the top level, we can also set provider as a top level subcommand:
#[derive(Debug, Clone, ValueEnum)]
#[value(rename_all = "snake_case")]
pub enum ProviderType {
    Linear,
    Slack,
}

impl ProviderType {
    pub fn name(&self) -> String {
        match self {
            ProviderType::Linear => String::from("Linear"),
            ProviderType::Slack => String::from("Slack"),
        }
    }

    pub fn slug(&self) -> String {
        // add a mapping of provider types to slugs if needed
        self.name().to_lowercase()
    }

    pub fn allowed_in_team_context(&self) -> bool {
        match self {
            ProviderType::Linear => true,
            ProviderType::Slack => true,
        }
    }

    pub fn allowed_in_personal_context(&self) -> bool {
        match self {
            ProviderType::Linear => false,
            ProviderType::Slack => false,
        }
    }
}

#[derive(Debug, Clone, Args)]
#[command(group(ArgGroup::new("scope").required(false)))]
pub struct SetupArgs {
    /// The type of provider to setup.
    pub provider_type: ProviderType,

    /// Setup provider for a team
    #[arg(long, group = "scope")]
    pub team: bool,
    /// Setup provider for a personal account
    #[arg(long, conflicts_with = "team", group = "scope")]
    pub personal: bool,
}
