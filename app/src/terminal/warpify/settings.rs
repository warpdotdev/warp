use anyhow::Result;
use lazy_static::lazy_static;
use regex::Regex;
use settings::{
    macros::{maybe_define_setting, register_settings_events},
    ChangeEventReason, RespectUserSyncSetting, Setting, SupportedPlatforms, SyncToCloud,
};
use strum_macros::EnumIter;
use warp_util::path::ShellFamily;
use warpui::{AppContext, ModelContext};
use warpui::{Entity, SingletonEntity};

use crate::terminal::ssh::util::{parse_interactive_ssh_command, SshWarpifyCommand};

// Cannot directly use Vec<Regex> here b/c Regex doesn't impl Eq, Serialize, and Deserialize.
maybe_define_setting!(AddedSubshellCommands, group: WarpifySettings, {
    type: Vec<String>,
    default: Vec::new(),
    supported_platforms: SupportedPlatforms::ALL,
    sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
    private: false,
    toml_path: "warpify.subshells.added_subshell_commands",
    description: "Additional regex patterns for commands that should be recognized as subshells.",
});

maybe_define_setting!(SubshellCommandsDenylist, group: WarpifySettings, {
    type: Vec<String>,
    default: Vec::new(),
    supported_platforms: SupportedPlatforms::ALL,
    sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
    private: false,
    toml_path: "warpify.subshells.subshell_commands_denylist",
    description: "Commands that should not trigger the subshell warpification prompt.",
});

maybe_define_setting!(SshHostsDenylist, group: WarpifySettings, {
    type: Vec<String>,
    default: Vec::new(),
    supported_platforms: SupportedPlatforms::ALL,
    sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
    private: false,
    toml_path: "warpify.ssh.ssh_hosts_denylist",
    description: "SSH hosts that should not trigger the warpification prompt.",
});

maybe_define_setting!(EnableSshWarpification, group: WarpifySettings, {
    type: bool,
    default: true,
    supported_platforms: SupportedPlatforms::ALL,
    sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
    private: false,
    toml_path: "warpify.ssh.enable_ssh_warpification",
    description: "Whether to enable Warp features in SSH sessions.",
});

maybe_define_setting!(UseSshTmuxWrapper, group: WarpifySettings, {
    type: bool,
    default: false,
    supported_platforms: SupportedPlatforms::OR(SupportedPlatforms::MAC.into(), SupportedPlatforms::LINUX.into()),
    sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
    private: false,
    toml_path: "warpify.ssh.use_ssh_tmux_wrapper",
    description: "Whether to use a tmux-based wrapper for SSH warpification.",
});

/// Controls how Warp handles the SSH extension (remote server binary) when connecting
/// to a remote host that does not already have it installed.
#[derive(
    Default,
    Debug,
    serde::Serialize,
    serde::Deserialize,
    PartialEq,
    Copy,
    Clone,
    EnumIter,
    schemars::JsonSchema,
    settings_value::SettingsValue,
)]
#[serde(rename_all = "snake_case")]
#[schemars(
    description = "Controls SSH extension installation behavior.",
    rename_all = "snake_case"
)]
pub enum SshExtensionInstallMode {
    /// Always prompt the user before installing (default).
    #[default]
    AlwaysAsk,
    /// Automatically install and connect without prompting.
    AlwaysInstall,
    /// Never install; fall back to legacy warpification.
    NeverInstall,
}

maybe_define_setting!(SshExtensionInstallModeSetting, group: WarpifySettings, {
    type: SshExtensionInstallMode,
    default: SshExtensionInstallMode::default(),
    supported_platforms: SupportedPlatforms::ALL,
    sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
    private: false,
    toml_path: "warpify.ssh.ssh_extension_install_mode",
    description: "Controls SSH extension installation behavior.",
});

impl SshExtensionInstallMode {
    pub fn display_name(&self) -> &'static str {
        match self {
            SshExtensionInstallMode::AlwaysAsk => "Always ask",
            SshExtensionInstallMode::AlwaysInstall => "Always install",
            SshExtensionInstallMode::NeverInstall => "Never install",
        }
    }
}

/// Normally we use the define_settings_group! macro for singleton models of settings like this.
/// However, this model needs to do some extra processing on the added_subshell_commands and store
/// an enriched representation in parsed_added_subshell_commands.
pub struct WarpifySettings {
    /// A list of regexes that users can add to define new subshell-compatible commands. This
    /// represents the raw, serialized value. Therefore, it is Vec<String>.
    pub added_subshell_commands: AddedSubshellCommands,
    /// This is added_subshell_commands compiled to actual executable Regex. This is a Result as we
    /// cannot guarantee the values are valid regex. Even if we prevent them in the UI from entering
    /// invalid regex, it's possible that the serialized value in user-defaults is invalid. This
    /// needs to be kept up-to-date as added_subshell_commands changes. See the Self::register
    /// method for how this is done.
    pub parsed_added_subshell_commands: Vec<Result<Regex, regex::Error>>,
    /// A list of commands that we shouldn't attempt to warpify. These can be added either b/c the
    /// "don't ask again" button was clicked in the trigger banner, or it was added explicitly on
    /// the Warpify settings page. This represents the raw, serialized value.
    pub subshell_command_denylist: SubshellCommandsDenylist,
    /// This is subshell_command_denylist compiled to actual executable Regex. This is a Result as we
    /// cannot guarantee the values are valid regex. Even if we prevent them in the UI from entering
    /// invalid regex, it's possible that the serialized value in user-defaults is invalid. This
    /// needs to be kept up-to-date as subshell_command_denylist changes. See the Self::register
    /// method for how this is done.
    pub parsed_subshell_command_denylist: Vec<Result<Regex, regex::Error>>,

    /// A list of hosts that we shouldn't attempt to warpify. This supports regex.
    /// These can be added either b/c the "don't ask again" button was clicked in the trigger banner,
    /// or it was added explicitly on the Warpify settings page.
    /// While this could live in the `SshSettings` group, the custom processing shared with the other
    /// subshell logic better justifies it living in the `WarpifySettings` group.
    pub ssh_hosts_denylist: SshHostsDenylist,
    /// This is ssh_hosts_denylist compiled to actual executable Regex. This is a Result as we
    /// cannot guarantee the values are valid regex. Even if we prevent them in the UI from entering
    /// invalid regex, it's possible that the serialized value in user-defaults is invalid. This
    /// needs to be kept up-to-date as ssh_hosts_denylist changes. See the Self::register
    /// method for how this is done.
    pub parsed_ssh_hosts_denylist: Vec<Result<Regex, regex::Error>>,

    /// This setting controls whether we should ever warpify ssh sessions.
    pub enable_ssh_warpification: EnableSshWarpification,

    /// This setting controls whether we should prompt the user to warpify an ssh session using the
    /// tmux wrapper instead of the default legacy wrapper.
    pub use_ssh_tmux_wrapper: UseSshTmuxWrapper,

    /// Controls the installation behavior for the SSH extension (remote server) when the binary
    /// is not installed on the remote host.
    pub ssh_extension_install_mode: SshExtensionInstallModeSetting,
}

#[cfg(windows)]
lazy_static! {
    /// Matches `wsl` commands which is for Windows Subsystem for Linux. Calling this can open
    /// interactive shells into Linux VMs.
    pub static ref WSL_SUBSHELL_REGEX: Regex = Regex::new(r"^wsl(\.exe)?($|\s)").expect("wsl regex must compile");
    /// We filter out `wsl` commands that are not for opening interactive shells.
    pub static ref WSL_IGNORE_REGEX: Regex = Regex::new(r" --(default-user|enable-wsl1|export|help|import|import-in-place|inbox|install|list|mount|no-distribution|no-launch|set-default|shutdown|status|terminate|uninstall|unmount|unregister|update|version|web-download)").expect("wsl ignore regex invalid");
}

lazy_static! {
    pub static ref POETRY_SUBSHELL_COMMAND_REGEX: Regex  = Regex::new(r"^poetry\s+shell").expect("Poetry subshell regex invalid");
    pub static ref PIPENV_SUBSHELL_COMMAND_REGEX: Regex  = Regex::new(r"^pipenv\s+shell").expect("pipenv subshell regex invalid");

    /// These are known compatible subshell commands
    static ref SUBSHELL_COMMAND_REGEXES: Vec<Regex> = vec![
        // Matches "bash", "/bin/bash", any "./any/path/to/bash", plus the zsh/fish equivalents
        Regex::new(r"^/?([\w\.-]+/)*(bash|zsh|fish)$").expect("Direct shell regex invalid"),

        // Matches "docker/podman run [whatever args] bash", plus zsh/fish equivalents.
        // Optionally allows single or double quotes around the shell name.
        Regex::new(r#"^(docker|podman)\s+run\s+.*?['"]?(bash|zsh|fish)['"]?$"#).expect("docker/podman run regex invalid"),

        // Matches "docker/podman exec [whatever args] bash", plus zsh/fish equivalents.
        // Optionally allows single or double quotes around the shell name.
        Regex::new(r#"^(docker|podman)\s+exec\s+.*?['"]?(bash|zsh|fish)['"]?$"#).expect("docker/podman exec regex invalid"),

        // Matches commands that spawn a poetry subshell.
        POETRY_SUBSHELL_COMMAND_REGEX.clone(),

        // Matches commands that spawn a pipenv subshell.
        PIPENV_SUBSHELL_COMMAND_REGEX.clone(),

        // https://github.com/warpdotdev/Warp/issues/2736
        Regex::new(r"^aws-vault\s+exec\b").expect("aws-vault regex invalid"),

        // https://flox.dev/docs/reference/command-reference/flox-activate/
        // https://github.com/flox/flox/issues/2784
        Regex::new(r"^flox\s+(-\S+\s+)*activate\b").expect("flox activate regex invalid"),
    ];
}

/// There are two impl blocks for SubshellSettings. This block is an inlined version of the
/// define_settings_group! macro, which is the basic template for user-defaults-backed settings.
/// I have separated this stuff from the other impl block, which contains the subshell-specific
/// logic, because this is basically boilerplate.
impl WarpifySettings {
    fn new_from_storage(ctx: &mut ModelContext<Self>) -> Self {
        let added_subshell_commands = AddedSubshellCommands::new_from_storage(ctx);
        let subshell_command_denylist = SubshellCommandsDenylist::new_from_storage(ctx);
        let ssh_hosts_denylist = SshHostsDenylist::new_from_storage(ctx);
        Self {
            parsed_added_subshell_commands: Self::parse_added_subshell_commands(
                &added_subshell_commands,
            ),
            added_subshell_commands,
            parsed_subshell_command_denylist: Self::parse_subshell_command_denylist(
                &subshell_command_denylist,
            ),
            subshell_command_denylist,
            parsed_ssh_hosts_denylist: Self::parse_ssh_hosts_denylist(&ssh_hosts_denylist),
            ssh_hosts_denylist,
            enable_ssh_warpification: EnableSshWarpification::new_from_storage(ctx),
            use_ssh_tmux_wrapper: UseSshTmuxWrapper::new_from_storage(ctx),
            ssh_extension_install_mode: SshExtensionInstallModeSetting::new_from_storage(ctx),
        }
    }

    #[cfg(any(test, feature = "integration_tests"))]
    #[allow(dead_code)]
    pub fn new_with_defaults(_ctx: &mut ModelContext<Self>) -> Self {
        let added_subshell_commands = AddedSubshellCommands::new(None);
        let subshell_command_denylist = SubshellCommandsDenylist::new(None);
        let ssh_hosts_denylist = SshHostsDenylist::new(None);
        Self {
            parsed_added_subshell_commands: Self::parse_added_subshell_commands(
                &added_subshell_commands,
            ),
            added_subshell_commands,
            parsed_subshell_command_denylist: Self::parse_subshell_command_denylist(
                &subshell_command_denylist,
            ),
            subshell_command_denylist,
            parsed_ssh_hosts_denylist: Self::parse_ssh_hosts_denylist(&ssh_hosts_denylist),
            ssh_hosts_denylist,
            enable_ssh_warpification: EnableSshWarpification::new(None),
            use_ssh_tmux_wrapper: UseSshTmuxWrapper::new(None),
            ssh_extension_install_mode: SshExtensionInstallModeSetting::new(None),
        }
    }

    /// This is different from the typical register method, as it also ensures that
    /// our parsed regexes stay in sync with the underlying data by having the
    /// model subscribe to itself after it's registered.
    pub fn register(ctx: &mut AppContext) {
        let handle = ctx.add_singleton_model(Self::new_from_storage);
        handle.clone().update(ctx, |_, ctx| {
            ctx.subscribe_to_model(&handle, |me, event, _| match event {
                WarpifySettingsChangedEvent::AddedSubshellCommands { .. } => {
                    me.parsed_added_subshell_commands =
                        Self::parse_added_subshell_commands(&me.added_subshell_commands)
                }
                WarpifySettingsChangedEvent::SubshellCommandsDenylist { .. } => {
                    me.parsed_subshell_command_denylist =
                        Self::parse_subshell_command_denylist(&me.subshell_command_denylist)
                }
                WarpifySettingsChangedEvent::SshHostsDenylist { .. } => {
                    me.parsed_ssh_hosts_denylist =
                        Self::parse_ssh_hosts_denylist(&me.ssh_hosts_denylist)
                }
                WarpifySettingsChangedEvent::EnableSshWarpification { .. } => {}
                WarpifySettingsChangedEvent::UseSshTmuxWrapper { .. } => {}
                WarpifySettingsChangedEvent::SshExtensionInstallModeSetting { .. } => {}
            })
        });

        register_settings_events!(
            WarpifySettings,
            added_subshell_commands,
            AddedSubshellCommands,
            handle.clone(),
            ctx
        );

        register_settings_events!(
            WarpifySettings,
            subshell_command_denylist,
            SubshellCommandsDenylist,
            handle.clone(),
            ctx
        );

        register_settings_events!(
            WarpifySettings,
            enable_ssh_warpification,
            EnableSshWarpification,
            handle.clone(),
            ctx
        );

        register_settings_events!(
            WarpifySettings,
            use_ssh_tmux_wrapper,
            UseSshTmuxWrapper,
            handle.clone(),
            ctx
        );

        register_settings_events!(
            WarpifySettings,
            ssh_extension_install_mode,
            SshExtensionInstallModeSetting,
            handle.clone(),
            ctx
        );

        register_settings_events!(
            WarpifySettings,
            ssh_hosts_denylist,
            SshHostsDenylist,
            handle,
            ctx
        );
    }
}

/// This is also something that would normally be generated by
/// define_settings_group!(WarpifySettings). Since we didn't use that macro we define it manually
/// here. It's the event emitted by the setter methods when a setting value changes.
pub enum WarpifySettingsChangedEvent {
    AddedSubshellCommands {
        change_event_reason: ChangeEventReason,
    },
    SubshellCommandsDenylist {
        change_event_reason: ChangeEventReason,
    },
    SshHostsDenylist {
        change_event_reason: ChangeEventReason,
    },
    EnableSshWarpification {
        change_event_reason: ChangeEventReason,
    },
    UseSshTmuxWrapper {
        change_event_reason: ChangeEventReason,
    },
    SshExtensionInstallModeSetting {
        change_event_reason: ChangeEventReason,
    },
}

impl Entity for WarpifySettings {
    type Event = WarpifySettingsChangedEvent;
}

impl SingletonEntity for WarpifySettings {}

/// This is the other impl block for this model. This one contains the actual subshell-specific
/// logic.
impl WarpifySettings {
    fn is_built_in_subshell_match(command: &str) -> bool {
        for command_regex in SUBSHELL_COMMAND_REGEXES.iter() {
            if command_regex.is_match(command) {
                return true;
            }
        }
        #[cfg(windows)]
        {
            if WSL_SUBSHELL_REGEX.is_match(command) && !WSL_IGNORE_REGEX.is_match(command) {
                return true;
            }
        }
        false
    }

    /// This function determines if we should ask the user whether they want to bootstrap a subshell.
    /// It determines this by matching their command against some hardcoded regexes and those added
    /// manually by the user.
    pub fn is_compatible_subshell_command(&self, command: &str, shell_family: ShellFamily) -> bool {
        let command = command.trim();
        if Self::is_built_in_subshell_match(command) {
            return true;
        }

        if !self.use_ssh_tmux_wrapper.value()
            && SshWarpifyCommand::matches(command)
                .is_some_and(|command| command.is_ssh_like_command())
        {
            return true;
        }

        for command_regex in self.parsed_added_subshell_commands.iter().flatten() {
            if command_regex.is_match(command) {
                return true;
            }
        }

        // While in-band generators are our best option for warpifying ssh sessions from powershell, hard-code
        // the warpify subshell banner to show up.
        if matches!(shell_family, ShellFamily::PowerShell)
            && parse_interactive_ssh_command(command).is_some()
        {
            return true;
        }

        false
    }

    /// This function determines if we should ask the user whether they want to bootstrap an ssh session.
    /// It determines this by matching the host against a denylist of hosts, which can include regex.
    pub fn is_ssh_host_denylisted(&self, ssh_host: &str) -> bool {
        self.parsed_ssh_hosts_denylist
            .iter()
            .flatten()
            .any(|regex| regex.is_match(ssh_host.trim()))
    }

    fn parse_added_subshell_commands(
        added_subshell_commands: &AddedSubshellCommands,
    ) -> Vec<Result<Regex, regex::Error>> {
        added_subshell_commands
            .iter()
            .map(|user_pattern| Regex::new(user_pattern))
            .collect()
    }

    fn parse_subshell_command_denylist(
        subshell_command_denylist: &SubshellCommandsDenylist,
    ) -> Vec<Result<Regex, regex::Error>> {
        subshell_command_denylist
            .iter()
            .map(|user_pattern| Regex::new(user_pattern))
            .collect()
    }

    fn parse_ssh_hosts_denylist(
        ssh_hosts_denylist: &SshHostsDenylist,
    ) -> Vec<Result<Regex, regex::Error>> {
        ssh_hosts_denylist
            .iter()
            .map(|user_pattern| Regex::new(user_pattern))
            .collect()
    }

    /// The user has indicated that they don't want to be asked to bootstrap a subshell for this
    /// command, so save it in user-defaults.
    pub fn denylist_subshell_command(
        &mut self,
        command_to_denylist: &str,
        ctx: &mut ModelContext<Self>,
    ) {
        let mut new_denylist = self.subshell_command_denylist.to_vec();
        new_denylist.push(command_to_denylist.trim().to_owned());
        self.subshell_command_denylist
            .set_value(new_denylist, ctx)
            .expect("subshell_command_denylist failed to serialize");

        ctx.notify();
    }

    /// The user has indicated that they don't want to be asked to bootstrap an ssh session
    /// for this host, so save it in user-defaults.
    pub fn denylist_ssh_host(&mut self, host_to_denylist: &str, ctx: &mut ModelContext<Self>) {
        let mut new_denylist = self.ssh_hosts_denylist.to_vec();
        new_denylist.push(host_to_denylist.trim().to_owned());
        self.ssh_hosts_denylist
            .set_value(new_denylist, ctx)
            .expect("ssh_hosts_denylist failed to serialize");

        ctx.notify();
    }

    /// Add a new regex to the list of subshell-compatible commands.
    pub fn add_subshell_command(&mut self, command_to_add: &str, ctx: &mut ModelContext<Self>) {
        let mut new_added_commands_list = self.added_subshell_commands.to_vec();
        new_added_commands_list.push(command_to_add.trim().to_owned());

        // The set_value method generated by the maybe_define_setting! macro will take
        // care of emitting the WarpifySettingsChangedEvent::AddedSubshellCommands event to keep
        // parsed_added_subshell_commands in sync.
        self.added_subshell_commands
            .set_value(new_added_commands_list, ctx)
            .expect("added_subshell_commands failed to serialize");

        ctx.notify();
    }

    /// Check if the user has asked us to remember a command and avoid asking to warpify a subshell.
    pub fn is_denylisted_subshell_command(&self, command: &str) -> bool {
        let command = command.trim();
        self.parsed_subshell_command_denylist
            .iter()
            .flatten()
            .any(|command_regex| command_regex.is_match(command))
    }

    pub fn remove_denylisted_subshell_command(
        &mut self,
        index: usize,
        ctx: &mut ModelContext<Self>,
    ) {
        let mut new_denylist = self.subshell_command_denylist.to_vec();
        new_denylist.remove(index);
        self.subshell_command_denylist
            .set_value(new_denylist, ctx)
            .expect("subshell_command_denylist failed to serialize");
        ctx.notify();
    }

    pub fn remove_added_subshell_command(&mut self, index: usize, ctx: &mut ModelContext<Self>) {
        let mut new_added_list = self.added_subshell_commands.to_vec();
        new_added_list.remove(index);
        self.added_subshell_commands
            .set_value(new_added_list, ctx)
            .expect("added_subshell_commands failed to serialize");
        ctx.notify();
    }

    pub fn remove_denylisted_ssh_host(&mut self, index: usize, ctx: &mut ModelContext<Self>) {
        let mut new_denylist = self.ssh_hosts_denylist.to_vec();
        new_denylist.remove(index);
        self.ssh_hosts_denylist
            .set_value(new_denylist, ctx)
            .expect("ssh_hosts_denylist failed to serialize");
        ctx.notify();
    }
}

#[cfg(test)]
#[path = "settings_tests.rs"]
mod tests;
