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
