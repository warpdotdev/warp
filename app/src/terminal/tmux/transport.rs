use std::ffi::OsString;
use std::path::PathBuf;

use warp_core::SessionId;
use warp_terminal::shell::ShellType;
use warp_terminal::tmux::WARP_CONTROL_SOCKET_NAME;

use super::protocol::{PaneBootstrap, control_client_argv, in_place_pane_spawn};

/// How a tmux `-CC` byte stream is obtained.
///
/// Product path is in-place on the **current** shell PTY: the user (or `/tmux`) runs
/// `tmux -CC` in the active local or already-remote shell, the event loop detects
/// `DCS 1000p`, and that same writable PTY becomes the control stream. Locality is
/// implicit in whichever shell is attached.
///
/// [`Self::LocalDedicated`] is only a feature-flagged test harness that spawns a
/// private socket. It must not define product ownership, and a sibling SSH
/// ControlMaster exec is not the remote product path.
#[derive(Debug, Clone)]
pub enum ControlTransportSpec {
    /// Test harness: `tmux -CC` on a Warp-owned local socket.
    LocalDedicated {
        tmux_path: PathBuf,
        socket: PathBuf,
        config: PathBuf,
        bootstrap: PaneBootstrap,
        columns: usize,
        rows: usize,
    },
}

impl ControlTransportSpec {
    pub fn spawn_argv(&self) -> Vec<OsString> {
        match self {
            Self::LocalDedicated {
                tmux_path,
                socket,
                config,
                bootstrap,
                columns,
                rows,
            } => control_client_argv(tmux_path, socket, config, bootstrap, *columns, *rows),
        }
    }
}

const DEFAULT_SESSION: &str = "warp";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TmuxCommandError {
    IsolatedSocketOverride,
}

type TmuxCcArgv = (Vec<String>, Option<SessionId>, Option<String>);

/// Shell command written to the **active** PTY to enter or resume control mode.
///
/// `-A` attaches if `session_name` already exists so SSH reconnect can rediscover
/// the Warp-managed session instead of creating a second one. Managed sessions
/// always use the dedicated `-L warp-control-v1` server.
pub fn in_place_tmux_cc_command(
    session_name: &str,
    columns: usize,
    rows: usize,
    pane_shell: ShellType,
) -> (String, SessionId, Option<String>) {
    let (command, session_id, zsh_init) =
        tmux_cc_shell_command("", Some(session_name), columns, rows, Some(pane_shell))
            .expect("bare /tmux never overrides the Warp socket");
    (
        command,
        session_id.expect("bare /tmux always includes a pane spawn"),
        zsh_init,
    )
}

/// Build a `tmux -CC -L warp-control-v1 …` command from `/tmux` arguments.
pub fn tmux_cc_shell_command(
    user_args: &str,
    default_session: Option<&str>,
    columns: usize,
    rows: usize,
    pane_shell: Option<ShellType>,
) -> Result<(String, Option<SessionId>, Option<String>), TmuxCommandError> {
    let tokens = tokenize_args(user_args);
    let (argv, expected_session_id, zsh_init) = tmux_cc_argv(
        &tokens,
        default_session.unwrap_or(DEFAULT_SESSION),
        columns,
        rows,
        pane_shell,
    )?;
    let mut command = shell_join(&argv);
    command.push('\n');
    Ok((command, expected_session_id, zsh_init))
}

pub fn tmux_cc_argv(
    user_tokens: &[String],
    default_session: &str,
    columns: usize,
    rows: usize,
    pane_shell: Option<ShellType>,
) -> Result<TmuxCcArgv, TmuxCommandError> {
    if user_tokens.iter().any(|token| is_socket_override(token)) {
        return Err(TmuxCommandError::IsolatedSocketOverride);
    }
    let (globals, command) = split_tmux_globals(user_tokens)?;
    let mut argv = vec![
        "tmux".to_owned(),
        "-CC".to_owned(),
        "-L".to_owned(),
        WARP_CONTROL_SOCKET_NAME.to_owned(),
    ];
    argv.extend(
        globals
            .into_iter()
            .filter(|token| token != "-CC" && token != "-C"),
    );
    if command.is_empty() {
        argv.extend([
            "new-session".to_owned(),
            "-A".to_owned(),
            "-s".to_owned(),
            default_session.to_owned(),
            "-n".to_owned(),
            "warp".to_owned(),
            "-x".to_owned(),
            columns.to_string(),
            "-y".to_owned(),
            rows.to_string(),
        ]);
        let (expected_session_id, zsh_init) = append_in_place_pane_spawn(&mut argv, pane_shell);
        return Ok((argv, expected_session_id, zsh_init));
    }
    let mut command = command;
    if command[0] == "attach" {
        command[0] = "attach-session".to_owned();
    } else if command[0] == "new" {
        command[0] = "new-session".to_owned();
    }
    if command[0] == "new-session" {
        maybe_insert_size(&mut command, columns, rows);
    }
    argv.extend(command);
    Ok((argv, None, None))
}

fn append_in_place_pane_spawn(
    argv: &mut Vec<String>,
    pane_shell: Option<ShellType>,
) -> (Option<SessionId>, Option<String>) {
    let Some(shell_type) = pane_shell else {
        return (None, None);
    };
    let bootstrap = in_place_pane_spawn(shell_type);
    argv.push("--".to_owned());
    argv.extend(
        bootstrap
            .command_argv()
            .into_iter()
            .map(|arg| arg.to_string_lossy().into_owned()),
    );
    (Some(bootstrap.session_id), bootstrap.init_script)
}

fn split_tmux_globals(tokens: &[String]) -> Result<(Vec<String>, Vec<String>), TmuxCommandError> {
    let mut globals = Vec::new();
    let mut i = 0;
    while i < tokens.len() {
        let token = &tokens[i];
        if token == "--" {
            return Ok((globals, tokens[i + 1..].to_vec()));
        }
        if is_socket_override(token) {
            return Err(TmuxCommandError::IsolatedSocketOverride);
        }
        if matches!(
            token.as_str(),
            "-2" | "-C" | "-CC" | "-D" | "-l" | "-N" | "-u" | "-v" | "-V"
        ) {
            globals.push(token.clone());
            i += 1;
            continue;
        }
        if matches!(token.as_str(), "-c" | "-f" | "-T") {
            globals.push(token.clone());
            if let Some(value) = tokens.get(i + 1) {
                globals.push(value.clone());
                i += 2;
            } else {
                i += 1;
            }
            continue;
        }
        break;
    }
    Ok((globals, tokens[i..].to_vec()))
}

fn maybe_insert_size(command: &mut Vec<String>, columns: usize, rows: usize) {
    if !command.iter().any(|token| token == "-x") {
        command.extend(["-x".to_owned(), columns.to_string()]);
    }
    if !command.iter().any(|token| token == "-y") {
        command.extend(["-y".to_owned(), rows.to_string()]);
    }
}

fn is_socket_override(token: &str) -> bool {
    matches!(
        token,
        "-L" | "-S" | "--socket" | "--socket-name" | "--tmux-socket"
    ) || (token.starts_with("-L") && token != "-L")
        || (token.starts_with("-S") && token != "-S")
        || token.starts_with("--socket=")
        || token.starts_with("--socket-name=")
}

fn tokenize_args(input: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    for ch in input.chars() {
        match quote {
            Some(q) if ch == q => quote = None,
            Some(_) => current.push(ch),
            None if ch == '\'' || ch == '"' => quote = Some(ch),
            None if ch.is_whitespace() => {
                if !current.is_empty() {
                    tokens.push(std::mem::take(&mut current));
                }
            }
            None => current.push(ch),
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

fn shell_join(argv: &[String]) -> String {
    argv.iter()
        .map(|token| shell_quote(token))
        .collect::<Vec<_>>()
        .join(" ")
}

fn shell_quote(token: &str) -> String {
    if token.is_empty() {
        return "''".to_owned();
    }
    if token.bytes().all(|b| {
        b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'/' | b'%' | b'@' | b':')
    }) {
        return token.to_owned();
    }
    format!("'{}'", token.replace('\'', "'\\''"))
}

#[cfg(test)]
#[path = "transport_tests.rs"]
mod tests;
