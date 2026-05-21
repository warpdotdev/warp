mod unescape;

use std::collections::{HashMap, HashSet};
use std::ops::Deref;
use std::path::{Path, PathBuf};

use anyhow::Result;
use channel_versions::overrides::TargetOS;
use enum_iterator::Sequence;
use itertools::Itertools;
use lazy_static::lazy_static;
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;
use typed_path::{TypedPath, TypedPathBuf, WindowsPath};
use version_compare::{Cmp, Version};
use warp_completer::completer::{CommandExitStatus, CommandOutput};
#[cfg(windows)]
use warp_core::paths::base_config_dir;
use warp_core::platform::SessionPlatform;
use warp_util::path::{
    convert_msys2_to_windows_native_path, convert_wsl_to_windows_host_path, msys2_exe_to_root,
};

use crate::model::escape_sequences;

use self::unescape::unescape_quotes;

const ZSH_META: u8 = 0x83;

/// These are file extensions of executable files on Windows.
///
/// Commands ending with any of these extensions may be executed with the extension elided, e.g.
/// you can type `git` in a shell instead of `git.exe`.
/// This is the contents of `$env:PATHEXT` on a default Windows 11 installation. See docs:
/// https://renenyffenegger.ch/notes/Windows/development/environment-variables/PATHEXT
/// TODO(CORE-2948) Fetch this dynamically instead.
const PATHEXT: [&str; 12] = [
    ".COM", ".EXE", ".BAT", ".CMD", ".VBS", ".VBE", ".JS", ".JSE", ".WSF", ".WSH", ".MSC", ".CPL",
];

lazy_static! {
    static ref BASH_INPUT_REPORTING_MINIMUM_VERSION: Version<'static> =
        Version::from("4.0").expect("version parses successfully");
}

/// Strips the extended history prefix from a zsh history line, if present.
///
/// The zsh history can have two types of output depending on if `extended_history` mode is enabled.
/// * If enabled, the history is of the form: `: <beginning time>:<elapsed seconds>;<command>`.
/// * If not enabled, history is simply the command on each line with no additional metadata.
///
/// We avoid using a regex here in favor of simple string manipulation for better performance in this
/// hot path.
fn strip_zsh_extended_prefix(line: &str) -> &str {
    let Some(rest) = line.strip_prefix(": ") else {
        return line;
    };

    let Some(semi_idx) = rest.find(';') else {
        return line;
    };

    let prefix = &rest[..semi_idx];
    let Some((timestamp, elapsed)) = prefix.split_once(':') else {
        return line;
    };

    if !timestamp.is_empty()
        && timestamp.bytes().all(|b| b.is_ascii_digit())
        && !elapsed.is_empty()
        && elapsed.bytes().all(|b| b.is_ascii_digit())
    {
        &rest[semi_idx + 1..]
    } else {
        line
    }
}

/// Represents a shell and its configuration.
#[derive(Clone, Debug)]
pub struct Shell {
    shell_type: ShellType,
    version: Option<String>,
    options: Option<HashSet<String>>,

    /// Shell plugins like Powerlevel10k that we autodetect while bootstrapping.
    /// This is not at all exhaustive, it's just for common plugins that need
    /// special handling (like warning the user that they're incompatible).
    plugins: HashSet<String>,

    /// The full path to the running shell binary on the host (e.g. "/usr/bin/zsh").
    /// Populated from the `Bootstrapped` DCS payload. For local sessions this is
    /// redundant with `ShellLaunchData::executable_path`; for SSH sessions this
    /// is the authoritative path on the remote host.
    shell_path: Option<String>,
}

impl Shell {
    pub fn new(
        shell_type: ShellType,
        version: Option<String>,
        options: Option<HashSet<String>>,
        plugins: HashSet<String>,
        shell_path: Option<String>,
    ) -> Self {
        Self {
            shell_type,
            version,
            options,
            plugins,
            shell_path,
        }
    }

    pub fn shell_path(&self) -> &Option<String> {
        &self.shell_path
    }

    pub fn shell_type(&self) -> ShellType {
        self.shell_type
    }

    pub fn version(&self) -> &Option<String> {
        &self.version
    }

    pub fn options(&self) -> &Option<HashSet<String>> {
        &self.options
    }

    pub fn plugins(&self) -> &HashSet<String> {
        &self.plugins
    }

    /// Returns true if the shell requires the use of an in-band command executor.
    /// This applies to both local and remote sessions.
    pub fn force_in_band_command_executor(&self) -> bool {
        self.shell_type.force_in_band_command_executor()
    }

    /// Returns whether the current shell supports native shell completions.
    pub fn supports_native_shell_completions(&self) -> bool {
        self.shell_type.supports_native_shell_completions()
    }

    /// Whether the shell supports "autocd" (`cd`ing into a directory without specifying
    /// `cd`).
    pub fn supports_autocd(&self) -> bool {
        match self.shell_type {
            ShellType::Zsh | ShellType::Bash => self
                .options
                .as_ref()
                .is_some_and(|map| map.contains("autocd")),
            // autocd is always enabled in Fish, see https://fishshell.com/docs/current/cmds/cd.html.
            ShellType::Fish => true,
            ShellType::PowerShell => false,
        }
    }

    /// If the particular version of this shell supports input reporting, return the byte sequence
    /// to trigger input reporting.
    ///
    /// These sequences are bound to Warp shell functions during session bootstrap that print the
    /// shell's input buffer, wrapped within the 'InputBuffer' DCS hook when triggered. PowerShell
    /// cannot use a binding that contains the letter "i"  because it does virtual key code
    /// translation based on the current layout, and not all layouts have the letter "i".
    pub fn input_reporting_sequence(&self) -> Option<[u8; 2]> {
        match self.shell_type {
            ShellType::PowerShell => Some([escape_sequences::C0::ESC, b'1']),
            ShellType::Fish | ShellType::Zsh => Some([escape_sequences::C0::ESC, b'i']),
            ShellType::Bash => self
                .version
                .as_ref()
                .and_then(|version| Version::from(version.as_str()))
                .and_then(|version| {
                    version
                        .compare_to(&*BASH_INPUT_REPORTING_MINIMUM_VERSION, Cmp::Ge)
                        .then_some([escape_sequences::C0::ESC, b'i'])
                }),
        }
    }

    /// Returns `true` if the given command should be written to history based on the shell's
    /// options.
    pub fn should_add_command_to_history(&self, command: &str) -> bool {
        if command.trim().is_empty() {
            return false;
        }
        match &self.options {
            Some(options) => {
                if !command.starts_with(' ') {
                    return true;
                }

                // If the command starts with a space, check the shell's options to determine if it
                // should be added to history.
                match self.shell_type {
                    ShellType::Zsh => !options.contains("histignorespace"),
                    ShellType::Bash => {
                        // Look for our fake option that contains the value of the HISTCONTROL
                        // environment variable.
                        if let Some(histcontrol) =
                            options.iter().find(|opt| opt.starts_with("!histcontrol"))
                        {
                            // HISTCONTROL can contain a single value or a list of values separated
                            // by colons.  In either case, we want to know whether the "ignorespace"
                            // or "ignoreboth" (ignorespace+ignoredups) values are present.
                            !histcontrol.contains("ignorespace")
                                && !histcontrol.contains("ignoreboth")
                        } else {
                            true
                        }
                    }
                    _ => true,
                }
            }
            None => true,
        }
    }
}

/// Brief, human-readable description of a shell session.
#[derive(Clone, Debug)]
pub enum ShellName {
    /// The description isn't more descriptive than the [`ShellType`], so the [`ShellType`] should
    /// take precedent if that is known.
    LessDescriptive(String),
    /// The description is more descriptive than the [`ShellType`], so it should take precendent
    /// over the [`ShellType`] even if it is known.
    MoreDescriptive(String),
}

impl Deref for ShellName {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        match self {
            Self::MoreDescriptive(name) | Self::LessDescriptive(name) => name,
        }
    }
}

impl ShellName {
    pub fn blank() -> Self {
        Self::MoreDescriptive(String::new())
    }
}

#[derive(Copy, Clone, Debug, Hash, PartialEq, Eq, Serialize, Deserialize, Sequence)]
pub enum ShellType {
    Zsh,
    Bash,
    Fish,
    PowerShell,
}

impl From<ShellType> for command_corrections::Shell {
    fn from(s: ShellType) -> command_corrections::Shell {
        match s {
            ShellType::Bash => command_corrections::Shell::Bash,
            ShellType::Zsh => command_corrections::Shell::Zsh,
            ShellType::Fish => command_corrections::Shell::Fish,
            ShellType::PowerShell => command_corrections::Shell::PowerShell,
        }
    }
}

impl From<ShellType> for warp_util::path::ShellFamily {
    fn from(value: ShellType) -> Self {
        match value {
            ShellType::Zsh | ShellType::Bash | ShellType::Fish => Self::Posix,
            ShellType::PowerShell => Self::PowerShell,
        }
    }
}

impl ShellType {
    // Returns a shell type from a shell executable name
    pub fn from_name(name: &str) -> Option<Self> {
        // Support (/usr/bin/zsh /bin/zsh -zsh or zsh)
        if name == "bash"
            || name == "-bash"
            || name.ends_with("/bash")
            || name.ends_with("bash.exe")
        {
            Some(ShellType::Bash)
        } else if name == "zsh" || name == "-zsh" || name.ends_with("/zsh") {
            Some(ShellType::Zsh)
        } else if name == "fish" || name == "-fish" || name.ends_with("/fish") {
            Some(ShellType::Fish)
        } else if name == "pwsh"
            || name.ends_with("/pwsh")
            || name.ends_with("pwsh.exe")
            || name == "powershell"
            || name.ends_with("/powershell")
            || name.ends_with("powershell.exe")
        {
            Some(ShellType::PowerShell)
        } else {
            None
        }
    }

    // Returns a shell type from a markdown code block language specifier
    pub fn from_markdown_language_spec(language: &str) -> Option<Self> {
        match language {
            "bash" | "shell" | "sh" => Some(ShellType::Bash),
            "zsh" => Some(ShellType::Zsh),
            "fish" => Some(ShellType::Fish),
            "powershell" | "pwsh" => Some(ShellType::PowerShell),
            _ => None,
        }
    }

    /// Returns locations of history files in order of search precedence
    pub fn history_files(self) -> Vec<String> {
        match self {
            ShellType::Zsh => vec!["~/.zsh_history".to_string(), "~/.zhistory".to_string()],
            ShellType::Bash => vec!["~/.bash_history".to_string()],
            ShellType::Fish => vec!["~/.local/share/fish/fish_history".to_string()],
            #[cfg(not(windows))]
            ShellType::PowerShell => {
                vec!["~/.local/share/powershell/PSReadLine/ConsoleHost_history.txt".to_string()]
            }
            #[cfg(windows)]
            ShellType::PowerShell => {
                vec![base_config_dir()
                    .join("Microsoft/Windows/PowerShell/PSReadLine/ConsoleHost_history.txt")
                    .display()
                    .to_string()]
            }
        }
    }

    /// Returns the potential paths to the RC file relative to the `home` directory.
    pub fn rc_file_paths(&self, os: TargetOS) -> Vec<PathBuf> {
        let home_dir = Path::new(match os {
            TargetOS::Windows => "$HOME",
            _ => "~",
        });
        let relative_paths = match (self, os) {
            (ShellType::PowerShell, TargetOS::Windows) => {
                vec![Path::new(
                    ".config/powershell/Microsoft.PowerShell_profile.ps1",
                )]
            }
            // We need to make sure this works for either editor of PowerShell (PowerShell Core or
            // Windows PowerShell) so just write the file to both.
            (ShellType::PowerShell, _) => vec![
                Path::new("Documents/PowerShell/Microsoft.PowerShell_profile.ps1"),
                Path::new("Documents/WindowsPowerShell/Microsoft.PowerShell_profile.ps1"),
            ],
            (_, TargetOS::Windows) => vec![],
            (ShellType::Bash, _) => vec![Path::new(".bashrc")],
            (ShellType::Zsh, _) => vec![Path::new(".zshrc")],
            (ShellType::Fish, _) => vec![Path::new(".config/fish/config.fish")],
        };
        relative_paths
            .iter()
            .map(|relative_path| home_dir.join(relative_path))
            .collect()
    }

    /// Returns the syntax to use to run a second command only if the first one succeeds.
    /// NOTE: Guarded with `cfg(unix)` b/c PowerShell didn't have the `&&` operator until v7. On
    /// Unix, we can safely assume v7, but Windows comes with PowerShell v5 out of the box.
    #[cfg(unix)]
    pub fn and_combiner(self) -> &'static str {
        match self {
            ShellType::Bash | ShellType::Zsh | ShellType::PowerShell => " && ",
            ShellType::Fish => "; and ",
        }
    }

    /// Returns whether the current shell supports native shell completions.
    fn supports_native_shell_completions(&self) -> bool {
        matches!(self, ShellType::Zsh)
    }

    /// Returns the syntax to run a second command regardless if the first one succeeds.
    pub fn or_combiner(self) -> &'static str {
        match self {
            ShellType::Bash | ShellType::Zsh | ShellType::PowerShell => " ; ",
            ShellType::Fish => "; or ",
        }
    }

    /// Given the output of the `alias` command, returns a map of alias keys to values.
    pub fn aliases(self, alias_output: &str) -> HashMap<SmolStr, String> {
        match self {
            ShellType::Zsh => alias_output
                .lines()
                .filter_map(|line| {
                    // ZSH outputs aliases in a key pair of `name=val`, with both the name and value
                    // optionally escaped. Aliases that span multiple lines are properly escaped.

                    line.split_once('=')
                        .and_then(|(key, value)| unescape_alias_key_value(key, value))
                })
                .collect(),
            ShellType::Bash => {
                // Bash outputs aliases in a key pair of `alias name=val`. Alias values that
                // span multiple lines are not escaped. For simplicity in parsing this, append a
                // newline on the front of the string and then split on "\nalias"
                let alias_output = format!("\n{alias_output}");
                alias_output
                    .split("\nalias ")
                    .filter_map(|line| {
                        line.split_once('=')
                            .and_then(|(key, value)| unescape_alias_key_value(key, value))
                    })
                    .collect()
            }
            ShellType::Fish => {
                let alias_output = format!("\n{alias_output}");
                alias_output
                    .split("\nalias ")
                    .filter_map(|line| {
                        // Fish outputs aliases in the form:
                        // alias name val
                        // For simplicity in parsing this, append a newline on the front of
                        // the string and then split on "\nalias"
                        // Note: Currently doesn't support alias values that span multiple
                        // lines due to fish not respecting their specified format for
                        // outputing alias values with multiple lines.
                        line.split_once(' ')
                            .and_then(|(key, value)| unescape_alias_key_value(key, value))
                    })
                    .collect()
            }
            ShellType::PowerShell => alias_output
                .lines()
                .filter_map(|line| {
                    // PowerShell outputs aliases in a key pair of `name -> val`, with both the name
                    // and value optionally escaped. Aliases that span multiple lines are properly
                    // escaped.
                    line.split_once(" -> ")
                        .filter(|(_, value)| !value.trim().is_empty())
                        .and_then(|(key, value)| unescape_alias_key_value(key, value))
                })
                .collect(),
        }
    }

    pub fn abbreviations(self, abbr_output: &str) -> HashMap<SmolStr, String> {
        match self {
            ShellType::Fish => {
                abbr_output
                    .lines()
                    .filter_map(|line| {
                        // Fish outputs abbreviations in the form:
                        // abbr -a -U -- name val # optional comment
                        // First, strip off the comment at the end if it exists. Then,
                        // strip off the preamble characters and separate the key and value
                        // Note: The key cannot have spaces in it (fish won't allow you to define an
                        // abbreviation with a space in the name)
                        line.split_once(" #")
                            .map_or(line, |split_line| split_line.0)
                            .split_once(" -- ")
                            .and_then(|(_, abbr)| abbr.split_once(' '))
                    })
                    .filter_map(|(key, value)| unescape_alias_key_value(key, value))
                    .collect()
            }
            // Abbreviations are currently only supported in fish
            _ => HashMap::new(),
        }
    }

    /// Parse the contents of the shell's history file
    pub fn parse_history(self, history_file_bytes: &[u8]) -> Vec<String> {
        let mut history_lines: Vec<String> = Vec::new();
        match self {
            ShellType::Zsh => {
                let mut current_line = String::new();

                let unmetafied_content = zsh_unmetafy(history_file_bytes);

                for line in unmetafied_content.lines() {
                    // Only strip the extended history prefix on the first line of each command.
                    // Continuation lines are raw command text and should not be stripped.
                    let command_part = if current_line.is_empty() {
                        strip_zsh_extended_prefix(line)
                    } else {
                        line
                    };
                    current_line.push_str(command_part);

                    // ZSH considers a command to be multi-line if it ends in a backslash, see
                    // https://github.com/johan/zsh/blob/master/Src/hist.c#L2192-L2220.
                    if line.ends_with('\\') {
                        // Replace the last backslash with a new line.
                        current_line.pop();
                        current_line.push('\n');
                    } else if !current_line.is_empty() {
                        history_lines.push(current_line.clone());
                        current_line.clear();
                    }
                }
            }
            ShellType::Bash => {
                let history_file_contents = String::from_utf8_lossy(history_file_bytes);
                // Bash format if HISTTIMEFORMAT not set
                // <command>
                // Bash format if HISTTIMEFORMAT set
                // #<timestamp>
                // <command>
                history_lines = history_file_contents
                    .lines()
                    .filter_map(|line| {
                        if line.starts_with('#') || line.is_empty() {
                            None
                        } else {
                            Some(line.to_owned())
                        }
                    })
                    .collect()
            }

            ShellType::Fish => {
                let history_file_contents = String::from_utf8_lossy(history_file_bytes);
                // fish has pseudo-yaml.
                // The commands start with "- cmd: ".
                history_lines = history_file_contents
                    .lines()
                    .filter_map(|line| line.strip_prefix("- cmd: "))
                    .map(fish_unescape_history_yaml)
                    .collect()
            }

            ShellType::PowerShell => {
                let history_file_contents = String::from_utf8_lossy(history_file_bytes);
                history_lines = history_file_contents
                    .lines()
                    .filter(|s| !s.is_empty())
                    .map(|s| s.to_owned())
                    .collect()
            }
        }

        history_lines
    }

    /// Bytes used to notify the shell to delete the current buffer.
    ///
    /// In zsh this is implemented by killing the ZLE buffer and must match the `bindkey` call in
    /// the boostrap `zsh.sh`. In bash this is implemented via a custom `bind` that calls
    /// `kill-whole-line` and must match the `bind` in `bash.sh`.  In fish this is a custom `bind`
    /// that clears the command line. PowerShell cannot use a binding that contains the letter "p"
    /// (DLE maps to ctrl-p) because it does virtual key code translation based on the current
    /// layout, and not all layouts have the letter "p".
    pub fn kill_buffer_bytes(self) -> &'static [u8] {
        const POWERSHELL_BINDING: [u8; 2] = [escape_sequences::C0::ESC, b'2'];
        const OTHER_BINDING: [u8; 1] = [escape_sequences::C0::DLE];
        match self {
            ShellType::PowerShell => POWERSHELL_BINDING.as_slice(),
            ShellType::Zsh | ShellType::Bash | ShellType::Fish => OTHER_BINDING.as_slice(),
        }
    }

    /// Bytes used to execute a command, once the command text is sent
    pub fn execute_command_bytes(self) -> &'static [u8] {
        match self {
            ShellType::Bash | ShellType::Zsh => &b"\n"[..],
            ShellType::PowerShell => &b"\r"[..],
            // For Fish, we send an extra space, immediately followed by backspace, and then
            // the newline character. The backspace ensures that any autosuggestions are
            // suppressed, so we don't get erroneous ghosted autosuggestion text in the command
            // grid.
            ShellType::Fish => &b" \x7f\n"[..],
        }
    }

    /// The name of the shell
    pub fn name(self) -> &'static str {
        match self {
            ShellType::Zsh => "zsh",
            ShellType::Bash => "bash",
            ShellType::Fish => "fish",
            ShellType::PowerShell => "pwsh",
        }
    }

    /// If true, Warp will bootstrap the shell if it's the login shell on the remote host.
    pub fn is_fully_supported_remotely(&self) -> bool {
        match self {
            ShellType::Zsh | ShellType::Bash => true,
            ShellType::Fish | ShellType::PowerShell => false,
        }
    }

    pub fn shell_command_to_get_executables(&self) -> &'static str {
        match self {
            ShellType::Bash => {
                // Since `compgen -c` returns more than just executables (and the output itself
                // doesn't include any info about what the type of the word is), we filter down to
                // executable "file"s. Note that we do this at the shell level since if we did this
                // as a post-process step, it would require N filesystem calls, which would not scale
                // well for remote sessions. Additionally, we invoke `type` once, passing it the full
                // list of commands, to avoid a lot of overhead invoking it thousands of times for
                // systems with a lot of installed commands.
                r#"COMMANDS=($(compgen -c)); TYPES=($(type -t ${COMMANDS[@]})); for i in "${!COMMANDS[@]}"; do if [[ ${TYPES[$i]} == "file" ]]; then echo ${COMMANDS[$i]}; fi; done"#
            }
            ShellType::Fish => {
                // Although `complete -C` returns more than just executables, we don't check the type here
                // since the output of `complete` already tells us what is an executable ('command') and what isn't
                // (whereas the output of `compgen` doesn't so we need to check the `type` there).
                // Instead we post-process the output below.  We try to use the `--escape` argument, but
                // if we're running a version of fish that doesn't support it, try again without it.
                "complete -C --escape '' || complete -C ''"
            }
            ShellType::Zsh => {
                // zsh is cool and has an API for just fetching executables
                "builtin print -l -- ${(ok)commands}"
            }
            ShellType::PowerShell => {
                // PowerShell does not deal in strings, but in Objects and Object lists
                // Get-Command and Select-Object each return a list of Objects. In the shell,
                // this will print one item per line. However, when it is converted to a string,
                // it will join the entries together with a space. So to make sure we get one item
                // per line, we explicitly join the results with a newline.
                "Get-Command -CommandType Application | Select-Object -ExpandProperty Name"
            }
        }
    }

    /// Returns a Vec containing executable commands parsed from the given `output`.
    ///
    /// If `output` is `Err(..)`, returns an empty Vec.
    pub fn executables_from_shell_command_output(
        &self,
        output: Result<CommandOutput>,
        is_msys2: bool,
    ) -> Vec<SmolStr> {
        match output {
            Ok(command_output) if command_output.status == CommandExitStatus::Success => {
                let Ok(output_string) = command_output.to_string() else {
                    return Vec::new();
                };
                match self {
                    ShellType::Bash | ShellType::Zsh => {
                        // For bash and zsh, we wrote the command such that the output is just
                        // a list of executable files.
                        if !is_msys2 {
                            return output_string.lines().map(Into::into).collect();
                        }

                        // TODO add this to fish
                        output_string
                            .lines()
                            // Remove all `.dll` files.
                            .filter(|line| !line.to_lowercase().ends_with("dll"))
                            .flat_map(|line| {
                                // Those suffixes are contained in `PATHEXT`.
                                for ext in PATHEXT {
                                    // If the command ends with one of those suffixes, tell
                                    // Warp about this command as-is and also sans-suffix, e.g.
                                    // "git" and "git.exe".
                                    if line.to_lowercase().ends_with(&ext.to_lowercase()) {
                                        let trimmed = &line[..line.len() - ext.len()];
                                        return Box::<[&str]>::from([trimmed, line]);
                                    }
                                }

                                // Otherwise, pass it through unaltered.
                                Box::<[&str]>::from([line])
                            })
                            .map_into()
                            .collect()
                    }
                    ShellType::Fish => {
                        // This is the post-processing for Fish explained above.
                        output_string
                            .lines()
                            .filter_map(|line| {
                                line.split_once(char::is_whitespace).and_then(
                                    |(command, command_type)| {
                                        let is_executable = command_type == "command"
                                            || command_type == "command link"
                                            || command_type.starts_with("Executable");
                                        is_executable.then_some(command.into())
                                    },
                                )
                            })
                            .collect()
                    }
                    ShellType::PowerShell => {
                        // Windows allows certain suffixes, e.g. "exe", to be elided.
                        if cfg!(windows) {
                            output_string
                                .lines()
                                .flat_map(|line| {
                                    // Those suffixes are contained in `PATHEXT`.
                                    for ext in PATHEXT {
                                        // If the command ends with one of those suffixes, tell
                                        // Warp about this command as-is and also sans-suffix, e.g.
                                        // "git" and "git.exe".
                                        if line.to_lowercase().ends_with(&ext.to_lowercase()) {
                                            let trimmed = &line[..line.len() - ext.len()];
                                            return Box::<[&str]>::from([trimmed, line]);
                                        }
                                    }
                                    // Otherwise, pass it through unaltered.
                                    Box::<[&str]>::from([line])
                                })
                                .map_into()
                                .collect()
                        } else {
                            output_string.lines().map_into().collect()
                        }
                    }
                }
            }
            Ok(output) => {
                log::warn!("Generator for executable names failed");
                if let Ok(output_string) = output.to_string() {
                    log::warn!("{output_string}");
                };
                Vec::new()
            }
            Err(e) => {
                log::warn!("Generator for executable names failed: {e:#}");
                Vec::new()
            }
        }
    }

    pub fn force_in_band_command_executor(&self) -> bool {
        // TODO: Remove this function once we have confidence in using a local executor in
        // powershell.
        false
    }
}

/// Provides the necessary info to be able to launch and bootstrap the selected AvailableShell. For
/// executables, this is the path to the executable and the shell type. For WSL, this is the distro
/// name. For Docker sandboxes, this is the `sbx` CLI path plus the base Docker
/// image; the shell inside the container is whatever the image provides.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ShellLaunchData {
    Executable {
        executable_path: PathBuf,
        shell_type: ShellType,
    },
    /// Windows Subsystem for Linux.
    #[cfg_attr(not(windows), allow(dead_code))]
    WSL { distro: String },
    MSYS2 {
        executable_path: PathBuf,
        shell_type: ShellType,
    },
    /// A shell running inside a `sbx`-managed Docker sandbox container.
    ///
    /// A dedicated variant ensures callers can't accidentally execute the
    /// sandbox as if it were a regular local shell: the `sbx` binary at
    /// `sbx_path` is not a shell, it's the CLI we use to enter the container.
    DockerSandbox {
        sbx_path: PathBuf,
        /// Base Docker image to use when creating the sandbox (passed as
        /// `sbx run --template <image>`). `None` means "use sbx's default
        /// image".
        base_image: Option<String>,
    },
}

impl ShellLaunchData {
    /// Converts the given path string to a OS-native PathBuf, performing any necessary shell-informed conversions.
    pub fn maybe_convert_absolute_path(&self, path_str: &str) -> Option<PathBuf> {
        match self {
            ShellLaunchData::Executable { .. } => Some(PathBuf::from(path_str)),
            ShellLaunchData::WSL { distro } => {
                let unix_path = TypedPath::unix(path_str);
                convert_wsl_to_windows_host_path(&unix_path, distro).ok()
            }
            ShellLaunchData::MSYS2 {
                executable_path, ..
            } => {
                let unix_path = TypedPath::unix(path_str);
                convert_msys2_to_windows_native_path(
                    &unix_path,
                    &msys2_exe_to_root(WindowsPath::new(
                        executable_path.as_os_str().as_encoded_bytes(),
                    )),
                )
                .ok()
            }
            // Paths inside the sandbox container are plain Unix paths.
            ShellLaunchData::DockerSandbox { .. } => Some(PathBuf::from(path_str)),
        }
    }

    /// Converts a shell-encoded [`typed_path::TypedPathBuf`] into an OS-native path.
    fn maybe_convert_shell_encoded_path(
        &self,
        shell_encoded_path: TypedPathBuf,
    ) -> Option<PathBuf> {
        match self {
            ShellLaunchData::Executable { .. } | ShellLaunchData::DockerSandbox { .. } => {
                PathBuf::try_from(shell_encoded_path).ok()
            }
            ShellLaunchData::WSL { distro } => {
                convert_wsl_to_windows_host_path(&shell_encoded_path.to_path(), distro).ok()
            }
            ShellLaunchData::MSYS2 {
                executable_path, ..
            } => convert_msys2_to_windows_native_path(
                &shell_encoded_path.to_path(),
                &msys2_exe_to_root(WindowsPath::new(
                    executable_path.as_os_str().as_encoded_bytes(),
                )),
            )
            .ok(),
        }
    }

    /// Naively changes the path string to an OS-native encoding, without performing shell-informed conversions.
    fn to_native_path_encoding(&self, path_str: &str) -> Option<PathBuf> {
        match self {
            ShellLaunchData::Executable { .. } => Some(PathBuf::from(path_str)),
            ShellLaunchData::WSL { .. } | ShellLaunchData::MSYS2 { .. } => {
                let windows_encoding = TypedPath::unix(path_str).with_windows_encoding();
                PathBuf::try_from(windows_encoding).ok()
            }
            // The container is Unix; Warp runs on the host, so paths are
            // already in the host's native encoding. Pass through unchanged.
            ShellLaunchData::DockerSandbox { .. } => Some(PathBuf::from(path_str)),
        }
    }

    /// Converts a path string to a shell's encoding.
    fn to_shell_encoding<'a>(&self, path_str: &'a str) -> TypedPath<'a> {
        match self {
            ShellLaunchData::Executable { .. } => {
                if cfg!(unix) {
                    TypedPath::unix(path_str)
                } else {
                    TypedPath::windows(path_str)
                }
            }
            ShellLaunchData::WSL { .. }
            | ShellLaunchData::MSYS2 { .. }
            | ShellLaunchData::DockerSandbox { .. } => TypedPath::unix(path_str),
        }
    }

    /// Attempts to append the relative path to the base path and convert it into a OS-native PathBuf,
    /// performing any necessary shell-informed conversions.
    pub fn maybe_convert_relative_path(
        &self,
        base_path_str: &str,
        relative_path_str: &str,
    ) -> Option<PathBuf> {
        let base_path = self.to_shell_encoding(base_path_str);
        let rest_of_path = self.to_shell_encoding(relative_path_str);
        let absolute_typed_path = base_path.join(rest_of_path);
        self.maybe_convert_shell_encoded_path(absolute_typed_path)
    }

    /// Joins the given path string to the base path, ensuring the given path string is encoded correctly.
    pub fn join_to_native_path(&self, base_path: &Path, path_str: &str) -> Option<PathBuf> {
        self.to_native_path_encoding(path_str)
            .map(|rest_of_path| base_path.join(rest_of_path))
    }

    /// How to present this data to the user for error messages.
    pub fn shell_detail(&self) -> String {
        match self {
            Self::Executable {
                executable_path, ..
            }
            | Self::MSYS2 {
                executable_path, ..
            } => executable_path.to_string_lossy().into_owned(),
            Self::WSL { distro } => distro.to_owned(),
            Self::DockerSandbox { base_image, .. } => match base_image {
                Some(image) => format!("Docker sandbox ({image})"),
                None => "Docker sandbox".to_owned(),
            },
        }
    }
}

impl From<ShellLaunchData> for SessionPlatform {
    fn from(data: ShellLaunchData) -> Self {
        match data {
            ShellLaunchData::Executable { .. } => SessionPlatform::Native,
            ShellLaunchData::WSL { .. } => SessionPlatform::WSL,
            ShellLaunchData::MSYS2 { .. } => SessionPlatform::MSYS2,
            ShellLaunchData::DockerSandbox { .. } => SessionPlatform::DockerSandbox,
        }
    }
}

/// Unescape the key and value for an alias, returning None if either fails
fn unescape_alias_key_value(key: &str, value: &str) -> Option<(SmolStr, String)> {
    let key = unescape_quotes(key)
        .map_err(|e| {
            log::error!("Unable to unescape key for alias: {e}");
            e
        })
        .ok()?;
    let value = unescape_quotes(value)
        .map_err(|e| {
            log::error!("Unable to unescape value for alias: {e}");
            e
        })
        .ok()?;
    Some((key.into(), value))
}

// fish history replaces newlines with \n, and \ with \\.
// This does the reverse.
fn fish_unescape_history_yaml(line: &str) -> String {
    let mut result = String::new();
    result.reserve(line.len());
    let mut escaped = false;
    for ch in line.chars() {
        match ch {
            '\\' => {
                if escaped {
                    result.push('\\');
                }
                escaped = !escaped;
            }
            'n' if escaped => {
                result.push('\n');
                escaped = false;
            }
            _ => {
                result.push(ch);
                escaped = false;
            }
        }
    }
    result
}

// Basically a translation of this function into rust:
// http://mika.l3ib.org/code/unmetafy.c
// To unmetafy a string from zsh's internal format, we need to skip each meta symbol
// and XOR the next symbol with 32.
fn zsh_unmetafy(content: &[u8]) -> String {
    let mut unmetafied = Vec::new();

    match content.last() {
        None => "".into(),
        Some(byte) => {
            let mut following_byte = *byte;

            content.iter().rev().skip(1).for_each(|current_byte| {
                if *current_byte == ZSH_META {
                    following_byte ^= 32;
                } else {
                    unmetafied.push(following_byte);
                    following_byte = *current_byte;
                }
            });

            unmetafied.push(following_byte);
            unmetafied.reverse();
            String::from_utf8_lossy(&unmetafied).into()
        }
    }
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
