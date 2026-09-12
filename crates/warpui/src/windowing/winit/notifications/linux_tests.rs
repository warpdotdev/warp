use std::io;
use std::path::PathBuf;

use super::notify_rust_exe_name;

#[test]
fn maps_current_exe_lookup_failure_to_error_message() {
    let err = io::Error::new(io::ErrorKind::NotFound, "No such file or directory");

    let result = notify_rust_exe_name(Err(err));

    assert_eq!(result, Err("No such file or directory".to_string()));
}

#[test]
fn maps_utf8_file_name_from_current_exe_path() {
    let result = notify_rust_exe_name(Ok(PathBuf::from("/usr/bin/warp")));

    assert_eq!(result, Ok("warp".to_string()));
}

#[test]
fn rejects_current_exe_path_without_utf8_file_name() {
    use std::os::unix::ffi::OsStringExt;

    let path = PathBuf::from(std::ffi::OsString::from_vec(vec![0xff, 0xfe]));

    let result = notify_rust_exe_name(Ok(path));

    assert_eq!(
        result,
        Err("current executable path is missing a UTF-8 file name".to_string())
    );
}
