//! The environment a hook process runs in. It is built in two layers so the two parties that know
//! the inputs can each contribute theirs: the runner supplies the host layer (its own process
//! environment under the scrub policy, the home directory, the captured login shell `PATH`, and
//! the protocol version), and the dispatcher supplies the per-invocation layer (the hook-session
//! variables and the protected identity variables, which are set last so nothing can override
//! them). [`build_environment`] is the whole policy in one call.

use std::ffi::{OsStr, OsString};
use std::path::PathBuf;

use crate::hooks::model::HookFormat;

/// Variables Warp's own process may carry that a hook must not inherit: every credential Warp
/// reads from its own environment (the CLI API key, the workload token a cloud run uses to fetch
/// its secrets and forge credentials, and the tracing token), and the session identity and
/// shell-bootstrap switches an enclosing Warp terminal exports to the shells it starts (present
/// when Warp itself was launched from inside a Warp tab). A credential Warp learns to read from
/// its environment belongs here.
pub const HOST_SCRUB_LIST: &[&str] = &[
    "WARP_API_KEY",
    "WARP_WORKLOAD_TOKEN",
    "WARP_CLOUD_AGENT_OTLP_TOKEN",
    "WARP_CLOUD_AGENT_OTLP_TOKEN_EXPIRES_AT",
    "WARP_IS_LOCAL_SHELL_SESSION",
    "WARP_HONOR_PS1",
    "WARP_USE_SSH_WRAPPER",
    "WARP_SSH_REUSE_CONTROL_MASTER",
    "WARP_SHELL_DEBUG_MODE",
    "WARP_PROMPT_NODE_VERSION_ENABLED",
    "WARP_PATH_APPEND",
    "WARP_INITIAL_WORKING_DIR",
    "WARP_INITIAL_HISTFILESIZE",
    "WARP_INITIAL_HISTSIZE",
    "WARP_TERMINAL_SESSION_UUID",
    "WARP_FOCUS_URL",
    "WARP_CLIENT_VERSION",
    "WARP_CLI_AGENT_PROTOCOL_VERSION",
    "SSH_SOCKET_DIR",
];

/// Name prefixes removed from the host environment. `WARP_METADATA_*` is where a worker injects
/// run metadata, which managed secrets can be stored under.
pub const HOST_SCRUB_PREFIXES: &[&str] = &["WARP_METADATA_"];

/// Variables only Warp sets. They are removed from the host environment, dropped from the
/// hook-session environment, and written last, so a hook sees Warp's own value or nothing.
pub const PROTECTED_NAMES: &[&str] = &[
    "WARP_PROJECT_DIR",
    "WARP_VERSION",
    "WARP_HOOK_PROTOCOL_VERSION",
    "WARP_RUN_ID",
    "WARP_USER_EMAIL",
    "WARP_CODE_REMOTE",
    "CURSOR_PROJECT_DIR",
    "CURSOR_VERSION",
    "CURSOR_USER_EMAIL",
    "CURSOR_CODE_REMOTE",
    "CURSOR_TRANSCRIPT_PATH",
    "CLAUDE_PROJECT_DIR",
];

/// What the runner itself knows about the host it runs hooks on.
#[derive(Debug, Clone, Default)]
pub struct RunnerEnvironment {
    /// The runner's own process environment.
    pub variables: Vec<(OsString, OsString)>,
    /// The login shell `PATH` captured by the application, when it has one.
    pub login_path: Option<OsString>,
    /// The home directory to give the hook; when `None`, an inherited `HOME` is left alone.
    pub home: Option<PathBuf>,
    /// The stdin and stdout contract the runner speaks.
    pub protocol_version: u32,
}

/// What one invocation knows about the hook and the run it belongs to.
#[derive(Debug, Clone)]
pub struct HookIdentity {
    /// The format of the hook's source. Vendor aliases follow it, not where the file lives.
    pub format: HookFormat,
    pub project_dir: PathBuf,
    pub warp_version: String,
    pub run_id: Option<String>,
    pub user_email: Option<String>,
    /// Whether the run's workspace is remote from the user's machine.
    pub remote: bool,
    /// Variables earlier hooks in this conversation asked to set for later ones.
    pub session_env: Vec<(String, String)>,
}

/// Everything [`build_environment`] needs.
#[derive(Debug, Clone)]
pub struct EnvironmentInput {
    pub runner: RunnerEnvironment,
    pub hook: HookIdentity,
}

/// The complete environment for one hook process: the host layer, then the invocation layer.
pub fn build_environment(input: EnvironmentInput) -> Vec<(OsString, OsString)> {
    let mut variables = host_environment(&input.runner);
    for (name, value) in invocation_environment(&input.hook) {
        set(&mut variables, name, value);
    }
    variables
}

/// The host layer: the runner's process environment without the scrubbed and protected names,
/// then `HOME`, `PATH`, and the protocol version.
pub fn host_environment(runner: &RunnerEnvironment) -> Vec<(OsString, OsString)> {
    let mut variables = runner
        .variables
        .iter()
        .filter(|(name, _)| !is_scrubbed(name) && !is_protected(name))
        .cloned()
        .collect::<Vec<_>>();
    if let Some(home) = &runner.home {
        set(&mut variables, "HOME".into(), home.clone().into_os_string());
    }
    if let Some(path) = &runner.login_path {
        set(&mut variables, "PATH".into(), path.clone());
    }
    set(
        &mut variables,
        "WARP_HOOK_PROTOCOL_VERSION".into(),
        runner.protocol_version.to_string().into(),
    );
    variables
}

/// The invocation layer: the hook-session variables, then the protected identity variables and
/// the format's aliases. A session variable with a protected or unusable name is dropped.
pub fn invocation_environment(hook: &HookIdentity) -> Vec<(OsString, OsString)> {
    let mut variables = Vec::new();
    for (name, value) in &hook.session_env {
        if is_protected(OsStr::new(name)) {
            log::warn!("hook-session variable {name} was ignored: Warp sets it");
            continue;
        }
        if !is_usable_name(name) || value.contains('\0') {
            log::warn!("hook-session variable {name:?} was ignored: not a valid variable");
            continue;
        }
        set(&mut variables, name.into(), value.into());
    }

    let project_dir = hook.project_dir.clone().into_os_string();
    set(
        &mut variables,
        "WARP_PROJECT_DIR".into(),
        project_dir.clone(),
    );
    set(
        &mut variables,
        "WARP_VERSION".into(),
        hook.warp_version.clone().into(),
    );
    if let Some(run_id) = &hook.run_id {
        set(&mut variables, "WARP_RUN_ID".into(), run_id.into());
    }
    if let Some(email) = &hook.user_email {
        set(&mut variables, "WARP_USER_EMAIL".into(), email.into());
    }
    if hook.remote {
        set(&mut variables, "WARP_CODE_REMOTE".into(), "true".into());
    }

    match hook.format {
        HookFormat::Cursor => {
            set(
                &mut variables,
                "CURSOR_PROJECT_DIR".into(),
                project_dir.clone(),
            );
            set(
                &mut variables,
                "CURSOR_VERSION".into(),
                hook.warp_version.clone().into(),
            );
            if let Some(email) = &hook.user_email {
                set(&mut variables, "CURSOR_USER_EMAIL".into(), email.into());
            }
            if hook.remote {
                set(&mut variables, "CURSOR_CODE_REMOTE".into(), "true".into());
            }
            set(&mut variables, "CLAUDE_PROJECT_DIR".into(), project_dir);
        }
    }
    variables
}

fn is_scrubbed(name: &OsStr) -> bool {
    HOST_SCRUB_LIST
        .iter()
        .any(|scrubbed| names_match(name, scrubbed))
        || HOST_SCRUB_PREFIXES
            .iter()
            .any(|prefix| name_has_prefix(name, prefix))
}

fn is_protected(name: &OsStr) -> bool {
    PROTECTED_NAMES
        .iter()
        .any(|protected| names_match(name, protected))
}

/// A name the operating system accepts for an environment variable.
fn is_usable_name(name: &str) -> bool {
    !name.is_empty() && !name.contains('=') && !name.contains('\0')
}

/// Replaces any existing variable called `name`, so every name appears once and the last writer
/// wins regardless of the order the layers were applied in.
fn set(variables: &mut Vec<(OsString, OsString)>, name: OsString, value: OsString) {
    variables.retain(|(existing, _)| !names_match(existing, &name.to_string_lossy()));
    variables.push((name, value));
}

/// Variable names are case-insensitive on Windows and case-sensitive everywhere else.
fn names_match(name: &OsStr, other: &str) -> bool {
    if cfg!(windows) {
        name.to_str()
            .is_some_and(|name| name.eq_ignore_ascii_case(other))
    } else {
        name == other
    }
}

fn name_has_prefix(name: &OsStr, prefix: &str) -> bool {
    name.to_str().is_some_and(|name| {
        if cfg!(windows) {
            name.get(..prefix.len())
                .is_some_and(|head| head.eq_ignore_ascii_case(prefix))
        } else {
            name.starts_with(prefix)
        }
    })
}

#[cfg(test)]
#[path = "env_tests.rs"]
mod tests;
