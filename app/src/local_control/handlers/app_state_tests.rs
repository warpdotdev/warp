use ::local_control::{ActionKind, ErrorCode};

#[cfg(feature = "local_fs")]
use super::resolve_against_working_directory;
use super::{ensure_surface_available, validate_staged_input_text};
use crate::features::FeatureFlag;
use crate::local_control::handlers::metadata::SurfaceDestination;

#[test]
fn staged_input_rejects_line_breaks_and_control_sequences() {
    assert!(validate_staged_input_text(ActionKind::InputInsert, "safe staged text").is_ok());

    for text in ["line\nbreak", "line\rbreak", "tab\tbreak", "\u{1b}[31m"] {
        let error = validate_staged_input_text(ActionKind::InputInsert, text).err();
        assert!(error.is_some_and(|error| error.code == ErrorCode::InvalidParams));
    }
}

#[cfg(feature = "local_fs")]
#[test]
fn file_open_resolves_relative_paths_against_the_session_working_directory() {
    use std::path::{Path, PathBuf};

    let temp_dir = tempfile::tempdir().expect("temp dir");
    let working_directory = dunce::canonicalize(temp_dir.path()).expect("canonical temp dir");
    let nested = working_directory.join("docs");
    std::fs::create_dir(&nested).expect("nested dir");
    std::fs::write(working_directory.join("README.md"), "# hi").expect("readme");
    std::fs::write(nested.join("guide.md"), "# guide").expect("guide");

    assert_eq!(
        resolve_against_working_directory(Path::new("README.md"), &working_directory),
        working_directory.join("README.md")
    );
    assert_eq!(
        resolve_against_working_directory(Path::new("./README.md"), &working_directory),
        working_directory.join("README.md")
    );
    assert_eq!(
        resolve_against_working_directory(Path::new("docs/guide.md"), &working_directory),
        nested.join("guide.md")
    );
    assert_eq!(
        resolve_against_working_directory(Path::new("../README.md"), &nested),
        working_directory.join("README.md")
    );

    // Paths that do not exist still resolve against the session directory, so a genuine
    // failure reports the session-relative file rather than a process-relative one.
    assert_eq!(
        resolve_against_working_directory(Path::new("missing.md"), &working_directory),
        working_directory.join("missing.md")
    );

    let absolute = PathBuf::from(if cfg!(windows) {
        r"C:\tmp\absolute.md"
    } else {
        "/tmp/absolute.md"
    });
    assert_eq!(
        resolve_against_working_directory(&absolute, &working_directory),
        absolute
    );
}

#[test]
fn unavailable_surface_open_returns_structured_error() {
    let flag_guard = FeatureFlag::AgentManagementView.override_enabled(false);
    warpui::App::test((), |mut app| async move {
        let error = app
            .update(|ctx| {
                ensure_surface_available(
                    ActionKind::SurfaceAgentManagementOpen,
                    SurfaceDestination::AgentManagement,
                    ctx,
                )
            })
            .expect_err("disabled surface is rejected");
        assert_eq!(error.code, ErrorCode::UnsupportedAction);
        assert!(error.message.contains("surface.agent_management.open"));
    });
    drop(flag_guard);
}
