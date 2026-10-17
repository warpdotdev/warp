use opentelemetry::trace::{TraceContextExt as _, TracerProvider as _};
use opentelemetry_sdk::trace::SdkTracerProvider;
use tracing_opentelemetry::OpenTelemetrySpanExt as _;
use tracing_subscriber::layer::SubscriberExt as _;

use super::*;
use std::sync::atomic::{AtomicBool, Ordering};

struct OptInProvider(AtomicBool);

impl IapTokenProvider for OptInProvider {
    fn cached_token(&self) -> Option<String> {
        Some("test-iap-token".into())
    }

    fn routing_opt_in_enabled(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }
}

#[test]
fn routing_opt_in_updates_requests_without_replacing_application_auth() {
    let provider = Arc::new(OptInProvider(AtomicBool::new(false)));
    let client = Client::new_with_iap_token_provider(provider.clone());
    let url = "https://staging.warp.dev/graphql/v2";

    let request = client.post(url).bearer_auth("app-token").build().unwrap();
    assert!(!request.wrapped.headers().contains_key(iap::IAP_OPT_IN_HEADER));
    provider.0.store(true, Ordering::Relaxed);
    let mut request = client.post(url).bearer_auth("app-token").build().unwrap();
    assert_eq!(request.wrapped.headers()[iap::IAP_OPT_IN_HEADER], "1");
    assert_eq!(request.wrapped.headers()[AUTHORIZATION], "Bearer app-token");

    provider.0.store(false, Ordering::Relaxed);
    client.add_iap_routing_header(&mut request.wrapped);
    assert!(!request.wrapped.headers().contains_key(iap::IAP_OPT_IN_HEADER));
    assert_eq!(request.wrapped.headers()[AUTHORIZATION], "Bearer app-token");
}

#[test]
fn configured_provider_cannot_send_routing_selector_to_excluded_destinations() {
    let client = Client::new_with_iap_token_provider(Arc::new(OptInProvider(AtomicBool::new(true))));
    for url in [
        "https://app.warp.dev/graphql/v2",
        "https://third-party.example/graphql/v2",
        "https://sessions.staging.warp.dev/api/v1/session",
        "https://rtc.staging.warp.dev/graphql/v2",
        "https://admin.staging.warp.dev/",
        "https://staging.warp.dev/shared_sessions/test",
    ] {
        let request = client.get(url).header(iap::IAP_OPT_IN_HEADER, "1").build().unwrap();
        assert!(!request.wrapped.headers().contains_key(iap::IAP_OPT_IN_HEADER), "{url}");
    }
}
#[test]
fn only_selector_bearing_transport_stops_redirects_outside_application_routes() {
    use std::io::{Read, Write};
    use std::net::TcpListener;

    let server = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/application", server.local_addr().unwrap());
    let responder = std::thread::spawn(move || {
        let redirect = "HTTP/1.1 302 Found\r\nLocation: /public\r\nX-Goog-IAP-Generated-Response: true\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
        let success = "HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
        for response in [redirect, success, redirect] {
            let (mut connection, _) = server.accept().unwrap();
            connection.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
            let mut buffer = [0; 4096];
            assert!(connection.read(&mut buffer).unwrap() > 0);
            connection.write_all(response.as_bytes()).unwrap();
        }
    });

    let client = Client::new_with_iap_token_provider(Arc::new(TestIapTokenProvider));
    futures::executor::block_on(Compat::new(async {
        let response = client.get(&url).timeout(Duration::from_secs(5)).send().await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let request = client.wrapped.get(&url)
            .header(iap::IAP_OPT_IN_HEADER, "1")
            .timeout(Duration::from_secs(5))
            .build().unwrap();
        let response = client.transport_for(&request).execute(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::FOUND);
        assert!(iap::is_iap_challenge(response.status(), response.headers()));
    }));
    responder.join().unwrap();
}

struct TestIapTokenProvider;

impl IapTokenProvider for TestIapTokenProvider {
    fn cached_token(&self) -> Option<String> {
        Some("test-iap-token".into())
    }
}

#[test]
fn external_client_omits_automatic_headers_and_preserves_signing_headers() {
    with_active_span(|| {
        let client = Client::new_with_iap_token_provider(Arc::new(TestIapTokenProvider));
        let url = format!(
            "{}/object?X-Amz-Signature=test%2Bsignature&x-id=PutObject",
            ChannelState::server_root_url()
        );
        let api_request = client.get(&url).build().unwrap().wrapped;
        assert!(
            api_request
                .headers()
                .contains_key(headers::WARP_OS_CATEGORY)
        );
        assert!(
            api_request
                .headers()
                .contains_key(headers::TRACE_LINK_HEADER)
        );
        assert!(
            api_request
                .headers()
                .contains_key(iap::IAP_PROXY_AUTH_HEADER)
        );

        let external_client = Client::new_external();
        let request = external_client
            .put(&url)
            .header("x-amz-checksum-crc32c", "EjRWeA==")
            .header(AUTHORIZATION, "AWS4-HMAC-SHA256 test-signature")
            .build()
            .unwrap()
            .wrapped;
        assert_eq!(request.url().as_str(), url);
        assert_eq!(request.headers().len(), 2);
        assert_eq!(request.headers()["x-amz-checksum-crc32c"], "EjRWeA==");
        assert_eq!(
            request.headers()[AUTHORIZATION],
            "AWS4-HMAC-SHA256 test-signature"
        );
    });
}

/// Runs `f` with a real OpenTelemetry subscriber installed and a span entered,
/// so `Span::current()` resolves to a valid OTEL span context.
fn with_active_span<R>(f: impl FnOnce() -> R) -> R {
    let provider = SdkTracerProvider::builder().build();
    let tracer = provider.tracer("http_client-test");
    let subscriber =
        tracing_subscriber::registry().with(tracing_opentelemetry::layer().with_tracer(tracer));
    tracing::subscriber::with_default(subscriber, || {
        let span = tracing::info_span!("test-request");
        let _enter = span.enter();
        f()
    })
}

#[test]
fn injects_trace_link_header_when_span_active() {
    let (header, span_context) = with_active_span(|| {
        let header = current_trace_link_header();
        let span_context = tracing::Span::current()
            .context()
            .span()
            .span_context()
            .clone();
        (header, span_context)
    });

    assert!(span_context.is_valid());
    let header = header.expect("header should be present when a valid span is active");

    // W3C traceparent wire format: 00-<32 hex trace-id>-<16 hex span-id>-<2 hex flags>.
    let parts: Vec<&str> = header.split('-').collect();
    assert_eq!(parts.len(), 4, "unexpected header shape: {header}");
    assert_eq!(parts[0], "00");
    assert_eq!(parts[1], span_context.trace_id().to_string());
    assert_eq!(parts[2], span_context.span_id().to_string());
    assert_eq!(parts[1].len(), 32);
    assert_eq!(parts[2].len(), 16);
    assert_eq!(parts[3].len(), 2);
}

#[test]
fn omits_trace_link_header_when_no_span() {
    // No OTEL subscriber installed on this thread => no valid span context.
    let header = tracing::subscriber::with_default(
        tracing::subscriber::NoSubscriber::new(),
        current_trace_link_header,
    );
    assert!(header.is_none());
}

#[test]
fn request_carries_trace_link_header_on_warp_header_path() {
    // The header rides the same `include_warp_http_headers` gate as every other
    // X-Warp-* header (added only inside `add_warp_http_headers`), so building a
    // request through the client while a span is active carries it.
    let value = with_active_span(|| {
        let client = Client::new();
        let request = client
            .get("http://example.com/")
            .build()
            .expect("request should build");
        request
            .wrapped
            .headers()
            .get(headers::TRACE_LINK_HEADER)
            .map(|value| value.to_str().unwrap().to_string())
    });

    let value = value.expect("trace-link header should be added on the warp-header path");
    assert!(value.starts_with("00-"), "unexpected header value: {value}");
}
