use clap::{Args, Subcommand, ValueEnum};

use crate::scope::ObjectScope;

/// Provider-related subcommands.
#[derive(Debug, Clone, Subcommand)]
pub enum ProviderCommand {
    Setup(SetupArgs),
    List,
}

impl ProviderCommand {
    pub(crate) fn as_str_for_tracing(&self) -> &'static str {
        match self {
            ProviderCommand::Setup(_) => "provider setup",
            ProviderCommand::List => "provider list",
        }
    }
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
pub struct SetupArgs {
    /// The type of provider to setup.
    pub provider_type: ProviderType,

    #[command(flatten)]
    pub scope: ObjectScope,
}
