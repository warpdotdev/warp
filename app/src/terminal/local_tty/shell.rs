use itertools::Itertools as _;
use serde::{Deserialize, Serialize};
use std::{
    ffi::OsString,
    io,
    path::{Path, PathBuf},
    process,
};
use typed_path::UnixPathBuf;
use warp_core::channel::{Channel, ChannelState};
use warp_util::path::{canonicalize_git_bash_path, is_msys2_path, warp_shell_path};

use crate::{
    terminal::{
        available_shells::AvailableShell,
        bootstrap::init_shell_script_for_shell,
        local_tty::docker_sandbox::DockerSandboxShellStarter,
        shell::{ShellName, ShellType},
        ShellLaunchData,
    },
    util::path::resolve_executable,
};

#[cfg(windows)]
use crate::util::windows::{powershell_5_path, powershell_7_path, wsl_path};

pub const ZSH_SHELL_PATH: &str = "/bin/zsh";
pub const BASH_SHELL_PATH: &str = "/bin/bash";
pub const FISH_SHELL_PATH: &str = "/bin/fish";

/// Returns an iterator of additional PATH entries to append to the shell's PATH.
/// * On macOS, this includes `$APP_PATH/Contents/Resources/bin`, in which we put a wrapper around the Warp CLI.
/// * On all other platforms, this is empty.
pub fn extra_path_entries() -> impl Iterator<Item = PathBuf> {
    cfg_if::cfg_if! {
        if #[cfg(target_os = "macos")] {
            use itertools::Either;

            if let Some(resources_path) = warp_core::paths::bundled_resources_dir() {
                let bin_path = resources_path.join("bin");
                Either::Left(std::iter::once(bin_path))
            } else {
                Either::Right(std::iter::empty())
            }
        } else {
            std::iter::empty()
        }
    }
}

/// Returns `true` if the given `path_or_command` is a valid, executable command or path to a
/// executable binary for one of Warp's supported shell types (bash, fish, zsh).
pub fn is_valid_path_or_command_for_supported_shell(path_or_command: &str) -> bool {
    supported_shell_path_and_type(path_or_command).is_some()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ShellStarter {
    /// Bootstrap the shell directly.
    Direct(DirectShellStarter),
    /// Bootstrap the shell through WSL.
    Wsl(WslShellStarter),
    MSYS2(DirectShellStarter),
    /// Bootstrap a shell running inside a Docker sandbox via `sbx run`.
    /// The final `sbx` args are computed at PTY spawn time so we can include
    /// the resolved workspace path, read-only init-script mount, and base
    /// Docker image (`--template <base_image>`).
    DockerSandbox(DockerSandboxShellStarter),
}

impl ShellStarter {
    /// Constructs a `ShellStarter` represent the shell binary (and corresponding arguments) to be
    /// used to spawn a shell process for a new top-level Warp session. If a WSL Distribution is
    /// given, then it will always construct a `ShellStarter` starting the default shell for that
    /// WSL Distribution.
    ///
    /// Returns an enum indicating the source from which the shell was determined. If the fallback
    /// default shell is used, also includes the requested but unsupported shell information.
    pub fn init(preferred_shell: AvailableShell) -> Option<ShellStarterSourceOrWslName> {
        if let Some(launch_data) = preferred_shell.get_valid_shell_path_and_type() {
            match launch_data {
                ShellLaunchData::Executable {
                    executable_path,
                    shell_type,
                } => {
                    if cfg!(windows) {
                        let executable_path = canonicalize_git_bash_path(executable_path.clone());
                        if is_msys2_path(&executable_path) {
                            return Some(
                                ShellStarterSource::Override(ShellStarter::MSYS2(
                                    DirectShellStarter {
                                        args: msys2_arguments_for_session_spawning_command(
                                            shell_type,
                                        ),
                                        shell_path: executable_path,
                                        shell_type,
                                    },
                                ))
                                .into(),
                            );
                        }
                    }
                    return Some(
                        ShellStarterSource::Override(ShellStarter::Direct(DirectShellStarter {
                            args: arguments_for_session_spawning_command(
                                executable_path.to_string_lossy().as_ref(),
                                shell_type,
                            ),
                            shell_path: executable_path,
                            shell_type,
                        }))
                        .into(),
                    );
                }
                ShellLaunchData::WSL { distro } => {
                    return Some(ShellStarterSourceOrWslName::WSLName {
                        distro_name: distro,
                    })
                }
                ShellLaunchData::MSYS2 {
                    executable_path,
                    shell_type,
                } => {
                    return Some(
                        ShellStarterSource::Override(ShellStarter::MSYS2(DirectShellStarter {
                            args: msys2_arguments_for_session_spawning_command(shell_type),
                            shell_path: executable_path,
                            shell_type,
                        }))
                        .into(),
                    )
                }
                ShellLaunchData::DockerSandbox {
                    sbx_path,
                    base_image,
                } => {
                    // The sandbox runs `sbx` on the host; the actual shell
                    // lives inside the container (conventionally bash). We
                    // still thread a `DirectShellStarter` with `shell_type =
                    // Bash` through so existing code that asks for the
                    // "shell type" of the session gets a sensible answer.
                    return Some(
                        ShellStarterSource::Override(ShellStarter::DockerSandbox(
                            DockerSandboxShellStarter::new(
                                DirectShellStarter {
                                    args: Vec::new(),
                                    shell_path: sbx_path,
                                    shell_type: ShellType::Bash,
                                },
                                base_image,
                            ),
                        ))
                        .into(),
                    );
                }
            }
        }

        if let Some(warp_shell_env_var) = warp_shell_path() {
            let (warp_shell_path, shell_type) = supported_shell_path_and_type(&warp_shell_env_var)
                .unwrap_or_else(|| {
                    panic!("Cannot spawn shell; $WARP_SHELL_PATH is invalid: {warp_shell_env_var}")
                });
            return Some(
                ShellStarterSource::Environment(DirectShellStarter {
                    args: arguments_for_session_spawning_command(
                        warp_shell_path.as_path().to_string_lossy().as_ref(),
                        shell_type,
                    ),
                    shell_path: warp_shell_path,
                    shell_type,
                })
                .into(),
            );
        }

        Self::compute_fallback_shell().map(|fallback_shell| fallback_shell.into())
    }

    fn compute_fallback_shell() -> Option<ShellStarterSource> {
        cfg_if::cfg_if! {
            if #[cfg(unix)] {
                let pw_shell_path = nix::unistd::User::from_uid(nix::unistd::getuid())
                    .expect("should not fail to read user information")
                    .expect("current user should exist")
                    .shell
                    .display()
                    .to_string();
                if let Some((resolved_pw_shell_path, shell_type)) =
                    supported_shell_path_and_type(&pw_shell_path)
                {
                    return Some(ShellStarterSource::UserDefault(DirectShellStarter {
                        args: arguments_for_session_spawning_command(
                            resolved_pw_shell_path.as_path().to_string_lossy().as_ref(),
                            shell_type,
                        ),
                        shell_path: resolved_pw_shell_path,
                        shell_type,
                    }));
                }
                let unsupported_shell = Some(pw_shell_path);

                let (resolved_default_shell_path, shell_type) = if let Some(shell_path_and_type) =
                    supported_shell_path_and_type(ZSH_SHELL_PATH)
                {
                    shell_path_and_type
                } else if let Some(shell_path_and_type) = supported_shell_path_and_type(BASH_SHELL_PATH) {
                    shell_path_and_type
                } else if let Some(shell_path_and_type) = supported_shell_path_and_type(FISH_SHELL_PATH) {
                    shell_path_and_type
                } else {
                    log::warn!("Did not find valid binaries when attempting to load fallback shell (not bash, fish, or zsh).");
                    return None;
                };

                Some(ShellStarterSource::Fallback {
                    unsupported_shell,
                    starter: DirectShellStarter {
                        args: arguments_for_session_spawning_command(
                            resolved_default_shell_path.as_path().to_string_lossy().as_ref(),
                            shell_type,
                        ),
                        shell_path: resolved_default_shell_path,
                        shell_type,
                    },
                })
            } else if #[cfg(target_os = "windows")] {
                let (resolved_default_shell_path, shell_type) = if let Some(shell_path_and_type) = powershell_7_path().and_then(|path| parse_shell_type_from_path(path)) {
                    shell_path_and_type
                } else if let Some(shell_path_and_type) = powershell_5_path().and_then(|path| parse_shell_type_from_path(path)) {
                    shell_path_and_type
                } else if let Some(shell_path_and_type) = wsl_path().and_then(|path| parse_shell_type_from_path(path)) {
                    shell_path_and_type
                } else {
                    // TODO(PLAT-807): Consider adding Command Prompt as a fallback shell.
                    log::warn!("Did not find valid binaries when attempting to load fallback shell (not PowerShell or WSL).");
                    return None;
                };

                Some(ShellStarterSource::UserDefault(DirectShellStarter {
                    args: arguments_for_session_spawning_command(
                        resolved_default_shell_path.as_path().to_string_lossy().as_ref(),
                        shell_type,
                    ),
                    shell_path: resolved_default_shell_path,
                    shell_type,
                }))
            }
        }
    }

    pub fn shell_type(&self) -> ShellType {
        match self {
            ShellStarter::Direct(starter) | ShellStarter::MSYS2(starter) => starter.shell_type(),
            ShellStarter::DockerSandbox(starter) => starter.shell_type(),
            ShellStarter::Wsl(starter) => starter.shell_type(),
        }
    }

    pub fn is_msys2(&self) -> bool {
        matches!(self, ShellStarter::MSYS2(_))
    }

    pub fn is_docker_sandbox(&self) -> bool {
        matches!(self, ShellStarter::DockerSandbox(_))
    }

    fn display_name(&self) -> &str {
        match self {
            Self::Direct(starter) => starter.display_name(),
            Self::DockerSandbox(starter) => starter.display_name(),
            Self::Wsl(starter) => starter.distribution(),
            Self::MSYS2(starter) => {
                if starter
                    .logical_shell_path()
                    .iter()
                    .any(|component| component.eq_ignore_ascii_case("git"))
                {
                    "Git Bash"
                } else {
                    starter.display_name()
                }
            }
        }
    }

    /// How to present this data to the user for error messages.
    #[cfg(windows)]
    pub(super) fn shell_detail(&self) -> String {
        match self {
            Self::Direct(starter) | Self::MSYS2(starter) => {
                starter.logical_shell_path().to_string_lossy().into_owned()
            }
            Self::DockerSandbox(starter) => {
                starter.logical_shell_path().to_string_lossy().into_owned()
            }
            Self::Wsl(starter) => starter.distribution().to_owned(),
        }
    }
}

/// Wraps up a shell type and the command to start it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DirectShellStarter {
    shell_type: ShellType,
    shell_path: PathBuf,

    /// Arguments to be passed to the shell binary at [`shell_path`] when spawning a new Warp
    /// session.
    args: Vec<OsString>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WslShellStarter {
    shell_type: ShellType,
    shell_path: String,

    /// Arguments to be passed to the shell binary at [`shell_path`] when spawning a new Warp
    /// session.
    args: Vec<OsString>,
    distribution: String,
}

#[derive(Debug)]
pub enum ShellStarterSource {
    /// The user chose the path by setting a custom shell path in settings or selecting a WSL
    /// distribution.
    Override(ShellStarter),
    /// The user chose the path to the shell by setting the `WARP_SHELL_PATH` environment variable.
    Environment(DirectShellStarter),
    /// The default shell for the user (as indicated by the user's passwd entry on UNIX).
    /// On Windows, this an ordered list of shells hardcoded _by Warp_.
    UserDefault(DirectShellStarter),
    /// We weren't able to find a shell that could be bootstrapped for the user.
    Fallback {
        unsupported_shell: Option<String>,
        starter: DirectShellStarter,
    },
}

impl ShellStarterSource {
    #[cfg(test)]
    pub fn shell_type(&self) -> ShellType {
        match self {
            Self::Override(starter) => starter.shell_type(),
            Self::Environment(starter) => starter.shell_type(),
            Self::UserDefault(starter) => starter.shell_type(),
            Self::Fallback { starter, .. } => starter.shell_type(),
        }
    }

    fn display_name(&self) -> &str {
        match self {
            Self::Override(starter) => starter.display_name(),
            Self::Environment(starter) => starter.display_name(),
            Self::UserDefault(starter) => starter.display_name(),
            Self::Fallback { starter, .. } => starter.display_name(),
        }
    }
}

impl From<ShellStarterSource> for ShellStarter {
    fn from(value: ShellStarterSource) -> Self {
        match value {
            ShellStarterSource::Override(starter) => starter,
            ShellStarterSource::Environment(starter) => Self::Direct(starter),
            ShellStarterSource::UserDefault(starter) => Self::Direct(starter),
            ShellStarterSource::Fallback { starter, .. } => Self::Direct(starter),
        }
    }
}

/// A [`ShellStarterSource`] if a shell is not WSL or the name of a WSL distribution if this is a
/// WSL session.  
pub enum ShellStarterSourceOrWslName {
    Source(ShellStarterSource),
    WSLName { distro_name: String },
}

impl ShellStarterSourceOrWslName {
    /// Converts the [`ShellStarterSourceOrWslName`] to a [`ShellStarterSource`].
    /// For non WSL shells this is a trivial conversion and is synchronous.
    /// For WSL shells this requires starting a new WSL instance so we can compute the shell type,
    /// which can potentially be extremely latent.
    pub async fn to_shell_starter_source(self) -> Option<ShellStarterSource> {
        match self {
            ShellStarterSourceOrWslName::Source(source) => Some(source),
            ShellStarterSourceOrWslName::WSLName { distro_name } => {
                if let Some(wsl_shell_starter) =
                    WslShellStarter::init_from_wsl_distribution(distro_name.as_ref()).await
                {
                    return Some(ShellStarterSource::Override(ShellStarter::Wsl(
                        wsl_shell_starter,
                    )));
                }

                ShellStarter::compute_fallback_shell()
            }
        }
    }

    pub fn name(&self) -> ShellName {
        match self {
            ShellStarterSourceOrWslName::Source(shell_starter_source) => {
                ShellName::MoreDescriptive(shell_starter_source.display_name().to_owned())
            }
            ShellStarterSourceOrWslName::WSLName { distro_name } => {
                ShellName::LessDescriptive(distro_name.to_owned())
            }
        }
    }
}

impl From<ShellStarterSource> for ShellStarterSourceOrWslName {
    fn from(source: ShellStarterSource) -> Self {
        ShellStarterSourceOrWslName::Source(source)
    }
}

impl DirectShellStarter {
    #[cfg(test)]
    pub fn new_for_test(shell_type: ShellType, shell_path: PathBuf, args: Vec<OsString>) -> Self {
        Self {
            shell_type,
            shell_path,
            args,
        }
    }

    pub fn shell_path(&self) -> &Path {
        &self.shell_path
    }

    /// Returns the logical path to the shell binary referred to by this `ShellStarter's`
    /// `ShellPath`.
    pub fn logical_shell_path(&self) -> &Path {
        self.shell_path.as_ref()
    }

    pub fn shell_type(&self) -> ShellType {
        self.shell_type
    }

    pub fn args(&self) -> &Vec<OsString> {
        &self.args
    }

    pub(super) fn display_name(&self) -> &str {
        if self
            .shell_path
            .file_stem()
            .is_some_and(|stem| stem.eq_ignore_ascii_case("powershell"))
        {
            "Windows PowerShell"
        } else if self.shell_type == ShellType::PowerShell && cfg!(windows) {
            "PowerShell Core"
        } else {
            self.shell_type.name()
        }
    }
}

impl WslShellStarter {
    async fn init_from_wsl_distribution(distribution: &str) -> Option<Self> {
        // We store the path as a String because we can't easily store a Unix path on Windows.
        // This command can have a lot of latency because it might spin up a VM.
        let command_result = command::r#async::Command::new("wsl")
            .arg("--distribution")
            .arg(distribution)
            .arg("--shell-type")
            .arg("standard")
            .arg("--")
            .arg("printenv")
            .arg("SHELL")
            .output()
            .await;
        let shell_path = decode_wsl_path_result(command_result)?
            .to_string_lossy()
            .into_owned();

        // We don't need to check the validity of the path or the existence of the binary since
        // we get this information directly from a spun-up shell in WSL.
        let shell_type = if shell_path.contains("bash") {
            ShellType::Bash
        } else if shell_path.contains("zsh") {
            ShellType::Zsh
        } else if shell_path.contains("fish") {
            ShellType::Fish
        } else {
            log::warn!("The shell {shell_path:#} is not yet supported in WSL");
            return None;
        };

        let args =
            wsl_arguments_for_session_spawning_command(distribution, &shell_path, shell_type);

        Some(Self {
            shell_type,
            shell_path,
            args,
            distribution: distribution.to_string(),
        })
    }

    pub fn args(&self) -> &Vec<OsString> {
        &self.args
    }

    pub fn shell_type(&self) -> ShellType {
        self.shell_type
    }

    pub fn shell_path(&self) -> String {
        self.shell_path.clone()
    }

    pub fn wsl_command() -> OsString {
        "wsl".to_string().into()
    }

    pub fn distribution(&self) -> &str {
        &self.distribution
    }

    /// Gives the Windows path to the WSL home directory (e.g. `\\WSL$\home\user`).
    pub(super) fn home_directory(&self) -> Option<PathBuf> {
        let command_result = command::blocking::Command::new("wsl")
            .arg("--distribution")
            .arg(&self.distribution)
            .arg("--shell-type")
            .arg("standard")
            .arg("--")
            .arg("printenv")
            .arg("HOME")
            .output();
        let home_dir =
            decode_wsl_path_result(command_result).filter(|s| !s.as_bytes().is_empty())?;
        warp_util::path::convert_wsl_to_windows_host_path(
            &home_dir.to_typed_path(),
            &self.distribution,
        )
        .inspect_err(|err| log::error!("error conversion WSL home dir for host: {err:#}"))
        .ok()
    }
}

/// If the given `path_or_command` resolves to a supported shell binary, returns a tuple
/// containing the resolved path to the binary and the corresponding `ShellType`. Else, returns
/// None.
pub fn supported_shell_path_and_type(path_or_command: &str) -> Option<(PathBuf, ShellType)> {
    resolve_executable(path_or_command)
        .and_then(|resolved_path| parse_shell_type_from_path(resolved_path.as_ref()))
}

/// If the given `path` is a supported shell binary, returns a tuple containing
/// the path and the corresponding `ShellType`. This function does not validate
/// that the path exists or is executable.
fn parse_shell_type_from_path(path: &Path) -> Option<(PathBuf, ShellType)> {
    path.file_name()
        .and_then(|file_name| file_name.to_str().and_then(ShellType::from_name))
        .map(|shell_type| (path.to_path_buf(), shell_type))
}

fn arguments_for_session_spawning_command(
    resolved_shell_path: &str,
    shell_type: ShellType,
) -> Vec<OsString> {
    // Note we typically go through bash so that we can launch the user's shell
    // with a leading '-', making it a login shell.
    match shell_type {
        ShellType::Zsh => {
            // The --no-rcs option executes the minimal level of startup files so we can
            // take over. The one exception: "Commands are first read from /etc/zshenv; this cannot be overridden."
            // The -g option sets the HIST_IGNORE_SPACE option, which ignores a command from history if it
            // begins with a space. We use this to hide Warp bootstrap commands from the history.
            vec![
                "-c".to_owned().into(),
                format!("exec -a -zsh '{resolved_shell_path}' -g --no-rcs").into(),
            ]
        }
        ShellType::Bash => {
            /*
             * There are many layers of bash happening here
             * 1. We pass the command we want to be running to bash -c to ensure the shell
             * is interpreting the arguments, rather than passing literal strings
             * 2. Make a call to exec when we launch the subshell. From FreeBSD porter's
             * handbook:  "The exec statement replaces the shell process with the
             * specified program. If exec is omitted, the shell process remains
             * in memory while the program is executing, and needlessly consumes system resources."
             * 3. The rcfile option reads the startup script from a file
             * 4. Process substitution i.e. <() send the output of a process via
             * /dev/fd/<n> (or temp files if this is unavailable) to another process
             * 5. Send an InitShell message to Warp through escape sequences.
             * The warp_send_message function is inlined here.
             * 6. We disable PS2 and the line editor to work around a gnarly bug involving
             * garbage being inserted in every line. We further disable PS1 and echo'ing
             * in order to show nothing to the user when we input characters. We later
             * restore the echo'ing in the bootstrap script.
             *
             * TODO(zheng) Add error handling
             */
            vec![
                "-c".to_owned().into(),
                // Keep this command up-to-date with the one in the bootstrap script
                // Notice the first level of escaping is the double-brackets in the macro string {{}}
                format!(
                    r#"exec -a bash '{}' --rcfile <(echo '{}')"#,
                    resolved_shell_path,
                    init_shell_script_for_shell(ShellType::Bash, &crate::ASSETS)
                )
                .into(),
            ]
        }
        ShellType::Fish => {
            // For now, we are going to plug the init cmd into single quotes.
            // Note it contains single quotes and there is no way to escape a single quote,
            // so instead we exit single quotes, emit an (escaped) quote, and re-enter them.
            //
            // TODO: we should eventually refactor this and build the init cmd up so that
            // we don't need to do complicated escaping.
            // We should also probably store the hex encoded json as a static string
            // rather than computing it at runtime for each shell we start.
            vec![
                // fish sources configuration files whenever it's invoked,
                // including in non-interactive mode (i.e. '-c'). This differs
                // from other shells like zsh, so we have to explicitly tell fish
                // to not source config files.
                //
                // There's an open GH issue against fish contesting this behaviour:
                // https://github.com/fish-shell/fish-shell/issues/5394.
                "--no-config".to_owned().into(),
                "-c".to_owned().into(),
                format!(
                    // We do _not_ specify `--no-config` here because
                    // we want fish to source config files for us (we don't
                    // manually do so in the bootstrap script like we do for zsh, for example).
                    // `-f no-mark-prompt` disables OSC 133 (the non-standard FinalTerm escape codes).
                    // Fish's implementation of this breaks Warp by emitting `OSC 133 A` but not
                    // `OSC 133 B` afterwards, which we have assumed. This is a temporary workaround.
                    // See this issue: https://github.com/warpdotdev/Warp/issues/7588
                    r#"exec '{}' -f no-mark-prompt --login --init-command '{}'"#,
                    resolved_shell_path,
                    init_shell_script_for_shell(ShellType::Fish, &crate::ASSETS)
                )
                .into(),
            ]
        }
        ShellType::PowerShell => vec![
            // When PowerShell starts a session, it writes "PowerShell <version>" to the PTY. This
            // option suppresses that message.
            "-NoLogo".to_owned().into(),
            // Skip RC files. We load these manually later.
            "-NoProfile".to_owned().into(),
            // Normally, passing the "-Command" option causes the shell to exit after executing
            // those commands. Passing "-NoExit" suppresses that so PowerShell remains interactive
            // afterwards.
            "-NoExit".to_owned().into(),
            // This arg must be last, as everything positioned after the "-Command" flag is treated
            // as the value for this arg.
            "-Command".to_owned().into(),
            init_shell_script_for_shell(ShellType::PowerShell, &crate::ASSETS).into(),
        ],
    }
}

fn wsl_arguments_for_session_spawning_command(
    distribution: &str,
    shell_path: &str,
    shell_type: ShellType,
) -> Vec<OsString> {
    let mut args = vec![
        "--distribution".into(),
        distribution.into(),
        "--shell-type".into(),
        "standard".into(),
        "--exec".into(),
        shell_path.into(),
    ];
    // Note we typically go through bash so that we can launch the user's shell
    // with a leading '-', making it a login shell.
    match shell_type {
        ShellType::Bash | ShellType::Zsh | ShellType::Fish => {
            args.extend(arguments_for_session_spawning_command(
                shell_path, shell_type,
            ));
            args
        }
        _ => todo!("We don't yet support bootstrapping {shell_type:?} on WSL"),
    }
}

fn msys2_arguments_for_session_spawning_command(shell_type: ShellType) -> Vec<OsString> {
    match shell_type {
        ShellType::Zsh => {
            vec!["-g".to_string().into(), "--no-rcs".to_string().into()]
        }
        ShellType::Bash => {
            vec![
                "--noprofile".to_string().into(),
                "--norc".to_string().into(),
            ]
        }
        ShellType::Fish => {
            vec![
                "--login".to_string().into(),
                "--no-config".to_string().into(),
            ]
        }
        ShellType::PowerShell => panic!("MSYS2 not supported for PowerShell"),
    }
}

pub fn ssh_socket_dir() -> String {
    let mut socket_dir = if ChannelState::channel() == Channel::Integration {
        std::env::var("ORIGINAL_HOME").unwrap_or("~".into())
    } else {
        "~".into()
    };
    socket_dir.push_str("/.ssh");
    socket_dir
}

/// Take the output of a wsl.exe subcommand and try to decode it while reporting errors.
/// NOTE: The empty string Some("") may be returned.
fn decode_wsl_path_result(result: io::Result<process::Output>) -> Option<UnixPathBuf> {
    match result {
        Err(err) => {
            log::error!("error finding wsl.exe: {err:#}");
            None
        }
        Ok(output) => {
            if !output.status.success() {
                // Errors with wsl.exe usage itself outputs error messages in UTF-16.
                cfg_if::cfg_if! {
                    // WSL is Windows only, but most of the WSL code isn't cfg-guarded. This
                    // snipped does need to be guarded.
                    if #[cfg(windows)] {
                        use std::os::windows::ffi::OsStringExt as _;
                        let wsl_err_msg = OsString::from_wide(bytemuck::cast_slice(&output.stdout));
                    } else {
                        let wsl_err_msg = "";
                    }
                }
                // If wsl.exe was correctly invoked but the Linux command had an error, that will
                // be UTF-8.
                if wsl_err_msg.is_empty() {
                    if let Ok(inner_err_msg) = String::from_utf8(output.stderr) {
                        log::error!("Error from WSL command: {inner_err_msg}");
                    }
                } else {
                    log::error!("Error invoking wsl.exe: {wsl_err_msg:?}");
                }
                return None;
            }

            Some(UnixPathBuf::from(
                take_until_utf16_crlf(output.stdout)
                    .into_iter()
                    .take_while(|b| *b != b'\n')
                    .collect_vec(),
            ))
        }
    }
}

/// Takes bytes until [13, 0, 10, 0] is found in the byte sequence, dropping the rest.
///
/// This is useful for removing warning/error messages from successful wsl.exe commands. Even
/// successful invocations of wsl.exe may have warnings or errors appended to the end. In those
/// cases the format looks like:
/// 1. UTF-8 encoded output from the WSL distro.
/// 2. A UTF-16 encoded CRLF.
/// 3. A UTF-16 error message.
/// See this ticket for an example and why this is necessary:
/// https://linear.app/warpdotdev/issue/CORE-3539
fn take_until_utf16_crlf(bytes: Vec<u8>) -> Vec<u8> {
    const UTF16_CRLF: &[u8] = b"\r\0\n\0";
    match bytes
        .windows(UTF16_CRLF.len())
        .position(|bytes| bytes == UTF16_CRLF)
    {
        Some(index) => Vec::from(&bytes[0..index]),
        None => bytes,
    }
}

#[cfg(test)]
#[path = "shell_tests.rs"]
mod tests;
