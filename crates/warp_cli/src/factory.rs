use clap::{Args, Subcommand};

/// Factory-related local configuration commands.
#[derive(Debug, Clone, Subcommand)]
pub enum FactoryCommand {
    /// Manage the default factory used by Factory MCP workflows.
    #[command(subcommand)]
    Default(FactoryDefaultCommand),
}

/// Read or update the locally-stored default factory
/// (`<warp home config dir>/factory/config.json`).
#[derive(Debug, Clone, Subcommand)]
pub enum FactoryDefaultCommand {
    /// Print the saved default factory as JSON (an empty object when none is set).
    Get,
    /// Save a default factory by its uid, preserving any unknown keys in the file.
    Set(SetDefaultFactoryArgs),
    /// Remove the saved default factory, preserving any unknown keys in the file.
    Clear,
}

#[derive(Debug, Clone, Args)]
pub struct SetDefaultFactoryArgs {
    /// The authoritative factory uid to save as the default.
    #[arg(value_name = "UID")]
    pub uid: String,

    /// Optional advisory display name. Recorded for display only and never used
    /// to resolve or match a factory.
    #[arg(long = "name", value_name = "NAME")]
    pub name: Option<String>,
}

impl FactoryCommand {
    pub(crate) fn as_str_for_tracing(&self) -> &'static str {
        match self {
            FactoryCommand::Default(command) => command.as_str_for_tracing(),
        }
    }
}

impl FactoryDefaultCommand {
    pub(crate) fn as_str_for_tracing(&self) -> &'static str {
        match self {
            FactoryDefaultCommand::Get => "factory default get",
            FactoryDefaultCommand::Set(_) => "factory default set",
            FactoryDefaultCommand::Clear => "factory default clear",
        }
    }
}
