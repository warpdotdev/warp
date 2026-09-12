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

/// Resolves `path` as a system-default open, with the "is Warp the OS handler"
/// probe stubbed so the result does not depend on the host's file associations.
#[cfg(feature = "local_fs")]
fn resolve_system_default(path: &Path, handler_is_warp: bool) -> FileTarget {
    resolve_file_target_with_system_default_handler(
        path,
        EditorChoice::SystemDefault,
        true, /* prefer_markdown_viewer */
        EditorLayout::SplitPane,
        None,
        |_| handler_is_warp,
    )
}

/// When the OS hands the file straight back to Warp, opening it in-process is
/// the only path that can carry the requested line: the `file://` round trip
/// through the OS drops it.
#[test]
#[cfg(feature = "local_fs")]
fn test_resolve_file_target_system_default_handled_by_warp_opens_code_editor() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("main.rs");
    std::fs::write(&path, "fn main() {}\n").unwrap();

    assert_eq!(
        resolve_system_default(&path, true),
        FileTarget::CodeEditor(EditorLayout::SplitPane)
    );
}

#[test]
#[cfg(feature = "local_fs")]
fn test_resolve_file_target_system_default_handled_by_other_app_is_unchanged() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("main.rs");
    std::fs::write(&path, "fn main() {}\n").unwrap();

    assert_eq!(
        resolve_system_default(&path, false),
        FileTarget::SystemDefault
    );
}

/// Short-circuiting the OS must not change *where* a file lands, only whether
/// it keeps its line. A runnable script comes back from the OS through
/// `uri::classify_open_file_action` and runs in a session, so it has to keep
/// taking that route even when Warp is the registered handler.
#[test]
#[cfg(all(unix, feature = "local_fs"))]
fn test_resolve_file_target_runnable_script_still_goes_to_system_default() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("deploy.sh");
    std::fs::write(&path, b"#!/bin/bash\necho deploying\n").unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();

    assert_eq!(
        resolve_system_default(&path, true),
        FileTarget::SystemDefault
    );

    // The same script without the execute bit is just text, and short-circuits
    // like any other file.
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert_eq!(
        resolve_system_default(&path, true),
        FileTarget::CodeEditor(EditorLayout::SplitPane)
    );
}

/// Only real files reach the editor on the far side of the round trip, so a
/// directory must not be short-circuited into one. It never reaches the guard
/// to begin with: a directory is not openable in Warp, so rule 4 hands it to
/// the OS generically before the handler is ever consulted.
#[test]
#[cfg(feature = "local_fs")]
fn test_resolve_file_target_directory_is_not_short_circuited() {
    let dir = tempfile::tempdir().unwrap();

    assert_eq!(
        resolve_system_default(dir.path(), true),
        FileTarget::SystemGeneric
    );
}

/// Binary files short-circuit to `SystemGeneric` before the handler is
/// consulted: Warp cannot render them, so handing them to the OS stays correct
/// even when Warp is the registered handler.
#[test]
#[cfg(feature = "local_fs")]
fn test_resolve_file_target_binary_stays_system_generic_when_warp_is_handler() {
    assert_eq!(
        resolve_system_default(Path::new("image.png"), true),
        FileTarget::SystemGeneric
    );
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

// Regression coverage for issue #9005: shell scripts opened via `file://` should run,
// not open in the editor. Exercised through the pure routing helper to avoid standing
// up a full `AppContext`.

#[test]
#[cfg(unix)]
fn test_open_file_executable_sh_routes_to_execute() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("run.sh");
    std::fs::write(&p, b"#!/bin/sh\n:\n").unwrap();
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
    let action = classify_open_file_action(&p, true);
    assert_eq!(action, OpenFileAction::ExecuteInSession);
}

#[test]
#[cfg(unix)]
fn test_open_file_non_executable_sh_routes_to_editor() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("view.sh");
    std::fs::write(&p, b"#!/bin/sh\n:\n").unwrap();
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert_eq!(classify_open_file_action(&p, true), OpenFileAction::Editor);
}

#[test]
#[cfg(unix)]
fn test_open_file_executable_bash_zsh_fish_route_to_execute() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    for name in ["run.bash", "run.zsh", "run.fish", "run.command"] {
        let p = dir.path().join(name);
        std::fs::write(&p, b"#!/bin/sh\n:\n").unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(
            classify_open_file_action(&p, true),
            OpenFileAction::ExecuteInSession,
            "{name} should route to ExecuteInSession",
        );
    }
}

#[test]
fn test_open_file_markdown_routes_to_notebook_when_viewer_enabled() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("README.md");
    std::fs::write(&p, b"# hi\n").unwrap();
    assert_eq!(
        classify_open_file_action(&p, true),
        OpenFileAction::Notebook
    );
}

#[test]
fn test_open_file_markdown_routes_to_editor_when_viewer_disabled() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("README.md");
    std::fs::write(&p, b"# hi\n").unwrap();
    assert_eq!(classify_open_file_action(&p, false), OpenFileAction::Editor);
}

#[test]
fn test_open_file_ipynb_routes_to_notebook_when_enabled() {
    // A `.ipynb` opened via `file://` (e.g. "Open with Warp" from Finder) opens
    // in the notebook viewer, not the raw-JSON code editor.
    let _flag = FeatureFlag::JupyterNotebookRendering.override_enabled(true);
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("analysis.ipynb");
    std::fs::write(&p, b"{\"nbformat\": 4, \"cells\": []}\n").unwrap();
    assert_eq!(
        classify_open_file_action(&p, false),
        OpenFileAction::Notebook
    );
}

#[test]
fn test_open_file_ipynb_opens_in_editor_when_disabled() {
    // Without the feature flag, `.ipynb` is not rendered in the notebook viewer
    // and falls through to the code editor.
    let _flag = FeatureFlag::JupyterNotebookRendering.override_enabled(false);
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("analysis.ipynb");
    std::fs::write(&p, b"{\"nbformat\": 4, \"cells\": []}\n").unwrap();
    assert_eq!(classify_open_file_action(&p, true), OpenFileAction::Editor);
}

#[test]
#[cfg(feature = "local_fs")]
fn test_open_file_rust_source_still_opens_in_editor() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("main.rs");
    std::fs::write(&p, b"fn main() {}\n").unwrap();
    assert_eq!(classify_open_file_action(&p, true), OpenFileAction::Editor);
}

#[test]
fn test_open_file_directory_routes_to_session() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(
        classify_open_file_action(dir.path(), true),
        OpenFileAction::ExecuteInSession
    );
}

#[test]
#[cfg(unix)]
fn test_open_file_non_runnable_shebang_routes_to_editor() {
    // Extensionless `#!/bin/sh` file without the user-execute bit. Without the
    // shebang fall-through this would hit `ExecuteInSession` and the shell would
    // refuse to run it; the editor is the right place to view it.
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("noext");
    std::fs::write(&p, b"#!/bin/sh\necho hi\n").unwrap();
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert_eq!(classify_open_file_action(&p, true), OpenFileAction::Editor);
}

/// Pins the invariant the short circuit rests on: among files that reach it,
/// it fires exactly when the OS round trip would itself have opened an editor.
/// Both sides consult `classify_open_file_action`, so this fails loudly if a
/// future change to the classifier stops lining up with the resolver's own
/// rules — the Notebook arm against rules 0 and 1, the non-openable arm
/// against rule 4.
#[test]
#[cfg(feature = "local_fs")]
fn test_short_circuit_matches_os_round_trip_classification() {
    const PREFER_MARKDOWN_VIEWER: bool = true;
    // Pinned so the Jupyter row does not depend on the dogfood default.
    let _flag = FeatureFlag::JupyterNotebookRendering.override_enabled(false);

    let dir = tempfile::tempdir().unwrap();
    let code = dir.path().join("main.rs");
    std::fs::write(&code, "fn main() {}\n").unwrap();
    let markdown = dir.path().join("README.md");
    std::fs::write(&markdown, "# hi\n").unwrap();
    let notebook = dir.path().join("analysis.ipynb");
    std::fs::write(&notebook, "{\"nbformat\": 4, \"cells\": []}\n").unwrap();
    let missing = dir.path().join("gone.rs");

    // An executable script only exists as a distinct case on unix, where the
    // execute bit is what makes it runnable.
    #[cfg(unix)]
    let script = {
        use std::os::unix::fs::PermissionsExt;
        let script = dir.path().join("deploy.sh");
        std::fs::write(&script, b"#!/bin/bash\n:\n").unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        Some(script)
    };
    #[cfg(not(unix))]
    let script = None;

    for path in [code, markdown, notebook, missing]
        .into_iter()
        .chain(script)
    {
        let opens_editor_via_os =
            classify_open_file_action(&path, PREFER_MARKDOWN_VIEWER) == OpenFileAction::Editor;
        let short_circuited = matches!(
            resolve_system_default(&path, true),
            FileTarget::CodeEditor(_)
        );
        assert_eq!(
            short_circuited, opens_editor_via_os,
            "{path:?} short-circuits to the code editor only when the OS round trip would open one"
        );
    }
}

/// The one place the two verdicts disagree, pinned so it stays visible and
/// cannot widen unnoticed.
///
/// `classify_open_file_action` admits any file starting with a shebang, while
/// `is_file_openable_in_warp` calls an extensionless file with no recognized
/// name binary. Such a file is diverted by rule 4 before the short circuit is
/// reachable, so it goes out to the OS and comes back into an editor at line 1.
/// Pre-existing: `master` behaves the same way. Closing it means relaxing what
/// rule 4 considers openable, which reaches well beyond opening files at a
/// line.
#[test]
#[cfg(all(unix, feature = "local_fs"))]
fn test_extensionless_shebang_file_is_diverted_before_the_short_circuit() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("noext");
    std::fs::write(&path, b"#!/bin/sh\necho hi\n").unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();

    assert_eq!(
        classify_open_file_action(&path, true),
        OpenFileAction::Editor,
        "the far side of the round trip opens this in an editor"
    );
    assert!(
        is_file_openable_in_warp(&path).is_none(),
        "but rule 4 treats it as binary and diverts it first"
    );
    assert_eq!(
        resolve_system_default(&path, true),
        FileTarget::SystemGeneric
    );
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
