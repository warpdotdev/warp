use super::{extract_captured_path, read_capped, PATH_CAPTURE_END, PATH_CAPTURE_START};

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

#[test]
fn read_capped_truncates_unbounded_input() {
    // Simulate a runaway shell that emits far more than the cap. The read must stop at the
    // limit rather than buffering everything (this is the safeguard against the observed
    // 16 GiB allocation).
    let limit = 1024u64;
    let huge = vec![b'x'; 8 * 1024];
    let out =
        futures::executor::block_on(read_capped(futures::io::Cursor::new(huge), limit)).unwrap();
    assert_eq!(out.len() as u64, limit);
}

#[test]
fn read_capped_reads_small_input_fully() {
    let data = b"a small amount of output".to_vec();
    let out = futures::executor::block_on(read_capped(
        futures::io::Cursor::new(data.clone()),
        1024 * 1024,
    ))
    .unwrap();
    assert_eq!(out, data);
}

#[test]
fn read_capped_output_still_extracts_path_within_cap() {
    // A normal capture (banner + sentinels) fits comfortably within the cap and PATH is
    // still recovered after the bounded read.
    let output = format!(
        "startup banner line\n{PATH_CAPTURE_START}/opt/homebrew/bin:/usr/bin{PATH_CAPTURE_END}\n"
    );
    let out = futures::executor::block_on(read_capped(
        futures::io::Cursor::new(output.into_bytes()),
        1024 * 1024,
    ))
    .unwrap();
    let stdout = String::from_utf8_lossy(&out);
    assert_eq!(
        extract_captured_path(&stdout),
        Some("/opt/homebrew/bin:/usr/bin")
    );
}
