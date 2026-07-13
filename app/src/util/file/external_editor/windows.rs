//! Module containing logic to determine to open a file in a text editor, if it is installed.
//! TODO(PLAT-749): Add support for more editors.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use command::r#async::Command;
use enum_iterator::{all, cardinality};
use warp_errors::report_error;
use warp_util::path::LineAndColumnArg;
use warpui::AppContext;
use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};
use winreg::{RegKey, HKEY};

use super::Editor;

static INSTALLED_EDITOR_METADATA: OnceLock<HashMap<Editor, EditorMetadata>> = OnceLock::new();

struct EditorMetadata {
    #[allow(unused)]
    executable_path: PathBuf,
}

/// Enum denoting the method to determine the installation location for a supported editor.
enum ExecutableLocationMethod {
    /// Use the "DisplayIcon" Windows registry key to determine where the app was installed.
    DisplayIcon,
    /// Use the "InstallLocation" Windows registry key to determine where the app was installed.
    InstallLocation {
        /// The path to the _executable_ from the top level directory where the executable is
        /// installed.
        path_to_executable: PathBuf,
    },
}

impl ExecutableLocationMethod {
    fn get_executable_path(&self, application_info: RegKey) -> Option<PathBuf> {
        match self {
            ExecutableLocationMethod::DisplayIcon => {
                let display_icon = application_info
                    .get_value::<String, _>("DisplayIcon")
                    .ok()?;

                // Paths for the DisplayIcon key include:
                // * An icon index after a comma (e.g., "C:\Path\app.exe,0")
                // * Optionally, surrounding quotes (e.g., ""C:\Path\app.exe",0")
                // Remove the trailing comma and the surrounding quotes.
                // This is also the approach GitHub Desktop takes: https://github.com/desktop/desktop/blob/development/app/src/lib/editors/win32.ts#L153.
                let (path, _) = display_icon.rsplit_once(',')?;
                Some(path.replace("\"", "").into())
            }
            ExecutableLocationMethod::InstallLocation { path_to_executable } => {
                let install_location = application_info
                    .get_value::<String, _>("InstallLocation")
                    .ok()?;
                Some(PathBuf::from(install_location).join(path_to_executable))
            }
        }
    }
}

/// Computes a list of installed editors, and any corresponding metadata.
fn compute_installed_editors() -> HashMap<Editor, EditorMetadata> {
    const SCOPES: [(HKEY, &str); 3] = [
        (
            HKEY_LOCAL_MACHINE,
            "SOFTWARE\\Wow6432Node\\Microsoft\\Windows\\CurrentVersion\\Uninstall",
        ),
        (
            HKEY_LOCAL_MACHINE,
            "SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Uninstall",
        ),
        (
            HKEY_CURRENT_USER,
            "SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Uninstall",
        ),
    ];

    let mut installed_editors = HashMap::with_capacity(cardinality::<Editor>());

    // Generate a mapping from each app ID to the editor the app ID corresponds to.
    let app_id_to_editors: HashMap<&'static str, Editor> = all::<Editor>()
        .flat_map(|editor| editor.app_ids().iter().map(move |app_id| (*app_id, editor)))
        .collect();

    // Determine all the installed applications by reading out install metadata from the windows
    // registry.
    for (scope, key) in SCOPES {
        let uninstall_key = match RegKey::predef(scope).open_subkey(key) {
            Ok(k) => k,
            Err(_) => continue,
        };

        for application_id in uninstall_key.enum_keys().flatten() {
            let Some(editor) = app_id_to_editors.get(application_id.as_str()) else {
                continue;
            };

            let Ok(application_info) = uninstall_key.open_subkey(&application_id) else {
                continue;
            };

            let Some(executable_path) = editor
                .executable_location_method()
                .and_then(|key| key.get_executable_path(application_info))
            else {
                continue;
            };

            let editor_metadata = EditorMetadata { executable_path };
            installed_editors.insert(*editor, editor_metadata);
        }
    }

    installed_editors
}

impl Editor {
    pub fn is_installed(&self, _ctx: &mut AppContext) -> bool {
        INSTALLED_EDITOR_METADATA
            .get_or_init(compute_installed_editors)
            .contains_key(self)
    }

    /// Returns the set of IDs that identify a given Editor.
    fn app_ids(self) -> &'static [&'static str] {
        match self {
            Editor::VSCode => {
                &[
                    // 64-bit version of VSCode (user) - provided by default in 64-bit Windows
                    "{771FD6B0-FA20-440A-A002-3B3BAC16DC50}_is1",
                    // 32-bit version of VSCode (user)
                    "{D628A17A-9713-46BF-8D57-E671B46A741E}_is1",
                    // ARM64 version of VSCode (user)
                    "{D9E514E7-1A56-452D-9337-2990C0DC4310}_is1",
                    // 64-bit version of VSCode (system) - was default before user scope installation
                    "EA457B21-F73E-494C-ACAB-524FDE069978}_is1",
                    // 32-bit version of VSCode (system)
                    "{F8A2A208-72B3-4D61-95FC-8A65D340689B}_is1",
                    // ARM64 version of VSCode (system)
                    "{A5270FC5-65AD-483E-AC30-2C276B63D0AC}_is1",
                ]
            }
            Editor::Cursor => &["62625861-8486-5be9-9e46-1da50df5f8ff"],
            Editor::Windsurf => &["{5A8B7D94-9B5F-4D1F-93FC-5609F7159349}_is1"],
            _ => &[],
        }
    }

    fn executable_location_method(&self) -> Option<ExecutableLocationMethod> {
        match self {
            Editor::VSCode => Some(ExecutableLocationMethod::InstallLocation {
                path_to_executable: Path::new("bin").join("code.exe"),
            }),
            Editor::Windsurf => Some(ExecutableLocationMethod::InstallLocation {
                path_to_executable: Path::new("bin").join("windsurf.exe"),
            }),
            Editor::Cursor => Some(ExecutableLocationMethod::DisplayIcon),
            _ => None,
        }
    }

    pub fn command(
        &self,
        line_column_number: Option<LineAndColumnArg>,
        full_path: &Path,
    ) -> Option<Command> {
        let command = match self {
            Editor::VSCode => {
                let mut command = Command::new("explorer.exe");
                let suffix = line_column_number
                    .as_ref()
                    .map(LineAndColumnArg::to_string_suffix)
                    .unwrap_or_default();
                command.arg(format!("vscode://file/{}{suffix}", full_path.display()));
                command
            }
            Editor::Cursor => {
                let mut command = Command::new("explorer.exe");
                let suffix = line_column_number
                    .as_ref()
                    .map(LineAndColumnArg::to_string_suffix)
                    .unwrap_or_default();
                command.arg(format!("cursor://file/{}{suffix}", full_path.display()));
                command
            }
            Editor::Windsurf => {
                let mut command = Command::new("explorer.exe");
                let suffix = line_column_number
                    .as_ref()
                    .map(LineAndColumnArg::to_string_suffix)
                    .unwrap_or_default();
                command.arg(format!("windsurf://file/{}{suffix}", full_path.display()));
                command
            }
            _ => return None,
        };

        Some(command)
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
    mut with_editor: Option<Editor>,
    full_path: &Path,
    ctx: &mut AppContext,
) {
    if full_path.is_file() {
        with_editor = with_editor.filter(|editor| editor.is_installed(ctx));
        if let Some(editor) = with_editor {
            if let Some(mut command) = editor.command(line_column_number, full_path) {
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
