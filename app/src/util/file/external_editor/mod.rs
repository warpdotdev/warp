#[cfg(any(target_os = "linux", target_os = "freebsd"))]
mod linux;
#[cfg(target_os = "macos")]
mod mac;
pub mod settings;
#[cfg(target_os = "windows")]
mod windows;

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use settings::EditorChoice;
use warp_util::path::LineAndColumnArg;
use warpui::{AppContext, SingletonEntity};

pub use self::settings::{EditorLayout, EditorSettings};

pub const SUPPORTED_EDITORS: &[Editor] = &[
    Editor::VSCode,
    Editor::VSCodeInsiders,
    Editor::Atom,
    Editor::CLion,
    Editor::CLionCE,
    Editor::RustRoverPreview,
    Editor::RustRover,
    Editor::IntelliJ,
    Editor::IntelliJCE,
    Editor::PyCharm,
    Editor::PyCharmCE,
    Editor::WebStorm,
    Editor::PhpStorm,
    Editor::RubyMine,
    #[cfg(not(target_os = "macos"))]
    // On Linux, all versions of sublime use the same app-ids, so
    // we only have one entry
    Editor::Sublime,
    #[cfg(target_os = "macos")]
    Editor::Sublime2,
    #[cfg(target_os = "macos")]
    Editor::Sublime3,
    #[cfg(target_os = "macos")]
    Editor::Sublime4,
    #[cfg(any(target_os = "macos", any(target_os = "linux", target_os = "freebsd")))]
    // Zed is available on macos and linux
    Editor::Zed,
    #[cfg(any(target_os = "macos", any(target_os = "linux", target_os = "freebsd")))]
    // Zed Preview is available on macos and linux
    Editor::ZedPreview,
    Editor::GoLand,
    Editor::Rider,
    Editor::DataSpell,
    Editor::DataGrip,
    Editor::AndroidStudio,
    #[cfg(any(target_os = "macos", windows))]
    // Cursor *can* run on linux, but does not have a .desktop file
    Editor::Cursor,
    Editor::Windsurf,
];

#[derive(
    Debug,
    Clone,
    Copy,
    Serialize,
    Deserialize,
    PartialEq,
    Eq,
    Hash,
    enum_iterator::Sequence,
    schemars::JsonSchema,
    settings_value::SettingsValue,
)]
#[schemars(description = "An external code editor.", rename_all = "snake_case")]
pub enum Editor {
    VSCode,
    VSCodeInsiders,
    PyCharm,
    PyCharmCE,
    IntelliJ,
    IntelliJCE,
    CLion,
    CLionCE,
    RustRoverPreview,
    RustRover,
    #[cfg(not(target_os = "macos"))]
    Sublime,
    #[cfg(target_os = "macos")]
    Sublime4,
    #[cfg(target_os = "macos")]
    Sublime3,
    #[cfg(target_os = "macos")]
    Sublime2,
    Atom,
    WebStorm,
    PhpStorm,
    RubyMine,
    Zed,
    ZedPreview,
    GoLand,
    Rider,
    DataSpell,
    DataGrip,
    AndroidStudio,
    Cursor,
    Windsurf,
}

impl std::fmt::Display for Editor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}",
            match self {
                Editor::VSCode => "VSCode",
                Editor::VSCodeInsiders => "VSCode Insiders",
                Editor::PyCharm => "PyCharm",
                Editor::PyCharmCE => "PyCharm Community Edition",
                Editor::IntelliJ => "IntelliJ",
                Editor::IntelliJCE => "IntelliJ Community Edition",
                Editor::CLion => "CLion",
                Editor::CLionCE => "CLion Community Edition",
                #[cfg(not(target_os = "macos"))]
                Editor::Sublime => "Sublime",
                #[cfg(target_os = "macos")]
                Editor::Sublime4 => "Sublime 4",
                #[cfg(target_os = "macos")]
                Editor::Sublime3 => "Sublime 3",
                #[cfg(target_os = "macos")]
                Editor::Sublime2 => "Sublime 2",
                Editor::Atom => "Atom",
                Editor::WebStorm => "WebStorm",
                Editor::PhpStorm => "PhpStorm",
                Editor::RubyMine => "RubyMine",
                Editor::Zed => "Zed",
                Editor::ZedPreview => "Zed Preview",
                Editor::GoLand => "GoLand",
                Editor::Rider => "Rider",
                Editor::DataSpell => "DataSpell",
                Editor::DataGrip => "DataGrip",
                Editor::AndroidStudio => "Android Studio",
                Editor::Cursor => "Cursor",
                Editor::RustRoverPreview => "Rust Rover (Preview)",
                Editor::RustRover => "Rust Rover",
                Editor::Windsurf => "Windsurf",
            },
        )
    }
}

impl TryFrom<&str> for Editor {
    type Error = ();

    /// Maps an editor command name to a supported Editor enum if available.
    /// This allows us to use existing editor integrations instead of shell commands when possible.
    fn try_from(editor_name: &str) -> Result<Self, Self::Error> {
        let editor_base = std::path::Path::new(editor_name)
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or(editor_name)
            .to_lowercase();

        match editor_base.as_str() {
            "code" => Ok(Editor::VSCode),
            "code-insiders" => Ok(Editor::VSCodeInsiders),
            "zed" => Ok(Editor::Zed),
            "zed-preview" => Ok(Editor::ZedPreview),
            "cursor" => Ok(Editor::Cursor),
            "windsurf" => Ok(Editor::Windsurf),
            "clion" => Ok(Editor::CLion),
            "pycharm" => Ok(Editor::PyCharm),
            "pycharm-ce" => Ok(Editor::PyCharmCE),
            "intellij" => Ok(Editor::IntelliJ),
            "intellij-ce" => Ok(Editor::IntelliJCE),
            "webstorm" => Ok(Editor::WebStorm),
            "phpstorm" => Ok(Editor::PhpStorm),
            "rubymine" => Ok(Editor::RubyMine),
            "goland" => Ok(Editor::GoLand),
            "rider" => Ok(Editor::Rider),
            "datagrip" => Ok(Editor::DataGrip),
            "dataspell" => Ok(Editor::DataSpell),
            "android-studio" => Ok(Editor::AndroidStudio),
            "rustrover" => Ok(Editor::RustRover),
            "rustrover-preview" => Ok(Editor::RustRoverPreview),
            "atom" => Ok(Editor::Atom),
            #[cfg(not(target_os = "macos"))]
            "sublime" | "subl" => Ok(Editor::Sublime),
            #[cfg(target_os = "macos")]
            "sublime" | "subl" => Ok(Editor::Sublime4), // Default to latest on macOS
            _ => Err(()),
        }
    }
}

/// Generate an editor command string using the provided editor (or $EDITOR as fallback)
/// and handle line/column positioning for common command-line editors.
/// This is primarily used for generating shell commands when opening files with $EDITOR.
pub fn generate_editor_command(
    path: &std::path::Path,
    line_col: Option<LineAndColumnArg>,
    editor: Option<&str>,
) -> String {
    let file_path_str = path.to_string_lossy();
    let quoted_path = shell_words::quote(&file_path_str);

    let editor_cmd = editor.unwrap_or("\"$EDITOR\"").to_owned();

    // Add line/column support for common editors if provided
    let Some(line_and_col) = line_col else {
        return format!("{editor_cmd} {quoted_path}");
    };
    let Some(editor_name) = editor else {
        return format!("{editor_cmd} {quoted_path}");
    };

    let editor_base = std::path::Path::new(editor_name)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(editor_name)
        .to_lowercase();

    match editor_base.as_str() {
        // Vim and Neovim: +line or +line:column
        "vim" | "nvim" | "neovim" => {
            let line_arg = if let Some(col) = line_and_col.column_num {
                format!("+{}:{}", line_and_col.line_num, col)
            } else {
                format!("+{}", line_and_col.line_num)
            };
            format!("{editor_cmd} {line_arg} {quoted_path}")
        }
        // Emacs: +line:column
        "emacs" => {
            let line_arg = if let Some(col) = line_and_col.column_num {
                format!("+{}:{}", line_and_col.line_num, col)
            } else {
                format!("+{}", line_and_col.line_num)
            };
            format!("{editor_cmd} {line_arg} {quoted_path}")
        }
        // Nano: +line,column
        "nano" => {
            let line_arg = if let Some(col) = line_and_col.column_num {
                format!("+{},{}", line_and_col.line_num, col)
            } else {
                format!("+{}", line_and_col.line_num)
            };
            format!("{editor_cmd} {line_arg} {quoted_path}")
        }
        // Pico: +line,column (same as nano)
        "pico" => {
            let line_arg = if let Some(col) = line_and_col.column_num {
                format!("+{},{}", line_and_col.line_num, col)
            } else {
                format!("+{}", line_and_col.line_num)
            };
            format!("{editor_cmd} {line_arg} {quoted_path}")
        }
        // Micro: +line:column
        "micro" => {
            let line_arg = if let Some(col) = line_and_col.column_num {
                format!("+{}:{}", line_and_col.line_num, col)
            } else {
                format!("+{}", line_and_col.line_num)
            };
            format!("{editor_cmd} {line_arg} {quoted_path}")
        }
        // Helix: file:line:column
        "hx" | "helix" => {
            let file_with_pos = if let Some(col) = line_and_col.column_num {
                format!("{}:{}:{}", quoted_path, line_and_col.line_num, col)
            } else {
                format!("{}:{}", quoted_path, line_and_col.line_num)
            };
            format!("{editor_cmd} {}", shell_words::quote(&file_with_pos))
        }
        // VS Code: --goto file:line:column
        "code" => {
            let goto_arg = if let Some(col) = line_and_col.column_num {
                format!("{}:{}:{}", quoted_path, line_and_col.line_num, col)
            } else {
                format!("{}:{}", quoted_path, line_and_col.line_num)
            };
            format!("{editor_cmd} --goto {}", shell_words::quote(&goto_arg))
        }
        // For unknown editors, fall through to basic command without line support
        _ => format!("{editor_cmd} {quoted_path}"),
    }
}

/// Opens a file in an external editor, respecting the user's editor settings.
/// This reads the configured external editor from EditorSettings and uses it if set,
/// otherwise falls back to system default.
pub fn open_file_path_in_external_editor(
    line_column_number: Option<LineAndColumnArg>,
    full_path: PathBuf,
    ctx: &mut AppContext,
) {
    let editor = match *EditorSettings::as_ref(ctx).open_file_editor {
        EditorChoice::ExternalEditor(editor) => Some(editor),
        _ => None,
    };
    open_file_path_with_editor(line_column_number, full_path, editor, ctx);
}

pub fn open_file_path_with_editor(
    line_column_number: Option<LineAndColumnArg>,
    full_path: PathBuf,
    editor: Option<Editor>,
    ctx: &mut AppContext,
) {
    cfg_if::cfg_if! {
        if #[cfg(target_os = "macos")] {
            mac::open_file_path_with_line_and_col(line_column_number, editor, &full_path, ctx);
        } else if #[cfg(any(target_os = "linux", target_os = "freebsd"))] {
            linux::open_file_path_with_line_and_col(line_column_number, editor, &full_path, ctx);
        } else if #[cfg(windows)]{
            windows::open_file_path_with_line_and_col(line_column_number, editor, &full_path, ctx);
        } else {
            ctx.open_file_path(&full_path);
        }
    }
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
