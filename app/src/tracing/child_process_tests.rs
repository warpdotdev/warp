use std::sync::{Arc, Mutex};

use chrono::{TimeDelta, Utc};
use opentelemetry::KeyValue;
use opentelemetry::baggage::BaggageExt as _;
use opentelemetry::trace::{TraceContextExt as _, TraceState, TracerProvider as _};
use opentelemetry_sdk::error::OTelSdkResult;
use opentelemetry_sdk::trace::{SdkTracerProvider, SpanData, SpanExporter};
use tracing_subscriber::layer::SubscriberExt as _;
use warp_cli::environment_checkout::EnvironmentCheckoutArgs;

use super::*;
use crate::tracing::cloud_agent_auth::AuthContext;

fn config() -> ChildProcessTracingConfig {
    serde_json::from_value(serde_json::json!({
        "endpoint": "https://collector.example.com",
        "credential": {
            "token": "handoff-test-token",
            "expires_at": Utc::now() + TimeDelta::minutes(17),
        }
    }))
    .unwrap()
}

#[test]
fn request_with_otlp_extension_is_not_consumed_as_credentials() {
    let directory = tempfile::tempdir().unwrap();
    let requests_file = directory.path().join("requests.otlp");
    fs::write(&requests_file, "checkout requests").unwrap();
    let guard = config().write(&requests_file).unwrap();
    assert!(
        ChildProcessTracingConfig::consume(&requests_file)
            .unwrap()
            .is_some()
    );
    assert!(!guard.exists());
    assert_eq!(
        fs::read_to_string(&requests_file).unwrap(),
        "checkout requests"
    );
}

#[test]
fn handoff_preserves_credential_and_expiry_then_disappears() {
    let directory = tempfile::tempdir().unwrap();
    let requests_file = directory.path().join("requests.json");
    let config = config();
    let expected = serde_json::to_value(&config).unwrap();
    let guard = config.write(&requests_file).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        assert_eq!(
            fs::metadata(&guard).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }

    let received = ChildProcessTracingConfig::consume(&requests_file)
        .unwrap()
        .unwrap();
    assert_eq!(serde_json::to_value(&received).unwrap(), expected);
    assert!(!guard.exists());
    assert!(AuthContext::from_snapshot(received.credential).is_ok());
    assert!(
        ChildProcessTracingConfig::consume(&requests_file)
            .unwrap()
            .is_none()
    );
}

#[test]
fn dropping_unconsumed_handoff_cleans_up() {
    let directory = tempfile::tempdir().unwrap();
    let requests_file = directory.path().join("requests.json");
    let guard = config().write(&requests_file).unwrap();
    let path = guard.to_path_buf();
    drop(guard);
    assert!(!path.exists());
}

#[test]
fn malformed_handoff_is_removed_without_echoing_values() {
    let directory = tempfile::tempdir().unwrap();
    let requests_file = directory.path().join("requests.json");
    let guard = config().write(&requests_file).unwrap();
    fs::write(
        &guard,
        br#"{"endpoint":"secret-test-token","credential":{"expires_at":"secret-test-token"}}"#,
    )
    .unwrap();

    let error = ChildProcessTracingConfig::consume(&requests_file)
        .err()
        .unwrap();
    assert!(!format!("{error:?}").contains("secret-test-token"));
    assert!(!guard.exists());
}

#[test]
fn oversized_handoff_is_removed() {
    let directory = tempfile::tempdir().unwrap();
    let requests_file = directory.path().join("requests.json");
    let guard = config().write(&requests_file).unwrap();
    fs::write(&guard, vec![b'x'; MAX_HANDOFF_BYTES as usize + 1]).unwrap();
    assert!(ChildProcessTracingConfig::consume(&requests_file).is_err());
    assert!(!guard.exists());
}

#[cfg(unix)]
#[test]
fn exposed_handoff_is_rejected_and_removed() {
    use std::os::unix::fs::PermissionsExt as _;
    let directory = tempfile::tempdir().unwrap();
    let requests_file = directory.path().join("requests.json");
    let guard = config().write(&requests_file).unwrap();
    fs::set_permissions(&guard, fs::Permissions::from_mode(0o644)).unwrap();
    assert!(ChildProcessTracingConfig::consume(&requests_file).is_err());
    assert!(!guard.exists());
}

#[cfg(unix)]
#[test]
fn handoff_symlink_does_not_read_or_remove_target() {
    use std::os::unix::fs::symlink;
    let directory = tempfile::tempdir().unwrap();
    let requests_file = directory.path().join("requests.json");
    let target = directory.path().join("other-secret");
    fs::write(&target, "untouched").unwrap();
    symlink(&target, handoff_path(&requests_file)).unwrap();
    assert!(ChildProcessTracingConfig::consume(&requests_file).is_err());
    assert_eq!(fs::read_to_string(&target).unwrap(), "untouched");
}

#[cfg(unix)]
#[test]
fn handoff_fifo_is_rejected_without_waiting_for_a_writer() {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt as _;

    let directory = tempfile::tempdir().unwrap();
    let scope_file = directory.path().join("requests.json");
    let path = handoff_path(&scope_file);
    let name = CString::new(path.as_os_str().as_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);

    let error = ChildProcessTracingConfig::consume(&scope_file)
        .err()
        .unwrap();
    assert_eq!(
        error.to_string(),
        "Child-process tracing handoff is not a file"
    );
    assert!(!path.exists());
}

#[derive(Clone, Debug, Default)]
struct RecordingExporter(Arc<Mutex<Vec<SpanData>>>);

impl SpanExporter for RecordingExporter {
    async fn export(&self, batch: Vec<SpanData>) -> OTelSdkResult {
        self.0.lock().unwrap().extend(batch);
        Ok(())
    }
}

#[cfg(feature = "local_fs")]
#[test]
fn handoff_connects_checkout_roots_to_the_spawning_span_without_baggage() {
    let directory = tempfile::tempdir().unwrap();
    let args = EnvironmentCheckoutArgs {
        requests_file: directory.path().join("requests.json"),
        report_file: directory.path().join("report.json"),
        remove_origins_only: true,
        fail_if_target_exists: false,
    };
    let provider = SdkTracerProvider::builder().build();
    let subscriber = tracing_subscriber::registry()
        .with(tracing_opentelemetry::layer().with_tracer(provider.tracer("parent-test")));
    let incoming = Context::new()
        .with_remote_span_context(opentelemetry::trace::SpanContext::new(
            opentelemetry::trace::TraceId::from(1),
            opentelemetry::trace::SpanId::from(2),
            opentelemetry::trace::TraceFlags::SAMPLED,
            true,
            TraceState::from_key_value([("vendor", "value")]).unwrap(),
        ))
        .with_baggage([KeyValue::new("secret", "not-for-child")]);
    let (handoff, spawning_context) = tracing::subscriber::with_default(subscriber, || {
        let span = tracing::info_span!("spawning");
        span.set_parent(incoming).unwrap();
        let _entered = span.enter();
        let context = span.context().span().span_context().clone();
        let configured = config();
        let config = ChildProcessTracingConfig::capture(configured.endpoint, configured.credential);
        assert_eq!(config.trace_context.len(), 2);
        assert!(!config.trace_context.contains_key("baggage"));
        (config.write(&args.requests_file).unwrap(), context)
    });
    let received = ChildProcessTracingConfig::consume(&args.requests_file)
        .unwrap()
        .unwrap();
    assert!(!handoff.exists());
    let parent_context = received.parent_context();
    assert!(parent_context.baggage().is_empty());
    assert!(parent_context.span().span_context().is_remote());
    let exporter = RecordingExporter::default();
    let provider = SdkTracerProvider::builder()
        .with_batch_exporter(exporter.clone())
        .build();
    let subscriber = tracing_subscriber::registry()
        .with(tracing_opentelemetry::layer().with_tracer(provider.tracer("child-test")));
    let initialization = crate::tracing::Initialization {
        initialization_warning: None,
        active_spans: None,
        provider: Some(provider),
        shutdown_timeout: crate::tracing::DEFAULT_EXPORT_TIMEOUT,
    };
    tracing::subscriber::with_default(subscriber, || {
        let _parent = parent_context.attach();
        fs::write(
            &args.requests_file,
            serde_json::to_vec(&serde_json::json!({
                "working_dir": directory.path(),
                "repositories": [],
            }))
            .unwrap(),
        )
        .unwrap();
        crate::ai::agent_sdk::run_environment_checkout(&args).unwrap();
        fs::write(
            &args.requests_file,
            br#"{"working_dir":"https://user:private-value@example.com","repositories":[{"source":{"code_forge":"https://user:private-value@example.com"}}]}"#,
        ).unwrap();
        assert!(crate::ai::agent_sdk::run_environment_checkout(&args).is_err());
    });
    drop(initialization);
    let spans = exporter.0.lock().unwrap();
    let roots = spans
        .iter()
        .filter(|span| span.name == "environment_checkout")
        .collect::<Vec<_>>();
    assert_eq!(roots.len(), 2);
    for span in &roots {
        assert_eq!(span.span_context.trace_id(), spawning_context.trace_id());
        assert_eq!(span.parent_span_id, spawning_context.span_id());
        assert_eq!(
            span.span_context.trace_state(),
            spawning_context.trace_state()
        );
        assert!(span.end_time > span.start_time);
    }
    assert!(
        roots[0]
            .attributes
            .contains(&KeyValue::new("result_ok", true))
    );
    assert!(
        roots[1]
            .attributes
            .contains(&KeyValue::new("result_ok", false))
    );
    let err = roots[1]
        .attributes
        .iter()
        .find(|attribute| attribute.key.as_str() == "err")
        .unwrap();
    assert!(err.value.to_string().contains("unknown variant"));
    assert!(!err.value.to_string().contains("private-value"));
}

#[cfg(feature = "local_fs")]
#[test]
fn authenticated_child_export_flushes_checkout_returns() {
    let directory = tempfile::tempdir().unwrap();
    let args = EnvironmentCheckoutArgs {
        requests_file: directory.path().join("requests.json"),
        report_file: directory.path().join("report.json"),
        remove_origins_only: true,
        fail_if_target_exists: false,
    };
    let mut collector = mockito::Server::new();
    let bodies = Arc::new(Mutex::new(Vec::new()));
    let captured = bodies.clone();
    let export = collector
        .mock("POST", "/v1/traces")
        .match_header("authorization", "Bearer handoff-test-token")
        .with_body_from_request(move |request| {
            captured
                .lock()
                .unwrap()
                .push(request.utf8_lossy_body().unwrap().into_owned());
            Vec::new()
        })
        .expect_at_least(1)
        .create();
    let mut configured = config();
    configured.endpoint = collector.url();
    let handoff = configured.write(&args.requests_file).unwrap();
    let (initialization, parent_context) =
        crate::tracing::init_child_process(&args.requests_file).unwrap();
    let _parent = parent_context.attach();
    assert!(!handoff.exists());
    fs::write(
        &args.requests_file,
        serde_json::to_vec(&serde_json::json!({
            "working_dir": directory.path(),
            "repositories": [],
        }))
        .unwrap(),
    )
    .unwrap();
    crate::ai::agent_sdk::run_environment_checkout(&args).unwrap();
    fs::write(&args.requests_file, b"{").unwrap();
    assert!(crate::ai::agent_sdk::run_environment_checkout(&args).is_err());
    drop(initialization);
    export.assert();
    let body = bodies.lock().unwrap().join("");
    // Source-location attributes also contain this name, so only count OTLP Span.name (field 5).
    assert_eq!(body.matches("\x2a\x14environment_checkout").count(), 2);
    assert!(body.contains("could not parse environment checkout requests"));
    assert!(!body.contains("handoff-test-token"));
}
