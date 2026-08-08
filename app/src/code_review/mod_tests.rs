use warp_completer::completer::PathSeparators;

use super::format_path_for_display;

#[test]
fn formats_nested_repo_relative_path_for_windows() {
    assert_eq!(
        format_path_for_display(
            ".github/actions/prepare_environment/action.yml",
            &PathSeparators::for_windows(),
        ),
        r".github\actions\prepare_environment\action.yml"
    );
}

#[test]
fn preserves_single_segment_path_for_windows() {
    assert_eq!(
        format_path_for_display("Cargo.toml", &PathSeparators::for_windows()),
        "Cargo.toml"
    );
}

#[test]
fn formats_renamed_old_path_for_windows() {
    assert_eq!(
        format_path_for_display("src/previous/file_name.rs", &PathSeparators::for_windows(),),
        r"src\previous\file_name.rs"
    );
}

#[test]
fn preserves_git_style_path_for_unix_sessions() {
    assert_eq!(
        format_path_for_display(
            ".github/actions/prepare_environment/action.yml",
            &PathSeparators::for_unix(),
        ),
        ".github/actions/prepare_environment/action.yml"
    );
}
