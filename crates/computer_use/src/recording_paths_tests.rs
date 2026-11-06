use super::*;

#[test]
fn allocates_a_uniquely_named_mp4_with_an_already_created_log_sibling() {
    let (path, log_path, _log_file) = new_recording_path().expect("should allocate a path");

    let file_name = path.file_name().unwrap().to_string_lossy();
    assert!(
        file_name.starts_with(&format!("{RECORDING_FILE_PREFIX}-")),
        "unexpected file name: {file_name}"
    );
    assert_eq!(path.extension().unwrap(), "mp4");
    assert_eq!(log_path, path.with_extension("log"));
    assert!(log_path.exists(), "log file should already be created");

    let _ = std::fs::remove_file(&log_path);
}

#[test]
fn allocates_distinct_paths_on_each_call() {
    let (first, ..) = new_recording_path().expect("should allocate a path");
    let (second, log_path, _log_file) = new_recording_path().expect("should allocate a path");

    assert_ne!(first, second);

    let _ = std::fs::remove_file(&log_path);
}
