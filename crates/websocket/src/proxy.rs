//! HTTP proxy support for WebSocket connections.
//!
//! Reads standard proxy environment variables (`HTTPS_PROXY`, `HTTP_PROXY`, `ALL_PROXY`)
//! and establishes tunneled connections via HTTP CONNECT.
//!
//! TODO: Switch to tungstenite's native proxy support once it is available and remove this
//! module: <https://github.com/snapview/tungstenite-rs/pull/530>

use std::env;
use std::time::Duration;

use anyhow::{bail, Context};
use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine as _;
use http_body_util::Empty;
use hyper::body::Bytes;
use hyper_util::rt::TokioIo;
use percent_encoding::percent_decode_str;
use tokio::net::TcpStream;
use tokio::time::timeout;
use url::Url;

const PROXY_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const PROXY_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

/// Proxy connection info parsed from environment variables.
#[derive(Debug)]
pub struct ProxyInfo {
    pub host: String,
    pub port: u16,
    /// Base64-encoded `user:password` for `Proxy-Authorization: Basic` header.
    pub basic_auth: Option<String>,
}

/// Reads proxy environment variables and returns proxy info if a proxy should be used
/// for the given target URI.
///
/// Env var precedence:
/// - For TLS targets (`wss://`): `HTTPS_PROXY` / `https_proxy`, then `ALL_PROXY` / `all_proxy`.
/// - For plain targets (`ws://`): `HTTP_PROXY` / `http_proxy`, then `ALL_PROXY` / `all_proxy`.
/// - `NO_PROXY` / `no_proxy` is checked to bypass the proxy for specific hosts.
pub fn resolve_proxy(uri: &http::Uri) -> anyhow::Result<Option<ProxyInfo>> {
    let is_tls = uri.scheme_str() == Some("wss") || uri.scheme_str() == Some("https");
    let target_host = uri.host().unwrap_or_default();

    let proxy_env = if is_tls {
        read_env_var("HTTPS_PROXY").or_else(|| read_env_var("ALL_PROXY"))
    } else {
        read_env_var("HTTP_PROXY").or_else(|| read_env_var("ALL_PROXY"))
    };

    let Some((proxy_env_name, proxy_url)) = proxy_env else {
        return Ok(None);
    };

    if is_no_proxy(target_host) {
        return Ok(None);
    }

    parse_proxy_url(&proxy_url)
        .with_context(|| format!("Invalid proxy URL configured in {proxy_env_name}"))
        .map(Some)
}

/// Establishes a TCP connection through an HTTP proxy using the CONNECT method.
///
/// Uses hyper's HTTP/1 client to send the CONNECT request and then extracts
/// the underlying `TcpStream` via the upgrade mechanism.
pub async fn connect_via_proxy(
    proxy: &ProxyInfo,
    target_uri: &http::Uri,
) -> anyhow::Result<TcpStream> {
    let target_host = target_uri.host().context("Target URI has no host")?;
    let is_tls = target_uri.scheme_str() == Some("wss") || target_uri.scheme_str() == Some("https");
    let default_port: u16 = if is_tls { 443 } else { 80 };
    let target_port = target_uri.port_u16().unwrap_or(default_port);

    // 1. TCP connect to the proxy.
    let stream = timeout(
        PROXY_CONNECT_TIMEOUT,
        TcpStream::connect((&*proxy.host, proxy.port)),
    )
    .await
    .context("Timed out connecting to proxy")?
    .with_context(|| format!("Failed to connect to proxy {}:{}", proxy.host, proxy.port))?;

    // 2. HTTP/1 handshake over the proxy TCP stream.
    let (mut sender, conn) = timeout(
        PROXY_HANDSHAKE_TIMEOUT,
        hyper::client::conn::http1::handshake(TokioIo::new(stream)),
    )
    .await
    .context("Timed out during HTTP handshake with proxy")?
    .context("HTTP handshake with proxy failed")?;

    // Drive the connection in the background with upgrade support.
    tokio::spawn(async move {
        if let Err(err) = conn.with_upgrades().await {
            log::warn!("Proxy connection driver error: {err}");
        }
    });

    // 3. Build and send the CONNECT request.
    let authority = format!("{target_host}:{target_port}");
    let mut req = hyper::Request::builder()
        .method(hyper::Method::CONNECT)
        .uri(&authority)
        .header(hyper::header::HOST, &authority)
        .body(Empty::<Bytes>::new())
        .context("Failed to build CONNECT request")?;

    if let Some(credentials) = &proxy.basic_auth {
        req.headers_mut().insert(
            "proxy-authorization",
            format!("Basic {credentials}")
                .parse()
                .context("Invalid Proxy-Authorization header value")?,
        );
    }

    let response = timeout(PROXY_HANDSHAKE_TIMEOUT, sender.send_request(req))
        .await
        .context("Timed out waiting for CONNECT response from proxy")?
        .context("Failed to send CONNECT request to proxy")?;

    if !response.status().is_success() {
        bail!("Proxy CONNECT failed with status: {}", response.status());
    }

    // 4. Upgrade the connection to get the raw stream.
    let upgraded = hyper::upgrade::on(response)
        .await
        .context("Failed to upgrade proxy connection after CONNECT")?;

    // 5. Downcast back to the underlying TcpStream.
    let downcast = upgraded.downcast::<TokioIo<TcpStream>>().map_err(|_| {
        anyhow::anyhow!("Failed to downcast upgraded proxy connection to TcpStream")
    })?;

    Ok(downcast.io.into_inner())
}

/// Reads an environment variable by its canonical (uppercase) name, falling back to lowercase.
fn read_env_var(uppercase_name: &str) -> Option<(String, String)> {
    env::var(uppercase_name)
        .ok()
        .filter(|v| !v.is_empty())
        .map(|value| (uppercase_name.to_string(), value))
        .or_else(|| {
            let lowercase_name = uppercase_name.to_lowercase();
            env::var(&lowercase_name)
                .ok()
                .filter(|v| !v.is_empty())
                .map(|value| (lowercase_name, value))
        })
}

/// Returns `true` if `target_host` matches any entry in `NO_PROXY` / `no_proxy`.
///
/// Supported patterns:
/// - `*` matches all hosts.
/// - Exact match (case-insensitive).
/// - Suffix match with leading `.` (e.g. `.example.com` matches `foo.example.com`).
/// - Suffix match without leading `.` (e.g. `example.com` matches `foo.example.com`).
fn is_no_proxy(target_host: &str) -> bool {
    let no_proxy = read_env_var("NO_PROXY")
        .map(|(_, value)| value)
        .unwrap_or_default();
    if no_proxy.is_empty() {
        return false;
    }

    let target = target_host.to_lowercase();
    for entry in no_proxy.split(',') {
        let entry = entry.trim().to_lowercase();
        if entry.is_empty() {
            continue;
        }
        if entry == "*" {
            return true;
        }
        if target == entry {
            return true;
        }
        // Suffix match: ".example.com" matches "foo.example.com"
        if entry.starts_with('.') && target.ends_with(&entry) {
            return true;
        }
        // Suffix match without leading dot: "example.com" matches "foo.example.com"
        if target.ends_with(&format!(".{entry}")) {
            return true;
        }
    }

    false
}

/// Parses a proxy URL string into a `ProxyInfo`.
fn parse_proxy_url(raw: &str) -> anyhow::Result<ProxyInfo> {
    // Many proxy URLs are specified without a scheme (e.g. "proxy.corp:8080").
    // Prepend "http://" if no scheme is present so the URL parser can handle it.
    let normalized = if raw.contains("://") {
        raw.to_string()
    } else {
        format!("http://{raw}")
    };
    let url = Url::parse(&normalized).context("failed to parse proxy URL")?;
    match url.scheme() {
        "http" => {}
        "https" => bail!("HTTPS proxy URLs are not supported"),
        scheme => bail!("Unsupported proxy scheme '{scheme}'"),
    }

    let host = url
        .host_str()
        .context("proxy URL is missing a host")?
        .to_string();
    let port = url.port_or_known_default().unwrap_or(8080);

    let username = percent_decode_str(url.username())
        .decode_utf8()
        .context("proxy username contains invalid percent-encoding")?
        .into_owned();
    let password = url
        .password()
        .map(|password| {
            percent_decode_str(password)
                .decode_utf8()
                .context("proxy password contains invalid percent-encoding")
        })
        .transpose()?
        .map(|password| password.into_owned());

    let basic_auth = if !username.is_empty() || password.is_some() {
        let userinfo = format!("{username}:{}", password.unwrap_or_default());
        Some(BASE64.encode(userinfo))
    } else {
        None
    };

    Ok(ProxyInfo {
        host,
        port,
        basic_auth,
    })
}

#[cfg(test)]
#[path = "proxy_tests.rs"]
mod tests;
