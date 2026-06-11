use super::{extract_captured_path, PATH_CAPTURE_END, PATH_CAPTURE_START};

#[test]
fn extracts_clean_path() {
    let output = format!("{PATH_CAPTURE_START}/opt/homebrew/bin:/usr/bin{PATH_CAPTURE_END}");
    assert_eq!(
        extract_captured_path(&output),
        Some("/opt/homebrew/bin:/usr/bin")
    );
}

#[test]
fn ignores_startup_banner_output() {
    // rc files printing to stdout (fastfetch/MOTD) before the PATH line.
    let output = format!(
        "ascii art line 1\nOS macOS shell zsh\n\
         {PATH_CAPTURE_START}/opt/homebrew/bin:/usr/bin:/bin{PATH_CAPTURE_END}\n"
    );
    assert_eq!(
        extract_captured_path(&output),
        Some("/opt/homebrew/bin:/usr/bin:/bin")
    );
}

#[test]
fn missing_markers_returns_none() {
    assert_eq!(extract_captured_path("/opt/homebrew/bin:/usr/bin"), None);
}

#[test]
fn missing_end_marker_returns_none() {
    let output = format!("{PATH_CAPTURE_START}/opt/homebrew/bin");
    assert_eq!(extract_captured_path(&output), None);
}

#[test]
fn empty_path_between_markers() {
    let output = format!("{PATH_CAPTURE_START}{PATH_CAPTURE_END}");
    assert_eq!(extract_captured_path(&output), Some(""));
}

#[test]
fn preserves_colons_and_surrounding_noise() {
    let path = "/a:/b:/c";
    let output = format!("before{PATH_CAPTURE_START}{path}{PATH_CAPTURE_END}after");
    assert_eq!(extract_captured_path(&output), Some(path));
}
