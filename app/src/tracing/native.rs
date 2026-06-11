//! Configures opt-in OpenTelemetry export for cloud-agent traces on native platforms.
//!
//! The global `tracing` subscriber observes the whole application, while
//! [`CloudAgentSpanExporter`] limits OTLP export to spans explicitly marked with
//! [`CLOUD_AGENT_MARKER`]. Keeping this selection at the exporter boundary lets callers use the
//! normal `tracing` macros and propagation machinery without installing a second subscriber or
//! coupling generic task executors to cloud-agent tracing.
//!
//! # Why spans must be ended during shutdown
//!
//! An OpenTelemetry span becomes exportable and passes owned span data to a processor's `on_end`
//! callback only after it ends. Shutting down an [`SdkTracerProvider`] flushes spans that have
//! reached `on_end`, but it does not end spans that are still active. Some `tracing::Span`
//! references are intentionally propagated into asynchronous task machinery and can therefore
//! remain alive when the application terminates. Shutting down the provider before those spans end
//! would silently discard them.
//!
//! [`ShutdownAwareTracer`] and [`ShutdownAwareSpan`] wrap the SDK tracer and spans used by
//! `tracing-opentelemetry`. This keeps existing `tracing` instrumentation unchanged while allowing
//! [`ActiveSpanRegistry`] to explicitly end still-reachable, registered SDK spans before shutting
//! down the provider. The standard application lifecycle retains [`Initialization`] in its
//! termination callback so this ordering happens before platforms that terminate the process
//! without running Rust destructors. [`Initialization`]'s `Drop` implementation remains a fallback
//! for ordinary returns. Explicit process exits bypass both forms of cleanup.
//!
//! # Span ownership and synchronization
//!
//! `tracing-opentelemetry` creates SDK spans lazily during several `tracing` span lifecycle
//! operations, including when it needs a span's context and when a span closes. Every SDK span that
//! reaches [`ShutdownAwareTracer::build_with_context`] is wrapped in an `Arc<Mutex<_>>`. Before
//! shutdown begins, it is weakly registered; after shutdown begins, it is immediately ended
//! instead. The wrapper remains the span's owner; the registry uses weak references so tracking
//! does not extend normal span lifetimes. An SDK span that has not yet been built when shutdown
//! begins cannot reach `on_end` before provider shutdown. If it materializes later, it is ended too
//! late for export.
//!
//! SDK-span creation and shutdown are serialized by the registry-state mutex. Shutdown keeps that
//! mutex locked while it ends every still-upgradeable registered span and shuts down the provider,
//! preventing an SDK span from being created in the otherwise-dangerous gap between those
//! operations. A final span owner can begin dropping after its weak reference becomes impossible
//! to upgrade, so shutdown cannot strictly guarantee that every previously registered span has
//! finished ending. The lock order is always registry state followed by an individual SDK span.
//! Normal span operations lock only the individual SDK span and never attempt to lock the registry.
//! Mutex acquisition recovers poisoned inner values because trace export and shutdown are
//! best-effort cleanup that should continue after an unrelated panic.

use std::borrow::Cow;
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, SystemTime};

use anyhow::{anyhow, Context as _};
use opentelemetry::trace::{
    Span as _, SpanBuilder, SpanContext, Status, Tracer as _, TracerProvider as _,
};
use opentelemetry::{Context as OtelContext, KeyValue, Value};
use opentelemetry_otlp::{Protocol, WithExportConfig as _};
use opentelemetry_sdk::error::OTelSdkResult;
use opentelemetry_sdk::resource::{EnvResourceDetector, TelemetryResourceDetector};
use opentelemetry_sdk::trace::{
    SdkTracer, SdkTracerProvider, Span as SdkSpan, SpanData, SpanExporter,
};
use opentelemetry_sdk::Resource;
use tracing::subscriber;
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::EnvFilter;
use url::Url;

use super::Initialization;
use crate::channel::ChannelState;
use crate::tracing::install_no_subscriber;

/// The tag used to mark spans related to cloud agents, which we use to filter out
/// spans we don't care about (e.g.: ones from dependencies).
const CLOUD_AGENT_MARKER: &str = "tags.cloud_agent";
/// The environment variable used to configure the cloud agent OTLP endpoint.
const CLOUD_AGENT_OTLP_ENDPOINT: &str = "WARP_CLOUD_AGENT_OTLP_ENDPOINT";
/// The environment variable used to configure the OTel service name.
const OTEL_SERVICE_NAME: &str = "OTEL_SERVICE_NAME";

/// Installs the native tracing subscriber and optional cloud-agent OTLP exporter.
///
/// Export is deliberately opt-in through [`CLOUD_AGENT_OTLP_ENDPOINT`]. When the endpoint is
/// absent or the exporter cannot be constructed, a no-op subscriber is installed so tracing
/// instrumentation remains safe without producing output or partially initializing export.
pub fn init() -> anyhow::Result<Initialization> {
    // INFO is the default because this is a global subscriber and DEBUG-level application spans
    // would otherwise create substantial work even though only marked cloud-agent spans are
    // exported. RUST_LOG can still override this when deeper tracing is needed.
    let env_filter = EnvFilter::builder()
        .with_default_directive(tracing::Level::INFO.into())
        .from_env_lossy();

    let Some(base_endpoint) = std::env::var(CLOUD_AGENT_OTLP_ENDPOINT)
        .ok()
        .filter(|endpoint| !endpoint.trim().is_empty())
    else {
        install_no_subscriber()?;
        return Ok(Initialization::default());
    };

    let shutdown_timeout = export_timeout();
    let provider = match build_provider(base_endpoint.trim()) {
        Ok(provider) => provider,
        Err(err) => {
            install_no_subscriber()?;
            return Ok(Initialization {
                initialization_warning: Some(err),
                active_spans: None,
                provider: None,
                shutdown_timeout,
            });
        }
    };

    let active_spans = ActiveSpanRegistry::default();
    let tracer =
        ShutdownAwareTracer::new(provider.tracer("warp-cloud-agent"), active_spans.clone());
    let subscriber = tracing_subscriber::registry()
        .with(env_filter)
        .with(tracing_opentelemetry::layer().with_tracer(tracer));
    subscriber::set_global_default(subscriber)?;

    Ok(Initialization {
        initialization_warning: None,
        active_spans: Some(active_spans),
        provider: Some(provider),
        shutdown_timeout,
    })
}

/// Builds the SDK provider and its batch exporter.
///
/// A batch exporter keeps network export off instrumentation call sites. The provider is retained
/// by [`Initialization`] so application termination can explicitly shut it down after attempting
/// to end registered active spans. Exported resources include Warp's version and channel alongside
/// standard environment-detected OpenTelemetry attributes, with [`OTEL_SERVICE_NAME`] taking
/// precedence over the default service name.
fn build_provider(base_endpoint: &str) -> anyhow::Result<SdkTracerProvider> {
    let endpoint = traces_endpoint(base_endpoint)?;
    let exporter = opentelemetry_otlp::SpanExporter::builder()
        .with_http()
        .with_protocol(Protocol::HttpBinary)
        .with_endpoint(endpoint)
        .build()
        .context("Failed to build the OTLP span exporter")?;

    let resource = Resource::builder_empty()
        .with_service_name("warp-cloud-agent")
        .with_attribute(KeyValue::new(
            "service.version",
            ChannelState::app_version().unwrap_or("<no tag>"),
        ))
        .with_attribute(KeyValue::new(
            "warp.channel",
            ChannelState::channel().to_string(),
        ))
        .with_detector(Box::new(TelemetryResourceDetector))
        .with_detector(Box::new(EnvResourceDetector::new()));
    let resource = match std::env::var(OTEL_SERVICE_NAME) {
        Ok(service_name) if !service_name.is_empty() => resource.with_service_name(service_name),
        Ok(_) | Err(_) => resource,
    }
    .build();

    Ok(SdkTracerProvider::builder()
        .with_batch_exporter(CloudAgentSpanExporter { inner: exporter })
        .with_resource(resource)
        .build())
}

/// Converts the configured OTLP base URL into the HTTP/protobuf traces endpoint.
///
/// The configuration is treated as a base URL rather than a complete signal-specific URL, so any
/// query or fragment is discarded before appending `v1/traces`.
fn traces_endpoint(base_endpoint: &str) -> anyhow::Result<String> {
    let mut endpoint = Url::parse(base_endpoint).context("Invalid cloud-agent OTLP endpoint")?;
    if !matches!(endpoint.scheme(), "http" | "https") {
        return Err(anyhow!("Cloud-agent OTLP endpoint must use HTTP or HTTPS"));
    }

    endpoint.set_query(None);
    endpoint.set_fragment(None);
    endpoint
        .path_segments_mut()
        .map_err(|_| anyhow!("Cloud-agent OTLP endpoint cannot be used as a base URL"))?
        .pop_if_empty()
        .extend(["v1", "traces"]);
    Ok(endpoint.into())
}

/// Returns the export shutdown timeout using the standard OpenTelemetry environment variables.
fn export_timeout() -> Duration {
    [
        "OTEL_EXPORTER_OTLP_TRACES_TIMEOUT",
        "OTEL_EXPORTER_OTLP_TIMEOUT",
    ]
    .into_iter()
    .find_map(|name| {
        std::env::var(name)
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .map(Duration::from_millis)
    })
    .unwrap_or(super::DEFAULT_EXPORT_TIMEOUT)
}

/// A registry of started SDK spans used for best-effort ending before provider shutdown.
///
/// This registry belongs beside the provider in [`Initialization`]. It stores only weak references
/// so a span that ends normally can be dropped without first unregistering itself.
#[derive(Clone, Debug, Default)]
pub(super) struct ActiveSpanRegistry {
    state: Arc<Mutex<ActiveSpanRegistryState>>,
}

#[derive(Debug, Default)]
struct ActiveSpanRegistryState {
    /// Prevents new spans from remaining active after shutdown begins.
    shutting_down: bool,
    /// Weak references avoid extending the lifetime of spans that end normally.
    spans: Vec<Weak<Mutex<SdkSpan>>>,
}

impl ActiveSpanRegistry {
    /// Builds an SDK span, registering it before shutdown or ending it after shutdown begins.
    ///
    /// `tracing-opentelemetry` calls this whenever it materializes an SDK span. If shutdown has
    /// already begun, the newly built span is ended immediately rather than being allowed to
    /// remain active.
    fn build_span(
        &self,
        tracer: &SdkTracer,
        builder: SpanBuilder,
        parent_cx: &OtelContext,
    ) -> ShutdownAwareSpan {
        let mut state = self.state.lock().unwrap_or_else(|err| err.into_inner());
        let span = tracer.build_with_context(builder, parent_cx);
        let span_context = span.span_context().clone();
        let span = Arc::new(Mutex::new(span));
        if state.shutting_down {
            span.lock().unwrap_or_else(|err| err.into_inner()).end();
        } else {
            // Dead weak references are pruned opportunistically to avoid requiring normal span
            // completion to acquire the registry lock.
            state.spans.retain(|span| span.strong_count() > 0);
            state.spans.push(Arc::downgrade(&span));
        }
        ShutdownAwareSpan {
            span_context,
            inner: span,
        }
    }

    /// Ends every still-upgradeable registered span and then shuts down the provider.
    ///
    /// The registry lock intentionally remains held through provider shutdown. This guarantees
    /// that no SDK span can be built between the final end attempt and the provider becoming unable
    /// to accept ended spans. It does not synchronize with the final drop of a span whose weak
    /// reference can no longer be upgraded, so ending previously built spans remains best-effort.
    pub(super) fn shutdown(
        &self,
        provider: &SdkTracerProvider,
        timeout: Duration,
    ) -> OTelSdkResult {
        let mut state = self.state.lock().unwrap_or_else(|err| err.into_inner());
        state.shutting_down = true;
        let spans = std::mem::take(&mut state.spans);

        for span in spans {
            if let Some(span) = span.upgrade() {
                span.lock().unwrap_or_else(|err| err.into_inner()).end();
            }
        }
        let result = provider.shutdown_with_timeout(timeout);
        drop(state);
        result
    }
}

/// An [`SdkTracer`] adapter that routes spans through shutdown-aware construction.
///
/// Wrapping the tracer, rather than the span processor, is necessary because processors receive
/// only a temporary mutable reference in `on_start` and receive owned exportable data only after
/// `on_end`. A processor therefore cannot retain handles to, or end, active spans during shutdown.
#[derive(Clone, Debug)]
struct ShutdownAwareTracer {
    inner: SdkTracer,
    active_spans: ActiveSpanRegistry,
}

impl ShutdownAwareTracer {
    fn new(inner: SdkTracer, active_spans: ActiveSpanRegistry) -> Self {
        Self {
            inner,
            active_spans,
        }
    }
}

impl opentelemetry::trace::Tracer for ShutdownAwareTracer {
    type Span = ShutdownAwareSpan;

    fn build_with_context(&self, builder: SpanBuilder, parent_cx: &OtelContext) -> Self::Span {
        self.active_spans
            .build_span(&self.inner, builder, parent_cx)
    }
}

/// A synchronized wrapper around an SDK span shared with [`ActiveSpanRegistry`].
///
/// The immutable [`SpanContext`] is cached outside the mutex because the OpenTelemetry
/// [`opentelemetry::trace::Span`] trait must return it by reference. All mutable SDK-span operations
/// are forwarded through the mutex, allowing shutdown to end the same underlying span. Repeated
/// end calls are harmless because SDK spans export only once.
#[derive(Debug)]
struct ShutdownAwareSpan {
    span_context: SpanContext,
    inner: Arc<Mutex<SdkSpan>>,
}

impl opentelemetry::trace::Span for ShutdownAwareSpan {
    fn add_event_with_timestamp<T>(
        &mut self,
        name: T,
        timestamp: SystemTime,
        attributes: Vec<KeyValue>,
    ) where
        T: Into<Cow<'static, str>>,
    {
        self.inner
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .add_event_with_timestamp(name, timestamp, attributes);
    }

    fn span_context(&self) -> &SpanContext {
        &self.span_context
    }

    fn is_recording(&self) -> bool {
        self.inner
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .is_recording()
    }

    fn set_attribute(&mut self, attribute: KeyValue) {
        self.inner
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .set_attribute(attribute);
    }

    fn set_status(&mut self, status: Status) {
        self.inner
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .set_status(status);
    }

    fn update_name<T>(&mut self, new_name: T)
    where
        T: Into<Cow<'static, str>>,
    {
        self.inner
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .update_name(new_name);
    }

    fn add_link(&mut self, span_context: SpanContext, attributes: Vec<KeyValue>) {
        self.inner
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .add_link(span_context, attributes);
    }

    fn end_with_timestamp(&mut self, timestamp: SystemTime) {
        self.inner
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .end_with_timestamp(timestamp);
    }
}

/// An exporter that restricts the shared tracing subscriber's output to explicitly marked
/// cloud-agent spans.
///
/// Filtering here preserves normal parent/context propagation inside the application while
/// ensuring unrelated application tracing is never sent to the configured cloud-agent endpoint.
/// The marker is a per-span routing attribute rather than an inherited property, so every span
/// intended for export must set it explicitly.
#[derive(Debug)]
struct CloudAgentSpanExporter {
    inner: opentelemetry_otlp::SpanExporter,
}

impl SpanExporter for CloudAgentSpanExporter {
    fn export(
        &self,
        batch: Vec<SpanData>,
    ) -> impl std::future::Future<Output = OTelSdkResult> + Send {
        let batch: Vec<_> = batch
            .into_iter()
            .filter_map(filter_cloud_agent_span)
            .collect();

        async move {
            if batch.is_empty() {
                return Ok(());
            }

            let result = self.inner.export(batch).await;
            if let Err(err) = &result {
                log::warn!("Failed to export cloud-agent OpenTelemetry spans: {err}");
            }
            result
        }
    }

    fn shutdown_with_timeout(&self, timeout: Duration) -> OTelSdkResult {
        let result = self.inner.shutdown_with_timeout(timeout);
        if let Err(err) = &result {
            log::warn!("Failed to shut down the cloud-agent OpenTelemetry span exporter: {err}");
        }
        result
    }

    fn force_flush(&self) -> OTelSdkResult {
        let result = self.inner.force_flush();
        if let Err(err) = &result {
            log::warn!("Failed to flush the cloud-agent OpenTelemetry span exporter: {err}");
        }
        result
    }

    fn set_resource(&mut self, resource: &Resource) {
        self.inner.set_resource(resource);
    }
}

/// Removes unrelated spans and strips the internal routing marker from spans and events before
/// export.
fn filter_cloud_agent_span(mut span: SpanData) -> Option<SpanData> {
    let is_cloud_agent_span = span.attributes.iter().any(|attribute| {
        attribute.key.as_str() == CLOUD_AGENT_MARKER && attribute.value == Value::Bool(true)
    });
    if !is_cloud_agent_span {
        return None;
    }

    span.attributes
        .retain(|attribute| attribute.key.as_str() != CLOUD_AGENT_MARKER);
    for event in &mut span.events.events {
        event
            .attributes
            .retain(|attribute| attribute.key.as_str() != CLOUD_AGENT_MARKER);
    }
    Some(span)
}
