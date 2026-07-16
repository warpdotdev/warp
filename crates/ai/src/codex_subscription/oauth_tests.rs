use std::collections::HashMap;
use std::io::{Read as _, Write as _};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::sync::mpsc;

use base64::Engine as _;

use super::*;

#[test]
fn authorize_url_contains_exact_codex_params() {
    let pkce = PkceParams::generate();
    let redirect = redirect_uri(1455);
    let url = authorize_url(&pkce, &redirect);
    let (base, query) = url.split_once('?').expect("authorize URL has a query");
    assert_eq!(base, AUTHORIZE_URL);

    let params: HashMap<String, String> = serde_urlencoded::from_str(query).unwrap();
    assert_eq!(params.len(), 9);
    assert_eq!(
        params.get("response_type").map(String::as_str),
        Some("code")
    );
    assert_eq!(params.get("client_id").map(String::as_str), Some(CLIENT_ID));
    assert_eq!(
        params.get("redirect_uri").map(String::as_str),
        Some(redirect.as_str())
    );
    assert_eq!(params.get("scope").map(String::as_str), Some(SCOPE));
    assert_eq!(
        params.get("code_challenge").map(String::as_str),
        Some(pkce.challenge.as_str())
    );
    assert_eq!(
        params.get("code_challenge_method").map(String::as_str),
        Some("S256")
    );
    assert_eq!(
        params.get("id_token_add_organizations").map(String::as_str),
        Some("true")
    );
    assert_eq!(
        params.get("codex_cli_simplified_flow").map(String::as_str),
        Some("true")
    );
    assert_eq!(
        params.get("state").map(String::as_str),
        Some(pkce.state.as_str())
    );
}

#[test]
fn callback_parses_code_and_state_and_returns_plain_success_html() {
    let (result, response) = invoke_callback(
        "GET /auth/callback?code=code%2Fvalue&state=state-value HTTP/1.1\r\nHost: localhost\r\n\r\n",
        "state-value",
    );
    assert_eq!(
        result.unwrap(),
        Some(CallbackData {
            code: "code/value".to_owned(),
            state: "state-value".to_owned(),
        })
    );
    assert!(response.starts_with("HTTP/1.1 200 OK\r\n"));
    assert!(response.contains("ChatGPT connected"));
    assert!(!response.contains("Access-Control-Allow-"));
}

#[test]
fn callback_provider_error_returns_failure_html() {
    let (result, response) = invoke_callback(
        "GET /auth/callback?error=access_denied&error_description=No%20thanks HTTP/1.1\r\nHost: localhost\r\n\r\n",
        "unused-for-provider-error",
    );
    assert_eq!(
        result.unwrap_err().to_string(),
        "Codex authorization was denied or failed: No thanks"
    );
    assert!(response.starts_with("HTTP/1.1 400 Bad Request\r\n"));
    assert!(response.contains("Authorization failed"));
    assert!(!response.contains("Access-Control-Allow-"));
}

#[test]
fn csrf_state_mismatch_returns_failure_html() {
    let (result, response) = invoke_callback(
        "GET /auth/callback?code=code&state=attacker-state HTTP/1.1\r\nHost: localhost\r\n\r\n",
        "expected-state",
    );
    assert!(result.unwrap_err().to_string().contains("prevent CSRF"));
    assert!(response.starts_with("HTTP/1.1 400 Bad Request\r\n"));
    assert!(response.contains("Authorization failed"));
    assert!(!response.contains("ChatGPT connected"));
}

#[test]
fn callback_listener_falls_back_to_second_port() {
    assert_eq!(REDIRECT_PORTS, [1455, 1457]);

    let first = TcpListener::bind((REDIRECT_HOST, 0)).unwrap();
    let first_port = first.local_addr().unwrap().port();
    let second = TcpListener::bind((REDIRECT_HOST, 0)).unwrap();
    let second_port = second.local_addr().unwrap().port();
    drop(second);

    let (listener, selected) = bind_callback_listener_on_ports(&[first_port, second_port]).unwrap();
    assert_eq!(selected, second_port);
    assert_eq!(listener.local_addr().unwrap().port(), second_port);
}

#[test]
fn cancellation_interrupts_an_accepted_idle_callback_and_releases_the_port() {
    let listener = TcpListener::bind((REDIRECT_HOST, 0)).unwrap();
    let address = listener.local_addr().unwrap();
    let idle_client = TcpStream::connect(address).unwrap();
    let (server_stream, _) = listener.accept().unwrap();
    drop(listener);

    let cancel_handle = OauthCancelHandle::new();
    let callback_cancel_handle = cancel_handle.clone();
    let callback = std::thread::spawn(move || {
        handle_callback_connection_with_cancel(
            server_stream,
            "expected-state",
            Some(&callback_cancel_handle),
        )
    });

    cancel_handle.cancel();
    let error = callback.join().unwrap().unwrap_err();
    assert!(error.to_string().contains("cancelled"));
    drop(idle_client);

    TcpListener::bind(address).expect("cancelled callback released its loopback port");
}

#[test]
fn token_exchange_posts_form_to_local_server() {
    let response =
        r#"{"id_token":"id","access_token":"access","refresh_token":"refresh","expires_in":3600}"#;
    let (url, request_rx, server) = spawn_http_server("200 OK", response);
    let redirect = redirect_uri(1457);
    let tokens = warpui_core::r#async::block_on(exchange_code_for_tokens_at(
        "auth/code",
        "verifier",
        &redirect,
        &url,
    ))
    .unwrap();
    assert_eq!(tokens.id_token.as_deref(), Some("id"));
    assert_eq!(tokens.access_token, "access");
    assert_eq!(tokens.refresh_token.as_deref(), Some("refresh"));
    assert_eq!(tokens.expires_in, Some(3600));

    let request = request_rx.recv().unwrap();
    assert!(request.starts_with("POST /oauth/token HTTP/1.1\r\n"));
    assert!(request
        .to_ascii_lowercase()
        .contains("content-type: application/x-www-form-urlencoded"));
    let body = request.split_once("\r\n\r\n").unwrap().1;
    let form: HashMap<String, String> = serde_urlencoded::from_str(body).unwrap();
    assert_eq!(
        form.get("grant_type").map(String::as_str),
        Some("authorization_code")
    );
    assert_eq!(form.get("code").map(String::as_str), Some("auth/code"));
    assert_eq!(
        form.get("redirect_uri").map(String::as_str),
        Some(redirect.as_str())
    );
    assert_eq!(form.get("client_id").map(String::as_str), Some(CLIENT_ID));
    assert_eq!(
        form.get("code_verifier").map(String::as_str),
        Some("verifier")
    );
    server.join().unwrap();
}

#[test]
fn refresh_posts_json_while_authorization_exchange_remains_form_encoded() {
    let response = r#"{"id_token":"id2","access_token":"access2","refresh_token":"refresh2"}"#;
    let (url, request_rx, server) = spawn_http_server("200 OK", response);
    let tokens =
        warpui_core::r#async::block_on(refresh_access_token_at("old-refresh", &url)).unwrap();
    assert_eq!(tokens.access_token, "access2");
    assert_eq!(tokens.expires_in, None);

    let request = request_rx.recv().unwrap();
    assert!(request
        .to_ascii_lowercase()
        .contains("content-type: application/json"));
    let body = request.split_once("\r\n\r\n").unwrap().1;
    let json: Value = serde_json::from_str(body).unwrap();
    assert_eq!(json["client_id"], CLIENT_ID);
    assert_eq!(json["grant_type"], "refresh_token");
    assert_eq!(json["refresh_token"], "old-refresh");
    server.join().unwrap();
}

#[test]
fn refresh_token_revoke_posts_openai_json_shape() {
    let (url, request_rx, server) = spawn_http_server("200 OK", "");
    warpui_core::r#async::block_on(revoke_token_at(
        "refresh-value",
        "refresh_token",
        Some(CLIENT_ID),
        &url,
    ))
    .unwrap();

    let request = request_rx.recv().unwrap();
    let body = request.split_once("\r\n\r\n").unwrap().1;
    let json: Value = serde_json::from_str(body).unwrap();
    assert_eq!(json["token"], "refresh-value");
    assert_eq!(json["token_type_hint"], "refresh_token");
    assert_eq!(json["client_id"], CLIENT_ID);
    server.join().unwrap();
}

#[test]
fn token_and_revoke_errors_expose_only_parsed_oauth_fields() {
    const SENTINEL: &str = "never-log-this-secret";
    let token_body = r#"{"error":"invalid_grant","error_description":"grant expired","internal":"never-log-this-secret"}"#;
    let (token_url, _token_request_rx, token_server) =
        spawn_http_server("400 Bad Request", token_body);
    let token_error =
        warpui_core::r#async::block_on(refresh_access_token_at("old-refresh", &token_url))
            .unwrap_err()
            .to_string();
    assert!(token_error.contains("400"));
    assert!(token_error.contains("invalid_grant: grant expired"));
    assert!(!token_error.contains(SENTINEL));
    assert!(!token_error.contains(token_body));
    token_server.join().unwrap();

    let revoke_body =
        r#"{"error":"invalid_token","error_description":"already revoked","debug":"never-log-this-secret"}"#;
    let (revoke_url, _revoke_request_rx, revoke_server) =
        spawn_http_server("401 Unauthorized", revoke_body);
    let revoke_error = warpui_core::r#async::block_on(revoke_token_at(
        "refresh-value",
        "refresh_token",
        Some(CLIENT_ID),
        &revoke_url,
    ))
    .unwrap_err()
    .to_string();
    assert!(revoke_error.contains("401"));
    assert!(revoke_error.contains("invalid_token: already revoked"));
    assert!(!revoke_error.contains(SENTINEL));
    assert!(!revoke_error.contains(revoke_body));
    revoke_server.join().unwrap();
}

#[test]
fn extracts_chatgpt_account_id_from_synthetic_jwt() {
    let header = URL_SAFE_NO_PAD.encode(br#"{"alg":"none"}"#);
    let payload = URL_SAFE_NO_PAD
        .encode(br#"{"https://api.openai.com/auth":{"chatgpt_account_id":"account-123"}}"#);
    let token = format!("{header}.{payload}.signature");
    assert_eq!(
        chatgpt_account_id_from_id_token(&token).unwrap(),
        "account-123"
    );
}

#[test]
fn rejects_id_token_without_account_claim() {
    let header = URL_SAFE_NO_PAD.encode(br#"{"alg":"none"}"#);
    let payload = URL_SAFE_NO_PAD.encode(br#"{"sub":"user"}"#);
    let token = format!("{header}.{payload}.signature");
    assert!(chatgpt_account_id_from_id_token(&token)
        .unwrap_err()
        .to_string()
        .contains("chatgpt_account_id"));
}

fn invoke_callback(
    request: &'static str,
    expected_state: &'static str,
) -> (anyhow::Result<Option<CallbackData>>, String) {
    let listener = TcpListener::bind((REDIRECT_HOST, 0)).unwrap();
    let address = listener.local_addr().unwrap();
    let client = std::thread::spawn(move || {
        let mut stream = TcpStream::connect(address).unwrap();
        stream.write_all(request.as_bytes()).unwrap();
        stream.shutdown(Shutdown::Write).unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        response
    });
    let (stream, _) = listener.accept().unwrap();
    let result = handle_callback_connection(stream, expected_state);
    (result, client.join().unwrap())
}

fn spawn_http_server(
    status: &'static str,
    body: &'static str,
) -> (String, mpsc::Receiver<String>, std::thread::JoinHandle<()>) {
    let listener = TcpListener::bind((REDIRECT_HOST, 0)).unwrap();
    let address = listener.local_addr().unwrap();
    let (request_tx, request_rx) = mpsc::channel();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let request = read_http_request(&mut stream);
        request_tx.send(request).unwrap();
        let response = format!(
            "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        stream.write_all(response.as_bytes()).unwrap();
        stream.flush().unwrap();
    });
    (format!("http://{address}/oauth/token"), request_rx, server)
}

fn read_http_request(stream: &mut TcpStream) -> String {
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let mut bytes = Vec::new();
    let mut chunk = [0u8; 1024];
    let header_end = loop {
        let count = stream.read(&mut chunk).unwrap();
        assert!(count > 0, "client closed before completing request headers");
        bytes.extend_from_slice(&chunk[..count]);
        if let Some(index) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
            break index + 4;
        }
    };
    let headers = String::from_utf8_lossy(&bytes[..header_end]);
    let content_length = headers
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse::<usize>().unwrap())
        })
        .unwrap_or(0);
    while bytes.len() < header_end + content_length {
        let count = stream.read(&mut chunk).unwrap();
        assert!(count > 0, "client closed before completing request body");
        bytes.extend_from_slice(&chunk[..count]);
    }
    String::from_utf8(bytes).unwrap()
}
