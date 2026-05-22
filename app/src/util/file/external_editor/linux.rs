use std::collections::HashMap;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use command::blocking::Command;
use freedesktop_desktop_entry::DesktopEntry;
use warp_util::path::LineAndColumnArg;
use warpui::AppContext;

use super::Editor;

static INSTALLED_EDITOR_METADATA: OnceLock<HashMap<Editor, EditorMetadata>> = OnceLock::new();

/// A data struct to hold relevant info pulled from a [freedesktop_desktop_entry::DesktopEntry].
/// Mostly here to get around the lack of an owned version of DesktopEntry.
struct EditorMetadata {
    /// Path to the .desktop file.
    desktop_file_path: PathBuf,

    /// The EXEC string from the .desktop file that details how
    /// to open the application. Contains field codes that need
    /// to be replaced.
    exec: String,

    /// The name of the app, localized to the user's language if
    /// possible.
    localized_name: Option<String>,

    // Path to a desktop icon.
    icon: Option<String>,
}

impl EditorMetadata {
    /// Builds a new metadata from a given desktop file path
    ///
    /// Reads in the file at `desktop_file_path`, and Attempts
    /// to build a new [`EditorMetdata`] from the file
    ///
    /// # errors
    /// - [`DesktopExecError::IoError`] if reading the file fails
    /// - [`DesktopExecError::DecodeError`] if parsing the desktop entry fails
    /// - [`DesktopExecError::NoExec`] if the desktop entry does not have an Exec field
    fn try_new(desktop_file_path: PathBuf) -> Result<Self, DesktopExecError> {
        let input = std::fs::read_to_string(&desktop_file_path)?;

        let entry = DesktopEntry::decode(&desktop_file_path, &input)?;

        let Some(exec) = entry.exec() else {
            return Err(DesktopExecError::NoExec);
        };

        // Doing all the calculations here to get owned versions of data fields,
        // so we can drop entry
        let exec = exec.to_string();
        let localized_name = entry.name(Some("en")).map(|x| x.to_string());
        let icon = entry.icon().map(str::to_string);

        Ok(Self {
            desktop_file_path,
            exec,
            localized_name,
            icon,
        })
    }

    /// Common implementation of building a command
    ///
    /// - Iterates over all characters in the Exec field, replacing field codes,
    ///   to generate a new command string
    /// - Builds a new command that executes `sh -c <command_string>`
    ///
    /// Field code replacement is handled by the `field_code_processor` callback.
    /// See [`Self::build_default_command`] and [`Self::process_field_code`]
    /// for examples of how these work.
    ///
    /// ```ignore
    /// use std::path::PathBuf;
    /// use warp::util::file::external_editor::linux::EditorMetadata;
    ///
    /// let desktop_file_path = PathBuf::from("/var/lib/snapd/desktop/applications/webstorm_webstorm.desktop");
    /// let metadata = EditorMetadata::try_new(desktop_file_path)?;
    ///
    /// let my_file_path = PathBuf::from("~/foo.rs");
    ///
    /// // This is identicial to metadata.build_default_command(my_file_path);
    /// let command = metadata.build_command(|me, acc, c| me.process_field_code(acc, c, my_file_path))?;
    ///
    /// // If I want to do some custom stuff, I can use a modified field code processor
    /// let command = metadata.build_command(|me, acc, c| {
    ///     match c {
    ///         'c' => acc += "foobar",
    ///         c =>  me.process_field_code(acc, c, my_file_path),
    ///     }
    /// });
    /// ```
    fn build_command<T>(&self, field_code_processor: T) -> Result<Command, DesktopExecError>
    where
        T: Fn(&Self, &mut String, char),
    {
        let raw_exec = &self.exec;

        let mut iter = raw_exec.chars();
        let mut processed_exec = String::new();
        while let Some(ch) = iter.next() {
            if ch != '%' {
                processed_exec.push(ch);
                continue;
            }
            let Some(next_char) = iter.next() else {
                return Err(DesktopExecError::MalformedFieldCode);
            };
            field_code_processor(self, &mut processed_exec, next_char);
        }

        let mut command = Command::new("sh");
        command.args(["-c", &processed_exec]);

        Ok(command)
    }

    /// The default handler for replacing field codes with values
    ///
    /// Takes in a `field_code`, and handles appending replacement values
    /// to the passed in `processed_exec` string. Follows the standard
    /// here: https://specifications.freedesktop.org/desktop-entry-spec/latest/ar01s07.html.
    /// Any fields like %f, %F, %u, and %U that rely on a file path use the `file_path`
    /// parameter.
    ///
    /// Any errors or missing information (ex: %i with no Icon field, %U wiht a non-existent path)
    /// will fail silently, and result in nothing being appended to `processed_exec`
    fn process_field_code(&self, processed_exec: &mut String, field_code: char, file_path: &Path) {
        match field_code {
            // file path
            'f' | 'F' => *processed_exec += file_path.to_str().unwrap_or_default(),
            // URI
            'u' | 'U' => {
                // TODO(daprahamian): B/c we are using canonicalize, this will fail
                // if the file we are checking here does not actually exist. Also
                // it requires an fs check, which is not fun. In the future, it would
                // be nice to replace this with the pending std::path::absolute in
                // the future
                //
                // See https://github.com/rust-lang/rust/issues/92750
                if let Ok(absolute) = file_path.canonicalize() {
                    if let Ok(file_url) = url::Url::from_file_path(absolute) {
                        *processed_exec += file_url.as_str();
                    }
                }
            }
            // Localized Name
            'c' => {
                if let Some(localized_name) = self.localized_name.as_ref() {
                    *processed_exec += localized_name;
                }
            }
            // Icon argument
            'i' => {
                if let Some(icon) = &self.icon {
                    *processed_exec += "--icon ";
                    *processed_exec += icon;
                }
            }
            // Path to the display file
            'k' => *processed_exec += self.desktop_file_path.to_str().unwrap_or_default(),
            // Just add the character
            other => processed_exec.push(other),
        };
    }

    /// Builds a command based on a FreeDesktop Desktop Entry Exec key.
    /// Will returns a `Command` object that invokes the Exec command,
    /// with all field codes replaced according to the standard.
    ///
    /// The values for %f, %F, %u, and %U are all computed based on a single file
    /// path passed in. We do not support multiple paths at this time.
    ///
    /// Any field code processing errors will fail silently
    ///
    /// See https://specifications.freedesktop.org/desktop-entry-spec/latest/ar01s07.html
    fn build_default_command(&self, file_path: &Path) -> Result<Command, DesktopExecError> {
        self.build_command(|me, acc, c| me.process_field_code(acc, c, file_path))
    }

    /// A variant of [`Self::build_default_command`] for jetbrains IDEs
    ///
    /// Works the same, except that for %f, %F, %u, and %U field codes.
    /// When adding a file or URL, additional CLI flags are injected to specify
    /// line and column number if available.
    ///
    /// NOTE: This is a non-standard behavior according to the .desktop specification.
    /// Any time we use this, it should be manually tested to verify that it works properly.
    fn build_jetbrains_command(
        &self,
        file_path: &Path,
        line_column_number: Option<LineAndColumnArg>,
    ) -> Result<Command, DesktopExecError> {
        self.build_command(|me, acc, field_code| match field_code {
            'f' | 'F' | 'u' | 'U' => {
                if let Some(file_path) = file_path.to_str() {
                    if let Some(line_column_number) = line_column_number {
                        *acc += &format!("--line {} ", line_column_number.line_num);
                        if let Some(column_num) = line_column_number.column_num {
                            *acc += &format!("--column {column_num} ");
                        }
                    }
                    *acc += file_path;
                }
            }
            other => me.process_field_code(acc, other, file_path),
        })
    }
    /// A variant of [`Self::build_default_command`] for sublime
    ///
    /// Works the same, except that for %f, %F, %u, and %U field codes.
    /// When adding a file or URL, the file name is appended with the line and column number if available.
    ///
    /// NOTE: This is a non-standard behavior according to the .desktop specification.
    /// Any time we use this, it should be manually tested to verify that it works properly.
    fn build_sublime_command(
        &self,
        file_path: &Path,
        line_column_number: Option<LineAndColumnArg>,
    ) -> Result<Command, DesktopExecError> {
        self.build_command(|me, acc, field_code| match field_code {
            'f' | 'F' | 'u' | 'U' => {
                if let Some(file_path) = file_path.to_str() {
                    *acc += file_path;
                    if let Some(line_column_number) = line_column_number {
                        *acc += &format!(":{}", line_column_number.line_num);
                        if let Some(column_num) = line_column_number.column_num {
                            *acc += &format!(":{column_num}");
                        }
                    }
                }
            }
            other => me.process_field_code(acc, other, file_path),
        })
    }
}

/// Opens the given file in the specified editor.
///
/// If `line_column_number` is `Some`, the file will be opened with the cursor
/// at the given location (if supported by the editor).
///
/// If with_editor is `None`, we attempt to compute the default editor for the
/// given file type, and open the file there.
pub fn open_file_path_with_line_and_col(
    line_column_number: Option<LineAndColumnArg>,
    with_editor: Option<Editor>,
    full_path: &Path,
    ctx: &mut AppContext,
) {
    if full_path.is_file() {
        let with_editor = with_editor.or_else(|| get_app_for_file_from_mime(full_path));
        if let Some(editor) = with_editor {
            if let Some(mut command) = editor.command(full_path, line_column_number) {
                if let Err(err) = command.spawn() {
                    log::error!("Error launching {editor:?}: {err:#}");
                }
                return;
            }
        }
    }

    ctx.open_file_path(full_path);
}

/// Attempt to match a file with an existing editor based on Mime type
///
/// Calls xdg-mime to first find the mime type of a file, and then find
/// the xdg default app for that file. We then check against existing
/// loaded editors to see if we have support for that file.
///
/// Used so that if xdg-open will work on a file we already know about,
/// we can use line and col numbers.
fn get_app_for_file_from_mime(path: &Path) -> Option<Editor> {
    let mime_type = String::from_utf8(
        Command::new("xdg-mime")
            .arg("query")
            .arg("filetype")
            .arg(path)
            .output()
            .ok()?
            .stdout,
    )
    .ok()?;

    let default_app = String::from_utf8(
        Command::new("xdg-mime")
            .args(["query", "default", mime_type.trim()])
            .output()
            .ok()?
            .stdout,
    )
    .ok()?;

    let app_id = default_app.trim().replace(".desktop", "");

    get_editor_by_app_id(compute_editors_by_id(), app_id.as_str())
}

static EDITORS_BY_ID: OnceLock<HashMap<&'static str, Editor>> = OnceLock::new();
// Compute a map from app ID to `Editor` for all supported editors.
fn compute_editors_by_id() -> &'static HashMap<&'static str, Editor> {
    EDITORS_BY_ID.get_or_init(|| {
        let mut editors_by_id = HashMap::new();
        for editor in enum_iterator::all::<Editor>() {
            if let Some(app_ids) = editor.app_ids() {
                for app_id in app_ids.iter() {
                    editors_by_id.insert(*app_id, editor);
                }
            }
        }
        editors_by_id
    })
}

/// Looks up the editor given an app_id
///
/// Special case for snap desktop files. snap desktop files follow XDG Desktop Entry
/// Specification 1.1, which predates standard naming conventions. We are winding up
/// with names of the format:
///
///    {snap-package-id}_{app-id}.desktop
/// Examples include "code_code.desktop", "code-insiders_code-insiders.desktop",
/// "code_code-url-handler.desktop", etc. So we check for the _ and use whatever follows.
///
/// See: https://snapcraft.io/docs/desktop-menu-support
/// See: https://forum.snapcraft.io/t/overriding-desktop-files-on-ubuntu-snaps/6599/4
fn get_editor_by_app_id(
    editors_by_id: &HashMap<&'static str, Editor>,
    app_id: &str,
) -> Option<Editor> {
    editors_by_id
        .get(app_id)
        .or_else(|| {
            let (_, app_id) = app_id.split_once('_')?;

            if app_id.is_empty() {
                return None;
            }

            editors_by_id.get(app_id)
        })
        .copied()
}

/// Computes the list of installed editors.
fn compute_installed_editors() -> HashMap<Editor, EditorMetadata> {
    let editors_by_id = compute_editors_by_id();

    // Iterate through the .desktop files in the places they are typically
    // installed and see if the app ID (file stem) matches a supported
    // editor.
    let mut editors = HashMap::new();
    for path in freedesktop_desktop_entry::Iter::new(freedesktop_desktop_entry::default_paths()) {
        let Some(app_id) = path.file_stem().and_then(OsStr::to_str) else {
            continue;
        };
        if let Some(editor) = get_editor_by_app_id(editors_by_id, app_id) {
            match EditorMetadata::try_new(path) {
                Ok(metadata) => {
                    editors.insert(editor, metadata);
                }
                Err(e) => log::warn!("Failed to load editor config: {e:#}"),
            };
            continue;
        }
    }
    editors
}

impl Editor {
    fn app_ids(&self) -> Option<&[&'static str]> {
        use Editor::*;
        match self {
            AndroidStudio => Some(&["android-studio", "jetbrains-studio"]),
            CLion => Some(&["clion", "jetbrains-clion"]),
            DataGrip => Some(&["datagrip", "jetbrains-datagrip"]),
            DataSpell => Some(&["dataspell", "jetbrains-dataspell"]),
            IntelliJ => Some(&["jetbrains-idea", "intellij-idea-ultimate"]),
            IntelliJCE => Some(&["jetbrains-idea-ce", "intellij-idea-community"]),
            GoLand => Some(&["goland", "jetbrains-goland"]),
            PhpStorm => Some(&["phpstorm", "jetbrains-phpstorm"]),
            PyCharm => Some(&["pycharm-professional", "jetbrains-pycharm"]),
            PyCharmCE => Some(&["pycharm-community", "jetbrains-pycharm-ce"]),
            Rider => Some(&["rider", "jetbrains-rider"]),
            RubyMine => Some(&["rubymine", "jetbrains-rubymine"]),
            Sublime => Some(&["sublime-text_subl", "sublime_text"]),
            VSCode => Some(&["code"]),
            VSCodeInsiders => Some(&["code-insiders"]),
            WebStorm => Some(&["webstorm", "jetbrains-webstorm"]),
            Windsurf => Some(&["windsurf"]),
            Zed => Some(&["dev.zed.Zed"]),
            ZedPreview => Some(&["dev.zed.Zed-Preview"]), // both Zed stable and preview use the same binary on Linux
            _ => None,
        }
    }

    fn installed_editors(&self) -> &HashMap<Editor, EditorMetadata> {
        INSTALLED_EDITOR_METADATA.get_or_init(compute_installed_editors)
    }

    pub fn is_installed(&self, _ctx: &mut AppContext) -> bool {
        use Editor::*;
        match self {
            // For Zed editors on Linux, we need to detect which channel is installed by checking both
            // the .desktop file and the actual binary location
            Zed | ZedPreview => {
                // First check if .desktop file exists
                if !self.installed_editors().contains_key(self) {
                    return false;
                }

                // Then verify the correct binary exists in its installation path
                let home = std::env::var("HOME").unwrap_or_default();
                let binary_path = match self {
                    Zed => format!("{home}/.local/zed.app/bin/zed"),
                    ZedPreview => format!("{home}/.local/zed-preview.app/bin/zed"),
                    _ => unreachable!(),
                };

                std::path::Path::new(&binary_path).exists()
            }
            // For all other editors, just check the desktop file
            _ => self.installed_editors().contains_key(self),
        }
    }

    fn get_metadata(&self) -> Option<&EditorMetadata> {
        self.installed_editors().get(self)
    }

    fn command(
        &self,
        file_path: &Path,
        line_column_number: Option<LineAndColumnArg>,
    ) -> Option<Command> {
        use Editor::*;
        match self {
            VSCode => {
                let suffix = line_column_number
                    .as_ref()
                    .map(LineAndColumnArg::to_string_suffix)
                    .unwrap_or_default();
                let mut command = Command::new("xdg-open");
                command.arg(format!("vscode://file{}{suffix}", file_path.display()));
                Some(command)
            }
            VSCodeInsiders => {
                let suffix = line_column_number
                    .as_ref()
                    .map(LineAndColumnArg::to_string_suffix)
                    .unwrap_or_default();
                let mut command = Command::new("xdg-open");
                command.arg(format!(
                    "vscode-insiders://file{}{suffix}",
                    file_path.display()
                ));
                Some(command)
            }
            Windsurf => {
                let suffix = line_column_number
                    .as_ref()
                    .map(LineAndColumnArg::to_string_suffix)
                    .unwrap_or_default();
                let mut command = Command::new("xdg-open");
                command.arg(format!("windsurf://file{}{suffix}", file_path.display()));
                Some(command)
            }
            AndroidStudio | CLion | CLionCE | DataGrip | DataSpell | GoLand | IntelliJ
            | IntelliJCE | PhpStorm | PyCharm | PyCharmCE | Rider | RubyMine | WebStorm => {
                match self.get_metadata() {
                    Some(metadata) => {
                        match metadata.build_jetbrains_command(file_path, line_column_number) {
                            Ok(command) => Some(command),
                            Err(err) => {
                                log::warn!("Failed to build editor open command: {err:#}");
                                None
                            }
                        }
                    }
                    None => None,
                }
            }
            Sublime => match self.get_metadata() {
                Some(metadata) => {
                    log::info!("Opening at {file_path:?} + {line_column_number:?}");
                    match metadata.build_sublime_command(file_path, line_column_number) {
                        Ok(command) => {
                            log::info!("Command: {command:?}");
                            Some(command)
                        }
                        Err(err) => {
                            log::warn!("Failed to build editor open command: {err:#}");
                            None
                        }
                    }
                }
                None => None,
            },
            Zed | ZedPreview => {
                // Get the correct binary path based on which editor was selected
                let home = std::env::var("HOME").unwrap_or_default();
                let binary_path = match self {
                    Zed => format!("{home}/.local/zed.app/bin/zed"),
                    ZedPreview => format!("{home}/.local/zed-preview.app/bin/zed"),
                    _ => unreachable!(),
                };

                // Format the file path with line/column if provided
                let file_path_str = file_path.display().to_string();
                let position = if let Some(line_col) = line_column_number {
                    if let Some(col) = line_col.column_num {
                        format!("{}:{}:{}", file_path_str, line_col.line_num, col)
                    } else {
                        format!("{}:{}", file_path_str, line_col.line_num)
                    }
                } else {
                    file_path_str
                };

                // Build command using setsid for proper detachment
                let mut command = Command::new("/usr/bin/setsid");
                command.args([
                    "-f",         // Fork to background
                    &binary_path, // The specific Zed binary to run
                    &position,    // File path with optional line/column
                ]);

                // Redirect all stdio to null
                command.stdin(std::process::Stdio::null());
                command.stdout(std::process::Stdio::null());
                command.stderr(std::process::Stdio::null());
                Some(command)
            }
            _ => match self.get_metadata() {
                Some(metadata) => match metadata.build_default_command(file_path) {
                    Ok(command) => Some(command),
                    Err(err) => {
                        log::error!("Failed to build editor open command: {err:#}");
                        None
                    }
                },
                None => None,
            },
        }
    }
}

#[derive(thiserror::Error, Debug)]
enum DesktopExecError {
    #[error("i/o error {0}")]
    IoError(#[from] std::io::Error),

    #[error("decode error {0}")]
    DecodeError(#[from] freedesktop_desktop_entry::DecodeError),

    #[error("Attempted to create command for desktop entry with no exec field")]
    NoExec,

    #[error("Malformed exec call: non-terminated field code")]
    MalformedFieldCode,
}

#[cfg(test)]
#[path = "linux_tests.rs"]
mod tests;
