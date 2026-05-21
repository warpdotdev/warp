use std::collections::HashMap;

use lazy_static::lazy_static;
use regex::bytes::Regex;

use crate::util::parse_ascii_u32;

// The below strings are used as a tag/prefix at the beginning of a response
// from tmux to determine the type of response. These strings must be unique
// and should not contain any special characters (alpha-num, dash, underline,
// and space are acceptable).
// This is because the response is parsed using regex, as well as to avoid
// messing up tmux's shell-like parsing.
const PRIMARY_WINDOW_PANE_PREFIX: &str = "primary window pane";
pub const BACKGROUND_WINDOW_PREFIX: &str = "background window";

#[derive(Clone, Debug)]
pub enum TmuxCommand {
    /// Gets the window id and pane id of the primary window pane.
    GetPrimaryWindowPane,
    /// Runs a command in the background in a new temporary window.
    RunInBackgroundWindow {
        command_id: String,
        current_directory_path: Option<String>,
        command: String,
        environment_variables: Option<HashMap<String, String>>,
    },
    /// Refreshes the client in Control Mode, at a target size of rows and cols.
    UpdateClientSize { num_rows: usize, num_cols: usize },
    /// Configures tmux to automatically terminate any sessions that don't have clients attached.
    SetDestroyUnattached,
    /// Forces the tmux session to inherit the smallest dimensions of any attached client.
    SetWindowSizeToSmallest,
}

fn safe_env_var_name(name: &str) -> bool {
    lazy_static! {
        static ref SAFE_NAME: Regex = Regex::new(r"^[[:word:]]+$").expect("Invalid regex!");
    }
    SAFE_NAME.is_match(name.as_bytes())
}

impl TmuxCommand {
    pub fn get_command_string(&self) -> String {
        // All commands must end with `\n`
        match self {
            TmuxCommand::GetPrimaryWindowPane => format!(
                "list-panes -F \"#{{?pane_active,{PRIMARY_WINDOW_PANE_PREFIX}: ,}}#{{window_id}} #{{pane_id}}\"\n"
            ),
            TmuxCommand::RunInBackgroundWindow {
                current_directory_path,
                command,
                environment_variables,
                command_id,
            } => {
                // We pass the command to tmux wrapped in single quotes. The tmux control mode interface
                // interprets escapes in a bash style, so bash-escape any single quotes in the command.
                // Note that we should always use bash-style escapes here regardless of the current shell,
                // because this command is going to tmux control mode.
                let escaped_command = escape_single_quotes(command);
                let has_new_line = escaped_command.contains('\n');
                debug_assert!(
                    !has_new_line,
                    "Tmux control mode commands must take place on one line: `{escaped_command}`"
                );
                if has_new_line {
                    log::error!(
                        "Tmux control mode command contains a newline: `{escaped_command}`"
                    );
                }
                // It's highly unreliable to try to strip newlines and make sure commands are still
                // valid, so we'd rather ensure no commands have newlines in them.
                let escaped_command = escaped_command.replace('\n', r"\n");

                let set_directory = if let Some(current_directory_path) = current_directory_path {
                    let escaped_path = escape_single_quotes(current_directory_path);
                    format!("-c '{escaped_path}'")
                } else {
                    String::new()
                };

                let mut set_env_vars = String::new();
                for (name, value) in environment_variables.iter().flatten() {
                    if safe_env_var_name(name) {
                        set_env_vars.push_str("-e ");
                        set_env_vars.push_str(name);
                        set_env_vars.push_str("='");
                        set_env_vars.push_str(&escape_single_quotes(value));
                        set_env_vars.push_str("' ");
                    }
                }

                // Constructs a tmux command string. Tmux control mode will first parse this string with a sh-like
                // parser. The parsed command will be executed in a new interactive terminal session.
                // Some things to note:
                // - This also prints formatted window info with the new window id and pane id with `-PF "background window: #{window_id} #{pane_id}"`.
                //   - This must be kept in sync with command output parsing in TmuxPerformer::tmux_message.
                // - The output is piped through `cat` to prevent the command from being directly attached
                //   to a pty (and therefore risking it running in an interactive mode).
                // - This sleeps for 1 second after execution to work around tmux bug which clips largs outputs
                //   when the window exits immediately.
                // - There's a newline at the end so control mode will start running the command.
                let newline = "\n";
                format!(
                    r#"new-window -d {set_directory} {set_env_vars} -PF "{BACKGROUND_WINDOW_PREFIX}: #{{window_id}} #{{pane_id}}" '(builtin echo -n "^^^{command_id}|||"; {escaped_command}; builtin echo "|||$?\$\$\$")|command cat; command sleep 1'{newline}"#
                )
            }
            TmuxCommand::UpdateClientSize { num_rows, num_cols } => {
                format!("refresh-client -C {num_cols},{num_rows}\n")
            }
            TmuxCommand::SetDestroyUnattached => "set destroy-unattached on\n".to_string(),
            TmuxCommand::SetWindowSizeToSmallest => "set window-size smallest\n".to_string(),
        }
    }
}

pub enum TmuxCommandResponse {
    SetPrimaryWindowPane { window_id: u32, pane_id: u32 },
    BackgroundWindow { window_id: u32, pane_id: u32 },
}

pub fn parse_command(line: Vec<u8>) -> Option<TmuxCommandResponse> {
    lazy_static! {
        pub static ref PRIMARY_WINDOW_PANE_REGEX: Regex = {
            let pattern = format!(
                r"^{PRIMARY_WINDOW_PANE_PREFIX}: @([[:digit:]]+) %([[:digit:]]+)$"
            );
            Regex::new(&pattern).expect("invalid regex")
        };

        // Must be kept in sync with the tmux command in TmuxExecutor::execute_command_internal.
        pub static ref BACKGROUND_WINDOW_REGEX: Regex = {
            let pattern = format!(
                r"^{BACKGROUND_WINDOW_PREFIX}: @([[:digit:]]+) %([[:digit:]]+)$"
            );
            Regex::new(&pattern).expect("invalid regex")
        };
    }

    if let Some(captures) = BACKGROUND_WINDOW_REGEX.captures(&line) {
        let window_id: u32 = parse_ascii_u32(&captures[1])
            .expect("impossible: encountered non-ASCII digit in ASCII digit pattern");
        let pane_id: u32 = parse_ascii_u32(&captures[2])
            .expect("impossible: encountered non-ASCII digit in ASCII digit pattern");
        return Some(TmuxCommandResponse::BackgroundWindow { window_id, pane_id });
    } else if let Some(captures) = PRIMARY_WINDOW_PANE_REGEX.captures(&line) {
        let window_id: u32 = parse_ascii_u32(&captures[1])
            .expect("impossible: encountered non-ASCII digit in ASCII digit pattern");
        let pane_id: u32 = parse_ascii_u32(&captures[2])
            .expect("impossible: encountered non-ASCII digit in ASCII digit pattern");
        return Some(TmuxCommandResponse::SetPrimaryWindowPane { window_id, pane_id });
    }
    None
}

/// Tmux control mode uses bash-style escaping, which replaces each single
/// quote with a '"'"' sequence. The first single quote completes the
/// single quoted string to the left, the next three characters: "'" evaluate
/// to a literal single quote in bash/zsh, and then the final single quote
/// starts a new single-quoted string to the right. Effectively, this
/// concatenates the left single-quoted string, a literal single quote char,
/// and the right single-quoted string.
fn escape_single_quotes(command: &str) -> String {
    command.replace('\'', r#"'"'"'"#)
}
