use std::io;

use super::{ClientError, InitializationError};

#[test]
fn is_permission_denied_true_for_permission_denied_io_error() {
    let err = ClientError::Initialization(InitializationError::Io(io::Error::new(
        io::ErrorKind::PermissionDenied,
        "Access is denied. (os error 5)",
    )));

    assert!(err.is_permission_denied());
}

#[test]
fn is_permission_denied_false_for_other_io_errors() {
    let err = ClientError::Initialization(InitializationError::Io(io::Error::new(
        io::ErrorKind::NotFound,
        "no such pipe",
    )));

    assert!(!err.is_permission_denied());
}

#[test]
fn is_permission_denied_false_for_non_initialization_errors() {
    assert!(!ClientError::Disconnected.is_permission_denied());
}
