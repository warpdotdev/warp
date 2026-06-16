//! OAuth flow for connecting an xAI / Grok subscription (e.g. SuperGrok) to
//! Warp, so users can "plug in" their subscription instead of pasting a
//! pay-as-you-go API key.
//!
//! This mirrors the public Grok-CLI desktop OAuth flow: an OAuth 2.0
//! Authorization Code grant with PKCE and a fixed loopback redirect URI. xAI's
//! auth server only accepts the loopback redirect for an allowlisted
//! `client_id` bound to a specific port, so we reuse the Grok-CLI client and
//! bind the callback server to that exact port.
//!
//! Some browsers/networks can't reach the loopback callback (e.g. Private
//! Network Access is blocked), in which case xAI's consent screen instead
//! *displays* the authorization code for the user to paste back into the app.
//! [`OauthAttempt::manual_code_exchange`] supports that fallback by capturing
//! the attempt's PKCE verifier so a pasted code can be exchanged directly,
//! without ever observing the loopback redirect.
//!
//! This module owns only the network/protocol side: building the authorize
//! URL, running the loopback callback server, and exchanging/refreshing tokens
//! at xAI's token endpoint. Persistence of the resulting tokens, proactive
//! refresh scheduling, and injection into the request live in the parent
//! [`crate::grok_subscription`] module (refresh orchestration) and
//! [`crate::api_keys::ApiKeyManager`] (storage + request injection).

use std::io::{ErrorKind, Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::time::Duration;

use anyhow::{bail, Context as _};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
// `std::time::Instant` is disallowed (no wasm support); `instant::Instant` is a
// drop-in that re-exports the std type on native targets.
use instant::Instant;
use rand::RngCore as _;
use serde::Deserialize;
use sha2::{Digest, Sha256};

const CLIENT_ID: &str = "b1a00492-073a-47ea-816f-4c329264a828";
const AUTHORIZE_URL: &str = "https://auth.x.ai/oauth2/authorize";
const TOKEN_URL: &str = "https://auth.x.ai/oauth2/token";
const SCOPE: &str = "openid profile email offline_access grok-cli:access api:access";

const REDIRECT_HOST: &str = "127.0.0.1";
const REDIRECT_PORT: u16 = 56121;

/// How long we keep the loopback server open waiting for the user to approve
/// the consent screen in their browser.
const CALLBACK_TIMEOUT: Duration = Duration::from_secs(300);
/// How long to nap between non-blocking `accept()` attempts.
const POLL_INTERVAL: Duration = Duration::from_millis(100);

/// xAI's browser consent screen fetches the loopback callback from these
/// origins. Since that request crosses origins (https://accounts.x.ai ->
/// http://127.0.0.1), browsers require CORS and Private Network Access headers
/// before the page can observe the callback response.
const CORS_ALLOWED_ORIGINS: [&str; 2] = ["https://accounts.x.ai", "https://auth.x.ai"];

fn redirect_uri() -> String {
    format!("http://{REDIRECT_HOST}:{REDIRECT_PORT}/callback")
}

/// One in-flight OAuth login attempt: the bound loopback callback listener
/// plus the per-attempt PKCE/CSRF secrets, which never leave this module.
///
/// Construct with [`OauthAttempt::start`], open [`OauthAttempt::authorize_url`]
/// in the browser, then await [`OauthAttempt::finish`] to obtain tokens. Tying
/// the secrets to the attempt guarantees the same PKCE verifier and CSRF state
/// are used for both the authorize URL and the code exchange.
pub struct OauthAttempt {
    listener: TcpListener,
    pkce: PkceParams,
}

impl OauthAttempt {
    /// Binds the loopback callback server and generates fresh per-attempt
    /// secrets. Call this before opening the browser so a bind failure (e.g.
    /// another login already in progress, or Grok-CLI holding the port)
    /// surfaces before a browser tab opens.
    pub fn start() -> anyhow::Result<Self> {
        Ok(Self {
            listener: bind_callback_listener()?,
            pkce: PkceParams::generate(),
        })
    }

    /// The authorization URL the user's browser should open to begin the flow.
    pub fn authorize_url(&self) -> String {
        authorize_url(&self.pkce)
    }

    /// Runs the rest of the browser-based PKCE flow: waits for the loopback
    /// callback, validates the CSRF state, and exchanges the authorization
    /// code for tokens. Consumes the attempt so its secrets can't be reused.
    pub async fn finish(self) -> anyhow::Result<TokenResponse> {
        run_oauth_flow(self.listener, self.pkce).await
    }

    /// Clones the PKCE verifier for the pasted-code fallback while the
    /// loopback flow continues racing in parallel.
    pub fn manual_code_exchange(&self) -> ManualCodeExchange {
        ManualCodeExchange {
            verifier: self.pkce.verifier.clone(),
        }
    }
}

/// Completes OAuth from a manually-pasted authorization code.
///
/// There is no redirect `state` to validate in this out-of-band path; PKCE
/// protects the exchange.
#[derive(Clone)]
pub struct ManualCodeExchange {
    verifier: String,
}

impl ManualCodeExchange {
    /// Exchanges a user-pasted authorization `code` with the attempt's PKCE verifier.
    pub async fn exchange(&self, code: &str) -> anyhow::Result<TokenResponse> {
        let code = code.trim();
        if code.is_empty() {
            bail!("enter the code shown in your browser to finish connecting");
        }
        exchange_code_for_tokens(code, &self.verifier).await
    }
}

/// The per-attempt secrets for one authorization request: the PKCE
/// verifier/challenge pair and the CSRF `state` value.
struct PkceParams {
    verifier: String,
    challenge: String,
    /// CSRF token echoed back on the redirect and validated against the
    /// response before the code is exchanged.
    state: String,
}

impl PkceParams {
    /// Generates a fresh PKCE verifier + S256 challenge and a random CSRF state.
    fn generate() -> Self {
        let verifier = random_url_safe_token();
        let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
        let state = random_url_safe_token();
        Self {
            verifier,
            challenge,
            state,
        }
    }
}

/// Returns a URL-safe, unpadded base64 string of 32 random bytes. This is used
/// for both the PKCE code verifier (RFC 7636 allows 43-128 chars from the
/// unreserved set) and the CSRF state.
fn random_url_safe_token() -> String {
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

/// Builds the authorization URL the user's browser should open to begin the
/// flow.
fn authorize_url(pkce: &PkceParams) -> String {
    let redirect = redirect_uri();
    // `plan=generic` opts the consent screen into xAI's generic OAuth plan tier
    // (required for loopback OAuth from non-allowlisted clients); `referrer`
    // is best-effort attribution in xAI's OAuth logs.
    let params: [(&str, &str); 9] = [
        ("response_type", "code"),
        ("client_id", CLIENT_ID),
        ("redirect_uri", &redirect),
        ("scope", SCOPE),
        ("code_challenge", &pkce.challenge),
        ("code_challenge_method", "S256"),
        ("state", &pkce.state),
        ("plan", "generic"),
        ("referrer", "warp"),
    ];
    let query =
        serde_urlencoded::to_string(params).expect("static OAuth params are always serializable");
    format!("{AUTHORIZE_URL}?{query}")
}

/// The token endpoint's response. Fields beyond `access_token` are optional
/// because xAI does not always return them. Other response fields (e.g.
/// `token_type`, `scope`) are ignored since nothing consumes them.
#[derive(Debug, Deserialize)]
pub struct TokenResponse {
    pub access_token: String,
    #[serde(default)]
    pub refresh_token: Option<String>,
    #[serde(default)]
    pub expires_in: Option<i64>,
}

/// The authorization code and state captured from the loopback redirect.
struct CallbackData {
    code: String,
    state: String,
}

/// Binds the loopback callback server to the fixed redirect address.
fn bind_callback_listener() -> anyhow::Result<TcpListener> {
    let listener = TcpListener::bind((REDIRECT_HOST, REDIRECT_PORT)).with_context(|| {
        format!(
            "couldn't bind the Grok OAuth callback server to {REDIRECT_HOST}:{REDIRECT_PORT}. \
             Another login may be in progress, or another app (e.g. Grok CLI) is using the port."
        )
    })?;
    listener
        .set_nonblocking(true)
        .context("failed to set the Grok OAuth callback listener to non-blocking mode")?;
    Ok(listener)
}

/// Runs the full browser-based PKCE flow: waits for the loopback callback on a
/// dedicated thread, validates the CSRF state, and exchanges the authorization
/// code for tokens.
async fn run_oauth_flow(listener: TcpListener, pkce: PkceParams) -> anyhow::Result<TokenResponse> {
    // The loopback accept loop is blocking, so run it on a dedicated OS thread
    // and bridge the result back through a runtime-agnostic async channel.
    let (tx, rx) = async_channel::bounded(1);
    std::thread::Builder::new()
        .name("grok-oauth-callback".to_owned())
        .spawn(move || {
            // `send_blocking` is disallowed (no wasm support); block this
            // dedicated thread on the async `send` instead.
            let _ = warpui_core::r#async::block_on(
                tx.send(wait_for_callback(&listener, CALLBACK_TIMEOUT)),
            );
        })
        .context("failed to spawn the Grok OAuth callback server thread")?;

    let callback = rx
        .recv()
        .await
        .context("the Grok OAuth callback server stopped unexpectedly")??;

    if callback.state != pkce.state {
        bail!("the authorization response state did not match — aborting to prevent CSRF");
    }

    exchange_code_for_tokens(&callback.code, &pkce.verifier).await
}

/// Blocks (on a non-blocking listener with polling) until the browser hits the
/// redirect URI, returning the captured code and state, or an error on timeout.
fn wait_for_callback(listener: &TcpListener, timeout: Duration) -> anyhow::Result<CallbackData> {
    let deadline = Instant::now() + timeout;
    loop {
        if Instant::now() >= deadline {
            bail!("timed out waiting for the Grok authorization callback");
        }
        match listener.accept() {
            Ok((stream, _)) => match handle_callback_connection(stream)? {
                Some(data) => return Ok(data),
                // Unrelated request (e.g. /favicon.ico); keep waiting.
                None => continue,
            },
            Err(ref e) if e.kind() == ErrorKind::WouldBlock => {
                std::thread::sleep(POLL_INTERVAL);
            }
            Err(e) => {
                return Err(anyhow::Error::new(e).context("Grok OAuth callback accept failed"))
            }
        }
    }
}

/// Reads a single HTTP request from the callback connection, writes back a
/// minimal HTML response, and extracts the OAuth parameters.
///
/// Returns `Ok(None)` for requests that aren't the OAuth callback (so the
/// caller keeps listening), `Ok(Some(..))` on a successful callback, and `Err`
/// when the provider reported an error or the callback was malformed.
fn handle_callback_connection(mut stream: TcpStream) -> anyhow::Result<Option<CallbackData>> {
    // The accepted stream may inherit the listener's non-blocking flag on some
    // platforms; force blocking reads with a timeout so we get the full request
    // line without spinning.
    stream.set_nonblocking(false).ok();
    stream.set_read_timeout(Some(Duration::from_secs(10))).ok();

    let mut buf = [0u8; 8192];
    let n = stream
        .read(&mut buf)
        .context("failed to read the Grok OAuth callback request")?;
    let request = String::from_utf8_lossy(&buf[..n]);

    let origin = request_header(&request, "Origin");

    // The request line looks like: "GET /callback?code=...&state=... HTTP/1.1".
    let mut request_line_parts = request
        .lines()
        .next()
        .unwrap_or_default()
        .split_whitespace();
    let method = request_line_parts.next().unwrap_or_default();
    let path = request_line_parts.next().unwrap_or_default();

    if method == "OPTIONS" && path.starts_with("/callback") {
        write_response(&mut stream, "204 No Content", "", origin.as_deref());
        return Ok(None);
    }

    let Some(query) = path
        .strip_prefix("/callback")
        .and_then(|rest| rest.strip_prefix('?'))
    else {
        write_response(
            &mut stream,
            "404 Not Found",
            "Not found.",
            origin.as_deref(),
        );
        return Ok(None);
    };

    let mut code = None;
    let mut state = None;
    let mut error = None;
    let mut error_description = None;
    let pairs: Vec<(String, String)> = serde_urlencoded::from_str(query).unwrap_or_default();
    for (key, value) in pairs {
        match key.as_str() {
            "code" => code = Some(value),
            "state" => state = Some(value),
            "error" => error = Some(value),
            "error_description" => error_description = Some(value),
            _ => {}
        }
    }

    if let Some(error) = error {
        write_response(
            &mut stream,
            "400 Bad Request",
            FAILURE_HTML,
            origin.as_deref(),
        );
        let detail = error_description.unwrap_or(error);
        bail!("Grok authorization was denied or failed: {detail}");
    }

    let (Some(code), Some(state)) = (code, state) else {
        write_response(
            &mut stream,
            "400 Bad Request",
            FAILURE_HTML,
            origin.as_deref(),
        );
        bail!("the Grok authorization callback was missing the code or state parameter");
    };
    write_response(&mut stream, "200 OK", SUCCESS_HTML, origin.as_deref());
    Ok(Some(CallbackData { code, state }))
}
fn request_header(request: &str, header_name: &str) -> Option<String> {
    request.lines().skip(1).find_map(|line| {
        let (name, value) = line.split_once(':')?;
        name.eq_ignore_ascii_case(header_name)
            .then(|| value.trim().to_owned())
    })
}

/// Writes a minimal HTTP/1.1 response and closes the connection.
fn write_response(stream: &mut TcpStream, status: &str, body: &str, origin: Option<&str>) {
    let cors_headers = cors_headers(origin);
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\n\
         {cors_headers}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(response.as_bytes());
    let _ = stream.flush();
    let _ = stream.shutdown(Shutdown::Both);
}

fn cors_headers(origin: Option<&str>) -> String {
    origin
        .filter(|origin| CORS_ALLOWED_ORIGINS.contains(origin))
        .map(|origin| {
            format!(
                "Access-Control-Allow-Origin: {origin}\r\n\
                 Access-Control-Allow-Methods: GET, OPTIONS\r\n\
                 Access-Control-Allow-Headers: Content-Type\r\n\
                 Access-Control-Allow-Private-Network: true\r\n\
                 Vary: Origin\r\n"
            )
        })
        .unwrap_or_default()
}

/// Exchanges the authorization code for OAuth tokens at xAI's token endpoint.
async fn exchange_code_for_tokens(code: &str, verifier: &str) -> anyhow::Result<TokenResponse> {
    let redirect = redirect_uri();
    let form: [(&str, &str); 5] = [
        ("grant_type", "authorization_code"),
        ("code", code),
        ("redirect_uri", &redirect),
        ("client_id", CLIENT_ID),
        ("code_verifier", verifier),
    ];
    post_token_request(&form).await
}

/// Exchanges a previously obtained refresh token for a fresh set of tokens via
/// the OAuth 2.0 `refresh_token` grant. Used to keep the connected Grok
/// subscription's access token valid without re-running the browser flow.
///
/// xAI may or may not return a new `refresh_token`; callers should fall back to
/// the existing one when [`TokenResponse::refresh_token`] is `None` (rotation is
/// optional in OAuth 2.0).
pub async fn refresh_access_token(refresh_token: &str) -> anyhow::Result<TokenResponse> {
    let form: [(&str, &str); 3] = [
        ("grant_type", "refresh_token"),
        ("refresh_token", refresh_token),
        ("client_id", CLIENT_ID),
    ];
    post_token_request(&form).await
}

/// POSTs a form-encoded body to xAI's token endpoint and parses the
/// [`TokenResponse`]. Shared by the initial code exchange and refresh grants.
async fn post_token_request<T: serde::Serialize + ?Sized>(
    form: &T,
) -> anyhow::Result<TokenResponse> {
    let response = http_client::Client::new()
        .post(TOKEN_URL)
        .form(form)
        .send()
        .await
        .context("failed to send the Grok token request")?;

    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        bail!("Grok token request failed ({status}): {body}");
    }

    response
        .json::<TokenResponse>()
        .await
        .context("failed to parse the Grok token response")
}

const SUCCESS_HTML: &str = "<!doctype html><html><head><meta charset=\"utf-8\">\
<title>Warp — Grok connected</title></head>\
<body style=\"font-family:system-ui,-apple-system,sans-serif;text-align:center;padding:3rem\">\
<h1>Grok connected</h1><p>You can close this window and return to Warp.</p></body></html>";

const FAILURE_HTML: &str = "<!doctype html><html><head><meta charset=\"utf-8\">\
<title>Warp — Grok authorization failed</title></head>\
<body style=\"font-family:system-ui,-apple-system,sans-serif;text-align:center;padding:3rem\">\
<h1>Authorization failed</h1><p>Something went wrong. Return to Warp and try again.</p></body></html>";

#[cfg(test)]
#[path = "oauth_tests.rs"]
mod tests;
