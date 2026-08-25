//! Classification of MCP transport and service errors.
//!
//! Reconnect and re-auth logic needs to know *why* an operation failed:
//! an expired Warp proxy session heals by re-minting a token, a closed
//! transport heals by reconnecting, and a rejected downstream OAuth grant
//! only heals when the user re-authenticates. The transports bury that
//! signal in nested error types; this module digs it out in one place.

use rmcp::transport::streamable_http_client::StreamableHttpError;

use crate::sse_transport::SseTransportError;

/// Why the Warp managed-MCP proxy rejected a proxy session token.
///
/// These correspond to the machine-readable reasons the proxy puts in its
/// `WWW-Authenticate` challenge (`error_description`) and JSON error bodies
/// (`code`). Both mean a freshly minted proxy session token will fix the
/// connection without user interaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProxyAuthReason {
    /// The proxy session token is past its expiry.
    ProxyTokenExpired,
    /// The token was minted against a previous OAuth grant that has since
    /// been re-authorized.
    ProxyTokenStale,
}

/// How a failed MCP operation should be handled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum McpErrorClass {
    /// An expired or stale Warp proxy session: re-mint the proxy token and
    /// reconnect without involving the user.
    AuthExpiredRecoverable(ProxyAuthReason),
    /// The server (or the downstream service behind the Warp proxy) rejected
    /// authentication in a way only the user can fix by re-authenticating.
    AuthRequiresUser,
    /// A likely-transient failure (closed transport, network error, 5xx);
    /// reconnecting or retrying may succeed.
    Transient,
    /// Retrying will not help (protocol or application-level errors).
    Fatal,
}

/// Parses the re-mint reason out of a `WWW-Authenticate` challenge.
///
/// The Warp proxy denies its own token problems with challenges like
/// `Bearer error="invalid_token", error_description="proxy_token_expired"`.
/// Challenges relayed from other servers never carry these descriptions.
pub fn parse_www_authenticate_reason(header: &str) -> Option<ProxyAuthReason> {
    if !header.contains(r#"error="invalid_token""#) {
        return None;
    }
    parse_proxy_reason(header)
}

/// Finds a proxy re-mint reason anywhere in a body or message string.
///
/// The proxy's JSON error bodies carry `"code":"proxy_token_expired"` /
/// `"code":"proxy_token_stale"`; rmcp surfaces those bodies only inside
/// formatted message strings, so this matches on the code substrings.
fn parse_proxy_reason(text: &str) -> Option<ProxyAuthReason> {
    if text.contains("proxy_token_expired") {
        Some(ProxyAuthReason::ProxyTokenExpired)
    } else if text.contains("proxy_token_stale") {
        Some(ProxyAuthReason::ProxyTokenStale)
    } else {
        None
    }
}

/// Classifies an [`rmcp::ServiceError`] from a failed MCP operation
/// (e.g. `call_tool`) into the action its caller should take.
pub fn classify_service_error(error: &rmcp::ServiceError) -> McpErrorClass {
    match error {
        rmcp::ServiceError::TransportClosed => McpErrorClass::Transient,
        rmcp::ServiceError::Timeout { .. } => McpErrorClass::Transient,
        rmcp::ServiceError::TransportSend(dynamic_error) => {
            classify_boxed_transport_error(dynamic_error.error.as_ref())
        }
        // McpError (application-level), UnexpectedResponse, Cancelled, and
        // any future variants: the connection itself worked.
        _ => McpErrorClass::Fatal,
    }
}

/// Classifies the boxed error inside a `TransportSend` failure by trying the
/// concrete error types of every transport Warp spawns.
fn classify_boxed_transport_error(
    error: &(dyn std::error::Error + Send + Sync + 'static),
) -> McpErrorClass {
    if let Some(http_error) = error.downcast_ref::<StreamableHttpError<reqwest::Error>>() {
        return classify_streamable_http_error(http_error);
    }
    if let Some(sse_error) = error.downcast_ref::<SseTransportError<reqwest::Error>>() {
        return classify_sse_error(sse_error, classify_reqwest_error);
    }
    // The OAuth SSE path nests the plain transport error inside `Client`
    // (see `sse_transport::auth_impl`).
    if let Some(sse_error) =
        error.downcast_ref::<SseTransportError<SseTransportError<reqwest::Error>>>()
    {
        return classify_sse_error(sse_error, |inner| {
            classify_sse_error(inner, classify_reqwest_error)
        });
    }
    // Unknown transports (e.g. the stdio child process): a failed send
    // usually means the connection is gone, which a reconnect can fix.
    McpErrorClass::Transient
}

fn classify_streamable_http_error(error: &StreamableHttpError<reqwest::Error>) -> McpErrorClass {
    match error {
        StreamableHttpError::AuthRequired(auth_required) => {
            match parse_www_authenticate_reason(&auth_required.www_authenticate_header) {
                Some(reason) => McpErrorClass::AuthExpiredRecoverable(reason),
                None => McpErrorClass::AuthRequiresUser,
            }
        }
        // rmcp formats non-success responses as "HTTP {status}: {body}",
        // which is the only place a 401 body survives on this transport.
        StreamableHttpError::UnexpectedServerResponse(message) => {
            classify_http_message(message, None)
        }
        StreamableHttpError::Client(client_error) => classify_reqwest_error(client_error),
        StreamableHttpError::Auth(_) | StreamableHttpError::InsufficientScope(_) => {
            McpErrorClass::AuthRequiresUser
        }
        StreamableHttpError::Sse(_)
        | StreamableHttpError::Io(_)
        | StreamableHttpError::UnexpectedEndOfStream
        | StreamableHttpError::TransportChannelClosed
        | StreamableHttpError::SessionExpired => McpErrorClass::Transient,
        // Deserialize, UnexpectedContentType, protocol-support errors, and
        // any future variants.
        _ => McpErrorClass::Fatal,
    }
}

fn classify_sse_error<E: std::error::Error + Send + Sync + 'static>(
    error: &SseTransportError<E>,
    classify_client: impl FnOnce(&E) -> McpErrorClass,
) -> McpErrorClass {
    match error {
        SseTransportError::HttpStatus {
            status,
            body,
            www_authenticate,
        } => classify_http_status(*status, body, www_authenticate.as_deref()),
        SseTransportError::Client(client_error) => classify_client(client_error),
        SseTransportError::Auth(_) => McpErrorClass::AuthRequiresUser,
        SseTransportError::Sse(_)
        | SseTransportError::Io(_)
        | SseTransportError::UnexpectedEndOfStream => McpErrorClass::Transient,
        SseTransportError::UnexpectedContentType(_)
        | SseTransportError::InvalidUri(_)
        | SseTransportError::InvalidUriParts(_) => McpErrorClass::Fatal,
    }
}

/// Classifies an HTTP error response with a known status code.
pub fn classify_http_status(
    status: http::StatusCode,
    body: &str,
    www_authenticate: Option<&str>,
) -> McpErrorClass {
    if let Some(reason) = www_authenticate.and_then(parse_www_authenticate_reason) {
        return McpErrorClass::AuthExpiredRecoverable(reason);
    }
    if let Some(reason) = parse_proxy_reason(body) {
        return McpErrorClass::AuthExpiredRecoverable(reason);
    }
    // The proxy's code for "the downstream OAuth grant is unusable": only
    // the user re-authenticating fixes that.
    if body.contains("downstream_auth_failed") {
        return McpErrorClass::AuthRequiresUser;
    }
    match status.as_u16() {
        401 | 403 => McpErrorClass::AuthRequiresUser,
        408 | 429 | 500..=599 => McpErrorClass::Transient,
        _ => McpErrorClass::Fatal,
    }
}

/// Classifies rmcp's "HTTP {status}: {body}" message strings.
fn classify_http_message(message: &str, www_authenticate: Option<&str>) -> McpErrorClass {
    if let Some(reason) = www_authenticate.and_then(parse_www_authenticate_reason) {
        return McpErrorClass::AuthExpiredRecoverable(reason);
    }
    if let Some(reason) = parse_proxy_reason(message) {
        return McpErrorClass::AuthExpiredRecoverable(reason);
    }
    if message.contains("downstream_auth_failed") {
        return McpErrorClass::AuthRequiresUser;
    }
    if message.starts_with("HTTP 401") || message.starts_with("HTTP 403") {
        return McpErrorClass::AuthRequiresUser;
    }
    if message.starts_with("HTTP 408")
        || message.starts_with("HTTP 429")
        || message.starts_with("HTTP 5")
    {
        return McpErrorClass::Transient;
    }
    McpErrorClass::Fatal
}

fn classify_reqwest_error(error: &reqwest::Error) -> McpErrorClass {
    if let Some(status) = error.status() {
        return classify_http_status(status, "", None);
    }
    // Connect, timeout, and request-level failures without a status are all
    // network conditions a retry can outlast.
    McpErrorClass::Transient
}

/// Whether a failed operation provably never delivered the request to the
/// server, making an automatic resend safe even for non-idempotent tools.
///
/// Failures where the server may have started executing the request (e.g. a
/// 5xx after accepting it, or a response timeout) return false.
pub fn is_safe_to_resend(error: &rmcp::ServiceError) -> bool {
    match error {
        // A closed transport fails the send before anything leaves.
        rmcp::ServiceError::TransportClosed => true,
        rmcp::ServiceError::TransportSend(dynamic_error) => {
            let error = &dynamic_error.error;
            if let Some(error) = error.downcast_ref::<StreamableHttpError<reqwest::Error>>() {
                return streamable_send_is_undelivered(error);
            }
            if let Some(error) = error.downcast_ref::<SseTransportError<reqwest::Error>>() {
                return sse_send_is_undelivered(error, reqwest_send_is_undelivered);
            }
            if let Some(error) =
                error.downcast_ref::<SseTransportError<SseTransportError<reqwest::Error>>>()
            {
                return sse_send_is_undelivered(error, |inner| {
                    sse_send_is_undelivered(inner, reqwest_send_is_undelivered)
                });
            }
            // Unknown transports (e.g. the stdio child process): a failed
            // send means the pipe broke before delivery.
            true
        }
        _ => false,
    }
}

/// Statuses a server returns without executing the request body.
fn status_rejected_before_execution(status: u16) -> bool {
    matches!(status, 401 | 403 | 407 | 408 | 429)
}

fn streamable_send_is_undelivered(error: &StreamableHttpError<reqwest::Error>) -> bool {
    match error {
        StreamableHttpError::AuthRequired(_) | StreamableHttpError::InsufficientScope(_) => true,
        StreamableHttpError::UnexpectedServerResponse(message) => message
            .strip_prefix("HTTP ")
            .and_then(|rest| rest.get(..3))
            .and_then(|status| status.parse::<u16>().ok())
            .is_some_and(status_rejected_before_execution),
        StreamableHttpError::Client(client_error) => reqwest_send_is_undelivered(client_error),
        StreamableHttpError::Sse(_)
        | StreamableHttpError::Io(_)
        | StreamableHttpError::UnexpectedEndOfStream
        | StreamableHttpError::TransportChannelClosed
        | StreamableHttpError::SessionExpired => true,
        _ => false,
    }
}

fn sse_send_is_undelivered<E: std::error::Error + Send + Sync + 'static>(
    error: &SseTransportError<E>,
    client_is_undelivered: impl FnOnce(&E) -> bool,
) -> bool {
    match error {
        SseTransportError::HttpStatus { status, .. } => {
            status_rejected_before_execution(status.as_u16())
        }
        SseTransportError::Client(client_error) => client_is_undelivered(client_error),
        // Token sourcing failed before any request was sent.
        SseTransportError::Auth(_) => true,
        SseTransportError::Sse(_)
        | SseTransportError::Io(_)
        | SseTransportError::UnexpectedEndOfStream => true,
        SseTransportError::UnexpectedContentType(_)
        | SseTransportError::InvalidUri(_)
        | SseTransportError::InvalidUriParts(_) => false,
    }
}

fn reqwest_send_is_undelivered(error: &reqwest::Error) -> bool {
    match error.status() {
        Some(status) => status_rejected_before_execution(status.as_u16()),
        // No status: the request never reached the server.
        None => true,
    }
}

#[cfg(test)]
#[path = "error_classification_tests.rs"]
mod tests;
