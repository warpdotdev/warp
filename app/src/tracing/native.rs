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

const CLOUD_AGENT_MARKER: &str = "tags.cloud_agent";
const CLOUD_AGENT_OTLP_ENDPOINT: &str = "WARP_CLOUD_AGENT_OTLP_ENDPOINT";
const OTEL_SERVICE_NAME: &str = "OTEL_SERVICE_NAME";

pub fn init() -> anyhow::Result<Initialization> {
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

#[derive(Clone, Debug, Default)]
pub(super) struct ActiveSpanRegistry {
    state: Arc<Mutex<ActiveSpanRegistryState>>,
}

#[derive(Debug, Default)]
struct ActiveSpanRegistryState {
    shutting_down: bool,
    spans: Vec<Weak<Mutex<SdkSpan>>>,
}

impl ActiveSpanRegistry {
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
            state.spans.retain(|span| span.strong_count() > 0);
            state.spans.push(Arc::downgrade(&span));
        }
        ShutdownAwareSpan {
            span_context,
            inner: span,
        }
    }

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
