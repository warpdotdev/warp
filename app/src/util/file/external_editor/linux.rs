use std::collections::HashMap;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use command::blocking::Command;
use freedesktop_desktop_entry::DesktopEntry;
use warp_errors::report_error;
use warp_util::path::LineAndColumnArg;
use warpui::AppContext;

use super::Editor;

static INSTALLED_EDITOR_METADATA: OnceLock<HashMap<Editor, EditorMetadata>> = OnceLock::new();

/// Tokenizes a freedesktop Exec string into a list of arguments.
///
/// Follows the quoting rules from the [Desktop Entry Specification](
/// https://specifications.freedesktop.org/desktop-entry-spec/latest/exec-variables.html):
/// - Arguments are separated by unquoted whitespace.
/// - Double-quoted strings are treated as a single argument (quotes stripped).
/// - Within double quotes, the escape sequences `\"`, `` \` ``, `\$`, and
///   `\\` are recognized and resolved.
///
/// Field codes (`%f`, `%u`, etc.) are left as-is in the output tokens; they
/// are expanded in a separate pass by the caller.
fn tokenize_exec(exec: &str) -> Result<Vec<String>, DesktopExecError> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut chars = exec.chars().peekable();
    let mut in_quotes = false;
    // Tracks whether we have started accumulating a token. This is separate
    // from `current.is_empty()` because a quoted empty string (`""`) is a
    // valid zero-length token that should be emitted.
    let mut in_token = false;

    while let Some(ch) = chars.next() {
        if in_quotes {
            match ch {
                '"' => {
                    // Closing quote. The quoted content has already been
                    // accumulated into `current`.
                    in_quotes = false;
                }
                '\\' => {
                    // Inside double quotes the spec recognizes four escape
                    // sequences: \", \`, \$, \\.
                    match chars.peek() {
                        Some('"' | '`' | '$' | '\\') => {
                            current.push(chars.next().unwrap());
                        }
                        _ => {
                            // Not a recognized escape; keep the backslash.
                            current.push('\\');
                        }
                    }
                }
                other => current.push(other),
            }
        } else {
            match ch {
                ' ' | '\t' | '\n' => {
                    if in_token {
                        tokens.push(std::mem::take(&mut current));
                        in_token = false;
                    }
                }
                '"' => {
                    in_quotes = true;
                    in_token = true;
                }
                other => {
                    current.push(other);
                    in_token = true;
                }
            }
        }
    }

    if in_quotes {
        return Err(DesktopExecError::UnterminatedQuote);
    }

    if in_token {
        tokens.push(current);
    }

    Ok(tokens)
}

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

    /// Common implementation of building a command from a .desktop Exec key.
    ///
    /// Tokenizes the Exec string (handling quoting per the freedesktop spec),
    /// expands field codes via `field_code_processor`, and returns a `Command`
    /// that executes the program directly — without going through a shell.
    ///
    /// Field code expansion is handled by the `field_code_processor` callback,
    /// which receives the field code character (the char after `%`) and pushes
    /// replacement arguments onto the provided `Vec<String>`.
    fn build_command<T>(&self, field_code_processor: T) -> Result<Command, DesktopExecError>
    where
        T: Fn(&Self, &mut Vec<String>, char),
    {
        let tokens = tokenize_exec(&self.exec)?;
        let mut args: Vec<String> = Vec::new();

        for token in &tokens {
            if let Some(field_code) = token.strip_prefix('%') {
                match field_code.len() {
                    // A bare `%` with nothing after it is malformed.
                    0 => return Err(DesktopExecError::MalformedFieldCode),
                    1 => {
                        let code_char = field_code.chars().next().unwrap();
                        if code_char == '%' {
                            // Literal percent.
                            args.push("%".to_string());
                        } else {
                            field_code_processor(self, &mut args, code_char);
                        }
                        continue;
                    }
                    // Tokens like `%foo` are not field codes; treat as literal.
                    _ => {}
                }
            }
            args.push(token.clone());
        }

        let program = args.first().ok_or(DesktopExecError::NoExec)?;
        let mut command = Command::new(program);
        command.args(&args[1..]);

        Ok(command)
    }

    /// The default handler for replacing field codes with argument values.
    ///
    /// Takes a `field_code` character (the char after `%`) and pushes the
    /// corresponding argument(s) onto `args`. Follows the [Desktop Entry
    /// Specification](https://specifications.freedesktop.org/desktop-entry-spec/latest/exec-variables.html).
    ///
    /// Any errors or missing information (e.g., `%i` with no Icon field,
    /// `%u` with a non-existent path) will fail silently, resulting in no
    /// arguments being pushed.
    fn process_field_code(&self, args: &mut Vec<String>, field_code: char, file_path: &Path) {
        match field_code {
            // Single file path or file list.
            'f' | 'F' => {
                if let Some(s) = file_path.to_str() {
                    args.push(s.to_string());
                }
            }
            // Single URI or URI list.
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
                        args.push(file_url.as_str().to_string());
                    }
                }
            }
            // Localized application name.
            'c' => {
                if let Some(localized_name) = self.localized_name.as_ref() {
                    args.push(localized_name.clone());
                }
            }
            // Icon key — expands to two arguments per the spec.
            'i' => {
                if let Some(icon) = &self.icon {
                    args.push("--icon".to_string());
                    args.push(icon.clone());
                }
            }
            // Location of the .desktop file.
            'k' => {
                if let Some(s) = self.desktop_file_path.to_str() {
                    args.push(s.to_string());
                }
            }
            // Unknown or deprecated field codes are silently dropped.
            _ => {}
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

    /// A variant of [`Self::build_default_command`] for JetBrains IDEs.
    ///
    /// For `%f`, `%F`, `%u`, and `%U` field codes, injects `--line` and
    /// optionally `--column` arguments before the file path.
    ///
    /// NOTE: This is non-standard behavior according to the .desktop spec.
    /// Any time we use this, it should be manually tested to verify that it
    /// works properly.
    fn build_jetbrains_command(
        &self,
        file_path: &Path,
        line_column_number: Option<LineAndColumnArg>,
    ) -> Result<Command, DesktopExecError> {
        self.build_command(|me, args, field_code| match field_code {
            'f' | 'F' | 'u' | 'U' => {
                if let Some(file_path) = file_path.to_str() {
                    if let Some(line_column_number) = line_column_number {
                        args.push("--line".to_string());
                        args.push(line_column_number.line_num.to_string());
                        if let Some(column_num) = line_column_number.column_num {
                            args.push("--column".to_string());
                            args.push(column_num.to_string());
                        }
                    }
                    args.push(file_path.to_string());
                }
            }
            other => me.process_field_code(args, other, file_path),
        })
    }

    /// A variant of [`Self::build_default_command`] for Sublime Text.
    ///
    /// For `%f`, `%F`, `%u`, and `%U` field codes, appends `:line:col` to
    /// the file path as a single argument.
    ///
    /// NOTE: This is non-standard behavior according to the .desktop spec.
    /// Any time we use this, it should be manually tested to verify that it
    /// works properly.
    fn build_sublime_command(
        &self,
        file_path: &Path,
        line_column_number: Option<LineAndColumnArg>,
    ) -> Result<Command, DesktopExecError> {
        self.build_command(|me, args, field_code| match field_code {
            'f' | 'F' | 'u' | 'U' => {
                if let Some(file_path) = file_path.to_str() {
                    let mut arg = file_path.to_string();
                    if let Some(line_column_number) = line_column_number {
                        arg += &format!(":{}", line_column_number.line_num);
                        if let Some(column_num) = line_column_number.column_num {
                            arg += &format!(":{column_num}");
                        }
                    }
                    args.push(arg);
                }
            }
            other => me.process_field_code(args, other, file_path),
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
                    report_error!(
                        anyhow::Error::new(err).context("Error launching editor"),
                        extra: { "editor" => ?editor }
                    );
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
                        report_error!(
                            anyhow::Error::new(err).context("Failed to build editor open command")
                        );
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

    #[error("Unterminated double quote in Exec string")]
    UnterminatedQuote,

    #[error("Malformed field code in Exec string (bare %)")]
    MalformedFieldCode,
}

#[cfg(test)]
#[path = "linux_tests.rs"]
mod tests;
