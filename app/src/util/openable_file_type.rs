//! File type detection utilities for determining if files can be opened in Warp.

#[cfg(feature = "local_fs")]
use crate::util::file::external_editor::{settings::EditorChoice, Editor, EditorSettings};
use serde::{Deserialize, Serialize};
use std::path::Path;
pub use warp_util::file_type::{is_binary_file, is_file_content_binary, is_markdown_file};

#[derive(
    Debug,
    Clone,
    Copy,
    Serialize,
    Deserialize,
    PartialEq,
    Eq,
    schemars::JsonSchema,
    settings_value::SettingsValue,
)]
#[schemars(
    description = "Layout used when opening files in the editor.",
    rename_all = "snake_case"
)]
pub enum EditorLayout {
    SplitPane,
    NewTab,
}

/// The type of file that can be opened in Warp. The in-product treatment for "opening" a file
/// depends on its type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenableFileType {
    /// A Markdown file, which should be opened in a Markdown viewer pane.
    Markdown,
    /// A code file, which should be opened in a code editor pane.
    Code,
    /// Other types of text files, e.g. txt, csv, svg files, which can still be opened in a code editor pane.
    Text,
}

/// The target application or viewer to use when opening a file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileTarget {
    /// Open in Warp's Markdown viewer.
    MarkdownViewer(EditorLayout),
    /// Open in Warp's Code Editor.
    CodeEditor(EditorLayout),
    /// Open in an external editor (e.g. VS Code, Emacs).
    #[cfg(feature = "local_fs")]
    ExternalEditor(Editor),
    /// Open in the environment editor ($EDITOR).
    EnvEditor,
    /// Open in the system default application.
    SystemDefault,
    /// Open in the system default application (generic open, e.g. for binary files).
    SystemGeneric,
}

/// Checks if a file is a code file with language support.
#[cfg(feature = "local_fs")]
pub fn is_supported_code_file(path: impl AsRef<Path>) -> bool {
    languages::language_by_local_filename(path.as_ref()).is_some()
}

#[cfg(not(feature = "local_fs"))]
pub fn is_supported_code_file(_path: impl AsRef<Path>) -> bool {
    false
}

pub fn is_supported_image_file(path: impl AsRef<Path>) -> bool {
    path.as_ref()
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| {
            matches!(
                ext.to_ascii_lowercase().as_str(),
                "jpg" | "jpeg" | "png" | "gif" | "webp" | "svg"
            )
        })
        .unwrap_or(false)
}

/// Returns true if `path` looks like a shell script the user intends to run when
/// "Open with Warp" is invoked from Finder/another app via a `file://` URL.
///
/// Policy: extension in {sh, bash, zsh, fish, ksh} with the user-execute bit set on Unix,
/// or extension in {ps1, bat, cmd} on Windows (no x-bit concept). On Unix, files with no
/// extension but a `#!` shebang and the user-execute bit set also qualify.
///
/// Narrow on purpose: this only affects the URI entry point, not "Open in New Tab" from
/// other UI surfaces, which still want shell scripts viewable in the editor.
/// Returns true if `path` exists and starts with a `#!` shebang. Reads only the
/// first two bytes — the URI entry point is reached from a `file://` URL, so the
/// file is attacker-controlled in size and `std::fs::read` would risk an OOM.
pub(crate) fn starts_with_shebang(path: &Path) -> bool {
    use std::io::Read;
    let mut prefix = [0u8; 2];
    match std::fs::File::open(path) {
        Ok(mut file) => file.read_exact(&mut prefix).is_ok() && prefix == [b'#', b'!'],
        Err(_) => false,
    }
}

#[cfg(unix)]
pub fn is_runnable_shell_script(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;

    // Match the documented routing policy: only the owner's execute bit counts.
    // A file `chmod 070` belongs to a group, not to the user invoking Warp.
    let has_user_x_bit = std::fs::metadata(path)
        .map(|m| m.permissions().mode() & 0o100 != 0)
        .unwrap_or(false);
    if !has_user_x_bit {
        return false;
    }
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase());
    if let Some(ext) = ext.as_deref() {
        return matches!(ext, "sh" | "bash" | "zsh" | "fish" | "ksh" | "command");
    }
    starts_with_shebang(path)
}

#[cfg(windows)]
pub fn is_runnable_shell_script(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .is_some_and(|ext| matches!(ext.as_str(), "ps1" | "bat" | "cmd"))
}

#[cfg(not(any(unix, windows)))]
pub fn is_runnable_shell_script(_path: &Path) -> bool {
    false
}

/// Determines if a file can be opened in Warp and returns its type.
/// Returns `None` if the file is binary and should not be opened.
pub fn is_file_openable_in_warp(path: &Path) -> Option<OpenableFileType> {
    if is_binary_file(path) {
        return None;
    }

    if is_markdown_file(path) {
        Some(OpenableFileType::Markdown)
    } else if is_supported_code_file(path) {
        Some(OpenableFileType::Code)
    } else {
        // We allow opening the file, even if we don't have particular syntax highlighting support
        // for it e.g. txt files.
        Some(OpenableFileType::Text)
    }
}

/// Only use this for UI elements that must explicitly open a file in Warp (i.e. "Open in New Tab").
/// Prefer `resolve_file_target` for all other cases to respect users' preferences.
/// This would also force any binary file to be opened in Warp's Code Editor, so you should likely check
/// `is_file_openable_in_warp` before rendering any such UI Elements.
#[cfg(feature = "local_fs")]
pub fn resolve_file_target_to_open_in_warp(
    path: &Path,
    settings: &EditorSettings,
    layout: Option<EditorLayout>,
) -> FileTarget {
    let openable_file_type = is_file_openable_in_warp(path);
    let is_markdown = matches!(openable_file_type, Some(OpenableFileType::Markdown));
    let layout = layout.unwrap_or(*settings.open_file_layout);

    if is_markdown && *settings.prefer_markdown_viewer {
        return FileTarget::MarkdownViewer(layout);
    }
    FileTarget::CodeEditor(layout)
}

/// Resolves the target application or viewer for opening a file based on its path and editor settings.
#[cfg(feature = "local_fs")]
pub fn resolve_file_target(
    path: &Path,
    settings: &EditorSettings,
    layout: Option<EditorLayout>,
) -> FileTarget {
    resolve_file_target_with_editor_choice(
        path,
        *settings.open_file_editor,
        *settings.prefer_markdown_viewer,
        *settings.open_file_layout,
        layout,
    )
}

#[cfg(feature = "local_fs")]
pub fn resolve_file_target_with_editor_choice(
    path: &Path,
    editor_choice: EditorChoice,
    prefer_markdown_viewer: bool,
    default_layout: EditorLayout,
    layout: Option<EditorLayout>,
) -> FileTarget {
    let is_openable_in_warp = is_file_openable_in_warp(path);
    let is_markdown = matches!(is_openable_in_warp, Some(OpenableFileType::Markdown));
    let layout = layout.unwrap_or(default_layout);
    let is_openable_in_warp = is_openable_in_warp.is_some();

    // 1. Markdown Viewer (only if user preference specified)
    if is_markdown && prefer_markdown_viewer {
        return FileTarget::MarkdownViewer(layout);
    }

    // 2. Warp Code Editor (Explicit user preference)
    if is_openable_in_warp && matches!(editor_choice, EditorChoice::Warp) {
        return FileTarget::CodeEditor(layout);
    }

    // 3. Env Editor
    if matches!(editor_choice, EditorChoice::EnvEditor) {
        return FileTarget::EnvEditor;
    }

    // 4. Binary files -> System Default
    if !is_openable_in_warp {
        return FileTarget::SystemGeneric;
    }

    // 5. External Editor or System Default (for text files)
    match editor_choice {
        EditorChoice::ExternalEditor(editor) => FileTarget::ExternalEditor(editor),
        EditorChoice::SystemDefault => FileTarget::SystemDefault,
        EditorChoice::Warp | EditorChoice::EnvEditor => unreachable!("Already matched above"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(feature = "local_fs")]
    use settings::Setting as _;
    use std::path::Path;

    #[test]
    fn test_binary_files_not_openable() {
        assert!(is_file_openable_in_warp(Path::new("image.png")).is_none());
        assert!(is_file_openable_in_warp(Path::new("video.mp4")).is_none());
        assert!(is_file_openable_in_warp(Path::new("binary.exe")).is_none());
        assert!(is_file_openable_in_warp(Path::new("archive.zip")).is_none());
    }

    #[test]
    #[cfg(feature = "local_fs")]
    fn test_open_code_panels_file_editor_default_is_warp() {
        use crate::util::file::external_editor::settings::OpenCodePanelsFileEditor;

        assert_eq!(
            OpenCodePanelsFileEditor::default_value(),
            EditorChoice::Warp
        );
    }

    #[test]
    #[cfg(feature = "local_fs")]
    fn test_resolve_file_target_markdown_viewer_precedence() {
        let target = resolve_file_target_with_editor_choice(
            Path::new("README.md"),
            EditorChoice::ExternalEditor(Editor::VSCode),
            true, /* prefer_markdown_viewer */
            EditorLayout::SplitPane,
            None,
        );

        assert_eq!(target, FileTarget::MarkdownViewer(EditorLayout::SplitPane));
    }

    #[test]
    #[cfg(feature = "local_fs")]
    fn test_resolve_file_target_warp_uses_default_layout() {
        let target = resolve_file_target_with_editor_choice(
            Path::new("data.txt"),
            EditorChoice::Warp,
            true, /* prefer_markdown_viewer */
            EditorLayout::NewTab,
            None,
        );

        assert_eq!(target, FileTarget::CodeEditor(EditorLayout::NewTab));
    }

    #[test]
    #[cfg(feature = "local_fs")]
    fn test_resolve_file_target_binary_is_system_generic() {
        let target = resolve_file_target_with_editor_choice(
            Path::new("image.png"),
            EditorChoice::Warp,
            true, /* prefer_markdown_viewer */
            EditorLayout::SplitPane,
            None,
        );

        assert_eq!(target, FileTarget::SystemGeneric);
    }

    #[test]
    #[cfg(feature = "local_fs")]
    fn test_resolve_file_target_binary_uses_env_editor() {
        let target = resolve_file_target_with_editor_choice(
            Path::new("image.png"),
            EditorChoice::EnvEditor,
            true, /* prefer_markdown_viewer */
            EditorLayout::SplitPane,
            None,
        );
        assert_eq!(target, FileTarget::EnvEditor);
    }

    #[test]
    fn test_markdown_files() {
        assert_eq!(
            is_file_openable_in_warp(Path::new("README.md")),
            Some(OpenableFileType::Markdown)
        );
        assert_eq!(
            is_file_openable_in_warp(Path::new("doc.markdown")),
            Some(OpenableFileType::Markdown)
        );
        assert_eq!(
            is_file_openable_in_warp(Path::new("README")),
            Some(OpenableFileType::Markdown)
        );
        assert_eq!(
            is_file_openable_in_warp(Path::new("CHANGELOG")),
            Some(OpenableFileType::Markdown)
        );
    }

    #[test]
    #[cfg(feature = "local_fs")]
    fn test_code_files() {
        assert_eq!(
            is_file_openable_in_warp(Path::new("main.rs")),
            Some(OpenableFileType::Code)
        );
        assert_eq!(
            is_file_openable_in_warp(Path::new("app.js")),
            Some(OpenableFileType::Code)
        );
        assert_eq!(
            is_file_openable_in_warp(Path::new("script.py")),
            Some(OpenableFileType::Code)
        );
        assert_eq!(
            is_file_openable_in_warp(Path::new("config.json")),
            Some(OpenableFileType::Code)
        );
    }

    #[test]
    #[cfg(not(feature = "local_fs"))]
    fn test_code_files() {
        assert_eq!(
            is_file_openable_in_warp(Path::new("main.rs")),
            Some(OpenableFileType::Text)
        );
        assert_eq!(
            is_file_openable_in_warp(Path::new("app.js")),
            Some(OpenableFileType::Text)
        );
        assert_eq!(
            is_file_openable_in_warp(Path::new("script.py")),
            Some(OpenableFileType::Text)
        );
        assert_eq!(
            is_file_openable_in_warp(Path::new("config.json")),
            Some(OpenableFileType::Text)
        );
    }

    #[test]
    fn test_text_files() {
        // Files that are text but don't have language support
        assert_eq!(
            is_file_openable_in_warp(Path::new("data.txt")),
            Some(OpenableFileType::Text)
        );
        assert_eq!(
            is_file_openable_in_warp(Path::new("data.csv")),
            Some(OpenableFileType::Text)
        );
        assert_eq!(
            is_file_openable_in_warp(Path::new("file.svg")),
            Some(OpenableFileType::Text)
        );
    }

    #[test]
    fn test_is_supported_code_file() {
        assert!(is_supported_code_file(Path::new("main.rs")));
        assert!(is_supported_code_file(Path::new("app.js")));
        assert!(is_supported_code_file(Path::new("script.py")));
        assert!(!is_supported_code_file(Path::new("data.txt")));
        assert!(!is_supported_code_file(Path::new("image.png")));
    }

    #[test]
    #[cfg(unix)]
    fn test_is_runnable_shell_script_executable_sh() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("hello.sh");
        std::fs::write(&p, b"#!/bin/bash\necho hi\n").unwrap();
        let mut perms = std::fs::metadata(&p).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&p, perms).unwrap();
        assert!(is_runnable_shell_script(&p));
    }

    #[test]
    #[cfg(unix)]
    fn test_is_runnable_shell_script_non_executable_sh() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("hello.sh");
        std::fs::write(&p, b"#!/bin/bash\necho hi\n").unwrap();
        let mut perms = std::fs::metadata(&p).unwrap().permissions();
        perms.set_mode(0o644);
        std::fs::set_permissions(&p, perms).unwrap();
        assert!(!is_runnable_shell_script(&p));
    }

    #[test]
    #[cfg(unix)]
    fn test_is_runnable_shell_script_group_only_executable_rejected() {
        // Mode 0o070: group-x and group-r/w only, no user-execute. Must NOT classify
        // as runnable — only the owner's execute bit drives the routing decision.
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("group_only.sh");
        std::fs::write(&p, b"#!/bin/bash\necho hi\n").unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o070)).unwrap();
        assert!(!is_runnable_shell_script(&p));
    }

    #[test]
    #[cfg(unix)]
    fn test_is_runnable_shell_script_other_shell_extensions() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        for name in ["run.bash", "run.zsh", "run.fish", "run.ksh", "run.command"] {
            let p = dir.path().join(name);
            std::fs::write(&p, b"#!/bin/sh\n:\n").unwrap();
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
            assert!(is_runnable_shell_script(&p), "{name} should be runnable");
        }
    }

    #[test]
    #[cfg(unix)]
    fn test_is_runnable_shell_script_shebang_no_extension() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("noext");
        std::fs::write(&p, b"#!/bin/sh\necho hi\n").unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(is_runnable_shell_script(&p));
    }

    #[test]
    #[cfg(unix)]
    fn test_is_runnable_shell_script_shebang_no_extension_no_x_bit() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("noext");
        std::fs::write(&p, b"#!/bin/sh\necho hi\n").unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(!is_runnable_shell_script(&p));
    }

    #[test]
    #[cfg(unix)]
    fn test_is_runnable_shell_script_plain_text_rejected() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("notes.txt");
        std::fs::write(&p, b"just some text\n").unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(!is_runnable_shell_script(&p));
    }

    #[test]
    #[cfg(unix)]
    fn test_is_runnable_shell_script_symlink_to_executable() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("real.sh");
        std::fs::write(&target, b"#!/bin/sh\n:\n").unwrap();
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o755)).unwrap();
        let link = dir.path().join("link.sh");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        assert!(is_runnable_shell_script(&link));
    }

    #[test]
    fn test_starts_with_shebang_present() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("script");
        std::fs::write(&p, b"#!/bin/sh\necho hi\n").unwrap();
        assert!(starts_with_shebang(&p));
    }

    #[test]
    fn test_starts_with_shebang_absent() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("plain");
        std::fs::write(&p, b"echo hi\n").unwrap();
        assert!(!starts_with_shebang(&p));
    }

    #[test]
    fn test_starts_with_shebang_one_byte_file() {
        // `read_exact(&mut [0u8; 2])` must short-read on a single-byte file.
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("tiny");
        std::fs::write(&p, b"#").unwrap();
        assert!(!starts_with_shebang(&p));
    }

    #[test]
    fn test_starts_with_shebang_missing_path() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("nope");
        assert!(!starts_with_shebang(&p));
    }
}
