use std::fs::{create_dir_all, write, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use itertools::Itertools as _;
use strum::IntoEnumIterator;
use strum_macros::EnumIter;
use version_compare::Version;
use warp::integration_testing::terminal::util::{
    current_shell_starter_and_version, default_histfile_directory, ExpectedOutput,
};
use warp::integration_testing::view_getters;
use warp::terminal::shell;
use warp::terminal::shell::ShellType;
use warpui_core::{App, WindowId};

use crate::builder::cargo_target_tmpdir;

pub fn get_input_buffer(
    app: &App,
    window_id: WindowId,
    tab_index: usize,
    pane_index: usize,
) -> String {
    view_getters::input_view(app, window_id, tab_index, pane_index)
        .read(app, |input, app| input.buffer_text(app))
}

#[derive(EnumIter)]
pub enum ShellRcType {
    Bash,
    Zsh,
    Fish,
    PowerShell,
}

impl ShellRcType {
    /// Returns the potential paths to the RC file relative to the `home` directory.
    fn rc_file_paths(&self, home_dir: impl AsRef<Path>) -> Vec<PathBuf> {
        let relative_paths = match self {
            ShellRcType::Bash => vec![Path::new(".bash_profile")],
            ShellRcType::Zsh => vec![Path::new(".zshrc")],
            ShellRcType::Fish => vec![Path::new(".config/fish/config.fish")],
            #[cfg(not(windows))]
            ShellRcType::PowerShell => {
                vec![Path::new(
                    ".config/powershell/Microsoft.PowerShell_profile.ps1",
                )]
            }
            // We need to make sure this works for either editor of PowerShell (PowerShell Core or
            // Windows PowerShell) so just write the file to both.
            #[cfg(windows)]
            ShellRcType::PowerShell => vec![
                Path::new("Documents/PowerShell/Microsoft.PowerShell_profile.ps1"),
                Path::new("Documents/WindowsPowerShell/Microsoft.PowerShell_profile.ps1"),
            ],
        };
        relative_paths
            .iter()
            .map(|relative_path| home_dir.as_ref().join(relative_path))
            .collect()
    }
}

/// Sets the location of the ZSH `HISTFILE` to the home directory.
/// ZSH does not have a default location for the HISTFILE. However, MacOS has a custom `/etc/zshrc`
/// file that sets the default location of the `HISTFILE` to be located within the home directory.
/// To ensure we our tests are consistent across platforms, we set the value of `HISTFILE` to
/// `HOME` in the same way MacOS does.
pub fn set_zsh_histfile_location(dir: impl AsRef<Path>) {
    let path = ShellRcType::Zsh
        .rc_file_paths(dir)
        .into_iter()
        .exactly_one()
        .expect("zsh only has one RC file path");
    create_dir_all(path.parent().expect("Parent of RC file should exist"))
        .expect("Should be able to create path to RC file");

    let mut rc_file = OpenOptions::new()
        .append(true)
        .create(true)
        .open(path)
        .expect("Cannot open zshrc file");

    rc_file
        .write_all("\nHISTFILE=${ZDOTDIR:-$HOME}/.zsh_history".as_bytes())
        .expect("Failed to write to zshrc file");
}

/// Usually, the title is the pwd where the home dir is shortened as "~".
/// However, this wasn't the case in Fish prior to version 3.4.0, see:
/// https://github.com/fish-shell/fish-shell/commit/698b8189356c8224443fdfc4399408f932d53aca
pub(crate) fn tab_title_in_home_dir(home_suffix: &str) -> String {
    let (shell_starter, shell_version) = current_shell_starter_and_version();
    let shell_version = Version::from(&shell_version).expect("shell version must be valid");
    if shell_starter.shell_type() == ShellType::Fish
        && shell_version < Version::from("3.4.0").expect("shell version must be valid")
    {
        let home_path = Path::new(&cargo_target_tmpdir::get()).join(home_suffix);
        format!(
            "fish {}",
            home_path.to_str().expect("path must be valid unicode")
        )
    } else {
        String::from("~")
    }
}

/// Writes the `rc_contents` into the corresponding RC files depending on the value of
/// `ShellRcType`.
pub fn write_rc_files_for_test<P, C>(
    dir: P,
    rc_contents: C,
    shell_rc_types: impl IntoIterator<Item = ShellRcType>,
) where
    P: AsRef<Path>,
    C: AsRef<str>,
{
    for rc_type in shell_rc_types.into_iter() {
        let path_ref = dir.as_ref();

        let paths = rc_type.rc_file_paths(path_ref);
        for path in paths {
            create_dir_all(path.parent().expect("Parent of RC file path should exist"))
                .expect("Should be able to create path to RC file");
            if let Err(e) = write(&path, rc_contents.as_ref()) {
                panic!("Could not write rc file {:?}: {}", path.to_str(), e);
            }
        }
    }
}

/// Writes the same `rc_contents` for all possible shell types supported by Warp.
pub fn write_all_rc_files_for_test<P, C>(dir: P, rc_contents: C)
where
    P: AsRef<Path>,
    C: AsRef<str>,
{
    write_rc_files_for_test(dir, rc_contents, ShellRcType::iter())
}

/// Writes a histfile for `shell_types` to the given `dir`.
///
/// `commands` are written in the order that they're specified in the given vector; this means the
/// commands at the beginning of the vector read as if they were executed before commands at the end
/// of the vector.
///
/// Each histfile is written in the `ShellType`'s expected format.
pub fn write_histfiles_for_test<P>(
    home_dir: P,
    commands: Vec<&'static str>,
    shell_types: impl IntoIterator<Item = ShellType>,
) where
    P: AsRef<Path>,
{
    for shell_type in shell_types.into_iter() {
        let histfile_dir = default_histfile_directory(&shell_type, home_dir.as_ref());
        let path_ref = histfile_dir.as_path();
        create_dir_all(path_ref)
            .expect("Should be able to create {shell_type:?} config directories");

        let path = match shell_type {
            ShellType::Bash => path_ref.join(".bash_history"),
            ShellType::Zsh => path_ref.join(".zsh_history"),
            ShellType::Fish => path_ref.join("fish_history"),
            ShellType::PowerShell => path_ref.join("ConsoleHost_history.txt"),
        };

        let histfile_contents = match shell_type {
            ShellType::Bash | ShellType::PowerShell => {
                let mut contents = "".to_owned();
                for command in commands.clone() {
                    contents += format!("{command}\n").as_str();
                }
                contents
            }
            ShellType::Fish => {
                let mut contents = "".to_owned();
                for command in commands.clone() {
                    println!("COMMAND:{command}");
                    contents += format!(
                        "- cmd: {}\n  when: {}\n",
                        command,
                        chrono::Local::now().timestamp()
                    )
                    .as_str();
                }
                contents
            }
            ShellType::Zsh => {
                let mut contents = "".to_owned();
                for command in commands.clone() {
                    contents +=
                        format!(": {}:0;{}\n", chrono::Local::now().timestamp(), command).as_str();
                }
                contents
            }
        };
        println!("histfile path:{:?}", &path);
        if let Err(e) = write(&path, histfile_contents.as_bytes()) {
            panic!("Could not write histfile {:?}: {}", path.to_str(), e);
        }
    }
}

/// Returns the string (ie. expected output etc) for the shell currently used for testing.
pub fn per_shell_output(
    per_shell_output: Vec<(shell::ShellType, &str)>,
) -> impl ExpectedOutput + '_ {
    let (starter, _) = current_shell_starter_and_version();
    for (shell_type, output) in per_shell_output {
        if starter.shell_type() == shell_type {
            return Some(output);
        }
    }
    None
}

/// Indicates a test that currently does not work in powershell. As part of CORE-2303, we should
/// eventually be removing all uses of this function.
pub fn skip_if_powershell_core_2303() -> bool {
    let (starter, _) = current_shell_starter_and_version();
    !matches!(starter.shell_type(), ShellType::PowerShell)
}

/// Gets the name of the system user for which the test binary is running.
pub fn get_local_user() -> String {
    whoami::username()
}
