use super::windows_named_pipe_path;

// This transformation must produce the same path Windows actually names the pipe with, or
// `PipeSecurityAttributes`/`CreateNamedPipeW` (REV-1546) and the client's `ClientOptions::open`
// would silently target the wrong (or a nonexistent) pipe object.
#[test]
fn builds_windows_named_pipe_path_from_plain_name() {
    assert_eq!(
        windows_named_pipe_path("WarpDefault_URI_CHANNEL"),
        r"\\.\pipe\WarpDefault_URI_CHANNEL"
    );
}
