use warpui::keymap::{BindingDescription, PerPlatformKeystroke};

use super::StaticCommand;

pub enum DefaultSlashCommandBinding {
    None,
    Single(&'static str),
    PerPlatform(PerPlatformKeystroke),
}

pub fn default_binding_for_command(name: &'static str) -> DefaultSlashCommandBinding {
    match name {
        "/agent" => DefaultSlashCommandBinding::PerPlatform(PerPlatformKeystroke {
            mac: "cmd-enter",
            linux_and_windows: "ctrl-shift-enter",
        }),
        "/conversations" => DefaultSlashCommandBinding::PerPlatform(PerPlatformKeystroke {
            mac: "cmd-y",
            linux_and_windows: "ctrl-shift-Y",
        }),
        "/open-repo" => DefaultSlashCommandBinding::PerPlatform(PerPlatformKeystroke {
            mac: "alt-cmd-o",
            linux_and_windows: "ctrl-alt-o",
        }),
        _ => DefaultSlashCommandBinding::None,
    }
}

pub fn binding_description(command: &StaticCommand) -> BindingDescription {
    BindingDescription::new_preserve_case(format!("Slash command: {}", command.name))
}
