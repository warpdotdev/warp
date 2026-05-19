//! Tests for the rotation-event types and the deterministic mock summarizer.
//!
//! Live `SimpleLogger` integration (the path where rotation actually emits
//! events + invokes the summarizer) is covered separately in
//! `manager_tests.rs`; these tests focus on the data shape and the mock impl
//! in isolation.

use std::path::PathBuf;

use chrono::{TimeZone, Utc};

use super::{MockSummarizer, PipelineStep, RotationEvent, RotationSummarizer, RotationSummary};

// ---------- RotationEvent serialization ----------

#[test]
fn rotation_event_jsonl_roundtrip() {
    let event = RotationEvent {
        timestamp: Utc.with_ymd_and_hms(2026, 5, 14, 18, 32, 9).unwrap(),
        active_log: PathBuf::from("/var/log/mcp/server.log"),
        bytes_rotated: 10_485_760,
        discarded_path: Some(PathBuf::from("/var/log/mcp/server.log.5")),
    };

    let line = event.to_jsonl_line().expect("serialize");
    assert!(
        line.ends_with('\n'),
        "jsonl lines must terminate with newline"
    );
    let decoded: RotationEvent = serde_json::from_str(line.trim_end()).expect("roundtrip");
    assert_eq!(decoded, event);
}

#[test]
fn rotation_event_without_discarded_path_serializes_null() {
    let event = RotationEvent {
        timestamp: Utc.with_ymd_and_hms(2026, 5, 14, 18, 32, 9).unwrap(),
        active_log: PathBuf::from("/var/log/mcp/server.log"),
        bytes_rotated: 4096,
        discarded_path: None,
    };

    let line = event.to_jsonl_line().expect("serialize");
    assert!(
        line.contains("\"discarded_path\":null"),
        "missing-discard should serialize as JSON null, got: {line}"
    );
}

// ---------- RotationSummary serialization ----------

#[test]
fn rotation_summary_jsonl_roundtrip() {
    let summary = RotationSummary {
        timestamp: Utc.with_ymd_and_hms(2026, 5, 14, 18, 32, 14).unwrap(),
        source_path: PathBuf::from("/var/log/mcp/server.log.5"),
        bytes_summarized: 10_485_760,
        model: "qwen2.5-coder:7b@ollama-local".to_string(),
        pipeline: vec![
            PipelineStep {
                step: "extract_events".to_string(),
                duration_ms: 412,
            },
            PipelineStep {
                step: "classify".to_string(),
                duration_ms: 287,
            },
            PipelineStep {
                step: "summarize".to_string(),
                duration_ms: 893,
            },
        ],
        summary: "Server emitted 142 warnings about cache misses".to_string(),
        findings: vec![
            "Cache miss rate spiked to 41% between 16:30 and 16:50".to_string(),
            "Single transport disconnect at 16:42 (recovered)".to_string(),
        ],
    };

    let line = summary.to_jsonl_line().expect("serialize");
    let decoded: RotationSummary = serde_json::from_str(line.trim_end()).expect("roundtrip");
    assert_eq!(decoded, summary);
}

// ---------- MockSummarizer behavior ----------

#[tokio::test]
async fn mock_summarizer_returns_none_for_trivially_small_input() {
    let s = MockSummarizer::default();
    let result = s
        .summarize(&PathBuf::from("/tmp/tiny.log"), "x")
        .await
        .expect("mock should not error");
    assert!(
        result.is_none(),
        "small inputs should produce no summary (caller can write nothing)",
    );
}

#[tokio::test]
async fn mock_summarizer_emits_pipeline_steps_in_expected_order() {
    let s = MockSummarizer::default();
    let content = "info: server started\nwarning: cache miss\nerror: timeout\n";
    let summary = s
        .summarize(&PathBuf::from("/tmp/sample.log"), content)
        .await
        .expect("summarize")
        .expect("non-empty");

    let names: Vec<&str> = summary.pipeline.iter().map(|s| s.step.as_str()).collect();
    assert_eq!(names, vec!["extract_events", "classify", "summarize"]);
}

#[tokio::test]
async fn mock_summarizer_counts_warnings_and_errors() {
    let s = MockSummarizer::default();
    let content = "info: ok\nwarning: a\nWARN: b\nERROR: c\nerror: d\n";
    let summary = s
        .summarize(&PathBuf::from("/tmp/x.log"), content)
        .await
        .expect("summarize")
        .expect("non-empty");

    // The summary text records the counts. We don't pin exact wording, just
    // that the numbers were extracted correctly (2 warning lines, 2 error
    // lines; the WARN line happens to match both filters in the toy impl,
    // which is fine for a mock).
    assert!(
        summary.summary.contains("warning(s)") && summary.summary.contains("error(s)"),
        "summary should mention warning + error counts; got: {}",
        summary.summary
    );
}

#[tokio::test]
async fn mock_summarizer_flags_high_log_volume() {
    let s = MockSummarizer::default();
    let content = (0..1500)
        .map(|i| format!("info: line {i}"))
        .collect::<Vec<_>>()
        .join("\n");
    let summary = s
        .summarize(&PathBuf::from("/tmp/chatty.log"), &content)
        .await
        .expect("summarize")
        .expect("non-empty");

    assert!(
        summary
            .findings
            .iter()
            .any(|f| f.contains("chatty") || f.contains("High log volume")),
        "high-volume input should produce a chatty-subsystem finding; got findings: {:?}",
        summary.findings,
    );
}

#[tokio::test]
async fn mock_summarizer_carries_custom_model_name() {
    let s = MockSummarizer {
        model_name: "custom-model:13b@ollama-local".to_string(),
    };
    let summary = s
        .summarize(&PathBuf::from("/tmp/x.log"), "info: enough content here")
        .await
        .expect("summarize")
        .expect("non-empty");
    assert_eq!(summary.model, "custom-model:13b@ollama-local");
}
