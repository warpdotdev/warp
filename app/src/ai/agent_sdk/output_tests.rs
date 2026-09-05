use comfy_table::Cell;
use serde::Serialize;
use serde_json::json;
use warp_cli::agent::OutputFormat;
use warp_cli::json_filter::parse_jq_filter;

use super::{
    TableFormat, run_jq_filter, write_filter_output, write_json, write_json_line, write_list,
};

#[derive(Serialize)]
struct TestItem {
    id: &'static str,
    subject: &'static str,
}

impl TableFormat for TestItem {
    fn header() -> Vec<Cell> {
        vec![Cell::new("ID"), Cell::new("SUBJECT")]
    }

    fn row(&self) -> Vec<Cell> {
        vec![Cell::new(self.id), Cell::new(self.subject)]
    }
}

#[test]
fn write_list_emits_json_for_json_output_format() {
    let mut output = Vec::new();
    let items = [TestItem {
        id: "message-1",
        subject: "Build update",
    }];

    write_list(items, OutputFormat::Json, &mut output).unwrap();

    let rendered = String::from_utf8(output).unwrap();
    assert_eq!(rendered, r#"[{"id":"message-1","subject":"Build update"}]"#);
}

#[test]
fn write_list_emits_ndjson_for_ndjson_output_format() {
    let mut output = Vec::new();
    let items = [
        TestItem {
            id: "message-1",
            subject: "Build update",
        },
        TestItem {
            id: "message-2",
            subject: "Pivot",
        },
    ];

    write_list(items, OutputFormat::Ndjson, &mut output).unwrap();

    let rendered = String::from_utf8(output).unwrap();
    assert_eq!(
        rendered,
        "{\"id\":\"message-1\",\"subject\":\"Build update\"}\n{\"id\":\"message-2\",\"subject\":\"Pivot\"}\n"
    );
}

#[test]
fn write_json_emits_pretty_json_with_trailing_newline() {
    let mut output = Vec::new();
    let item = TestItem {
        id: "message-1",
        subject: "Build update",
    };

    write_json(&item, &mut output).unwrap();

    let rendered = String::from_utf8(output).unwrap();
    assert_eq!(
        rendered,
        "{\n  \"id\": \"message-1\",\n  \"subject\": \"Build update\"\n}\n"
    );
}

#[test]
fn write_json_line_emits_compact_json_with_trailing_newline() {
    let mut output = Vec::new();
    let item = TestItem {
        id: "message-1",
        subject: "Build update",
    };

    write_json_line(&item, &mut output).unwrap();

    let rendered = String::from_utf8(output).unwrap();
    assert_eq!(
        rendered,
        "{\"id\":\"message-1\",\"subject\":\"Build update\"}\n"
    );
}
/// A small fixture that matches the shape of `GET /api/v1/agent/runs`.
fn list_response_fixture() -> serde_json::Value {
    json!({
        "runs": [
            { "task_id": "01HX0000000000000000000001", "title": "Alpha", "state": "succeeded" },
            { "task_id": "01HX0000000000000000000002", "title": "Beta", "state": "failed" },
        ],
        "page_info": { "has_next_page": false, "next_cursor": null }
    })
}

/// Run a filter against `value` and return its output as a UTF-8 string.
fn run(value: serde_json::Value, filter: &str) -> String {
    let filter = parse_jq_filter(filter).expect("filter compiles");
    let mut buf = Vec::new();
    run_jq_filter(value, &filter, &mut buf).expect("filter runs without error");
    String::from_utf8(buf).expect("output is valid utf-8")
}

#[test]
fn identity_filter_matches_non_filtered_json_output() {
    let fixture = list_response_fixture();
    let filtered = run(fixture.clone(), ".");

    let mut expected = Vec::new();
    serde_json::to_writer_pretty(&mut expected, &fixture).unwrap();
    expected.push(b'\n');

    assert_eq!(filtered.as_bytes(), expected.as_slice());
}

#[test]
fn scalar_string_is_unwrapped() {
    let fixture = list_response_fixture();
    let out = run(fixture, ".runs[0].task_id");
    assert_eq!(out, "01HX0000000000000000000001\n");
}

#[test]
fn scalar_number_is_unwrapped() {
    let fixture = list_response_fixture();
    let out = run(fixture, ".runs | length");
    assert_eq!(out, "2\n");
}

#[test]
fn scalar_bool_and_null_are_unwrapped() {
    let fixture = list_response_fixture();
    assert_eq!(run(fixture.clone(), ".page_info.has_next_page"), "false\n");
    assert_eq!(run(fixture.clone(), ".page_info.next_cursor"), "null\n");
    assert_eq!(run(fixture, "true"), "true\n");
}

#[test]
fn multiple_scalar_outputs_each_on_own_line() {
    let fixture = list_response_fixture();
    let out = run(fixture, ".runs[].task_id");
    assert_eq!(
        out,
        "01HX0000000000000000000001\n01HX0000000000000000000002\n"
    );
}

#[test]
fn non_scalar_output_is_pretty_json() {
    let fixture = list_response_fixture();
    let out = run(fixture, ".runs[0]");
    let expected = serde_json::to_string_pretty(&json!({
        "task_id": "01HX0000000000000000000001",
        "title": "Alpha",
        "state": "succeeded",
    }))
    .unwrap();
    assert_eq!(out, format!("{expected}\n"));
}

#[test]
fn inner_scalars_stay_json_encoded() {
    let fixture = list_response_fixture();
    let out = run(fixture, ".runs");
    assert!(
        out.contains(r#""title": "Alpha""#),
        "inner strings should remain JSON-encoded, got:\n{out}"
    );
    assert!(
        out.contains(r#""task_id": "01HX0000000000000000000001""#),
        "inner task_id should remain JSON-encoded, got:\n{out}"
    );
}

#[test]
fn empty_filter_produces_no_output() {
    let fixture = list_response_fixture();
    let out = run(fixture, "empty");
    assert_eq!(out, "");
}

#[test]
fn runtime_error_is_surfaced_after_partial_output() {
    // `.runs[].title | .[0]` succeeds on the first string but fails on a
    // later element if we introduce a non-indexable value. Build a fixture
    // where the second element has a numeric title, which triggers a runtime
    // error when we try to index it as a string.
    let fixture = json!({
        "runs": [
            { "title": "hello" },
            { "title": 42 },
        ]
    });
    let filter = parse_jq_filter(".runs[].title | .[0:1]").expect("filter compiles");
    let mut buf = Vec::new();
    let result = run_jq_filter(fixture, &filter, &mut buf);

    // The filter should fail at runtime on the integer title.
    assert!(result.is_err(), "expected runtime error");
    // The valid output from the first element should already be on the buffer.
    let rendered = String::from_utf8(buf).unwrap();
    assert!(
        rendered.starts_with("h\n"),
        "expected partial output before the error, got: {rendered:?}"
    );
}

#[test]
fn write_filter_output_respects_scalar_unwrapping_for_direct_vals() {
    use jaq_json::Val;

    // Exercise `write_filter_output` directly with jaq values to lock in the
    // expected rendering of each scalar variant.
    let mut buf = Vec::new();
    write_filter_output(&Val::Null, &mut buf).unwrap();
    write_filter_output(&Val::Bool(true), &mut buf).unwrap();
    write_filter_output(&Val::Bool(false), &mut buf).unwrap();
    assert_eq!(String::from_utf8(buf).unwrap(), "null\ntrue\nfalse\n");
}

/// Regression tests for APP-5099: CLI table output was truncated mid-print when
/// stdout is a non-blocking PTY. `print_list` now writes through
/// `StdoutBlockingGuard`, which clears `O_NONBLOCK` (via `clear_nonblocking`)
/// for the duration of the write so a large table blocks instead of failing
/// with `EAGAIN` after a partial write.
#[cfg(unix)]
mod nonblocking_stdout {
    use std::fs::File;
    use std::io::{Read as _, Write as _};
    use std::os::unix::io::FromRawFd as _;

    use super::super::clear_nonblocking;

    /// Create a unix pipe, returning `(read_fd, write_fd)`.
    fn make_pipe() -> (libc::c_int, libc::c_int) {
        let mut fds = [0 as libc::c_int; 2];
        // SAFETY: `pipe` writes exactly two valid fds into the provided array.
        let rc = unsafe { libc::pipe(fds.as_mut_ptr()) };
        assert_eq!(rc, 0, "pipe() failed: {}", std::io::Error::last_os_error());
        (fds[0], fds[1])
    }

    /// Mark `fd` as `O_NONBLOCK`, matching how stdout can be configured when it
    /// is a PTY.
    fn set_nonblocking(fd: libc::c_int) {
        // SAFETY: `fcntl` F_GETFL/F_SETFL only read/write the fd status flags.
        unsafe {
            let flags = libc::fcntl(fd, libc::F_GETFL, 0);
            assert!(
                flags >= 0,
                "F_GETFL failed: {}",
                std::io::Error::last_os_error()
            );
            let rc = libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK);
            assert_eq!(rc, 0, "F_SETFL failed: {}", std::io::Error::last_os_error());
        }
    }

    /// A payload larger than any pipe buffer, so the writer cannot dump it all
    /// at once and must contend with backpressure.
    fn large_payload() -> Vec<u8> {
        vec![b'x'; 4 * 1024 * 1024]
    }

    /// Reproduces the defect: writing a large payload to a **non-blocking** fd
    /// whose reader is not draining fails with `WouldBlock` after a partial
    /// write. This is what truncates a CLI table mid-print on a non-blocking
    /// PTY, and what the fix prevents.
    #[test]
    fn nonblocking_write_truncates_without_the_fix() {
        let (r, w) = make_pipe();
        set_nonblocking(w);

        // No reader drains `r`, so the pipe buffer fills; a blocking write would
        // wait, but this fd is non-blocking, so it errors instead.
        // SAFETY: `w` is a valid fd we own; `File` takes ownership and closes it.
        let mut writer = unsafe { File::from_raw_fd(w) };
        let payload = large_payload();
        let err = writer
            .write_all(&payload)
            .expect_err("a non-blocking write of a large payload with no reader must fail");
        assert_eq!(err.kind(), std::io::ErrorKind::WouldBlock);

        // SAFETY: `r` is a valid fd we own and have not closed yet.
        unsafe { libc::close(r) };
    }

    /// Verifies the fix: after `clear_nonblocking` (what `StdoutBlockingGuard`
    /// applies to stdout), the same large write completes in full and the
    /// reader receives every byte — no truncation.
    #[test]
    fn clearing_nonblocking_lets_the_full_payload_through() {
        let (r, w) = make_pipe();
        set_nonblocking(w);

        // Apply the production fix to the write end.
        let restored = clear_nonblocking(w);
        assert!(
            restored.is_some(),
            "expected to clear O_NONBLOCK on the write fd"
        );

        let payload = large_payload();
        let expected_len = payload.len();

        // Drain the read end on another thread so the now-blocking writer can
        // make progress instead of deadlocking.
        let reader = std::thread::spawn(move || {
            // SAFETY: `r` is a valid fd we own; `File` takes ownership.
            let mut reader = unsafe { File::from_raw_fd(r) };
            let mut buf = Vec::new();
            reader.read_to_end(&mut buf).expect("read the full payload");
            buf
        });

        {
            // SAFETY: `w` is a valid fd we own; `File` takes ownership and closes
            // it at the end of this scope, giving the reader EOF.
            let mut writer = unsafe { File::from_raw_fd(w) };
            writer
                .write_all(&payload)
                .expect("blocking write completes in full");
            writer.flush().expect("flush succeeds");
        }

        let received = reader.join().expect("reader thread panicked");
        assert_eq!(
            received.len(),
            expected_len,
            "the reader must receive every byte of the payload"
        );
        assert!(
            received.iter().all(|&b| b == b'x'),
            "payload content must be preserved"
        );
    }
}
