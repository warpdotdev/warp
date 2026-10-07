use super::*;

#[test]
fn authorize_url_contains_required_params() {
    let pkce = PkceParams::generate();
    let url = authorize_url(&pkce);

    assert!(url.starts_with("https://auth.x.ai/oauth2/authorize?"));
    assert!(url.contains("response_type=code"));
    assert!(url.contains(&format!("client_id={CLIENT_ID}")));
    assert!(url.contains("code_challenge_method=S256"));
    assert!(url.contains("scope=openid"));
    assert!(url.contains("plan=generic"));
    assert!(url.contains("referrer=warp"));
    // The redirect URI must be percent-encoded and match the registered value.
    assert!(url.contains("redirect_uri=http%3A%2F%2F127.0.0.1%3A56121%2Fcallback"));
    assert!(url.contains(&format!("state={}", pkce.state)));
    assert!(url.contains(&format!("code_challenge={}", pkce.challenge)));
}

#[test]
fn token_response_parses_minimal_and_full() {
    let minimal: TokenResponse =
        serde_json::from_str(r#"{"access_token":"abc"}"#).expect("minimal response should parse");
    assert_eq!(minimal.access_token, "abc");
    assert!(minimal.refresh_token.is_none());
    assert!(minimal.expires_in.is_none());

    let full: TokenResponse = serde_json::from_str(
        r#"{"access_token":"a","refresh_token":"r","token_type":"Bearer","expires_in":3600,"scope":"api:access"}"#,
    )
    .expect("full response should parse");
    assert_eq!(full.access_token, "a");
    assert_eq!(full.refresh_token.as_deref(), Some("r"));
    assert_eq!(full.expires_in, Some(3600));
}

#[test]
fn manual_code_exchange_captures_attempt_verifier() {
    let pkce = PkceParams::generate();
    let exchange = ManualCodeExchange {
        verifier: pkce.verifier.clone(),
    };
    assert_eq!(exchange.verifier, pkce.verifier);
}

#[test]
fn manual_code_exchange_rejects_blank_code() {
    let exchange = ManualCodeExchange {
        verifier: "verifier".to_string(),
    };
    let result = warpui_core::r#async::block_on(exchange.exchange("   "));
    assert!(result.is_err());
}

/// Ports below every platform's ephemeral range, so a concurrent test's
/// ephemeral bind can't collide with the exact-port rebinds asserted below.
const TEST_PORT_BASE: u16 = 21121;
const TEST_PORT_SPAN: u16 = 200;

fn bind_test_listener() -> (TcpListener, std::net::SocketAddr) {
    // Scanning from a process-dependent offset keeps two test binaries off
    // each other's ports.
    let offset = (std::process::id() % u32::from(TEST_PORT_SPAN)) as u16;
    for candidate in 0..TEST_PORT_SPAN {
        let port = TEST_PORT_BASE + (offset + candidate) % TEST_PORT_SPAN;
        let Ok(listener) = TcpListener::bind((REDIRECT_HOST, port)) else {
            continue;
        };
        listener
            .set_nonblocking(true)
            .expect("test callback listener should be non-blocking");
        let address = listener
            .local_addr()
            .expect("test callback listener should have an address");
        return (listener, address);
    }
    panic!(
        "no free loopback port in {TEST_PORT_BASE}..{} for the test callback listener",
        TEST_PORT_BASE + TEST_PORT_SPAN
    );
}

/// `run_oauth_flow` must not hand back a result while the callback thread
/// still owns the loopback socket, or an immediate rebind fails with
/// "address in use". Serialized with the other tests here that call
/// [`bind_test_listener`]: they scan the same PID-derived port range.
#[test]
#[serial_test::serial(grok_oauth_loopback_port)]
fn cancelling_loopback_wait_releases_listener() {
    const CANCEL_CYCLES: usize = 100;

    for _ in 0..CANCEL_CYCLES {
        let (listener, address) = bind_test_listener();
        let cancellation = OauthCancellationHandle {
            cancelled: Arc::new(AtomicBool::new(false)),
        };
        cancellation.cancel();

        let (release_tx, _release_rx) = async_channel::bounded(1);
        let result = warpui_core::r#async::block_on(run_oauth_flow(
            listener,
            PkceParams::generate(),
            cancellation,
            release_tx,
        ));

        assert_eq!(
            result
                .expect_err("cancelled callback wait should fail")
                .to_string(),
            "Grok authorization was cancelled"
        );
        TcpListener::bind(address).expect("cancelled callback listener should release its port");
    }
}

/// A caller only needs the release signal, not the full result, to know the
/// port is free again -- proven here by rebinding without ever joining the
/// result thread.
#[test]
#[serial_test::serial(grok_oauth_loopback_port)]
fn release_signal_lets_a_caller_rebind_without_joining_the_result() {
    let (listener, address) = bind_test_listener();
    let cancellation = OauthCancellationHandle {
        cancelled: Arc::new(AtomicBool::new(false)),
    };
    let (release_tx, release_rx) = async_channel::bounded(1);

    let browser = std::thread::spawn(move || {
        let mut stream = TcpStream::connect(address).expect("test browser should connect");
        stream
            .write_all(
                b"GET /callback?code=test-code&state=unexpected-state HTTP/1.1\r\n\
                  Host: 127.0.0.1\r\n\r\n",
            )
            .expect("test browser should send the callback request");
        let mut response = String::new();
        stream
            .read_to_string(&mut response)
            .expect("test browser should read the callback response");
    });

    let result_thread = std::thread::spawn(move || {
        warpui_core::r#async::block_on(run_oauth_flow(
            listener,
            PkceParams::generate(),
            cancellation,
            release_tx,
        ))
    });

    warpui_core::r#async::block_on(release_rx.recv())
        .expect("release signal should fire once the listener is dropped");
    TcpListener::bind(address).expect("released listener should free its port immediately");

    let result = result_thread.join().expect("result future should finish");
    browser.join().expect("test browser thread should finish");
    assert!(
        result
            .expect_err("a mismatched callback state should fail")
            .to_string()
            .contains("state did not match")
    );
}

/// Guards the non-cancelled path: the callback is still delivered after the
/// listener closes.
#[test]
#[serial_test::serial(grok_oauth_loopback_port)]
fn loopback_callback_is_delivered_after_the_listener_closes() {
    let (listener, address) = bind_test_listener();
    let cancellation = OauthCancellationHandle {
        cancelled: Arc::new(AtomicBool::new(false)),
    };

    let browser = std::thread::spawn(move || {
        let mut stream = TcpStream::connect(address).expect("test browser should connect");
        stream
            .write_all(
                b"GET /callback?code=test-code&state=unexpected-state HTTP/1.1\r\n\
                  Host: 127.0.0.1\r\n\r\n",
            )
            .expect("test browser should send the callback request");
        let mut response = String::new();
        stream
            .read_to_string(&mut response)
            .expect("test browser should read the callback response");
    });

    let (release_tx, _release_rx) = async_channel::bounded(1);
    let result = warpui_core::r#async::block_on(run_oauth_flow(
        listener,
        PkceParams::generate(),
        cancellation,
        release_tx,
    ));
    browser.join().expect("test browser thread should finish");

    assert!(
        result
            .expect_err("a mismatched callback state should fail")
            .to_string()
            .contains("state did not match")
    );
}

/// Waits for `listener` (non-blocking) to accept a connection that was just
/// established. Bounded retry is about OS handshake scheduling, not the
/// behavior under test below.
fn accept_test_connection(listener: &TcpListener) -> TcpStream {
    for _ in 0..100 {
        match listener.accept() {
            Ok((stream, _)) => return stream,
            Err(ref e) if e.kind() == ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(1));
            }
            Err(e) => panic!("test listener failed to accept a connection: {e}"),
        }
    }
    panic!("test listener never accepted the connection");
}

/// A connection that's accepted but never sends a request must not hold the
/// read past a poll interval once cancelled, even though the per-connection
/// read timeout (`CALLBACK_READ_TIMEOUT`) is far longer -- otherwise
/// cancelling with such a connection pending parks the row in "Cancelling…"
/// for that long instead of finalizing.
///
/// Runs the read on its own thread against the silent, accepted socket. The
/// reader signals over a channel once it has started (ruling out cancelling
/// before it even runs), and cancellation is then held off for a couple of
/// poll intervals so it lands while the reader is asleep between polls
/// rather than racing its very first, sub-microsecond entry check -- there is
/// no externally observable signal for "the reader has completed one poll",
/// so this margin is sized off `POLL_INTERVAL` itself rather than guessed.
/// Confirmed by deliberately reverting the fix (see the PR discussion) and
/// observing this test fail at the full `CALLBACK_READ_TIMEOUT`.
#[test]
#[serial_test::serial(grok_oauth_loopback_port)]
fn cancelling_with_a_stalled_accepted_connection_still_releases_promptly() {
    let (listener, address) = bind_test_listener();
    let cancellation = OauthCancellationHandle {
        cancelled: Arc::new(AtomicBool::new(false)),
    };

    // Connect without ever sending data, standing in for a stray probe or a
    // browser tab that opened the socket but hasn't sent the request yet.
    let stalled_client = TcpStream::connect(address).expect("stalled client should connect");
    let mut accepted = accept_test_connection(&listener);

    let (started_tx, started_rx) = std::sync::mpsc::channel();
    let reader_cancellation = cancellation.clone();
    let reader = std::thread::spawn(move || {
        let _ = started_tx.send(());
        read_callback_request(&mut accepted, &reader_cancellation)
    });
    started_rx
        .recv()
        .expect("reader thread should signal that it has started");
    std::thread::sleep(POLL_INTERVAL * 2);

    cancellation.cancel();

    let started_waiting = Instant::now();
    let result = reader.join().expect("reader thread should finish");
    let elapsed = started_waiting.elapsed();

    assert_eq!(
        result
            .expect_err("a cancelled read should fail")
            .to_string(),
        "Grok authorization was cancelled"
    );
    assert!(
        elapsed < Duration::from_secs(1),
        "reading a stalled, cancelled connection took {elapsed:?}; expected it to return \
         immediately rather than blocking for CALLBACK_READ_TIMEOUT"
    );
    drop(stalled_client);
}
