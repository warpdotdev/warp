use std::io::{Read, Write};
use std::net::TcpListener;
use std::thread;
use std::time::Duration;

use futures::executor::block_on;
use instant::Instant;
use opentelemetry::trace::{TraceContextExt as _, TracerProvider as _};
use opentelemetry_sdk::trace::SdkTracerProvider;
use tracing_opentelemetry::OpenTelemetrySpanExt as _;
use tracing_subscriber::layer::SubscriberExt as _;

use super::*;

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

/// Spawns a background thread that accepts a single HTTP/1.1 connection, drains the request,
/// writes `head`, then writes each of `chunks` (already framed as needed) with `delay` between
/// them. Used to control exactly what a client sees on the wire, including bodies that are
/// larger than declared, or that dribble in slowly, without pulling in an HTTP mocking crate.
fn spawn_test_server(head: impl Into<Vec<u8>>, chunks: Vec<Vec<u8>>, delay: Duration) -> String {
    let head = head.into();
    let listener = TcpListener::bind("127.0.0.1:0").expect("failed to bind test server");
    let addr = listener
        .local_addr()
        .expect("failed to read test server address");
    thread::spawn(move || {
        let Ok((mut stream, _)) = listener.accept() else {
            return;
        };
        // Drain the request headers so the client isn't left waiting on us to read them.
        let mut buf = [0u8; 4096];
        loop {
            match stream.read(&mut buf) {
                Ok(0) => return,
                Ok(n) if buf[..n].windows(4).any(|window| window == b"\r\n\r\n") => break,
                Ok(_) => continue,
                Err(_) => return,
            }
        }
        if stream.write_all(&head).is_err() {
            return;
        }
        for chunk in chunks {
            thread::sleep(delay);
            if stream.write_all(&chunk).is_err() {
                return;
            }
        }
    });
    format!("http://{addr}")
}

/// Frames `body` as a single HTTP chunked-transfer-encoding chunk.
fn chunked_frame(body: &[u8]) -> Vec<u8> {
    let mut framed = format!("{:x}\r\n", body.len()).into_bytes();
    framed.extend_from_slice(body);
    framed.extend_from_slice(b"\r\n");
    framed
}

#[test]
fn bytes_limited_reads_body_under_limit_intact() {
    let body = b"{\"access_token\":\"abc\"}";
    let head = b"HTTP/1.1 200 OK\r\nContent-Length: 22\r\n\r\n".to_vec();
    assert_eq!(body.len(), 22, "fixture Content-Length must match the body");
    let base = spawn_test_server(head, vec![body.to_vec()], Duration::ZERO);

    let client = Client::new_for_test();
    let bytes = block_on(async {
        let response = client
            .get(&base)
            .send()
            .await
            .expect("request should succeed");
        response.bytes_limited(1024).await
    })
    .expect("body under the limit should read through intact");

    assert_eq!(&bytes[..], body);
}

#[test]
fn bytes_limited_accepts_body_exactly_at_limit() {
    // Exercises the exact boundary on both checks in `bytes_limited`: the declared
    // `Content-Length` and the streamed byte count are both precisely equal to `limit`. If
    // either `>` comparison there were accidentally written as `>=`, this exact-boundary body
    // would be wrongly rejected.
    const LIMIT: usize = 4096;
    let body = vec![b'z'; LIMIT];
    let head = format!("HTTP/1.1 200 OK\r\nContent-Length: {LIMIT}\r\n\r\n").into_bytes();
    let base = spawn_test_server(head, vec![body.clone()], Duration::ZERO);

    let client = Client::new_for_test();
    let bytes = block_on(async {
        let response = client
            .get(&base)
            .send()
            .await
            .expect("request should succeed");
        response.bytes_limited(LIMIT).await
    })
    .expect("a body exactly at the limit should be accepted, not rejected");

    assert_eq!(bytes.len(), LIMIT);
    assert_eq!(&bytes[..], &body[..]);
}

#[test]
fn bytes_limited_rejects_oversized_content_length_without_reading_body() {
    // The server advertises a huge body via `Content-Length` but the connection closes without
    // ever sending it. This isolates the early `Content-Length` check: without it, the closed
    // connection would look like an (empty) successful response -- `bytes_limited` would
    // return `Ok` with zero bytes instead of `Err(TooLarge)`, since the streaming loop would
    // simply see the stream end after zero chunks.
    let head = b"HTTP/1.1 200 OK\r\nContent-Length: 999999999\r\n\r\n".to_vec();
    let base = spawn_test_server(head, Vec::new(), Duration::ZERO);

    let client = Client::new_for_test();
    let result = block_on(async {
        let response = client
            .get(&base)
            .send()
            .await
            .expect("request should succeed");
        response.bytes_limited(1024).await
    });

    assert!(matches!(
        result,
        Err(BodyReadError::TooLarge { limit: 1024 })
    ));
}

#[test]
fn bytes_limited_aborts_mid_stream_once_limit_exceeded() {
    // No `Content-Length` here, so the only way to detect the oversized body is by counting
    // bytes as they stream in. The server pauses between chunks; if `bytes_limited` buffered
    // the whole body before checking its size (instead of aborting while streaming), this test
    // would take several seconds instead of a few hundred milliseconds.
    const CHUNK: &[u8] = &[b'a'; 4096];
    const LIMIT: usize = CHUNK.len() + 1;
    let head = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n".to_vec();
    let chunks = (0..50).map(|_| chunked_frame(CHUNK)).collect();
    let base = spawn_test_server(head, chunks, Duration::from_millis(200));

    let client = Client::new_for_test();
    let started = Instant::now();
    let result = block_on(async {
        let response = client
            .get(&base)
            .send()
            .await
            .expect("request should succeed");
        response.bytes_limited(LIMIT).await
    });
    let elapsed = started.elapsed();

    assert!(matches!(result, Err(BodyReadError::TooLarge { limit }) if limit == LIMIT));
    assert!(
        elapsed < Duration::from_secs(3),
        "bytes_limited should abort well before all 50 chunks are sent (~10s); took {elapsed:?}"
    );
}
