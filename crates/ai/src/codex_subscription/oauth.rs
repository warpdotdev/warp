//! OAuth flow for connecting a ChatGPT / Codex account (OpenAI WHAM backend)
//! to Warp, so users can "plug in" their ChatGPT Plus or Pro subscription
//! instead of pasting a pay-as-you-go OpenAI API key.
//!
//! This mirrors the public Codex CLI desktop OAuth flow: an OAuth 2.0
//! Authorization Code grant with PKCE and a fixed loopback redirect URI.
//! OpenAI's auth server only accepts the loopback redirect for an allowlisted
//! `client_id` bound to a specific port, so we reuse the Codex CLI client and
//! bind the callback server to that exact port.
//!
//! This module owns only the network/protocol side: building the authorize
//! URL, running the loopback callback server, and exchanging/refreshing tokens
//! at OpenAI's token endpoint. Persistence of the resulting tokens, proactive
//! refresh scheduling, and injection into the request live in the parent
//! [`crate::codex_subscription`] module (refresh orchestration) and
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

const CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
const AUTHORIZE_URL: &str = "https://auth.openai.com/oauth/authorize";
const TOKEN_URL: &str = "https://auth.openai.com/oauth/token";
const SCOPE: &str = "openid profile email offline_access";

const REDIRECT_HOST: &str = "localhost";
const REDIRECT_PORT: u16 = 1455;

/// How long we keep the loopback server open waiting for the user to approve
/// the consent screen in their browser.
const CALLBACK_TIMEOUT: Duration = Duration::from_secs(300);
/// How long to nap between non-blocking `accept()` attempts.
const POLL_INTERVAL: Duration = Duration::from_millis(100);

fn redirect_uri() -> String {
    format!("http://{REDIRECT_HOST}:{REDIRECT_PORT}/auth/callback")
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
    /// another login already in progress, or Codex CLI holding the port)
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
/// flow. Includes OpenAI-specific params required by the Codex/WHAM OAuth flow.
fn authorize_url(pkce: &PkceParams) -> String {
    let redirect = redirect_uri();
    // `codex_cli_simplified_flow=true` identifies this as a Codex-style OAuth
    // flow to OpenAI's consent screen, required for the shared client_id.
    // `originator=warp` brands the authorization so OpenAI sees Warp as the
    // client, not an OpenCode or Codex CLI impersonation.
    // `id_token_add_organizations=true` ensures the id_token includes org info
    // needed for account ID extraction.
    let params: [(&str, &str); 10] = [
        ("response_type", "code"),
        ("client_id", CLIENT_ID),
        ("redirect_uri", &redirect),
        ("scope", SCOPE),
        ("code_challenge", &pkce.challenge),
        ("code_challenge_method", "S256"),
        ("state", &pkce.state),
        ("id_token_add_organizations", "true"),
        ("codex_cli_simplified_flow", "true"),
        ("originator", "warp"),
    ];
    let query =
        serde_urlencoded::to_string(params).expect("static OAuth params are always serializable");
    format!("{AUTHORIZE_URL}?{query}")
}

/// The token endpoint's response. Fields beyond `access_token` are optional
/// because OpenAI does not always return them. Other response fields (e.g.
/// `token_type`, `scope`) are ignored since nothing consumes them.
///
/// Note: OpenAI's `expires_in` is in **milliseconds**, not seconds — this is
/// handled during conversion to [`CodexTokens`] in the parent module.
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
            "couldn't bind the Codex OAuth callback server to {REDIRECT_HOST}:{REDIRECT_PORT}. \
             Another login may be in progress, or another app (e.g. Codex CLI) is using the port."
        )
    })?;
    listener
        .set_nonblocking(true)
        .context("failed to set the Codex OAuth callback listener to non-blocking mode")?;
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
        .name("codex-oauth-callback".to_owned())
        .spawn(move || {
            // `send_blocking` is disallowed (no wasm support); block this
            // dedicated thread on the async `send` instead.
            let _ = warpui_core::r#async::block_on(
                tx.send(wait_for_callback(&listener, CALLBACK_TIMEOUT)),
            );
        })
        .context("failed to spawn the Codex OAuth callback server thread")?;

    let callback = rx
        .recv()
        .await
        .context("the Codex OAuth callback server stopped unexpectedly")??;

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
            bail!("timed out waiting for the Codex authorization callback");
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
                return Err(anyhow::Error::new(e).context("Codex OAuth callback accept failed"))
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
        .context("failed to read the Codex OAuth callback request")?;
    let request = String::from_utf8_lossy(&buf[..n]);

    let origin = request_header(&request, "Origin");

    // The request line looks like: "GET /auth/callback?code=...&state=... HTTP/1.1".
    let mut request_line_parts = request
        .lines()
        .next()
        .unwrap_or_default()
        .split_whitespace();
    let method = request_line_parts.next().unwrap_or_default();
    let path = request_line_parts.next().unwrap_or_default();

    if method == "OPTIONS" && path.contains("/auth/callback") {
        write_response(&mut stream, "204 No Content", "", origin.as_deref());
        return Ok(None);
    }

    let Some(query) = path
        .split_once('?')
        .and_then(|(base, rest)| base.contains("/auth/callback").then_some(rest))
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
        bail!("Codex authorization was denied or failed: {detail}");
    }

    let (Some(code), Some(state)) = (code, state) else {
        write_response(
            &mut stream,
            "400 Bad Request",
            FAILURE_HTML,
            origin.as_deref(),
        );
        bail!("the Codex authorization callback was missing the code or state parameter");
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
fn write_response(stream: &mut TcpStream, status: &str, body: &str, _origin: Option<&str>) {
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\n\
         Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(response.as_bytes());
    let _ = stream.flush();
    let _ = stream.shutdown(Shutdown::Both);
}

/// Exchanges the authorization code for OAuth tokens at OpenAI's token endpoint.
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
/// the OAuth 2.0 `refresh_token` grant. Used to keep the connected ChatGPT
/// subscription's access token valid without re-running the browser flow.
///
/// OpenAI may or may not return a new `refresh_token`; callers should fall back to
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

/// POSTs a form-encoded body to OpenAI's token endpoint and parses the
/// [`TokenResponse`]. Shared by the initial code exchange and refresh grants.
async fn post_token_request<T: serde::Serialize + ?Sized>(
    form: &T,
) -> anyhow::Result<TokenResponse> {
    let response = http_client::Client::new()
        .post(TOKEN_URL)
        .form(form)
        .send()
        .await
        .context("failed to send the Codex token request")?;

    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        bail!("Codex token request failed ({status}): {body}");
    }

    response
        .json::<TokenResponse>()
        .await
        .context("failed to parse the Codex token response")
}

const SUCCESS_HTML: &str = "<!doctype html><html><head><meta charset=\"utf-8\">\
<title>Warp — ChatGPT connected</title></head>\
<body style=\"font-family:system-ui,-apple-system,sans-serif;text-align:center;padding:3rem\">\
<h1>ChatGPT account connected</h1><p>You can close this window and return to Warp.</p></body></html>";

const FAILURE_HTML: &str = "<!doctype html><html><head><meta charset=\"utf-8\">\
<title>Warp — Codex authorization failed</title></head>\
<body style=\"font-family:system-ui,-apple-system,sans-serif;text-align:center;padding:3rem\">\
<h1>Authorization failed</h1><p>Something went wrong. Return to Warp and try again.</p></body></html>";

#[cfg(test)]
#[path = "oauth_tests.rs"]
mod tests;
