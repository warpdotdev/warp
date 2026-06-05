use std::path::Path;

#[cfg(feature = "local_fs")]
use settings::Setting as _;

use super::*;

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
