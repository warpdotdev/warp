//! Tests for [`super::parse_jq_filter`] and the clap-flattened [`super::JsonOutput`].

use clap::Parser;

use super::*;

/// Tiny wrapper so we can exercise `JsonOutput` through clap in isolation.
#[derive(Debug, Parser)]
struct TestApp {
    #[clap(flatten)]
    json_filter: JsonOutput,
}

#[test]
fn parse_jq_filter_accepts_simple_filter() {
    parse_jq_filter(".foo").expect("valid filter should compile");
}

#[test]
fn parse_jq_filter_accepts_stdlib_functions() {
    // Exercises jaq-std defs/funs: .foo | length
    parse_jq_filter(".foo | length").expect("stdlib functions should compile");
}

#[test]
fn parse_jq_filter_rejects_syntax_error() {
    let err = parse_jq_filter("@").expect_err("syntax error should be rejected");
    assert!(
        err.contains("`@`"),
        "error should quote the filter source with backticks, got: {err}"
    );
}

#[test]
fn parse_jq_filter_rejects_empty_string() {
    let err = parse_jq_filter("").expect_err("empty filter should be rejected");
    assert!(
        err.contains("``"),
        "error should quote the (empty) filter source with backticks, got: {err}"
    );
}

#[test]
fn parse_jq_filter_rejects_unknown_function() {
    let err = parse_jq_filter(".foo | bogus_function_name")
        .expect_err("unknown function should be rejected");
    assert!(
        err.contains("bogus_function_name") || err.contains("jq filter"),
        "error should mention the filter or the unknown name, got: {err}"
    );
}

#[test]
fn clap_populates_filter_when_jq_is_provided() {
    let app = TestApp::try_parse_from(["test", "--jq", ".foo"]).expect("valid --jq parses");
    assert!(app.json_filter.filter.is_some());
}

#[test]
fn clap_filter_is_none_by_default() {
    let app = TestApp::try_parse_from(["test"]).expect("no --jq parses");
    assert!(app.json_filter.filter.is_none());
}

#[test]
fn clap_rejects_invalid_filter_at_parse_time() {
    // This is the core fail-fast invariant: an invalid filter fails during
    // clap parsing, not at runtime.
    let err = TestApp::try_parse_from(["test", "--jq", "@"])
        .expect_err("invalid --jq is rejected by clap");
    assert_eq!(err.kind(), clap::error::ErrorKind::ValueValidation);
}
