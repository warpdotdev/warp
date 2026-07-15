//! OAuth protocol support for connecting a ChatGPT (Codex) subscription.
//!
//! OpenAI uses an OAuth 2.0 authorization-code grant with PKCE and one of two
//! allow-listed loopback redirects. This module owns the browser URL, callback
//! listener, token exchange, refresh and revocation requests, and extraction of
//! the ChatGPT account id carried in the trusted token endpoint's ID token.

use std::io::{ErrorKind, Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::time::Duration;

use anyhow::{bail, Context as _};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
// `std::time::Instant` is disallowed because it is unavailable on wasm.
use instant::Instant;
use rand::RngCore as _;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

pub const CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
const AUTHORIZE_URL: &str = "https://auth.openai.com/oauth/authorize";
const TOKEN_URL: &str = "https://auth.openai.com/oauth/token";
const REVOKE_URL: &str = "https://auth.openai.com/oauth/revoke";
const SCOPE: &str = "openid profile email offline_access";

const REDIRECT_HOST: &str = "127.0.0.1";
const REDIRECT_PORTS: [u16; 2] = [1455, 1457];
const CALLBACK_PATH: &str = "/auth/callback";
const CALLBACK_TIMEOUT: Duration = Duration::from_secs(300);
const POLL_INTERVAL: Duration = Duration::from_millis(100);

/// One in-flight browser login, including its bound callback listener and
/// per-attempt PKCE and CSRF secrets.
pub struct OauthAttempt {
    listener: TcpListener,
    redirect_uri: String,
    pkce: PkceParams,
}

impl OauthAttempt {
    /// Binds port 1455, falling back to the other OpenAI-allow-listed port 1457,
    /// then generates fresh PKCE and CSRF values.
    pub fn start() -> anyhow::Result<Self> {
        let (listener, port) = bind_callback_listener()?;
        Ok(Self {
            listener,
            redirect_uri: redirect_uri(port),
            pkce: PkceParams::generate(),
        })
    }

    /// Returns the OpenAI authorization URL to open in the user's browser.
    pub fn authorize_url(&self) -> String {
        authorize_url(&self.pkce, &self.redirect_uri)
    }

    /// Waits for the callback, validates its CSRF state, then exchanges the
    /// authorization code. Consuming the attempt prevents secret reuse.
    pub async fn finish(self) -> anyhow::Result<TokenResponse> {
        run_oauth_flow(self.listener, self.redirect_uri, self.pkce).await
    }
}

#[derive(Debug)]
struct PkceParams {
    verifier: String,
    challenge: String,
    state: String,
}

impl PkceParams {
    fn generate() -> Self {
        let verifier = random_url_safe_token();
        let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
        Self {
            verifier,
            challenge,
            state: random_url_safe_token(),
        }
    }
}

fn random_url_safe_token() -> String {
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

fn redirect_uri(port: u16) -> String {
    format!("http://localhost:{port}{CALLBACK_PATH}")
}

fn authorize_url(pkce: &PkceParams, redirect_uri: &str) -> String {
    let params: [(&str, &str); 9] = [
        ("response_type", "code"),
        ("client_id", CLIENT_ID),
        ("redirect_uri", redirect_uri),
        ("scope", SCOPE),
        ("code_challenge", &pkce.challenge),
        ("code_challenge_method", "S256"),
        ("id_token_add_organizations", "true"),
        ("codex_cli_simplified_flow", "true"),
        ("state", &pkce.state),
    ];
    let query =
        serde_urlencoded::to_string(params).expect("static OAuth params are always serializable");
    format!("{AUTHORIZE_URL}?{query}")
}

/// Tokens returned by OpenAI's authorization-code and refresh grants.
#[derive(Clone, Debug, Deserialize)]
pub struct TokenResponse {
    pub id_token: String,
    pub access_token: String,
    pub refresh_token: String,
    #[serde(default)]
    pub expires_in: Option<u64>,
}

#[derive(Debug, PartialEq, Eq)]
struct CallbackData {
    code: String,
    state: String,
}

fn bind_callback_listener() -> anyhow::Result<(TcpListener, u16)> {
    bind_callback_listener_on_ports(&REDIRECT_PORTS).with_context(|| {
        "couldn't bind the Codex OAuth callback server to either allow-listed port (1455 or 1457)"
    })
}

fn bind_callback_listener_on_ports(ports: &[u16]) -> anyhow::Result<(TcpListener, u16)> {
    let mut last_error = None;
    for &port in ports {
        match TcpListener::bind((REDIRECT_HOST, port)) {
            Ok(listener) => {
                listener.set_nonblocking(true).with_context(|| {
                    format!(
                        "failed to set the Codex OAuth callback listener on port {port} to non-blocking mode"
                    )
                })?;
                return Ok((listener, port));
            }
            Err(error) => last_error = Some(error),
        }
    }

    match last_error {
        Some(error) => Err(error).context("all Codex OAuth callback ports were unavailable"),
        None => bail!("no Codex OAuth callback ports were configured"),
    }
}

async fn run_oauth_flow(
    listener: TcpListener,
    redirect_uri: String,
    pkce: PkceParams,
) -> anyhow::Result<TokenResponse> {
    let (tx, rx) = async_channel::bounded(1);
    let expected_state = pkce.state.clone();
    std::thread::Builder::new()
        .name("codex-oauth-callback".to_owned())
        .spawn(move || {
            let _ = warpui_core::r#async::block_on(tx.send(wait_for_callback(
                &listener,
                CALLBACK_TIMEOUT,
                &expected_state,
            )));
        })
        .context("failed to spawn the Codex OAuth callback server thread")?;

    let callback = rx
        .recv()
        .await
        .context("the Codex OAuth callback server stopped unexpectedly")??;
    exchange_code_for_tokens(&callback.code, &pkce.verifier, &redirect_uri).await
}

fn validate_callback_state(callback: &CallbackData, expected_state: &str) -> anyhow::Result<()> {
    if callback.state != expected_state {
        bail!("the authorization response state did not match — aborting to prevent CSRF");
    }
    Ok(())
}

fn wait_for_callback(
    listener: &TcpListener,
    timeout: Duration,
    expected_state: &str,
) -> anyhow::Result<CallbackData> {
    let deadline = Instant::now() + timeout;
    loop {
        if Instant::now() >= deadline {
            bail!("timed out waiting for the Codex authorization callback");
        }
        match listener.accept() {
            Ok((stream, _)) => match handle_callback_connection(stream, expected_state)? {
                Some(data) => return Ok(data),
                None => continue,
            },
            Err(error) if error.kind() == ErrorKind::WouldBlock => {
                std::thread::sleep(POLL_INTERVAL);
            }
            Err(error) => {
                return Err(anyhow::Error::new(error).context("Codex OAuth callback accept failed"));
            }
        }
    }
}

fn handle_callback_connection(
    mut stream: TcpStream,
    expected_state: &str,
) -> anyhow::Result<Option<CallbackData>> {
    stream.set_nonblocking(false).ok();
    stream.set_read_timeout(Some(Duration::from_secs(10))).ok();

    let mut buf = [0u8; 8192];
    let n = stream
        .read(&mut buf)
        .context("failed to read the Codex OAuth callback request")?;
    let request = String::from_utf8_lossy(&buf[..n]);
    let mut request_line = request
        .lines()
        .next()
        .unwrap_or_default()
        .split_whitespace();
    let method = request_line.next().unwrap_or_default();
    let path = request_line.next().unwrap_or_default();

    if method != "GET" {
        write_response(&mut stream, "405 Method Not Allowed", "Method not allowed.");
        return Ok(None);
    }

    let Some(query) = path
        .strip_prefix(CALLBACK_PATH)
        .and_then(|rest| rest.strip_prefix('?'))
    else {
        write_response(&mut stream, "404 Not Found", "Not found.");
        return Ok(None);
    };

    let callback = parse_callback_query(query).and_then(|callback| {
        validate_callback_state(&callback, expected_state)?;
        Ok(callback)
    });
    match callback {
        Ok(data) => {
            write_response(&mut stream, "200 OK", SUCCESS_HTML);
            Ok(Some(data))
        }
        Err(error) => {
            write_response(&mut stream, "400 Bad Request", FAILURE_HTML);
            Err(error)
        }
    }
}

fn parse_callback_query(query: &str) -> anyhow::Result<CallbackData> {
    let mut code = None;
    let mut state = None;
    let mut error = None;
    let mut error_description = None;
    let pairs: Vec<(String, String)> =
        serde_urlencoded::from_str(query).context("the Codex authorization callback query was invalid")?;
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
        let detail = error_description.unwrap_or(error);
        bail!("Codex authorization was denied or failed: {detail}");
    }
    let (Some(code), Some(state)) = (code, state) else {
        bail!("the Codex authorization callback was missing the code or state parameter");
    };
    Ok(CallbackData { code, state })
}

fn write_response(stream: &mut TcpStream, status: &str, body: &str) {
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\n\
         Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(response.as_bytes());
    let _ = stream.flush();
    let _ = stream.shutdown(Shutdown::Both);
}

async fn exchange_code_for_tokens(
    code: &str,
    verifier: &str,
    redirect_uri: &str,
) -> anyhow::Result<TokenResponse> {
    exchange_code_for_tokens_at(code, verifier, redirect_uri, TOKEN_URL).await
}

async fn exchange_code_for_tokens_at(
    code: &str,
    verifier: &str,
    redirect_uri: &str,
    token_url: &str,
) -> anyhow::Result<TokenResponse> {
    let form: [(&str, &str); 5] = [
        ("grant_type", "authorization_code"),
        ("code", code),
        ("redirect_uri", redirect_uri),
        ("client_id", CLIENT_ID),
        ("code_verifier", verifier),
    ];
    post_token_request_at(&form, token_url).await
}

/// Exchanges a refresh token for a new OpenAI token response.
pub async fn refresh_access_token(refresh_token: &str) -> anyhow::Result<TokenResponse> {
    refresh_access_token_at(refresh_token, TOKEN_URL).await
}

async fn refresh_access_token_at(
    refresh_token: &str,
    token_url: &str,
) -> anyhow::Result<TokenResponse> {
    let form: [(&str, &str); 3] = [
        ("client_id", CLIENT_ID),
        ("grant_type", "refresh_token"),
        ("refresh_token", refresh_token),
    ];
    post_token_request_at(&form, token_url).await
}

async fn post_token_request_at<T: Serialize + ?Sized>(
    form: &T,
    token_url: &str,
) -> anyhow::Result<TokenResponse> {
    let response = http_client::Client::new()
        .post(token_url)
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

#[derive(Serialize)]
struct RevokeRequest<'a> {
    token: &'a str,
    token_type_hint: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    client_id: Option<&'static str>,
}

/// Revokes an access token. Disconnect remains best-effort at the call site.
pub async fn revoke_access_token(access_token: &str) -> anyhow::Result<()> {
    revoke_token_at(access_token, "access_token", None, REVOKE_URL).await
}

/// Revokes a refresh token, including the OAuth client id required by OpenAI.
pub async fn revoke_refresh_token(refresh_token: &str) -> anyhow::Result<()> {
    revoke_token_at(refresh_token, "refresh_token", Some(CLIENT_ID), REVOKE_URL).await
}

async fn revoke_token_at(
    token: &str,
    token_type_hint: &'static str,
    client_id: Option<&'static str>,
    revoke_url: &str,
) -> anyhow::Result<()> {
    let request = RevokeRequest {
        token,
        token_type_hint,
        client_id,
    };
    let response = http_client::Client::new()
        .post(revoke_url)
        .json(&request)
        .send()
        .await
        .context("failed to send the Codex token revocation request")?;
    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        bail!("Codex token revocation failed ({status}): {body}");
    }
    Ok(())
}

/// Extracts the ChatGPT account id from the trusted token endpoint's ID token.
/// This decodes claims only; JWT signature verification remains OpenAI's job.
pub fn chatgpt_account_id_from_id_token(id_token: &str) -> anyhow::Result<String> {
    let mut segments = id_token.split('.');
    let (header, payload, signature, extra) = (
        segments.next(),
        segments.next(),
        segments.next(),
        segments.next(),
    );
    let (Some(header), Some(payload), Some(signature), None) =
        (header, payload, signature, extra)
    else {
        bail!("the Codex ID token was not a JWT");
    };
    if header.is_empty() || payload.is_empty() || signature.is_empty() {
        bail!("the Codex ID token was not a JWT");
    }

    let payload = URL_SAFE_NO_PAD
        .decode(payload)
        .context("failed to decode the Codex ID token payload")?;
    let claims: Value =
        serde_json::from_slice(&payload).context("failed to parse the Codex ID token payload")?;
    let account_id = claims
        .get("https://api.openai.com/auth")
        .and_then(Value::as_object)
        .and_then(|auth| auth.get("chatgpt_account_id"))
        .and_then(Value::as_str)
        .filter(|account_id| !account_id.is_empty())
        .context("the Codex ID token did not contain chatgpt_account_id")?;
    Ok(account_id.to_owned())
}

const SUCCESS_HTML: &str = "<!doctype html><html><head><meta charset=\"utf-8\">\
<title>Warp — ChatGPT connected</title></head>\
<body style=\"font-family:system-ui,-apple-system,sans-serif;text-align:center;padding:3rem\">\
<h1>ChatGPT connected</h1><p>You can close this window and return to Warp.</p></body></html>";

const FAILURE_HTML: &str = "<!doctype html><html><head><meta charset=\"utf-8\">\
<title>Warp — ChatGPT authorization failed</title></head>\
<body style=\"font-family:system-ui,-apple-system,sans-serif;text-align:center;padding:3rem\">\
<h1>Authorization failed</h1><p>Something went wrong. Return to Warp and try again.</p></body></html>";

#[cfg(test)]
#[path = "oauth_tests.rs"]
mod tests;
