use std::path::Path;

#[cfg(feature = "local_fs")]
use settings::Setting as _;
use warp_core::features::FeatureFlag;

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

/// `file.open` from local control relies on this resolver never routing to an
/// external editor or the system default app, even when user settings prefer one.
#[test]
#[cfg(feature = "local_fs")]
fn test_resolve_file_target_to_open_in_warp_never_leaves_warp() {
    use crate::util::file::external_editor::settings::{
        OpenCodePanelsFileEditor, OpenConversationLayoutPreference, OpenFileEditor, OpenFileLayout,
        PreferMarkdownViewer, PreferTabbedEditorView,
    };

    let settings = EditorSettings {
        open_file_editor: OpenFileEditor::new(Some(EditorChoice::ExternalEditor(Editor::VSCode))),
        open_code_panels_file_editor: OpenCodePanelsFileEditor::new(Some(
            EditorChoice::ExternalEditor(Editor::VSCode),
        )),
        open_file_layout: OpenFileLayout::new(None),
        prefer_markdown_viewer: PreferMarkdownViewer::new(Some(false)),
        prefer_tabbed_editor_view: PreferTabbedEditorView::new(None),
        open_conversation_layout_preference: OpenConversationLayoutPreference::new(None),
    };
    for path in ["README.md", "data.txt", "main.rs", "image.png", "script.sh"] {
        let target = resolve_file_target_to_open_in_warp(Path::new(path), &settings, None);
        assert!(
            matches!(
                target,
                FileTarget::CodeEditor(_) | FileTarget::MarkdownViewer(_)
            ),
            "{path} must resolve to an in-Warp surface, got {target:?}"
        );
    }
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
fn test_renders_in_warp_notebook_viewer() {
    // Markdown always renders in the notebook viewer, independent of the flag.
    let off = FeatureFlag::JupyterNotebookRendering.override_enabled(false);
    assert!(renders_in_warp_notebook_viewer(Path::new("README.md")));
    assert!(!renders_in_warp_notebook_viewer(Path::new(
        "notebook.ipynb"
    )));
    assert!(!renders_in_warp_notebook_viewer(Path::new("main.rs")));
    drop(off);

    // With the flag on, Jupyter notebooks also render in the notebook viewer.
    let _on = FeatureFlag::JupyterNotebookRendering.override_enabled(true);
    assert!(renders_in_warp_notebook_viewer(Path::new("notebook.ipynb")));
    assert!(renders_in_warp_notebook_viewer(Path::new("README.md")));
    assert!(!renders_in_warp_notebook_viewer(Path::new("main.rs")));
}

#[test]
#[cfg(feature = "local_fs")]
fn test_resolve_file_target_jupyter_notebook_flag_on() {
    let _flag = FeatureFlag::JupyterNotebookRendering.override_enabled(true);
    // Even with prefer_markdown_viewer off and an explicit Warp editor choice,
    // a Jupyter notebook routes to the notebook viewer (not the JSON editor).
    let target = resolve_file_target_with_editor_choice(
        Path::new("analysis.ipynb"),
        EditorChoice::Warp,
        false, /* prefer_markdown_viewer */
        EditorLayout::SplitPane,
        None,
    );
    assert_eq!(target, FileTarget::MarkdownViewer(EditorLayout::SplitPane));
}

#[test]
#[cfg(feature = "local_fs")]
fn test_resolve_file_target_jupyter_notebook_flag_off() {
    let _flag = FeatureFlag::JupyterNotebookRendering.override_enabled(false);
    // With the flag off, a Jupyter notebook opens as JSON in the code editor,
    // exactly as it does today.
    let target = resolve_file_target_with_editor_choice(
        Path::new("analysis.ipynb"),
        EditorChoice::Warp,
        true, /* prefer_markdown_viewer */
        EditorLayout::SplitPane,
        None,
    );
    assert_eq!(target, FileTarget::CodeEditor(EditorLayout::SplitPane));
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
