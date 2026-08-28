use std::net::IpAddr;
use std::time::Duration;

use rmcp::transport::AuthError;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpListener;
use url::Url;

use super::CallbackResult;

const CALLBACK_PATH: &str = "/mcp/oauth2callback";
const CALLBACK_TIMEOUT: Duration = Duration::from_secs(300);
const MAX_REQUEST_BYTES: usize = 16 * 1024;

pub struct LoopbackOAuthReceiver {
    listener: TcpListener,
    redirect_uri: String,
}

impl LoopbackOAuthReceiver {
    pub async fn bind() -> Result<Self, AuthError> {
        let listener = TcpListener::bind(("127.0.0.1", 0))
            .await
            .map_err(|err| AuthError::InternalError(err.to_string()))?;
        let address = listener
            .local_addr()
            .map_err(|err| AuthError::InternalError(err.to_string()))?;
        Ok(Self {
            listener,
            redirect_uri: format!("http://127.0.0.1:{}{CALLBACK_PATH}", address.port()),
        })
    }

    pub fn redirect_uri(&self) -> &str {
        &self.redirect_uri
    }

    pub async fn receive(self, expected_state: &str) -> Result<CallbackResult, AuthError> {
        tokio::time::timeout(CALLBACK_TIMEOUT, self.receive_inner(expected_state))
            .await
            .map_err(|_| {
                AuthError::AuthorizationFailed("MCP OAuth callback timed out".to_string())
            })?
    }

    async fn receive_inner(self, expected_state: &str) -> Result<CallbackResult, AuthError> {
        let (mut stream, peer) = self
            .listener
            .accept()
            .await
            .map_err(|err| AuthError::InternalError(err.to_string()))?;
        if !matches!(peer.ip(), IpAddr::V4(ip) if ip.is_loopback()) {
            return Err(AuthError::AuthorizationFailed(
                "Rejected non-loopback MCP OAuth callback".to_string(),
            ));
        }

        let mut request = Vec::new();
        loop {
            let mut chunk = [0; 1024];
            let bytes_read = stream
                .read(&mut chunk)
                .await
                .map_err(|err| AuthError::InternalError(err.to_string()))?;
            if bytes_read == 0 {
                break;
            }
            request.extend_from_slice(&chunk[..bytes_read]);
            if request.len() > MAX_REQUEST_BYTES {
                return Err(AuthError::AuthorizationFailed(
                    "MCP OAuth callback request was too large".to_string(),
                ));
            }
            if request.windows(4).any(|window| window == b"\r\n\r\n") {
                break;
            }
        }
        let request = std::str::from_utf8(&request).map_err(|_| {
            AuthError::AuthorizationFailed("Invalid MCP OAuth callback request".to_string())
        })?;
        let request_target = request
            .lines()
            .next()
            .and_then(|line| {
                let mut parts = line.split_whitespace();
                (parts.next() == Some("GET"))
                    .then(|| parts.next())
                    .flatten()
            })
            .ok_or_else(|| {
                AuthError::AuthorizationFailed("Invalid MCP OAuth callback request".to_string())
            })?;
        let callback_url =
            Url::parse(&format!("http://127.0.0.1{request_target}")).map_err(|_| {
                AuthError::AuthorizationFailed("Invalid MCP OAuth callback URL".to_string())
            })?;

        let result = if callback_url.path() != CALLBACK_PATH {
            Err(AuthError::AuthorizationFailed(
                "Invalid MCP OAuth callback path".to_string(),
            ))
        } else {
            let query: std::collections::HashMap<_, _> =
                callback_url.query_pairs().into_owned().collect();
            let state = query.get("state").ok_or_else(|| {
                AuthError::AuthorizationFailed(
                    "MCP OAuth callback did not include state".to_string(),
                )
            })?;
            if state != expected_state {
                Err(AuthError::AuthorizationFailed(
                    "MCP OAuth callback state did not match".to_string(),
                ))
            } else if let Some(code) = query.get("code") {
                Ok(CallbackResult::Success {
                    code: code.clone(),
                    csrf_token: state.clone(),
                })
            } else {
                Ok(CallbackResult::Error {
                    error: query.get("error").cloned(),
                })
            }
        };

        let (status, message) = if result.is_ok() {
            ("200 OK", "Authentication complete. You can return to Warp.")
        } else {
            (
                "400 Bad Request",
                "Authentication failed. Return to Warp for details.",
            )
        };
        let response = format!(
            "HTTP/1.1 {status}\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\n\r\n{message}",
            message.len()
        );
        let _ = stream.write_all(response.as_bytes()).await;
        let _ = stream.shutdown().await;
        result
    }
}
