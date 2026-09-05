use clap::{ArgAction, ArgGroup, Args, Subcommand};

use crate::scope::ObjectScope;

/// Maximum length for environment descriptions.
const MAX_DESCRIPTION_LENGTH: usize = 240;

/// Maximum number of setup commands allowed on a single environment.
/// Mirrors the server-side upsert validation (see REMOTE-1063 cross-repo
/// contract).
const MAX_SETUP_COMMANDS: usize = 100;

/// Maximum length (in Unicode runes / chars) of a single setup command. Mirrors
/// the server-side upsert validation.
const MAX_SETUP_COMMAND_RUNES: usize = 4096;

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

/// Parse a flat list of `[index, command, index, command, ...]` values (as
/// produced by clap `num_args = 2` + `ArgAction::Append`) into `(usize, String)`
/// pairs. Indexes are zero-based. Rejects an odd number of values, non-integer
/// indexes, and negative indexes, returning a clear user-facing error. `0` is a
/// valid index (the first slot / a prepend target for insert).
pub fn parse_indexed_setup_commands(flat: &[String]) -> Result<Vec<(usize, String)>, String> {
    if !flat.len().is_multiple_of(2) {
        return Err(format!(
            "expected an even number of values (index/command pairs) for --insert-setup-command/--edit-setup-command, got {}",
            flat.len()
        ));
    }
    let mut out = Vec::with_capacity(flat.len() / 2);
    for pair in flat.chunks_exact(2) {
        let index = pair[0].parse::<usize>().map_err(|_| {
            format!(
                "invalid setup-command index '{}': expected a non-negative integer (zero-based)",
                pair[0]
            )
        })?;
        out.push((index, pair[1].clone()));
    }
    Ok(out)
}

/// Validate a single setup command's text against the shared limits: non-empty
/// after trimming and at most [`MAX_SETUP_COMMAND_RUNES`] runes. Mirrors the
/// server-side per-command validation so the CLI fails fast with a clear error
/// instead of a server rejection.
pub fn validate_setup_command(cmd: &str) -> Result<(), String> {
    if cmd.trim().is_empty() {
        return Err("setup command must not be empty after trimming".to_string());
    }
    let runes = cmd.chars().count();
    if runes > MAX_SETUP_COMMAND_RUNES {
        // Report only the configured limit and the observed length — never the
        // command text itself. Setup commands may contain secrets, and this
        // error is surfaced via `report_fatal_error` (logged at error level),
        // so interpolating `{cmd}` would leak sensitive content (REMOTE-1063).
        return Err(format!(
            "setup command must be at most {} runes (got {})",
            MAX_SETUP_COMMAND_RUNES, runes
        ));
    }
    Ok(())
}

/// Apply the requested setup-command operations to `setup_commands` in a fixed
/// order: **clear → append → insert → edit → remove**.
///
/// Indexes are zero-based. Insert accepts `0..=len` (insert at `len` appends);
/// edit accepts `0..=len-1`. Each insert/edit index is validated against the
/// list length *at the point the operation runs*, so multiple inserts that grow
/// the list are handled correctly. Every appended/inserted/edited command is
/// validated with [`validate_setup_command`], and the total count must remain
/// `<=` [`MAX_SETUP_COMMANDS`] after all operations.
///
/// Removal keeps the existing first-exact-text-match behavior and emits a
/// warning (via `eprintln!`) for an absent command, matching the legacy
/// `--remove-setup-command` behavior.
///
/// On error, `setup_commands` is left in a partially-applied state; the caller
/// must discard it (the upsert is not sent on a validation failure).
pub fn apply_setup_command_operations(
    setup_commands: &mut Vec<String>,
    clear: bool,
    appends: &[String],
    inserts: &[(usize, String)],
    edits: &[(usize, String)],
    removals: &[String],
) -> Result<(), String> {
    if clear {
        setup_commands.clear();
    }

    for cmd in appends {
        validate_setup_command(cmd)?;
        setup_commands.push(cmd.clone());
    }

    for (index, cmd) in inserts {
        validate_setup_command(cmd)?;
        if *index > setup_commands.len() {
            return Err(format!(
                "cannot insert setup command at index {index}: environment has {} setup command(s), valid insert indexes are 0-{}",
                setup_commands.len(),
                setup_commands.len()
            ));
        }
        setup_commands.insert(*index, cmd.clone());
    }

    for (index, cmd) in edits {
        validate_setup_command(cmd)?;
        if setup_commands.is_empty() {
            return Err(format!(
                "cannot edit setup command at index {index}: environment has no setup commands to edit"
            ));
        }
        if *index >= setup_commands.len() {
            return Err(format!(
                "cannot edit setup command at index {index}: environment has {} setup command(s), valid edit indexes are 0-{}",
                setup_commands.len(),
                setup_commands.len() - 1
            ));
        }
        setup_commands[*index] = cmd.clone();
    }

    for cmd in removals {
        if let Some(pos) = setup_commands.iter().position(|c| c == cmd) {
            setup_commands.remove(pos);
        } else {
            eprintln!("Warning: setup command '{cmd}' not found in environment, skipping removal");
        }
    }

    if setup_commands.len() > MAX_SETUP_COMMANDS {
        return Err(format!(
            "environment would have {} setup command(s), which exceeds the maximum of {}",
            setup_commands.len(),
            MAX_SETUP_COMMANDS
        ));
    }

    Ok(())
}

/// Render the setup-commands section of `oz environment get`/update output as a
/// single string (with a trailing newline), numbering commands **zero-based**
/// so the displayed index matches the index a user types for
/// `--insert-setup-command` / `--edit-setup-command` (REMOTE-1063).
///
/// An empty list renders `Setup commands: None\n` (no numbering). This is the
/// user-facing display change and is unit-tested in `lib_tests.rs`; the app
/// crate's `print_environment_details` delegates here for the setup-commands
/// block.
pub fn format_setup_commands_listing(setup_commands: &[String]) -> String {
    if setup_commands.is_empty() {
        return "Setup commands: None\n".to_string();
    }
    let mut out = String::from("Setup commands:\n");
    for (i, cmd) in setup_commands.iter().enumerate() {
        out.push_str(&format!("  {}. {}\n", i, cmd));
    }
    out
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
        /// Insert a setup command at a zero-based <index>, shifting later commands down.
        /// <index> may be `0..=N` where N is the current setup-command count (insert at N
        /// appends). Provide the index and command as two values, e.g.
        /// `--insert-setup-command 1 "make build"`. May be specified multiple times;
        /// applied in command-line order. Each command must be non-empty after trimming
        /// and at most 4096 runes; the environment may hold at most 100 setup commands.
        /// Conflicts with --setup-command, --remove-setup-command, --edit-setup-command,
        /// and --clear-setup-commands.
        #[arg(
            long = "insert-setup-command",
            action = ArgAction::Append,
            num_args = 2,
            conflicts_with_all = [
                "setup_command",
                "remove_setup_command",
                "edit_setup_command",
                "clear_setup_commands",
            ]
        )]
        insert_setup_command: Vec<String>,
        /// Replace the setup command at a zero-based <index> with <command>, leaving every
        /// other command untouched. <index> must be `0..=N-1` where N is the current
        /// setup-command count. Provide the index and command as two values, e.g.
        /// `--edit-setup-command 1 "make build"`. May be specified multiple times; applied
        /// in command-line order. Each command must be non-empty after trimming and at
        /// most 4096 runes; the environment may hold at most 100 setup commands.
        /// Conflicts with --setup-command, --remove-setup-command, --insert-setup-command,
        /// and --clear-setup-commands.
        #[arg(
            long = "edit-setup-command",
            action = ArgAction::Append,
            num_args = 2,
            conflicts_with_all = [
                "setup_command",
                "remove_setup_command",
                "insert_setup_command",
                "clear_setup_commands",
            ]
        )]
        edit_setup_command: Vec<String>,
        /// Remove all setup commands from the environment. Combine with --setup-command to
        /// clear-and-rebuild the list to exactly the appended commands, in order. Conflicts
        /// with --remove-setup-command, --insert-setup-command, and --edit-setup-command.
        #[arg(
            long = "clear-setup-commands",
            default_value_t = false,
            conflicts_with_all = [
                "remove_setup_command",
                "insert_setup_command",
                "edit_setup_command",
            ]
        )]
        clear_setup_commands: bool,
        /// Force update without checking for integration usage
        #[arg(long, default_value_t = false)]
        force: bool,
    },
}

impl EnvironmentCommand {
    pub(crate) fn as_str_for_tracing(&self) -> &'static str {
        match self {
            EnvironmentCommand::List => "environment list",
            EnvironmentCommand::Image(_) => "environment image",
            EnvironmentCommand::Create { .. } => "environment create",
            EnvironmentCommand::Delete { .. } => "environment delete",
            EnvironmentCommand::Get { .. } => "environment get",
            EnvironmentCommand::Update { .. } => "environment update",
        }
    }
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
