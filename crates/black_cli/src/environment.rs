use clap::{ArgAction, ArgGroup, Args, Subcommand};

use crate::scope::ObjectScope;

/// Maximum length for environment descriptions.
const MAX_DESCRIPTION_LENGTH: usize = 240;

/// Validates that a description is within the allowed length.
fn validate_description(s: &str) -> Result<String, String> {
    let len = s.chars().count();
    if len > MAX_DESCRIPTION_LENGTH {
        Err(format!(
            "Description must be at most {} characters (got {})",
            MAX_DESCRIPTION_LENGTH, len
        ))
    } else {
        Ok(s.to_string())
    }
}

/// Environment-related subcommands.
#[derive(Debug, Clone, Subcommand)]
#[command(group(ArgGroup::new("scope").required(false)))]
#[command(visible_alias = "e")]
pub enum EnvironmentCommand {
    /// List cloud environments.
    List,
    /// Manage base images for cloud environments.
    #[command(subcommand)]
    Image(ImageCommand),
    /// Create a new cloud environment.
    Create {
        /// Name of the environment
        #[arg(long = "name", short = 'n')]
        name: String,
        /// Description of the environment (max 240 characters)
        #[arg(long = "description", value_parser = validate_description)]
        description: Option<String>,
        /// Docker image to use. Run `warp environment image list` to list suggested dev images.
        /// If not specified, you'll be prompted to select from available images.
        #[arg(long = "docker-image", short = 'd')]
        docker_image: Option<String>,
        /// Git repo in format "owner/repo" (can be specified multiple times)
        #[arg(long = "repo", short = 'r',  action = ArgAction::Append)]
        repo: Vec<String>,
        /// Accept multiple setup command args to be run after cloning
        #[arg(long = "setup-command", short = 'c', action = ArgAction::Append)]
        setup_command: Vec<String>,

        #[command(flatten)]
        scope: ObjectScope,
    },
    /// Delete a cloud environment.
    Delete {
        /// ID of the environment to delete
        id: String,
        /// Force delete without checking for integration usage
        #[arg(long, default_value_t = false)]
        force: bool,
    },
    /// Get details of a cloud environment.
    Get {
        /// ID of the environment to get
        id: String,
    },
    /// Update an existing cloud environment.
    Update {
        /// ID of the environment to update
        id: String,
        /// Name of the environment (optional, updates if present)
        #[arg(long = "name", short = 'n')]
        name: Option<String>,
        /// Description of the environment (max 240 characters)
        #[arg(
            long = "description",
            value_parser = validate_description,
            conflicts_with = "remove_description",
        )]
        description: Option<String>,
        /// Remove the description from the environment
        #[arg(long = "remove-description", conflicts_with = "description")]
        remove_description: bool,
        /// Docker image to use (optional, updates if present)
        #[arg(long = "docker-image", short = 'd')]
        docker_image: Option<String>,
        /// Git repo in format "owner/repo" to add (can be specified multiple times)
        #[arg(long = "repo", short = 'r',  action = ArgAction::Append)]
        repo: Vec<String>,
        /// Setup command to add to the end of the list (can be specified multiple times)
        #[arg(long = "setup-command", short = 'c', action = ArgAction::Append)]
        setup_command: Vec<String>,
        /// Git repo in format "owner/repo" to remove (can be specified multiple times)
        #[arg(long, action = ArgAction::Append)]
        remove_repo: Vec<String>,
        /// Setup command to remove from the list (can be specified multiple times)
        #[arg(long, action = ArgAction::Append)]
        remove_setup_command: Vec<String>,
        /// Force update without checking for integration usage
        #[arg(long, default_value_t = false)]
        force: bool,
    },
}

/// Common arguments for selecting an environment when creating an integration.
#[derive(Args, Clone, Debug)]
#[group(required = false, multiple = false)]
pub struct EnvironmentCreateArgs {
    /// Cloud environment to run the agent in.
    #[arg(long = "environment", value_name = "ENVIRONMENT_ID", short = 'e')]
    pub environment: Option<String>,

    /// Do not run the agent in an environment (not recommended).
    #[arg(long = "no-environment")]
    pub no_environment: bool,
}

/// Common arguments for selecting an environment when updating an integration.
#[derive(Args, Clone, Debug)]
#[group(required = false, multiple = false)]
pub struct EnvironmentUpdateArgs {
    /// Cloud environment to run the agent in.
    #[arg(long = "environment", value_name = "ENVIRONMENT_ID", short = 'e')]
    pub environment: Option<String>,

    /// Do not run the agent in an environment (not recommended).
    #[arg(long = "remove-environment")]
    pub remove_environment: bool,
}

/// Image-related subcommands.
#[derive(Debug, Clone, Subcommand)]
pub enum ImageCommand {
    /// List available Warp dev base images from Docker Hub.
    List,
}
