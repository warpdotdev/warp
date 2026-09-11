use std::io::{Error as IoError, ErrorKind};

use super::{ClientError, InitializationError};
use crate::protocol::ProtocolError;

fn io_failure(kind: ErrorKind) -> ClientError {
    ClientError::Initialization(InitializationError::Io(IoError::from(kind)))
}

#[test]
fn retries_a_transport_that_is_not_listening_yet() {
    // The case that matters for single-instance hand-off: the peer has claimed the role but has
    // not finished creating its pipe.
    assert!(io_failure(ErrorKind::NotFound).is_transient_connect_failure());
    assert!(io_failure(ErrorKind::ConnectionRefused).is_transient_connect_failure());
    assert!(io_failure(ErrorKind::ResourceBusy).is_transient_connect_failure());
    assert!(io_failure(ErrorKind::TimedOut).is_transient_connect_failure());
    assert!(io_failure(ErrorKind::Interrupted).is_transient_connect_failure());
    assert!(io_failure(ErrorKind::WouldBlock).is_transient_connect_failure());
}

#[test]
fn does_not_retry_a_rejected_connection() {
    // A denied connection is a permission mismatch between the two processes, not a race, and
    // will be denied again identically.
    assert!(!io_failure(ErrorKind::PermissionDenied).is_transient_connect_failure());
    assert!(!io_failure(ErrorKind::InvalidInput).is_transient_connect_failure());
    assert!(!io_failure(ErrorKind::AlreadyExists).is_transient_connect_failure());
}

#[test]
fn does_not_retry_non_connect_failures() {
    assert!(
        !ClientError::Initialization(InitializationError::UnsupportedPlatform)
            .is_transient_connect_failure()
    );
    assert!(!ClientError::Disconnected.is_transient_connect_failure());
    assert!(!ClientError::ResponseChannelClosed.is_transient_connect_failure());
    assert!(!ClientError::PendingRequestInfoChannelClosed.is_transient_connect_failure());
    assert!(
        !ClientError::InternalProtocol(ProtocolError::Other("boom".to_owned()))
            .is_transient_connect_failure()
    );
}
