use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::path::Path;
use std::sync::{Arc, Mutex};

use command::r#async::Command;
use futures::FutureExt as _;
use futures::executor::block_on;
use opentelemetry::Value;
use opentelemetry::trace::{Status, TracerProvider as _};
use opentelemetry_sdk::error::OTelSdkResult;
use opentelemetry_sdk::trace::{SdkTracerProvider, SpanData, SpanExporter};
use tracing::field::{Field, Visit};
use tracing::span::{Attributes, Id, Record};
use tracing::{Subscriber, subscriber};
use tracing_subscriber::layer::{Context, SubscriberExt as _};
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::{Layer, Registry};

use super::{MountResponse, detect_command, mount_command, run_spacectl_mount};
use crate::{CacheScope, CacheSetupError};

#[derive(Clone, Default)]
struct SpanFields(Arc<Mutex<BTreeMap<String, String>>>);

impl SpanFields {
    fn record(&self, values: &Record<'_>) {
        values.record(&mut FieldVisitor(Arc::clone(&self.0)));
    }

    fn get(&self, field: &str) -> Option<String> {
        self.0.lock().unwrap().get(field).cloned()
    }
}

#[derive(Clone, Debug, Default)]
struct CapturedSpans(Arc<Mutex<Vec<SpanData>>>);

impl SpanExporter for CapturedSpans {
    async fn export(&self, batch: Vec<SpanData>) -> OTelSdkResult {
        self.0.lock().unwrap().extend(batch);
        Ok(())
    }
}

struct FieldVisitor(Arc<Mutex<BTreeMap<String, String>>>);

impl Visit for FieldVisitor {
    fn record_i64(&mut self, field: &Field, value: i64) {
        self.0
            .lock()
            .unwrap()
            .insert(field.name().to_owned(), value.to_string());
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        self.0
            .lock()
            .unwrap()
            .insert(field.name().to_owned(), value.to_owned());
    }

    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        self.0
            .lock()
            .unwrap()
            .insert(field.name().to_owned(), format!("{value:?}"));
    }
}

impl<S> Layer<S> for SpanFields
where
    S: Subscriber + for<'lookup> LookupSpan<'lookup>,
{
    fn on_new_span(&self, attributes: &Attributes<'_>, id: &Id, context: Context<'_, S>) {
        if context
            .span(id)
            .is_some_and(|span| span.name() == "spacectl_cache_mount")
        {
            attributes.record(&mut FieldVisitor(Arc::clone(&self.0)));
        }
    }

    fn on_record(&self, id: &Id, values: &Record<'_>, context: Context<'_, S>) {
        if context
            .span(id)
            .is_some_and(|span| span.name() == "spacectl_cache_mount")
        {
            self.record(values);
        }
    }
}

fn args(command: &Command) -> Vec<&OsStr> {
    command.get_args().collect()
}

#[test]
fn detect_command_has_exact_argv_cache_root_and_repo_cwd() {
    let command = detect_command(Path::new("/cache/repos/key"), Path::new("/work/repo"));
    assert_eq!(command.get_program(), "spacectl");
    assert_eq!(
        args(&command),
        [
            "cache",
            "mount",
            "--detect=*",
            "--dry_run=true",
            "--cache_root",
            "/cache/repos/key",
            "-o",
            "json",
        ]
    );
    assert_eq!(command.get_current_dir(), Some(Path::new("/work/repo")));
}

#[test]
fn mount_command_has_exact_explicit_modes_dry_run_false_root_and_cwd() {
    let command = mount_command(
        Path::new("/cache/shared"),
        Path::new("/tmp/scratch"),
        &["cargo".to_owned(), "go".to_owned()],
    );
    assert_eq!(command.get_program(), "spacectl");
    assert_eq!(
        args(&command),
        [
            "cache",
            "mount",
            "--mode=cargo,go",
            "--dry_run=false",
            "--cache_root",
            "/cache/shared",
            "-o",
            "json",
        ]
    );
    assert_eq!(command.get_current_dir(), Some(Path::new("/tmp/scratch")));
}

#[test]
fn mount_response_deserializes_spacectl_output() {
    let response = serde_json::from_slice::<MountResponse>(
        br#"{
            "input": {"modes": ["cargo", "go"], "future": true},
            "output": {
                "add_envs": {"GOCACHE": "/cache/go"},
                "mounts": [
                    {"mode": "cargo", "cache_hit": true, "cache_path": "ignored", "mount_path": "ignored"},
                    {"mode": "go", "cache_hit": false}
                ],
                "disk_usage": {"total": "20G", "used": "4G"},
                "unknown": "ignored"
            },
            "unknown": []
        }"#,
    )
    .unwrap();
    assert_eq!(response.input.modes, ["cargo", "go"]);
    assert_eq!(response.output.add_envs["GOCACHE"], "/cache/go");
    assert!(response.output.mounts[0].cache_hit);
    assert!(!response.output.mounts[1].cache_hit);
    let disk_usage = response.output.disk_usage.unwrap();
    assert_eq!(disk_usage.total, "20G");
    assert_eq!(disk_usage.used, "4G");

    let response = serde_json::from_slice::<MountResponse>(br#"{"input":{},"output":{}}"#).unwrap();
    assert_eq!(response.output.disk_usage, None);
}

#[test]
fn mount_failures_record_safe_error_details_and_error_status_on_span() {
    let cases = [
        CacheSetupError::SpawnFailed,
        CacheSetupError::NonzeroExit {
            exit_code: Some(17),
        },
        CacheSetupError::Timeout,
    ];

    for error in cases {
        let fields = SpanFields::default();
        let subscriber = Registry::default().with(fields.clone());
        let report = subscriber::with_default(subscriber, || {
            block_on(run_spacectl_mount(
                CacheScope::Global,
                vec!["go".to_owned()],
                false,
                "shared".into(),
                Path::new("/cache/shared"),
                Path::new("/work"),
                &mut |_| futures::future::ready(Err(error.clone())),
            ))
        });

        assert_eq!(report.error.as_ref(), Some(&error));
        assert_eq!(
            fields.get("mount_error_kind").as_deref(),
            Some(error.kind())
        );
        assert_eq!(fields.get("otel.status_code").as_deref(), Some("ERROR"));
        assert_eq!(
            fields.get("otel.status_description").as_deref(),
            Some(error.to_string().as_str())
        );
        assert_eq!(
            fields.get("mount_error_exit_code"),
            error.exit_code().map(|exit_code| exit_code.to_string())
        );
    }
}

#[test]
fn invalid_spacectl_json_records_parse_failure_on_span() {
    let exporter = CapturedSpans::default();
    let provider = SdkTracerProvider::builder()
        .with_simple_exporter(exporter.clone())
        .build();
    let tracer = provider.tracer("build-cache-test");
    let subscriber = Registry::default().with(tracing_opentelemetry::layer().with_tracer(tracer));
    let report = subscriber::with_default(subscriber, || {
        run_spacectl_mount(
            CacheScope::Global,
            vec!["go".to_owned()],
            false,
            "shared".into(),
            Path::new("/cache/shared"),
            Path::new("/work"),
            &mut |_| futures::future::ready(Ok(b"not json".to_vec())),
        )
        .now_or_never()
        .expect("mocked spacectl call should be immediately ready")
    });

    assert_eq!(report.error, Some(CacheSetupError::JsonParseFailed));
    provider.force_flush().unwrap();
    let spans = exporter.0.lock().unwrap();
    let span = spans
        .iter()
        .find(|span| span.name == "spacectl_cache_mount")
        .expect("spacectl span should be exported");
    assert_eq!(
        span.status,
        Status::error(CacheSetupError::JsonParseFailed.to_string())
    );
    assert!(span.attributes.iter().any(|attribute| {
        attribute.key.as_str() == "mount_error_kind"
            && matches!(
                &attribute.value,
                Value::String(value) if value.as_str() == "json_parse_failed"
            )
    }));
}
