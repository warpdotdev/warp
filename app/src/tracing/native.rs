use std::time::Duration;

use anyhow::{anyhow, Context as _};
use opentelemetry::trace::TracerProvider as _;
use opentelemetry::{KeyValue, Value};
use opentelemetry_otlp::{Protocol, WithExportConfig as _};
use opentelemetry_sdk::error::OTelSdkResult;
use opentelemetry_sdk::resource::{EnvResourceDetector, TelemetryResourceDetector};
use opentelemetry_sdk::trace::{SdkTracerProvider, SpanData, SpanExporter};
use opentelemetry_sdk::Resource;
use tracing::subscriber;
use tracing_subscriber::layer::SubscriberExt as _;
use url::Url;

use super::Initialization;
use crate::channel::ChannelState;

const CLOUD_AGENT_MARKER: &str = "tags.cloud_agent";
const CLOUD_AGENT_OTLP_ENDPOINT: &str = "WARP_CLOUD_AGENT_OTLP_ENDPOINT";
const DEFAULT_EXPORT_TIMEOUT: Duration = Duration::from_secs(10);
const OTEL_SERVICE_NAME: &str = "OTEL_SERVICE_NAME";

pub fn init() -> anyhow::Result<Initialization> {
    let Some(base_endpoint) = std::env::var(CLOUD_AGENT_OTLP_ENDPOINT)
        .ok()
        .filter(|endpoint| !endpoint.trim().is_empty())
    else {
        install_no_subscriber()?;
        return Ok(Initialization {
            initialization_warning: None,
            provider: None,
            shutdown_timeout: DEFAULT_EXPORT_TIMEOUT,
        });
    };

    let shutdown_timeout = export_timeout();
    let provider = match build_provider(base_endpoint.trim()) {
        Ok(provider) => provider,
        Err(err) => {
            install_no_subscriber()?;
            return Ok(Initialization {
                initialization_warning: Some(err),
                provider: None,
                shutdown_timeout,
            });
        }
    };

    let tracer = provider.tracer("warp-cloud-agent");
    let subscriber =
        tracing_subscriber::registry().with(tracing_opentelemetry::layer().with_tracer(tracer));
    subscriber::set_global_default(subscriber)?;

    Ok(Initialization {
        initialization_warning: None,
        provider: Some(provider),
        shutdown_timeout,
    })
}

fn install_no_subscriber() -> anyhow::Result<()> {
    // Configure the global tracing subscriber to not care about any spans or
    // events.
    //
    // This is done so that we prevent the `tracing` crate from writing out log
    // lines for spans and trace events.
    subscriber::set_global_default(subscriber::NoSubscriber::new())?;
    Ok(())
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
    .unwrap_or(DEFAULT_EXPORT_TIMEOUT)
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
