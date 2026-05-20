#![allow(deprecated)]

use instant::Instant;
use std::slice;
use std::{fmt::Write, path::Path};

use cocoa::{
    base::{id, nil},
    foundation::{NSAutoreleasePool, NSString},
};
use command::r#async::Command;
use warp_core::{channel::ChannelState, AppId};
use warpui::{platform::mac::make_nsstring, ApplicationBundleInfo};

use super::*;

// Functions implemented in objC files.
extern "C" {
    fn get_default_app_bundle_for_file(file_path: id) -> id;
}

/// The executable we use to launch the editor.
#[derive(Debug)]
pub enum OpenFileInEditorMethod {
    // A custom binary (e.g. the code CLI tool for VSCode).
    Binary(String),
    // Default application bundle from the app registration info in Cocoa.
    FromApplicationBundleInfo,
    // Use /usr/bin/open to open the file directly using the Editor's registered URL protocol.
    // The optional bundle identifier parameter allows for two different use cases:
    //
    // 1. AppUrl(None) - Opens the URL directly with the system's default handler
    //    Example: `open vscode://file/hello.rs`
    //    Used by editors like VSCode that rely on URL scheme registration
    //
    // 2. AppUrl(Some(bundle_id)) - Opens the URL with a specific application bundle
    //    Example: `open -b dev.zed.Zed zed://file/hello.rs`
    //    Used by editors like Zed that need explicit bundle specification
    AppUrl(Option<&'static str>),
}

impl OpenFileInEditorMethod {
    pub fn command(&self, application_bundle_info: ApplicationBundleInfo) -> Command {
        let mut open_command = Command::new("/usr/bin/open");

        match self {
            OpenFileInEditorMethod::Binary(binary_path)
                if application_bundle_info.path.join(binary_path).exists() =>
            {
                Command::new(application_bundle_info.path.join(binary_path))
            }
            OpenFileInEditorMethod::AppUrl(_) => open_command,
            _ => {
                open_command.arg("-a").arg(application_bundle_info.path);
                open_command
            }
        }
    }
}

impl<'a> Editor {
    const VSCODE_IDENTIFIER: &'a str = "com.microsoft.VSCode";
    const VSCODE_INSIDERS_IDENTIFIER: &'a str = "com.microsoft.VSCodeInsiders";
    const PYCHARM_CE_IDENTIFIER: &'a str = "com.jetbrains.pycharm.ce";
    const INTELLIJ_CE_IDENTIFIER: &'a str = "com.jetbrains.intellij.ce";
    const CLION_CE_IDENTIFIER: &'a str = "com.jetbrains.clion.ce";

    /// Bundle identifier for the Rust Rover Preview build.
    const RUST_ROVER_PREVIEW_IDENTIFIER: &'a str = "com.jetbrains.rustrover-EAP";

    /// Bundle identifier for the Rust Rover build.
    const RUST_ROVER_IDENTIFIER: &'a str = "com.jetbrains.rustrover";

    const PYCHARM_IDENTIFIER: &'a str = "com.jetbrains.PyCharm";
    const INTELLIJ_IDENTIFIER: &'a str = "com.jetbrains.intellij";
    const CLION_IDENTIFIER: &'a str = "com.jetbrains.CLion";
    const PHPSTORM_IDENTIFIER: &'a str = "com.jetbrains.PhpStorm";
    const RUBYMINE_IDENTIFIER: &'a str = "com.jetbrains.RubyMine";
    const WEBSTORM_IDENTIFIER: &'a str = "com.jetbrains.WebStorm";
    const SUBLIME_4_IDENTIFIER: &'a str = "com.sublimetext.4";
    const SUBLIME_3_IDENTIFIER: &'a str = "com.sublimetext.3";
    const SUBLIME_2_IDENTIFIER: &'a str = "com.sublimetext.2";
    const ATOM_IDENTIFIER: &'a str = "com.github.atom";
    const ZED_IDENTIFIER: &'a str = "dev.zed.Zed";
    const ZED_PREVIEW_IDENTIFIER: &'a str = "dev.zed.Zed-Preview";
    const GOLAND_IDENTIFIER: &'a str = "com.jetbrains.goland";
    const RIDER_IDENTIFIER: &'a str = "com.jetbrains.rider";
    const DATASPELL_IDENTIFIER: &'a str = "com.jetbrains.dataspell";
    const DATAGRIP_IDENTIFIER: &'a str = "com.jetbrains.datagrip";
    const ANDROID_STUDIO_IDENTIFIER: &'a str = "com.google.android.studio";
    const CURSOR_IDENTIFIER: &'a str = "com.todesktop.230313mzl4w4u92";
    const WINDSURF_IDENTIFIER: &'a str = "com.exafunction.windsurf";

    pub fn new_from_identifier(app_identifier: &str) -> Option<Self> {
        match app_identifier {
            Editor::VSCODE_IDENTIFIER => Some(Editor::VSCode),
            Editor::VSCODE_INSIDERS_IDENTIFIER => Some(Editor::VSCodeInsiders),
            Editor::PYCHARM_CE_IDENTIFIER => Some(Editor::PyCharmCE),
            Editor::PYCHARM_IDENTIFIER => Some(Editor::PyCharm),
            Editor::INTELLIJ_CE_IDENTIFIER => Some(Editor::IntelliJCE),
            Editor::INTELLIJ_IDENTIFIER => Some(Editor::IntelliJ),
            Editor::CLION_IDENTIFIER => Some(Editor::CLion),
            Editor::CLION_CE_IDENTIFIER => Some(Editor::CLionCE),
            Editor::ATOM_IDENTIFIER => Some(Editor::Atom),
            Editor::SUBLIME_4_IDENTIFIER => Some(Editor::Sublime4),
            Editor::SUBLIME_3_IDENTIFIER => Some(Editor::Sublime3),
            Editor::SUBLIME_2_IDENTIFIER => Some(Editor::Sublime2),
            Editor::ZED_IDENTIFIER => Some(Editor::Zed),
            Editor::ZED_PREVIEW_IDENTIFIER => Some(Editor::ZedPreview),
            Editor::GOLAND_IDENTIFIER => Some(Editor::GoLand),
            Editor::RIDER_IDENTIFIER => Some(Editor::Rider),
            Editor::DATASPELL_IDENTIFIER => Some(Editor::DataSpell),
            Editor::DATAGRIP_IDENTIFIER => Some(Editor::DataGrip),
            Editor::ANDROID_STUDIO_IDENTIFIER => Some(Editor::AndroidStudio),
            Editor::CURSOR_IDENTIFIER => Some(Editor::Cursor),
            Editor::WINDSURF_IDENTIFIER => Some(Editor::Windsurf),
            _ => None,
        }
    }

    pub fn application_bundle_info(
        &'a self,
        ctx: &'a mut AppContext,
    ) -> Option<ApplicationBundleInfo<'a>> {
        ctx.application_bundle_info(match self {
            Editor::VSCode => Editor::VSCODE_IDENTIFIER,
            Editor::VSCodeInsiders => Editor::VSCODE_INSIDERS_IDENTIFIER,
            Editor::PyCharmCE => Editor::PYCHARM_CE_IDENTIFIER,
            Editor::PyCharm => Editor::PYCHARM_IDENTIFIER,
            Editor::IntelliJCE => Editor::INTELLIJ_CE_IDENTIFIER,
            Editor::IntelliJ => Editor::INTELLIJ_IDENTIFIER,
            Editor::CLionCE => Editor::CLION_CE_IDENTIFIER,
            Editor::CLion => Editor::CLION_IDENTIFIER,
            Editor::Sublime4 => Editor::SUBLIME_4_IDENTIFIER,
            Editor::Sublime3 => Editor::SUBLIME_3_IDENTIFIER,
            Editor::Sublime2 => Editor::SUBLIME_2_IDENTIFIER,
            Editor::Atom => Editor::ATOM_IDENTIFIER,
            Editor::PhpStorm => Editor::PHPSTORM_IDENTIFIER,
            Editor::WebStorm => Editor::WEBSTORM_IDENTIFIER,
            Editor::RubyMine => Editor::RUBYMINE_IDENTIFIER,
            Editor::Zed => Editor::ZED_IDENTIFIER,
            Editor::ZedPreview => Editor::ZED_PREVIEW_IDENTIFIER,
            Editor::GoLand => Editor::GOLAND_IDENTIFIER,
            Editor::Rider => Editor::RIDER_IDENTIFIER,
            Editor::DataSpell => Editor::DATASPELL_IDENTIFIER,
            Editor::DataGrip => Editor::DATAGRIP_IDENTIFIER,
            Editor::AndroidStudio => Editor::ANDROID_STUDIO_IDENTIFIER,
            Editor::Cursor => Editor::CURSOR_IDENTIFIER,
            Editor::RustRoverPreview => Editor::RUST_ROVER_PREVIEW_IDENTIFIER,
            Editor::RustRover => Editor::RUST_ROVER_IDENTIFIER,
            Editor::Windsurf => Editor::WINDSURF_IDENTIFIER,
        })
    }

    pub fn is_installed(&self, ctx: &mut AppContext) -> bool {
        self.application_bundle_info(ctx).is_some()
    }

    fn command_executable_and_arguments(
        &self,
        line_column_number: Option<LineAndColumnArg>,
        full_path: &Path,
    ) -> (OpenFileInEditorMethod, Vec<String>) {
        let full_path_with_line_column =
            Self::format_file_path_with_line_and_column(full_path, line_column_number);
        match self {
            Editor::VSCode => (
                OpenFileInEditorMethod::AppUrl(None),
                vec![format!("vscode://file{}", full_path_with_line_column)],
            ),
            Editor::VSCodeInsiders => (
                OpenFileInEditorMethod::AppUrl(None),
                vec![format!(
                    "vscode-insiders://file{}",
                    full_path_with_line_column
                )],
            ),
            Editor::Windsurf => (
                OpenFileInEditorMethod::AppUrl(None),
                vec![format!("windsurf://file{}", full_path_with_line_column)],
            ),
            Editor::PyCharm | Editor::PyCharmCE => {
                Self::jetbrains_command("pycharm", line_column_number, full_path)
            }
            Editor::IntelliJ | Editor::IntelliJCE => {
                Self::jetbrains_command("idea", line_column_number, full_path)
            }
            Editor::CLion | Editor::CLionCE => {
                Self::jetbrains_command("clion", line_column_number, full_path)
            }
            Editor::RubyMine => Self::jetbrains_command("rubymine", line_column_number, full_path),
            Editor::PhpStorm => Self::jetbrains_command("phpstorm", line_column_number, full_path),
            Editor::WebStorm => Self::jetbrains_command("webstorm", line_column_number, full_path),
            Editor::Sublime4 | Editor::Sublime3 | Editor::Sublime2 => (
                OpenFileInEditorMethod::Binary("Contents/SharedSupport/bin/subl".to_string()),
                vec![full_path_with_line_column],
            ),
            Editor::Atom => (
                OpenFileInEditorMethod::FromApplicationBundleInfo,
                vec![full_path_with_line_column],
            ),
            Editor::Zed => (
                OpenFileInEditorMethod::AppUrl(Some(Editor::ZED_IDENTIFIER)),
                vec![format!("zed://file{}", full_path_with_line_column)],
            ),
            Editor::ZedPreview => (
                OpenFileInEditorMethod::AppUrl(Some(Editor::ZED_PREVIEW_IDENTIFIER)),
                vec![format!("zed://file{}", full_path_with_line_column)],
            ),
            Editor::GoLand => Self::jetbrains_command("goland", line_column_number, full_path),
            Editor::Rider => Self::jetbrains_command("rider", line_column_number, full_path),
            Editor::DataSpell => {
                Self::jetbrains_command("dataspell", line_column_number, full_path)
            }
            Editor::DataGrip => Self::jetbrains_command("datagrip", line_column_number, full_path),
            Editor::AndroidStudio => {
                Self::jetbrains_command("studio", line_column_number, full_path)
            }
            Editor::Cursor => (
                OpenFileInEditorMethod::AppUrl(None),
                vec![format!("cursor://file{}", full_path_with_line_column)],
            ),
            Editor::RustRoverPreview | Editor::RustRover => {
                Self::jetbrains_command("rustrover", line_column_number, full_path)
            }
        }
    }

    fn jetbrains_command(
        cli_name: &str,
        line_column_number: Option<LineAndColumnArg>,
        full_path: &Path,
    ) -> (OpenFileInEditorMethod, Vec<String>) {
        let full_path = full_path.to_str().expect("full path exists").to_string();
        (
            OpenFileInEditorMethod::Binary(format!("Contents/MacOS/{cli_name}")),
            if let Some(line_column_number) = line_column_number {
                vec![
                    "--line".to_string(),
                    line_column_number.line_num.to_string(),
                    full_path,
                ]
            } else {
                vec![full_path]
            },
        )
    }

    pub fn open(
        &self,
        line_column_number: Option<LineAndColumnArg>,
        full_path: &Path,
        ctx: &mut AppContext,
    ) -> bool {
        let Some(application_bundle_info) = self.application_bundle_info(ctx) else {
            return false;
        };

        let (executable, arguments) =
            self.command_executable_and_arguments(line_column_number, full_path);

        // Build the command based on the executable type:
        // - For AppUrl(Some(bundle_id)): Use `open -b bundle_id` to explicitly specify the app
        // - For AppUrl(None): Use plain `open` command to let the system handle the URL scheme
        // - For other methods: Use the standard command creation logic
        let mut command = match &executable {
            OpenFileInEditorMethod::AppUrl(Some(bundle_id)) => {
                let mut cmd = Command::new("/usr/bin/open");
                cmd.arg("-b").arg(bundle_id);
                cmd
            }
            _ => executable.command(application_bundle_info),
        };

        match command.args(arguments).spawn() {
            Ok(mut child) => {
                ctx.background_executor()
                    .spawn(async move {
                        let now = Instant::now();
                        match child.status().await {
                            Ok(exit_code) => {
                                log::debug!(
                                    "process exited after {}ms with exit code: {}",
                                    now.elapsed().as_millis(),
                                    exit_code
                                );
                            }
                            Err(err) => {
                                log::error!("unable to await process {err:?}");
                            }
                        };
                    })
                    .detach();
                log::info!("Successfully launched {self:?}.");
                true
            }
            Err(e) => {
                log::error!("Error launching {self:?} {e:?}");
                false
            }
        }
    }

    // Given the line column number and the path, format into "path:line:column".
    fn format_file_path_with_line_and_column(
        full_path: &Path,
        line_column_number: Option<LineAndColumnArg>,
    ) -> String {
        let mut full_path_with_line_column = full_path.to_string_lossy().to_string();

        if let Some(line_column_number) = line_column_number {
            let _ = write!(
                &mut full_path_with_line_column,
                ":{}",
                line_column_number.line_num
            );

            if let Some(column_num) = line_column_number.column_num {
                let _ = write!(&mut full_path_with_line_column, ":{column_num}");
            }
        }

        full_path_with_line_column
    }
}

pub fn open_file_path_with_line_and_col(
    line_column_number: Option<LineAndColumnArg>,
    with_editor: Option<Editor>,
    full_path: &Path,
    ctx: &mut AppContext,
) {
    if full_path.is_file() {
        let editor = if with_editor.is_some_and(|editor| editor.is_installed(ctx)) {
            with_editor
        } else {
            let app_bundle_id = unsafe { default_app_to_open_path(full_path) };
            app_bundle_id
                .as_deref()
                .and_then(Editor::new_from_identifier)
        };

        if let Some(editor) = editor {
            if editor.open(line_column_number, full_path, ctx) {
                return;
            }
        }

        // NSWorkspace's default-app routing can hand files to a sibling
        // Warp channel (e.g. Stable handling files while Preview is running).
        // When the resolved default is a different Warp, open with the
        // running channel's bundle directly.
        let bundle_id = unsafe { default_app_to_open_path(full_path) };
        if let Some(bundle_id) = bundle_id.as_deref() {
            let current = ChannelState::app_id().to_string();
            if bundle_id != current
                && is_warp_bundle(bundle_id)
                && open_with_bundle(&current, full_path)
            {
                return;
            }
        }
    }
    ctx.open_file_path(full_path);
}

fn is_warp_bundle(bundle_id: &str) -> bool {
    AppId::parse(bundle_id)
        .map(|id| id.qualifier() == "dev" && id.organization() == "warp")
        .unwrap_or(false)
}

fn open_with_bundle(bundle_id: &str, path: &Path) -> bool {
    // Wait for `open` to exit; a non-zero status needs to bubble up so
    // the caller's ctx.open_file_path fallback still runs.
    command::blocking::Command::new("/usr/bin/open")
        .args(["-b", bundle_id])
        .arg(path)
        .status()
        .is_ok_and(|s| s.success())
}

// Get the Mac default app for opening the file path.
//
// The NSString returned by `-[NSBundle bundleIdentifier]` is autoreleased by
// Cocoa. We wrap the call in a local pool so the autoreleased string (and the
// one we pass in via `make_nsstring`) are drained before we return, and copy
// the UTF-8 bytes out into an owned `String` so no dangling pointer escapes.
unsafe fn default_app_to_open_path(file_path: &Path) -> Option<String> {
    let pool = NSAutoreleasePool::new(nil);
    let bundle_id = get_default_app_bundle_for_file(make_nsstring(file_path.to_string_lossy()));
    let result = if bundle_id == nil {
        None
    } else {
        let cstr = bundle_id.UTF8String() as *const u8;
        std::str::from_utf8(slice::from_raw_parts(cstr, bundle_id.len()))
            .ok()
            .map(ToOwned::to_owned)
    };
    pool.drain();
    result
}

#[cfg(test)]
#[path = "mac_tests.rs"]
mod tests;
