use std::io;

use super::{FileLoadError, file_load_error_message};

#[test]
fn file_load_error_message_describes_editor_limits() {
    assert_eq!(
        file_load_error_message(&FileLoadError::TooLarge {
            size_estimate: Some(2 * 1024 * 1024 * 1024),
            limit_bytes: 100 * 1024 * 1024,
        }),
        "File is larger than the 100.0 MiB limit (reported size ~2.0 GiB)."
    );
    assert_eq!(
        file_load_error_message(&FileLoadError::TooLarge {
            size_estimate: None,
            limit_bytes: 100 * 1024 * 1024,
        }),
        "File is larger than the 100.0 MiB limit."
    );
    assert_eq!(
        file_load_error_message(&FileLoadError::TooManyLineBreaks { limit: 1_000_000 }),
        "File exceeds the Code editor limit of 1000000 line breaks."
    );
}

#[test]
fn file_load_error_message_keeps_generic_io_failure_copy() {
    assert_eq!(
        file_load_error_message(&FileLoadError::IOError(io::Error::other("failure"))),
        "Failed to load file."
    );
}
