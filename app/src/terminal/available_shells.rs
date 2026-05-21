use std::borrow::Cow;
#[cfg(feature = "local_tty")]
use std::collections::{HashMap, HashSet};
#[cfg(feature = "local_tty")]
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;

#[cfg(feature = "local_tty")]
use settings::Setting as _;
#[cfg(feature = "local_tty")]
use warpui::{AppContext, ModelContext};
use warpui::{Entity, SingletonEntity};

#[cfg(feature = "local_tty")]
use crate::util::path::file_exists_and_is_executable;

use super::{
    session_settings::{NewSessionShell, StartupShell},
    shell::ShellType,
    ShellLaunchData,
};

#[derive(Debug, PartialEq, Eq, Hash)]
struct LocalConfig {
    command: String,
    executable_path: PathBuf,
    shell_type: ShellType,
}

impl TryFrom<StartupShell> for LocalConfig {
    type Error = ();

    #[cfg(feature = "local_tty")]
    fn try_from(value: StartupShell) -> Result<Self, Self::Error> {
        use crate::terminal::local_tty::shell::supported_shell_path_and_type;

        let command = value.shell_command().ok_or(())?;
        let (path, shell_type) = supported_shell_path_and_type(command).ok_or(())?;
        Ok(Self {
            command: command.to_string(),
            executable_path: path.to_path_buf(),
            shell_type,
        })
    }

    #[cfg(not(feature = "local_tty"))]
    fn try_from(_value: StartupShell) -> Result<Self, Self::Error> {
        Err(())
    }
}

/// The state for the AvailableShell model. Is kept private to the module, b/c we do not want people
/// manually destructuring or matching on this data.
#[derive(Debug, PartialEq, Eq, Hash)]
enum Config {
    SystemDefault,
    #[cfg_attr(not(feature = "local_tty"), allow(dead_code))]
    KnownLocal(LocalConfig),
    #[cfg_attr(not(feature = "local_tty"), allow(dead_code))]
    Wsl {
        distro: String,
    },
    #[cfg_attr(any(not(feature = "local_tty"), unix), allow(dead_code))]
    MSYS2(LocalConfig),
    #[cfg_attr(not(feature = "local_tty"), allow(dead_code))]
    Custom(LocalConfig),
    /// A shell running inside a Docker sandbox via `sbx run`.
    ///
    /// Unlike local shells, we don't pick the shell binary or path here: the
    /// shell that actually runs comes from the container image. This mirrors
    /// how [`Config::Wsl`] carries just the distro name and defers the shell
    /// choice to WSL itself.
    #[cfg_attr(not(feature = "local_tty"), allow(dead_code))]
    DockerSandbox {
        /// Path to the `sbx` CLI binary on the host.
        sbx_path: PathBuf,
        /// Base Docker image to use when creating the sandbox (passed as
        /// `sbx run --template <image>`). `None` means "use sbx's default
        /// image".
        base_image: Option<String>,
    },
}

// The concept of specifying an available shell does not exist on non-local filesystems. So we allow
// dead code so that the concept of the struct can exist, but remove any methods that do anything
// with it. That way, method calls can still take `Option<AvailableShell>` as an argument, but
// remote_ttys can just specify `None` for the value.
#[cfg_attr(not(feature = "local_tty"), allow(dead_code,))]
/// Contains the config describing a 'shell' that can be launched for a new session. Currently falls
/// into 4 categories:
/// - Known Local: A shell that is known to be installed on the local filesystem, and can be run
///   by invoking an executable.
/// - Known WSL: A WSL distro that can be launched by invoking WSL with a specific distro flag
/// - Custom: A user-specified custom executable that can be run locally.
/// - System Default: Uses the default shell for a given system.
///
/// All state is stored in an Arc so that it can be safely and easily copied. In general, unless you
/// are using a custom shell, you should not be constructing this struct directly. Instead, use the
/// methods available on the `AvailableShells` model.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct AvailableShell {
    id: Option<String>,
    state: Arc<Config>,
}

impl AvailableShell {
    /// Returns an "id" associated with a known shell. If
    /// shell1.is_some_and(|id1| shell2.is_some_and(|id2| id1 == id2)) holds,
    /// then shell1 and shell2 are the same shell.
    pub fn id(&self) -> Option<&str> {
        self.id.as_deref()
    }

    pub fn short_name(&self) -> Cow<'_, str> {
        match self.state.as_ref() {
            Config::SystemDefault => Cow::from("Default"),
            Config::KnownLocal(LocalConfig { command, .. })
            | Config::MSYS2(LocalConfig { command, .. }) => match command.as_str() {
                "bash" => Cow::from("Bash"),
                "zsh" => Cow::from("Zsh"),
                "fish" => Cow::from("Fish"),
                "pwsh" | "pwsh.exe" => Cow::from("PowerShell"),
                "powershell" | "powershell.exe" => Cow::from("Windows PowerShell"),
                _ => Cow::from(command),
            },
            Config::Wsl { distro } => Cow::from(distro),
            Config::Custom(_) => Cow::from("Custom"),
            Config::DockerSandbox { .. } => Cow::from("Docker Sandbox"),
        }
    }

    pub fn details(&self) -> Cow<'_, str> {
        match self.state.as_ref() {
            Config::SystemDefault => Cow::from("System default shell"),
            Config::KnownLocal(LocalConfig {
                executable_path, ..
            })
            | Config::MSYS2(LocalConfig {
                executable_path, ..
            }) => Cow::from(format!("{}", executable_path.display())),
            Config::Wsl { .. } => Cow::from("Windows Subsystem for Linux"),
            Config::Custom(LocalConfig {
                executable_path, ..
            }) => Cow::from(format!("Custom: {}", executable_path.display())),
            Config::DockerSandbox { .. } => Cow::from("Docker Sandbox"),
        }
    }

    /// Returns a telemetry string that represents the value of the shell without any PII involved.
    pub fn telemetry_value(&self) -> String {
        // NOTE: These explicitly do not log the shell path or wsl name, as those can contain PII.
        match self.state.as_ref() {
            Config::SystemDefault => "SystemDefault".to_string(),
            Config::KnownLocal(config) | Config::MSYS2(config) => config.command.clone(),
            Config::Wsl { .. } => "WSL".to_string(),
            Config::Custom(_) => "Custom".to_string(),
            Config::DockerSandbox { .. } => "DockerSandbox".to_string(),
        }
    }

    pub fn wsl_distro(&self) -> Option<String> {
        match self.state.as_ref() {
            Config::Wsl { distro } => Some(distro.clone()),
            _ => None,
        }
    }

    pub fn is_wsl(&self) -> bool {
        matches!(self.state.as_ref(), Config::Wsl { .. })
    }
}

#[cfg(feature = "local_tty")]
impl AvailableShell {
    /// The long name of the shell. For local shells, this includes the path to
    /// the executable.
    fn long_name(&self) -> String {
        match &self.state.as_ref() {
            Config::SystemDefault => "Default".to_string(),
            Config::KnownLocal(LocalConfig {
                executable_path, ..
            }) => format!("{} ({})", self.short_name(), executable_path.display()),
            Config::Wsl { distro } => distro.to_string(),
            Config::Custom(LocalConfig { command, .. }) => format!("Custom ({command})"),
            Config::MSYS2(LocalConfig {
                executable_path, ..
            }) => format!("{} ({})", self.short_name(), executable_path.display()),
            Config::DockerSandbox { .. } => "Docker Sandbox".to_string(),
        }
    }

    /// If the shell is a Custom shell, returns the custom shell path. Otherwise returns None. Can
    /// also be used to assert whether or not the shell is a custom shell using `is_some()`.
    pub fn get_custom_path(&self) -> Option<String> {
        if let Config::Custom(config) = self.state.as_ref() {
            Some(config.executable_path.display().to_string())
        } else {
            None
        }
    }

    fn matches_preference(&self, preference: &NewSessionShell) -> bool {
        match preference {
            NewSessionShell::SystemDefault => matches!(self.state.as_ref(), Config::SystemDefault),
            NewSessionShell::Executable(path) => {
                matches!(self.state.as_ref(), Config::KnownLocal(LocalConfig { executable_path, .. }) if executable_path == Path::new(path))
            }
            NewSessionShell::WSL(distro) => {
                matches!(self.state.as_ref(), Config::Wsl { distro: d } if d == distro)
            }
            NewSessionShell::Custom(path) => {
                matches!(self.state.as_ref(), Config::Custom(LocalConfig { executable_path, .. }) if executable_path == Path::new(path))
            }
            NewSessionShell::MSYS2(path) => {
                matches!(self.state.as_ref(), Config::MSYS2(LocalConfig{ executable_path, .. } ) if executable_path == Path::new(path))
            }
        }
    }

    /// Returns the launch data for a shell.
    ///
    /// Our conversion methodology is as follows:
    ///
    /// | [`AvailableShell`]         | Validation | [`Option<ShellLaunchData>`]        |
    /// |:---------------------------|:----------:|-----------------------------------:|
    /// | [`Config::SystemDefault`]  | No         | [`Option::None`]                   |
    /// | [`Config::KnownLocal`]     | Yes        | [`ShellLaunchData::Executable`]    |
    /// | [`Config::Wsl`]            | No         | [`ShellLaunchData::WSL`]           |
    /// | [`Config::Custom`]         | Yes        | [`ShellLaunchData::Executable`]    |
    /// | [`Config::MSYS2`]          | No         | [`ShellLaunchData::MSYS2`]         |
    /// | [`Config::DockerSandbox`]  | No         | [`ShellLaunchData::DockerSandbox`] |
    ///
    /// For `KnownLocal` and `Custom` we validate that the executable path
    /// is still valid.
    pub fn get_valid_shell_path_and_type(&self) -> Option<ShellLaunchData> {
        match self.state.as_ref() {
            Config::SystemDefault => None,
            Config::KnownLocal(LocalConfig {
                executable_path,
                shell_type,
                ..
            })
            | Config::Custom(LocalConfig {
                executable_path,
                shell_type,
                ..
            }) => {
                // We already did the supported_shell_path_and_type when constructing the model, but
                // in case the model is out of date we want to verify that the exe is still there.
                if file_exists_and_is_executable(executable_path) {
                    Some(ShellLaunchData::Executable {
                        executable_path: executable_path.clone(),
                        shell_type: *shell_type,
                    })
                } else {
                    None
                }
            }
            Config::Wsl { distro } => Some(ShellLaunchData::WSL {
                distro: distro.to_string(),
            }),
            Config::MSYS2(local_config) => Some(ShellLaunchData::MSYS2 {
                executable_path: local_config.executable_path.clone(),
                shell_type: local_config.shell_type,
            }),
            Config::DockerSandbox {
                sbx_path,
                base_image,
            } => Some(ShellLaunchData::DockerSandbox {
                sbx_path: sbx_path.clone(),
                base_image: base_image.clone(),
            }),
        }
    }

    fn new_local_executable(
        command: String,
        executable_path: PathBuf,
        shell_type: ShellType,
    ) -> Self {
        Self {
            id: Some(format!("local:{}", executable_path.display())),
            state: Arc::new(Config::KnownLocal(LocalConfig {
                command,
                executable_path,
                shell_type,
            })),
        }
    }

    #[cfg(windows)]
    fn new_msys2(command: String, executable_path: PathBuf, shell_type: ShellType) -> Self {
        Self {
            id: Some(format!("msys2:{}", executable_path.display())),
            state: Arc::new(Config::MSYS2(LocalConfig {
                command,
                executable_path,
                shell_type,
            })),
        }
    }

    #[cfg_attr(not(windows), allow(dead_code))]
    fn new_wsl(distro: String) -> Self {
        Self {
            id: Some(format!("wsl:{distro}")),
            state: Arc::new(Config::Wsl { distro }),
        }
    }

    pub(crate) fn new_custom_shell(
        command: String,
        executable_path: PathBuf,
        shell_type: ShellType,
    ) -> Self {
        Self {
            id: None,
            state: Arc::new(Config::Custom(LocalConfig {
                command,
                executable_path,
                shell_type,
            })),
        }
    }

    pub(crate) fn new_docker_sandbox_shell(sbx_path: PathBuf, base_image: Option<String>) -> Self {
        Self {
            id: None,
            state: Arc::new(Config::DockerSandbox {
                sbx_path,
                base_image,
            }),
        }
    }

    pub fn is_docker_sandbox(&self) -> bool {
        matches!(self.state.as_ref(), Config::DockerSandbox { .. })
    }
}

impl From<AvailableShell> for NewSessionShell {
    fn from(value: AvailableShell) -> Self {
        match value.state.as_ref() {
            Config::SystemDefault => NewSessionShell::SystemDefault,
            Config::KnownLocal(LocalConfig {
                executable_path, ..
            }) => NewSessionShell::Executable(executable_path.display().to_string()),
            Config::Wsl { distro } => NewSessionShell::WSL(distro.clone()),
            Config::Custom(LocalConfig {
                executable_path, ..
            }) => NewSessionShell::Custom(executable_path.display().to_string()),
            Config::MSYS2(local_config) => {
                NewSessionShell::MSYS2(local_config.executable_path.display().to_string())
            }
            // Docker sandbox isn't a persistable "preferred shell" today —
            // it's always launched on-demand via a tab action or slash
            // command. Round-trip through settings falls back to the system
            // default.
            // TODO(advait): If we ever let users pin the sandbox as their
            // default shell, add a `NewSessionShell::DockerSandbox` variant.
            Config::DockerSandbox { .. } => NewSessionShell::SystemDefault,
        }
    }
}

impl From<AvailableShell> for StartupShell {
    fn from(value: AvailableShell) -> Self {
        match value.state.as_ref() {
            Config::SystemDefault | Config::Wsl { .. } => StartupShell::Default,
            Config::KnownLocal(LocalConfig { shell_type, .. }) => {
                StartupShell::from(Some(shell_type.name().to_string()))
            }
            Config::Custom(LocalConfig {
                executable_path, ..
            }) => StartupShell::Custom(executable_path.display().to_string()),
            Config::MSYS2(local_config) => {
                StartupShell::from(Some(local_config.shell_type.name().to_string()))
            }
            // See the matching comment on `From<AvailableShell> for
            // NewSessionShell`: the sandbox isn't persistable as a startup
            // shell today, so fall back to default.
            Config::DockerSandbox { .. } => StartupShell::Default,
        }
    }
}

impl Default for AvailableShell {
    fn default() -> Self {
        Self {
            id: None,
            state: Arc::new(Config::SystemDefault),
        }
    }
}

#[cfg(feature = "local_tty")]
impl TryFrom<&str> for AvailableShell {
    type Error = ();

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        use crate::terminal::local_tty::shell::supported_shell_path_and_type;
        let (path, shell_type) = supported_shell_path_and_type(value).ok_or(())?;
        let command = path
            .file_name()
            .and_then(|file_name| file_name.to_str())
            .ok_or(())?
            .to_string();
        Ok(Self::new_custom_shell(command, path, shell_type))
    }
}

#[cfg(feature = "local_tty")]
impl TryFrom<NewSessionShell> for AvailableShell {
    type Error = ();

    fn try_from(value: NewSessionShell) -> Result<Self, Self::Error> {
        if let NewSessionShell::Custom(path) = &value {
            Ok(AvailableShell::try_from(path.as_str())?)
        } else {
            Err(())
        }
    }
}

#[cfg_attr(not(feature = "local_tty"), allow(dead_code))]
pub struct AvailableShells {
    shells: Vec<AvailableShell>,
    /// A map of shell name to the number of times it appears in the list.
    #[cfg(feature = "local_tty")]
    shell_counts: HashMap<String, usize>,
}

#[cfg(not(feature = "local_tty"))]
impl AvailableShells {
    pub fn get_available_shells(&self) -> impl Iterator<Item = &AvailableShell> {
        std::iter::empty()
    }

    pub fn get_from_shell_launch_data(&self, _config: &ShellLaunchData) -> Option<AvailableShell> {
        None
    }
}

#[cfg(feature = "local_tty")]
impl AvailableShells {
    pub fn new(_ctx: &mut ModelContext<Self>) -> Self {
        let fallback_shells_path = cfg!(unix).then_some(Path::new("/etc/shells"));

        let env_path = std::env::var_os("PATH").unwrap_or_default();
        #[cfg_attr(not(windows), allow(unused_mut))]
        let mut paths_to_search = std::env::split_paths(&env_path).collect::<Vec<PathBuf>>();

        // The PATH here is limited since it doesn't include the locations added
        // by the user's login shell. We add the Homebrew installer locations to
        // the search paths so we can detect shells installed via Homebrew.
        #[cfg(target_os = "macos")]
        {
            // Apple Silicon homebrew path
            paths_to_search.push(PathBuf::from("/opt/homebrew/bin"));
            // Intel homebrew path
            paths_to_search.push(PathBuf::from("/usr/local/bin"));
        }

        // The user may have installed PowerShell in locations not in the system
        // path. We add some typical install locations for PowerShell to the paths
        // to search.
        #[cfg(windows)]
        {
            use crate::util::windows;

            paths_to_search.extend(windows::powershell_7_install_paths());
            paths_to_search.push(windows::powershell_5_install_path());
        }

        #[cfg_attr(not(windows), allow(unused_mut))]
        let mut shells = Self::load_known_shells(&paths_to_search, fallback_shells_path);

        #[cfg(windows)]
        {
            let ctx = _ctx;
            shells.extend(
                super::wsl::WslInfo::as_ref(ctx)
                    .distributions()
                    .map(|distro| AvailableShell::new_wsl(distro.name.to_owned())),
            );
        }

        let shell_counts = shells.iter().fold(HashMap::new(), |mut counts, shell| {
            let count = counts.entry(shell.short_name().to_string()).or_insert(0);
            *count += 1;
            counts
        });
        Self {
            shells,
            shell_counts,
        }
    }

    /// Returns an iterable that iterates over all available shells.
    pub fn get_available_shells(&self) -> impl Iterator<Item = &AvailableShell> {
        self.shells.iter()
    }

    /// Returns the display name for a shell, context aware of the other
    /// available shells. If the shell appears only once, we return the short
    /// name. If the shell appears multiple times, we disambiguate by showing
    /// the full name with the path to the executable.
    pub fn display_name_for_shell<'a>(&self, shell: &'a AvailableShell) -> Cow<'a, str> {
        if self
            .shell_counts
            .get::<str>(shell.short_name().as_ref())
            .is_some_and(|count| *count > 1)
        {
            Cow::from(shell.long_name())
        } else {
            shell.short_name()
        }
    }

    /// Attempts to convert from ShellLaunchData into an AvailableShell
    ///
    /// **Note: This is here b/c we need to be able to get an AvailableShell
    /// from snapshot data. You probably should not be calling this.**
    ///
    /// The methodology is:
    ///
    /// 1. Search for a matching shell in the list of AvailableShells.
    ///    If it exists, then we return it. This covers all `KnownLocal` or `WSL` cases.
    /// 2. If we don't find it in the list, and the ShellLaunchData is `WSL`, that means
    ///    the wsl distro is no longer valid, so we return None.
    /// 3. If we have `Executable`` data that has not matched the list, we convert it to
    ///    a custom shell. This covers the case of either a Custom shell or a KnownLocal
    ///    that for whatever reason is no longer valid. An invalid Custom or KnownLocal
    ///    will still be validated before launching a shell (and fall back to default),
    ///    so it is fine to return a Custom here.
    ///
    /// See [`AvailableShell::get_valid_shell_path_and_type`] for more information on how
    /// we generate these launch configs in the other direction.
    pub fn get_from_shell_launch_data(&self, config: &ShellLaunchData) -> Option<AvailableShell> {
        self.shells
            .iter()
            .find(|shell| match (shell.state.as_ref(), config) {
                (Config::Wsl { distro }, ShellLaunchData::WSL { distro: d }) => distro == d,
                (
                    Config::MSYS2(LocalConfig {
                        executable_path, ..
                    }),
                    ShellLaunchData::MSYS2 {
                        executable_path: p, ..
                    },
                ) => executable_path == p,
                (
                    Config::KnownLocal(LocalConfig {
                        executable_path,
                        shell_type,
                        ..
                    }),
                    ShellLaunchData::Executable {
                        executable_path: p,
                        shell_type: t,
                    },
                ) => executable_path == p && shell_type == t,
                _ => false,
            })
            .cloned()
            .or_else(|| {
                if let ShellLaunchData::Executable {
                    executable_path,
                    shell_type,
                } = config
                {
                    Some(AvailableShell::new_custom_shell(
                        executable_path.file_name()?.to_str()?.to_string(),
                        executable_path.clone(),
                        *shell_type,
                    ))
                } else {
                    None
                }
            })
    }

    /// Checks the user preferences for the shell to use for new sessions, returning an available
    /// shell object.
    pub fn get_user_preferred_shell(&self, ctx: &AppContext) -> AvailableShell {
        let preference = self.get_user_preferred_shell_setting(ctx);

        match preference {
            NewSessionShell::SystemDefault => AvailableShell::default(),
            NewSessionShell::Custom(_) => AvailableShell::try_from(preference).unwrap_or_default(),
            // TODO(DAN): This should be cached in the model. Also handle custom
            preference => self
                .shells
                .iter()
                .find(|shell| shell.matches_preference(&preference))
                .cloned()
                .unwrap_or_default(),
        }
    }

    /// Sets the user-preferred shell for new sessions. Saves the value back to user settings.
    pub fn set_user_preferred_shell(
        &self,
        value: AvailableShell,
        ctx: &mut ModelContext<Self>,
    ) -> anyhow::Result<()> {
        use super::session_settings::SessionSettings;
        use warp_core::features::FeatureFlag;
        SessionSettings::handle(ctx).update(ctx, |settings, ctx| {
            if FeatureFlag::ShellSelector.is_enabled() {
                settings
                    .new_session_shell_override
                    .set_value(Some(NewSessionShell::from(value)), ctx)
            } else {
                settings
                    .startup_shell_override
                    .set_value(StartupShell::from(value), ctx)
            }
        })
    }

    fn load_known_shells(
        paths_to_search: &[PathBuf],
        fallback_path: Option<&Path>,
    ) -> Vec<AvailableShell> {
        use warp_core::features::FeatureFlag;

        if !FeatureFlag::ShellSelector.is_enabled() {
            return vec![
                StartupShell::Zsh,
                StartupShell::Bash,
                StartupShell::Fish,
                StartupShell::PowerShell,
            ]
            .into_iter()
            .filter_map(|shell| {
                let state = LocalConfig::try_from(shell).ok()?;
                Some(AvailableShell {
                    id: None,
                    state: Arc::new(Config::KnownLocal(state)),
                })
            })
            .collect();
        }
        let shell_types = Self::get_shell_types();

        let mut known_shells = vec![];
        let mut fallback_shell_map = fallback_path
            .and_then(|path| Self::load_fallback_shells(path, &shell_types).ok())
            .unwrap_or_default();

        for (shell_type, command_name) in shell_types {
            #[cfg(windows)]
            if ["bash.exe", "fish.exe", "zsh.exe"].contains(&command_name) {
                let msys2_executables = Self::locate_msys2_executables();
                for msys2_path in &msys2_executables {
                    if msys2_path.ends_with(command_name) {
                        known_shells.push(AvailableShell::new_msys2(
                            command_name.to_string(),
                            msys2_path.clone(),
                            shell_type,
                        ));
                    }
                }
                continue;
            }

            let mut fallback_shells = fallback_shell_map.remove(command_name).unwrap_or_default();
            for path in Self::resolve_all_executables(command_name, paths_to_search.iter()) {
                fallback_shells.remove(&path);
                known_shells.push(AvailableShell::new_local_executable(
                    command_name.to_string(),
                    path,
                    shell_type,
                ));
            }

            // We append shells found in /etc/shells but not the path after the shells found on the path.
            for path in fallback_shells.iter() {
                if file_exists_and_is_executable(path) {
                    known_shells.push(AvailableShell::new_local_executable(
                        command_name.to_string(),
                        path.clone(),
                        shell_type,
                    ));
                }
            }
        }

        known_shells
    }

    fn get_shell_types() -> Vec<(ShellType, &'static str)> {
        if cfg!(windows) {
            vec![
                (ShellType::PowerShell, "pwsh.exe"),
                (ShellType::PowerShell, "powershell.exe"),
                (ShellType::Zsh, "zsh.exe"),
                (ShellType::Bash, "bash.exe"),
                (ShellType::Fish, "fish.exe"),
            ]
        } else {
            vec![
                (ShellType::Zsh, "zsh"),
                (ShellType::Bash, "bash"),
                (ShellType::Fish, "fish"),
                (ShellType::PowerShell, "pwsh"),
            ]
        }
    }

    /// Looks for UNIX shells running "natively" on Windows via MSYS2.
    ///
    /// Attempts to find Git Bash executables at a few locations:
    /// 1. `$env:LocalAppData\Programs\Git\usr\bin`
    /// 2. `$env:ProgramFiles\Git\usr\bin`
    /// 3. Where scoop may have installed it
    /// We don't want to search the PATH for a `bash.exe` since there are false positives.
    /// Also search for bash/fish/zsh in the
    #[cfg(windows)]
    fn locate_msys2_executables() -> Vec<PathBuf> {
        use std::env;

        use warp_core::features::FeatureFlag;

        let mut paths = Vec::new();

        // We look for Git Bash at `$env:LocalAppData\Programs\Git\usr\bin`.
        match env::var("LocalAppData") {
            Ok(local_app_data_path) => {
                let user_location = Path::new(&local_app_data_path)
                    .join("Programs")
                    .join("Git")
                    .join("usr")
                    .join("bin")
                    .join("bash.exe");
                if file_exists_and_is_executable(&user_location) {
                    paths.push(user_location);
                }
            }
            Err(err) => {
                log::warn!("Environment variable LocalAppData not found {err:#}");
            }
        }

        // Next, we look for Git Bash at `$env:ProgramFiles\Git\usr\bin`.
        match env::var("ProgramFiles") {
            Ok(program_files_path) => {
                let global_location = Path::new(&program_files_path)
                    .join("Git")
                    .join("usr")
                    .join("bin")
                    .join("bash.exe");
                if file_exists_and_is_executable(&global_location) {
                    paths.push(global_location);
                }
            }
            Err(err) => {
                log::warn!("Environment variable ProgramFiles not found {err:#}");
            }
        }

        // Next, we look for Git Bash where scoop, a package manager, may have installed it at
        // `$env:USERPROFILE\scoop\apps\git\current\usr\bin`.
        // Note that `current` is a symbolic link to the folder with the latest version.
        if let Ok(user_profile) = env::var("USERPROFILE") {
            let maybe_scoop_path = Path::new(&user_profile)
                .join("scoop")
                .join("apps")
                .join("git")
                .join("current")
                .join("usr")
                .join("bin")
                .join("bash.exe");
            if file_exists_and_is_executable(&maybe_scoop_path) {
                paths.push(maybe_scoop_path);
            }
        }

        if FeatureFlag::MSYS2Shells.is_enabled() {
            // Search the default install location for MSYS2 shells (installed via pacman).
            match env::var("SystemDrive") {
                Ok(system_drive) => {
                    let msys_bin = Path::new(&format!(r"{system_drive}\"))
                        .join("msys64")
                        .join("usr")
                        .join("bin");
                    for shell in ["bash.exe", "fish.exe", "zsh.exe"] {
                        let msys_shell = msys_bin.join(shell);
                        if file_exists_and_is_executable(&msys_shell) {
                            paths.push(msys_shell);
                        }
                    }
                }
                Err(err) => {
                    log::warn!("Environment variable SystemDrive not found {err:#}");
                }
            }
        }

        paths
    }

    /// Resolves all full paths to executables of the given command name in PATH.
    ///
    /// `paths_to_search` should contain the locations in PATH along with any
    /// manually added paths that we want to search.
    fn resolve_all_executables<'a>(
        command: &str,
        paths_to_search: impl Iterator<Item = &'a PathBuf>,
    ) -> Vec<PathBuf> {
        use itertools::Itertools as _;

        paths_to_search
            .filter_map(|single_path| {
                let joined = single_path.join(command);
                let canonicalized = dunce::canonicalize(&joined).unwrap_or(joined);
                file_exists_and_is_executable(&canonicalized).then_some(canonicalized)
            })
            .unique()
            .collect()
    }

    fn load_fallback_shells(
        path: &Path,
        shell_types: &[(ShellType, &str)],
    ) -> anyhow::Result<HashMap<String, HashSet<PathBuf>>> {
        use std::fs::File;
        use std::io::{BufRead, BufReader};

        let mut shells = HashMap::new();

        let file = File::open(path)?;

        for (_, exe) in shell_types.iter() {
            shells.insert(exe.to_string(), HashSet::new());
        }

        let reader = BufReader::new(file);
        for line in reader.lines() {
            let line = line?;
            // For each line in /etc/shells, we check:
            // - does it not start with a #?
            // - is it not empty?
            // - does the "file_name" map to a shell that we support?
            //
            // If all of those are true, then we add it to the set of paths associated with that shell
            if !line.trim_start().starts_with('#') && !line.trim().is_empty() {
                let Ok(path) = dunce::canonicalize(line) else {
                    continue;
                };
                if let Some(file_name) = path.file_name().and_then(|name| name.to_str()) {
                    if let Some(set) = shells.get_mut(file_name) {
                        set.insert(path);
                    }
                }
            }
        }
        Ok(shells)
    }

    fn get_user_preferred_shell_setting(&self, ctx: &AppContext) -> NewSessionShell {
        use super::session_settings::SessionSettings;

        let new_session_shell_override = SessionSettings::as_ref(ctx)
            .new_session_shell_override
            .to_owned();

        new_session_shell_override.unwrap_or_else(|| {
            // Fallback logic in case the new_session_shell has not been set.
            // We attempt to read the legacy setting startup_shell_override, and map it to a
            // NewSessionShell. If that mapping fails at all, we set it to NewSessionShell::SystemDefault.
            // We also write back to the new_session_shell_override setting so that we don't have to do this
            // all the time.
            self.get_user_preferred_shell_setting_fallback(ctx)
        })
    }

    fn get_user_preferred_shell_setting_fallback(&self, ctx: &AppContext) -> NewSessionShell {
        use super::session_settings::SessionSettings;

        let startup_shell = SessionSettings::as_ref(ctx)
            .startup_shell_override
            .to_owned();
        match startup_shell {
            StartupShell::Default => NewSessionShell::SystemDefault,
            StartupShell::Custom(path) => NewSessionShell::Custom(path),
            _ => startup_shell
                .shell_command()
                .and_then(|command| {
                    for shell in self.shells.iter() {
                        if let Config::KnownLocal(LocalConfig {
                            command: shell_command,
                            executable_path,
                            ..
                        }) = shell.state.as_ref()
                        {
                            if shell_command == command {
                                return Some(NewSessionShell::Executable(
                                    executable_path.display().to_string(),
                                ));
                            }
                        }
                    }
                    None
                })
                .unwrap_or(NewSessionShell::SystemDefault),
        }
    }

    /// Finds the first shell that matches the given shell type.
    pub fn find_known_shell_by_type(&self, shell: ShellType) -> Option<AvailableShell> {
        self.shells
            .iter()
            .find(|s| matches!(s.state.as_ref(), Config::KnownLocal(LocalConfig { shell_type, .. }) if shell == *shell_type))
            .cloned()
    }

    /// Finds the first known shell whose command name matches `name`.
    ///
    /// This is used to resolve a bare shell name from a tab config (e.g.
    /// `shell = "pwsh"`) against shells that [`AvailableShells::new`] has
    /// already discovered. Because that discovery supplements the process
    /// `PATH` with well-known install locations (such as `/opt/homebrew/bin`
    /// on macOS), this lookup can find shells that a plain `PATH` search via
    /// [`AvailableShell::try_from`] would miss when Warp is launched outside
    /// an interactive shell.
    ///
    /// Comparison is case-sensitive on Unix. On Windows, where file names are
    /// case-insensitive and users commonly omit the `.exe` suffix, comparison
    /// ignores case and an optional trailing `.exe` on either side.
    pub fn find_by_command_name(&self, name: &str) -> Option<AvailableShell> {
        self.shells
            .iter()
            .find(|shell| {
                let command = match shell.state.as_ref() {
                    Config::KnownLocal(LocalConfig { command, .. })
                    | Config::MSYS2(LocalConfig { command, .. }) => command.as_str(),
                    Config::Custom(_)
                    | Config::SystemDefault
                    | Config::Wsl { .. }
                    | Config::DockerSandbox { .. } => {
                        return false;
                    }
                };
                command_name_matches(command, name, cfg!(windows))
            })
            .cloned()
    }
}

/// Returns whether two shell command names are equivalent when resolving a
/// tab config's `shell` field. See [`AvailableShells::find_by_command_name`].
///
/// `is_windows` selects the matching rules:
/// - On Windows, file names are case-insensitive and users commonly omit the
///   `.exe` suffix (e.g. `shell = "pwsh"`), so matching is case-insensitive
///   and ignores an optional trailing `.exe` on either side.
/// - On Unix, matching is a plain case-sensitive equality check.
///
/// Parameterized (rather than keying off `cfg!(windows)` internally) so both
/// branches can be unit-tested from any host platform.
#[cfg(feature = "local_tty")]
fn command_name_matches(stored: &str, requested: &str, is_windows: bool) -> bool {
    if is_windows {
        let stored_norm = stored.to_ascii_lowercase();
        let requested_norm = requested.to_ascii_lowercase();
        let stored_stem = stored_norm.strip_suffix(".exe").unwrap_or(&stored_norm);
        let requested_stem = requested_norm
            .strip_suffix(".exe")
            .unwrap_or(&requested_norm);
        stored_stem == requested_stem
    } else {
        stored == requested
    }
}

impl Entity for AvailableShells {
    type Event = ();
}
impl SingletonEntity for AvailableShells {}

#[cfg(feature = "local_tty")]
pub fn register(app: &mut impl warpui::AddSingletonModel) {
    #[cfg(windows)]
    app.add_singleton_model(super::wsl::WslInfo::new);
    app.add_singleton_model(AvailableShells::new);
}

#[cfg(test)]
#[cfg(not(windows))]
#[path = "available_shells_tests.rs"]
mod tests;
