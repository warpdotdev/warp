//! Context chips built into Warp

use chrono::Local;
use warp_util::path::user_friendly_path;

use crate::terminal::shell::ShellType;

use super::{
    context_chip::{GeneratorContext, ShellCommand, ShellCommandGenerator},
    ChipValue,
};

#[cfg(test)]
#[path = "builtins_tests.rs"]
mod tests;

/// Generator function for the current working directory.
pub fn working_directory(ctx: &GeneratorContext) -> Option<ChipValue> {
    let pwd = ctx.active_block_metadata.current_working_directory()?;
    let home_dir = ctx.active_session.and_then(|session| session.home_dir());
    Some(ChipValue::Text(
        user_friendly_path(pwd, home_dir).to_string(),
    ))
}

/// Generator function that always shows the username.
pub fn username(ctx: &GeneratorContext) -> Option<ChipValue> {
    ctx.active_session
        .map(|session| ChipValue::Text(session.user().to_owned()))
}

/// Generator function that always shows the host name.
pub fn hostname(ctx: &GeneratorContext) -> Option<ChipValue> {
    ctx.active_session
        .map(|session| ChipValue::Text(session.hostname().to_owned()))
}

/// Generator function that shows the current Python virtual environment.
pub fn virtual_environment(ctx: &GeneratorContext) -> Option<ChipValue> {
    ctx.current_environment
        .python_virtualenv()
        .cloned()
        .map(ChipValue::Text)
}

/// Generator function that shows the current Anaconda/conda environment.
pub fn conda_environment(ctx: &GeneratorContext) -> Option<ChipValue> {
    ctx.current_environment
        .conda_environment()
        .cloned()
        .map(ChipValue::Text)
}

/// Generator function that shows the current Node.js version.
pub fn node_version(ctx: &GeneratorContext) -> Option<ChipValue> {
    ctx.current_environment
        .node_version()
        .cloned()
        .map(ChipValue::Text)
}

/// Generator function that shows the current date.
pub fn date(_: &GeneratorContext) -> Option<ChipValue> {
    Some(ChipValue::Text(
        Local::now().format("%a %b %d %Y").to_string(),
    ))
}

/// Generator function that shows the current time in 12-hour format.
pub fn time12(_: &GeneratorContext) -> Option<ChipValue> {
    Some(ChipValue::Text(Local::now().format("%I:%M %P").to_string()))
}

/// Generator function that shows the current time in 24-hour format.
pub fn time24(_: &GeneratorContext) -> Option<ChipValue> {
    Some(ChipValue::Text(Local::now().format("%H:%M").to_string()))
}

/// Generator function that shows the current 12-hour time with seconds.
pub fn time12_with_seconds(_: &GeneratorContext) -> Option<ChipValue> {
    Some(ChipValue::Text(
        Local::now().format("%I:%M:%S %P").to_string(),
    ))
}

/// Generator function that shows the current 24-hour time with seconds.
pub fn time24_with_seconds(_: &GeneratorContext) -> Option<ChipValue> {
    Some(ChipValue::Text(Local::now().format("%H:%M:%S").to_string()))
}

/// Generator function for SSH session chip.
pub fn ssh_session(ctx: &GeneratorContext) -> Option<ChipValue> {
    let session = ctx.active_session?;
    if session.is_legacy_ssh_session()
        || matches!(
            session.session_type(),
            crate::terminal::model::session::SessionType::WarpifiedRemote { .. }
        )
    {
        let user = session.user();
        Some(ChipValue::Text(format!("{}@{}", user, session.hostname())))
    } else {
        None
    }
}

/// Generator function for Subshell session chip.
pub fn subshell(ctx: &GeneratorContext) -> Option<ChipValue> {
    let session = ctx.active_session?;
    let subshell_info = session.subshell_info().as_ref()?;

    let session_type = if let Some(env_var_collection_name) = &subshell_info.env_var_collection_name
    {
        env_var_collection_name.clone()
    } else {
        subshell_info
            .spawning_command
            .split_whitespace()
            .next()
            .unwrap_or("subshell")
            .to_string()
    };
    Some(ChipValue::Text(session_type))
}

/// Generator function that shows the current Git branch.
pub fn shell_git_branch() -> ShellCommandGenerator {
    // Note this command must stay in sync with how PrecmdValue::git_branch is generated in the
    // bootstrap scripts, at least until that is removed.
    const SH_COMMAND: &str = "GIT_OPTIONAL_LOCKS=0 git symbolic-ref --short HEAD 2> /dev/null || \
     GIT_OPTIONAL_LOCKS=0 git rev-parse --short HEAD 2> /dev/null";
    let pwsh_command = safe_git_powershell(
        "git symbolic-ref --short HEAD  2>$null; \
            if ($? -eq $false) { \
                git rev-parse --short HEAD 2>$null; \
            }",
    );

    let command = ShellCommand::shell_specific([
        (ShellType::PowerShell, pwsh_command),
        (ShellType::Bash, SH_COMMAND.to_string()),
        (ShellType::Zsh, SH_COMMAND.to_string()),
        (ShellType::Fish, SH_COMMAND.to_string()),
    ]);

    ShellCommandGenerator::new(command, Some(vec!["git".to_owned()]))
}

pub fn shell_other_git_branches() -> ShellCommandGenerator {
    const SH_COMMAND: &str = "git --no-optional-locks branch --no-color --sort=-committerdate; \
        printf '\\036\\n'; \
        git --no-optional-locks worktree list --porcelain";
    let pwsh_command = safe_git_powershell(
        "git --no-optional-locks branch --no-color --sort=-committerdate; \
        [char]30; \
        git --no-optional-locks worktree list --porcelain",
    );

    let command = ShellCommand::shell_specific([
        (ShellType::PowerShell, pwsh_command),
        (ShellType::Bash, SH_COMMAND.to_string()),
        (ShellType::Zsh, SH_COMMAND.to_string()),
        (ShellType::Fish, SH_COMMAND.to_string()),
    ]);

    ShellCommandGenerator::new(command, Some(vec!["git".to_owned()]))
}

/// Generator function to get summary of git diff (num files changed and num lines changed).
///
/// Used as a remote-session fallback when GitRepoStatusModel is unavailable.
pub fn shell_git_line_changes() -> ShellCommandGenerator {
    const GIT_COMMAND: &str =
        "GIT_OPTIONAL_LOCKS=0 git -c diff.autoRefreshIndex=false diff --shortstat HEAD";

    let command = ShellCommand::shell_specific([
        (ShellType::Bash, GIT_COMMAND.to_string()),
        (ShellType::Zsh, GIT_COMMAND.to_string()),
        (ShellType::Fish, GIT_COMMAND.to_string()),
        (
            ShellType::PowerShell,
            safe_git_powershell("git -c diff.autoRefreshIndex=false diff --shortstat HEAD"),
        ),
    ]);

    ShellCommandGenerator::new(command, Some(vec!["git".to_owned()]))
}

pub fn github_pull_request_url() -> ShellCommandGenerator {
    // `gh pr view` exits non-zero both when there is no PR for the current branch and when the
    // command actually fails. We inspect its output so that "no PR found" is treated as an empty
    // success, while auth/config/network failures still propagate as real failures.
    const SH_COMMAND: &str = include_str!("scripts/github_pull_request_prompt_chip.sh");
    const FISH_COMMAND: &str = include_str!("scripts/github_pull_request_prompt_chip.fish");
    const PWSH_COMMAND: &str = include_str!("scripts/github_pull_request_prompt_chip.ps1");

    let command = ShellCommand::shell_specific([
        (ShellType::PowerShell, PWSH_COMMAND.to_string()),
        (ShellType::Bash, SH_COMMAND.to_string()),
        (ShellType::Zsh, SH_COMMAND.to_string()),
        (ShellType::Fish, FISH_COMMAND.to_string()),
    ]);

    ShellCommandGenerator::new(command, Some(vec!["gh".to_owned(), "git".to_owned()]))
}

pub fn kubernetes_current_context() -> ShellCommandGenerator {
    ShellCommandGenerator::new(
        ShellCommand::portable("kubectl config current-context"),
        Some(vec!["kubectl".to_owned()]),
    )
}

/// Generator function that shows the current svn "branch".
/// Since svn uses directories for different branches and tags,
/// we take the latest directory of the working copy as the branch/tag name.
pub fn svn_branch_context() -> ShellCommandGenerator {
    const SH_COMMAND: &str = "basename $(svn info --show-item wc-root)";
    const PWSH_COMMAND: &str = "svn info --show-item wc-root | Split-Path -Leaf";
    let command = ShellCommand::shell_specific([
        (ShellType::PowerShell, PWSH_COMMAND.to_string()),
        (ShellType::Bash, SH_COMMAND.to_string()),
        (ShellType::Zsh, SH_COMMAND.to_string()),
        (ShellType::Fish, SH_COMMAND.to_string()),
    ]);

    ShellCommandGenerator::new(command, Some(vec!["svn".to_owned()]))
}

/// Generator function that shows the number of uncommitted svn files/directories.
pub fn svn_dirty_items() -> ShellCommandGenerator {
    const SH_COMMAND: &str = "count=$(svn status | wc -l) \
        && (( $count > 0 )) && echo $(( $count ))";
    const FISH_COMMAND: &str = "set count (svn status | wc -l) \
        && test $count -gt 0 && string trim $count";
    const PWSH_COMMAND: &str = "svn status | Measure-Object -line | \
        where {$_.Lines -gt 0 } | foreach { $_.Lines }";
    let command = ShellCommand::shell_specific([
        (ShellType::Bash, SH_COMMAND.to_string()),
        (ShellType::Zsh, SH_COMMAND.to_string()),
        (ShellType::Fish, FISH_COMMAND.to_string()),
        (ShellType::PowerShell, PWSH_COMMAND.to_string()),
    ]);
    ShellCommandGenerator::new(command, Some(vec!["svn".to_owned()]))
}

fn safe_git_powershell(cmd: &str) -> String {
    format!(
        "\
        $gitOptionalLocks = $env:GIT_OPTIONAL_LOCKS; \
        $env:GIT_OPTIONAL_LOCKS = 0; \
        try {{ \
            {cmd} \
        }} finally {{ \
            $success = $?; \
            $exitCode = $LASTEXITCODE; \
            $env:GIT_OPTIONAL_LOCKS = $gitOptionalLocks; \
            if ($exitCode -ne 0 -or -not $success) {{ \
                throw \
            }} \
        }}"
    )
}
