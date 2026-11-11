use base64::Engine as _;
use chrono::TimeDelta;
use opentelemetry::trace::{Span as _, Tracer as _, TracerProvider as _};
use opentelemetry_sdk::error::OTelSdkResult;
use opentelemetry_sdk::trace::{SpanData, SpanExporter};

use super::*;
#[cfg(not(feature = "skip_login"))]
use crate::auth::AuthStateProvider;
#[cfg(not(feature = "skip_login"))]
use crate::auth::credentials::Credentials;
#[cfg(not(feature = "skip_login"))]
use crate::server::server_api::ServerApiProvider;
#[cfg(not(feature = "skip_login"))]
use crate::test_util::terminal::initialize_app_for_terminal_view;

fn jwt_with_payload(payload: serde_json::Value) -> String {
    let encoder = base64::engine::general_purpose::URL_SAFE_NO_PAD;
    let header = encoder.encode(br#"{"alg":"none"}"#);
    let payload = encoder.encode(serde_json::to_vec(&payload).unwrap());
    format!("{header}.{payload}.test-signature")
}

#[test]
fn snapshot_uses_current_token_with_its_actual_expiry() {
    let context = AuthContext::from_snapshot(CredentialSnapshot {
        token: "dispatch-test-token".to_owned(),
        expires_at: Utc::now() + TimeDelta::hours(3),
    })
    .unwrap();
    let expires_at = Utc::now() + TimeDelta::minutes(17);
    context
        .token_store
        .replace("refreshed-test-token".to_owned(), expires_at)
        .unwrap();

    let snapshot = context.snapshot().unwrap();
    assert_eq!(snapshot.token, "refreshed-test-token");
    assert_eq!(snapshot.expires_at, expires_at);
    assert!(!format!("{snapshot:?}").contains("refreshed-test-token"));
    let child = AuthContext::from_snapshot(snapshot).unwrap();
    assert_eq!(
        child.token_store.valid_authorization_header().unwrap(),
        "Bearer refreshed-test-token"
    );
    assert_eq!(child.snapshot().unwrap().expires_at, expires_at);
}

#[test]
fn snapshot_refuses_expired_credentials() {
    let snapshot = CredentialSnapshot {
        token: "expired-test-token".to_owned(),
        expires_at: Utc::now() - TimeDelta::minutes(1),
    };
    let store = TokenStore::new(snapshot.token.clone(), snapshot.expires_at).unwrap();
    assert!(store.valid_snapshot().is_none());
    assert!(AuthContext::from_snapshot(snapshot).is_err());
}

#[test]
fn concurrent_refresh_cannot_mix_snapshot_token_and_expiry() {
    let first_expiry = Utc::now() + TimeDelta::minutes(20);
    let second_expiry = Utc::now() + TimeDelta::minutes(40);
    let store = TokenStore::new("first-test-token".to_owned(), first_expiry).unwrap();
    std::thread::scope(|scope| {
        scope.spawn(|| {
            for _ in 0..1000 {
                store
                    .replace("second-test-token".to_owned(), second_expiry)
                    .unwrap();
                store
                    .replace("first-test-token".to_owned(), first_expiry)
                    .unwrap();
            }
        });
        for _ in 0..1000 {
            let snapshot = store.valid_snapshot().unwrap();
            match snapshot.token.as_str() {
                "first-test-token" => assert_eq!(snapshot.expires_at, first_expiry),
                "second-test-token" => assert_eq!(snapshot.expires_at, second_expiry),
                _ => panic!("Unexpected snapshot credential"),
            }
        }
    });
}

#[test]
#[cfg(not(feature = "skip_login"))]
fn policy_disable_cancels_scheduled_refresh() {
    warpui::App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let auth = AuthContext::from_snapshot(CredentialSnapshot {
            token: "standby-token".into(),
            expires_at: Utc::now() + TimeDelta::hours(1),
        })
        .unwrap();
        app.update(|ctx| {
            let client = ServerApiProvider::as_ref(ctx).get_managed_secrets_client();
            start_refresh_coordinator(auth.clone(), client, ctx);
            AuthRefreshCoordinator::handle(ctx).update(ctx, |coordinator, ctx| {
                coordinator.stop();
                coordinator.schedule_proactive_refresh(Utc::now() + TimeDelta::hours(1), ctx);
                assert!(coordinator.scheduled_refresh.is_some());
            });

            auth.disable();
            stop_refresh_coordinator(&auth, ctx);

            AuthRefreshCoordinator::handle(ctx).update(ctx, |coordinator, ctx| {
                coordinator.start_refresh(ctx);
                assert!(coordinator.scheduled_refresh.is_none());
                assert!(coordinator.refresh_request.is_none());
            });
            assert!(auth.snapshot().is_none());
        });
    });
}

fn client_with_expiry(token: &str, expires_at: DateTime<Utc>) -> AuthenticatedHttpClient {
    let (refresh_hint_sender, _) = async_channel::bounded(1);
    AuthenticatedHttpClient {
        inner: reqwest::Client::new(),
        token_store: TokenStore::new(token.to_owned(), expires_at).unwrap(),
        refresh_hint_sender,
    }
}

#[test]
fn authorization_overwrites_supplied_header() {
    let client = client_with_expiry(
        "current-test-token",
        Utc::now() + TimeDelta::try_minutes(5).unwrap(),
    );
    let mut request = Request::builder()
        .header(AUTHORIZATION, "Bearer stale-test-token")
        .body(Bytes::new())
        .unwrap();

    client.authorize_request(&mut request).unwrap();

    assert_eq!(
        request.headers().get(AUTHORIZATION).unwrap(),
        "Bearer current-test-token"
    );
}

#[test]
fn expired_token_is_refused_and_supplied_header_is_removed() {
    let client = client_with_expiry(
        "expired-test-token",
        Utc::now() - TimeDelta::try_minutes(5).unwrap(),
    );
    let mut request = Request::builder()
        .header(AUTHORIZATION, "Bearer stale-test-token")
        .body(Bytes::new())
        .unwrap();

    assert!(matches!(
        client.authorize_request(&mut request),
        Err(AuthenticatedHttpError::NoValidToken)
    ));
    assert!(!request.headers().contains_key(AUTHORIZATION));
}

#[test]
fn debug_output_redacts_token() {
    let client = client_with_expiry(
        "secret-test-token",
        Utc::now() + TimeDelta::try_minutes(5).unwrap(),
    );

    let debug_output = format!("{client:?}");

    assert!(!debug_output.contains("secret-test-token"));
    assert!(debug_output.contains("expires_at"));
}

#[test]
fn authorized_request_debug_redacts_token() {
    let client = client_with_expiry(
        "secret-request-test-token",
        Utc::now() + TimeDelta::try_minutes(5).unwrap(),
    );
    let mut request = Request::builder().body(Bytes::new()).unwrap();

    client.authorize_request(&mut request).unwrap();
    let request_debug = format!("{request:?}");
    let headers_debug = format!("{:?}", request.headers());

    assert!(!request_debug.contains("secret-request-test-token"));
    assert!(!headers_debug.contains("secret-request-test-token"));
    assert!(request_debug.contains("Sensitive"));
    assert!(headers_debug.contains("Sensitive"));
}

#[test]
fn refreshed_token_run_id_exactly_matches() {
    let token = jwt_with_payload(serde_json::json!({ "run_id": "expected-run-id" }));

    validate_refreshed_token_identity(
        &token,
        Some(&ExpectedIdentity::Run("expected-run-id".into())),
    )
    .unwrap();
}

#[test]
fn refreshed_token_run_id_is_required() {
    let token = jwt_with_payload(serde_json::json!({}));
    assert!(
        validate_refreshed_token_identity(
            &token,
            Some(&ExpectedIdentity::Run("expected-run-id".into()))
        )
        .is_err()
    );
}

#[test]
fn expected_run_id_is_required() {
    let token = jwt_with_payload(serde_json::json!({ "run_id": "expected-run-id" }));

    assert!(validate_refreshed_token_identity(&token, None).is_err());
    assert!(
        validate_refreshed_token_identity(&token, Some(&ExpectedIdentity::Run("".into()))).is_err()
    );
}

#[test]
fn refreshed_token_run_id_must_match() {
    let token = jwt_with_payload(serde_json::json!({ "run_id": "wrong-run-id" }));

    assert!(
        validate_refreshed_token_identity(
            &token,
            Some(&ExpectedIdentity::Run("expected-run-id".into()))
        )
        .is_err()
    );
}

#[test]
fn refreshed_token_run_id_must_be_a_string() {
    let token = jwt_with_payload(serde_json::json!({ "run_id": 123 }));
    assert!(
        validate_refreshed_token_identity(
            &token,
            Some(&ExpectedIdentity::Run("expected-run-id".into()))
        )
        .is_err()
    );
}

#[test]
fn malformed_refreshed_tokens_are_rejected() {
    let invalid_json = {
        let encoder = base64::engine::general_purpose::URL_SAFE_NO_PAD;
        let payload = encoder.encode(b"not-json");
        format!("header.{payload}.signature")
    };

    for token in ["not-a-jwt", "header.!!!.signature", &invalid_json] {
        assert!(
            validate_refreshed_token_identity(
                token,
                Some(&ExpectedIdentity::Run("expected-run-id".into()))
            )
            .is_err()
        );
    }
}

#[test]
fn rejected_refreshed_token_preserves_previous_token() {
    let token_store = TokenStore::new(
        "current-test-token".to_owned(),
        Utc::now() + TimeDelta::try_minutes(5).unwrap(),
    )
    .unwrap();
    let wrong_run_token = jwt_with_payload(serde_json::json!({ "run_id": "wrong-run-id" }));

    assert!(
        token_store
            .replace_refreshed(
                wrong_run_token,
                Utc::now() + TimeDelta::try_minutes(5).unwrap(),
                Some(&ExpectedIdentity::Run("expected-run-id".into())),
            )
            .is_err()
    );
    assert_eq!(
        token_store.valid_authorization_header().unwrap(),
        "Bearer current-test-token"
    );
}

#[test]
fn standby_refresh_requires_matching_sandbox_identity() {
    let expected = ExpectedIdentity::Standby("sandbox-id".into());
    let matching = jwt_with_payload(serde_json::json!({ "sandbox_id": "sandbox-id" }));
    let mismatched = jwt_with_payload(serde_json::json!({ "sandbox_id": "other-sandbox" }));
    let run_only = jwt_with_payload(serde_json::json!({ "run_id": "sandbox-id" }));

    validate_refreshed_token_identity(&matching, Some(&expected)).unwrap();
    assert!(validate_refreshed_token_identity(&mismatched, Some(&expected)).is_err());
    assert!(validate_refreshed_token_identity(&run_only, Some(&expected)).is_err());
}

#[test]
fn startup_identity_prefers_run_and_falls_back_to_standby() {
    assert_eq!(
        ExpectedIdentity::from_ids(Some("run-id".into()), Some("sandbox-id".into())),
        Some(ExpectedIdentity::Run("run-id".into()))
    );
    assert_eq!(
        ExpectedIdentity::from_ids(Some(" ".into()), Some("sandbox-id".into())),
        Some(ExpectedIdentity::Standby("sandbox-id".into()))
    );
    assert_eq!(ExpectedIdentity::from_ids(None, None), None);
}

#[derive(Debug)]
struct CredentialRecordingExporter {
    store: TokenStore,
    exported: Arc<Mutex<Vec<(String, String)>>>,
}

impl SpanExporter for CredentialRecordingExporter {
    async fn export(&self, batch: Vec<SpanData>) -> OTelSdkResult {
        let credential = self.store.valid_snapshot().unwrap();
        self.exported.lock().unwrap().extend(
            batch
                .into_iter()
                .map(|span| (span.name.into_owned(), credential.token.clone())),
        );
        Ok(())
    }
}

#[test]
fn assignment_flushes_standby_spans_before_switching_export_and_refresh_identity() {
    let standby_token = jwt_with_payload(serde_json::json!({ "sandbox_id": "sandbox-id" }));
    let auth = AuthContext::from_snapshot(CredentialSnapshot {
        token: standby_token.clone(),
        expires_at: Utc::now() + TimeDelta::hours(1),
    })
    .unwrap();
    *auth.expected_identity.write().unwrap() = Some(ExpectedIdentity::Standby("sandbox-id".into()));
    let exported = Arc::new(Mutex::new(Vec::new()));
    let provider = SdkTracerProvider::builder()
        .with_batch_exporter(CredentialRecordingExporter {
            store: auth.token_store.clone(),
            exported: exported.clone(),
        })
        .build();
    let tracer = provider.tracer("assignment-test");
    tracer.start("standby").end();
    let run_token = jwt_with_payload(serde_json::json!({ "run_id": "run-id" }));
    let expires_at = Utc::now() + TimeDelta::minutes(45);
    let mut runtime_env = HashMap::from([
        (CLOUD_AGENT_OTLP_TOKEN.into(), run_token.clone()),
        (
            CLOUD_AGENT_OTLP_TOKEN_EXPIRES_AT.into(),
            expires_at.to_rfc3339(),
        ),
        (OZ_RUN_ID.into(), "run-id".into()),
    ]);

    auth.assign_run(
        AssignmentCredential::take(&mut runtime_env).unwrap(),
        &provider,
    )
    .unwrap();

    assert!(!runtime_env.contains_key(CLOUD_AGENT_OTLP_TOKEN));
    let snapshot = auth.snapshot().unwrap();
    assert_eq!(snapshot.token, run_token);
    assert_eq!(snapshot.expires_at, expires_at);
    let expected = auth.expected_identity.read().unwrap();
    validate_refreshed_token_identity(&run_token, expected.as_ref()).unwrap();
    assert!(validate_refreshed_token_identity(&standby_token, expected.as_ref()).is_err());
    drop(expected);
    tracer.start("run").end();
    provider.force_flush().unwrap();
    assert_eq!(
        *exported.lock().unwrap(),
        [("standby".into(), standby_token), ("run".into(), run_token),]
    );
    provider.shutdown().unwrap();
}

#[test]
#[cfg(not(feature = "skip_login"))]
fn active_exporter_distinguishes_invalid_credentials_from_policy_disable() {
    // The global subscriber and exporter require a process isolated from all other tests.
    if std::env::var_os("WARP_TEST_OTLP_INVALID_ASSIGNMENT").is_none() {
        let output = command::blocking::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "tracing::cloud_agent_auth::tests::active_exporter_distinguishes_invalid_credentials_from_policy_disable",
                "--nocapture",
            ])
            .env("WARP_TEST_OTLP_INVALID_ASSIGNMENT", "1")
            .env("WARP_CLOUD_AGENT_OTLP_ENDPOINT", "http://127.0.0.1:1")
            .env(CLOUD_AGENT_OTLP_TOKEN, "standby-token")
            .env(CLOUD_AGENT_OTLP_TOKEN_EXPIRES_AT, (Utc::now() + TimeDelta::hours(1)).to_rfc3339())
            .env(PREWARMED_SANDBOX_ID, "sandbox-id")
            .env_remove(OZ_RUN_ID)
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            output.status.success() && stdout.contains("1 passed"),
            "{stdout}{}",
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }

    let _initialization = crate::tracing::init().unwrap();
    warpui::App::test((), |mut app| async move {
        let original = crate::tracing::native::child_process_snapshot()
            .unwrap()
            .credential;
        let run_token = jwt_with_payload(serde_json::json!({ "run_id": "run-id" }));
        let wrong_run = jwt_with_payload(serde_json::json!({ "run_id": "other-run" }));
        let future = (Utc::now() + TimeDelta::hours(1)).to_rfc3339();
        let expired = (Utc::now() - TimeDelta::hours(1)).to_rfc3339();
        let cases = [
            (
                "empty credentials without policy",
                Some(""),
                Some(""),
                Some("run-id"),
            ),
            ("missing token", None, Some(future.as_str()), Some("run-id")),
            (
                "missing expiry",
                Some(run_token.as_str()),
                None,
                Some("run-id"),
            ),
            (
                "invalid expiry",
                Some(run_token.as_str()),
                Some("invalid-expiry"),
                Some("run-id"),
            ),
            (
                "non-UTC expiry",
                Some(run_token.as_str()),
                Some("2100-01-01T01:00:00+01:00"),
                Some("run-id"),
            ),
            (
                "missing run",
                Some(run_token.as_str()),
                Some(future.as_str()),
                None,
            ),
            (
                "empty run",
                Some(run_token.as_str()),
                Some(future.as_str()),
                Some(" "),
            ),
            (
                "expired token",
                Some(run_token.as_str()),
                Some(expired.as_str()),
                Some("run-id"),
            ),
            (
                "wrong run",
                Some(wrong_run.as_str()),
                Some(future.as_str()),
                Some("run-id"),
            ),
        ];
        initialize_app_for_terminal_view(&mut app);
        for (name, token, expiry, run_id) in cases {
            let mut runtime_env = HashMap::new();
            runtime_env.extend(token.map(|token| (CLOUD_AGENT_OTLP_TOKEN.into(), token.into())));
            runtime_env.extend(
                expiry.map(|expiry| (CLOUD_AGENT_OTLP_TOKEN_EXPIRES_AT.into(), expiry.into())),
            );
            runtime_env.extend(run_id.map(|run_id| (OZ_RUN_ID.into(), run_id.into())));
            app.update(|ctx| {
                let client = ServerApiProvider::as_ref(ctx).get_managed_secrets_client();
                crate::tracing::assign_run_credentials(&mut runtime_env, client, ctx);
            });
            assert!(!runtime_env.contains_key(CLOUD_AGENT_OTLP_TOKEN), "{name}");
            assert!(
                !runtime_env.contains_key(CLOUD_AGENT_OTLP_TOKEN_EXPIRES_AT),
                "{name}"
            );
            let snapshot = crate::tracing::native::child_process_snapshot()
                .unwrap()
                .credential;
            assert_eq!(snapshot.token, original.token, "{name}");
            assert_eq!(snapshot.expires_at, original.expires_at, "{name}");
        }
        app.update(|ctx| {
            let client = ServerApiProvider::as_ref(ctx).get_managed_secrets_client();
            crate::tracing::native::start_auth_refresh(client.clone(), ctx);
            let generation = AuthRefreshCoordinator::as_ref(ctx).generation;
            let mut runtime_env = HashMap::from([
                ("WARP_CLOUD_AGENT_OTLP_ENDPOINT".into(), String::new()),
                (CLOUD_AGENT_OTLP_TOKEN.into(), String::new()),
                (CLOUD_AGENT_OTLP_TOKEN_EXPIRES_AT.into(), String::new()),
            ]);

            crate::tracing::assign_run_credentials(&mut runtime_env, client, ctx);

            assert!(crate::tracing::native::child_process_snapshot().is_none());
            assert!(!runtime_env.contains_key(CLOUD_AGENT_OTLP_TOKEN));
            assert!(!runtime_env.contains_key(CLOUD_AGENT_OTLP_TOKEN_EXPIRES_AT));
            AuthRefreshCoordinator::handle(ctx).update(ctx, |coordinator, ctx| {
                coordinator.start_refresh(ctx);
                coordinator.finish_refresh(
                    generation,
                    Ok(TaskIdentityToken {
                        token: jwt_with_payload(serde_json::json!({ "sandbox_id": "sandbox-id" })),
                        expires_at: Utc::now() + TimeDelta::hours(1),
                        issuer: "test-issuer".into(),
                    }),
                    ctx,
                );
                assert!(
                    coordinator
                        .token_store
                        .valid_authorization_header()
                        .is_none()
                );
                assert!(coordinator.expected_identity.read().unwrap().is_none());
                assert!(coordinator.refresh_request.is_none());
                assert!(coordinator.scheduled_refresh.is_none());
                assert_eq!(coordinator.consecutive_failures, 0);
            });
        });
    });
}

#[test]
#[cfg(not(feature = "skip_login"))]
fn assignment_rearms_refresh_with_run_api_credential() {
    // Workload identity selects its platform from process environment and caches platform state.
    if std::env::var_os("WARP_TEST_OTLP_ASSIGNMENT").is_none() {
        let output = command::blocking::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "tracing::cloud_agent_auth::tests::assignment_rearms_refresh_with_run_api_credential",
                "--nocapture",
            ])
            .env("WARP_TEST_OTLP_ASSIGNMENT", "1")
            .env("WARP_ISOLATION_PLATFORM", "docker")
            .env("WARP_WORKLOAD_TOKEN", "test-workload-token")
            .env_remove(CLOUD_AGENT_OTLP_TOKEN)
            .output().unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            output.status.success() && stdout.contains("1 passed"),
            "{stdout}{}",
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }
    let (sent, received) = async_channel::bounded(1);
    let run_token = jwt_with_payload(serde_json::json!({ "run_id": "run-id" }));
    let mut server = crate::ChannelState::mock_server();
    let response = serde_json::json!({
        "data": { "issueTaskIdentityToken": {
            "__typename": "IssueTaskIdentityTokenOutput",
            "token": run_token,
            "expiresAt": (Utc::now() + TimeDelta::hours(1)).to_rfc3339(),
            "issuer": "test-issuer",
        } }
    })
    .to_string();
    let refresh = server
        .mock("POST", "/graphql/v2?op=IssueTaskIdentityToken")
        .match_header("authorization", "Bearer assigned-run-api-key")
        .with_body_from_request(move |_| {
            sent.try_send(()).unwrap();
            response.clone().into_bytes()
        })
        .expect(1)
        .create();
    // App initialization installs other mocks on this process-global server.
    drop(server);

    warpui::App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        app.add_singleton_model(|_| {
            warp_server_client::network_logging::NetworkLogModel::default()
        });
        let state = app.read(|ctx| AuthStateProvider::as_ref(ctx).get().clone());
        ServerApiProvider::handle(&app).update(&mut app, |provider, ctx| {
            *provider = ServerApiProvider::new(state.clone(), None, None, ctx);
        });
        let auth = AuthContext::from_snapshot(CredentialSnapshot {
            token: "standby-token".into(),
            expires_at: Utc::now() + TimeDelta::hours(1),
        })
        .unwrap();
        let provider = SdkTracerProvider::builder().build();
        app.update(|ctx| {
            let client = ServerApiProvider::as_ref(ctx).get_managed_secrets_client();
            start_refresh_coordinator(auth.clone(), client.clone(), ctx);
            let standby_generation = AuthRefreshCoordinator::as_ref(ctx).generation;
            state.set_credentials(Some(Credentials::ApiKey {
                key: "assigned-run-api-key".into(),
                owner_type: None,
            }));
            auth.assign_run(
                AssignmentCredential {
                    snapshot: CredentialSnapshot {
                        token: run_token.clone(),
                        expires_at: Utc::now() + TimeDelta::hours(1),
                    },
                    run_id: "run-id".into(),
                },
                &provider,
            )
            .unwrap();
            restart_refresh_coordinator(auth.clone(), client, ctx);
            AuthRefreshCoordinator::handle(ctx).update(ctx, |coordinator, ctx| {
                // A standby response may already be waiting for its main-thread callback.
                coordinator.finish_refresh(
                    standby_generation,
                    Ok(TaskIdentityToken {
                        token: jwt_with_payload(serde_json::json!({ "sandbox_id": "sandbox-id" })),
                        expires_at: Utc::now() + TimeDelta::hours(1),
                        issuer: "test-issuer".into(),
                    }),
                    ctx,
                );
                assert_eq!(auth.snapshot().unwrap().token, run_token);
                assert_eq!(coordinator.consecutive_failures, 0);
                assert!(coordinator.refresh_request.is_some());
            });
        });
        received
            .recv()
            .with_timeout(Duration::from_secs(10))
            .await
            .unwrap()
            .unwrap();
    });
    refresh.assert();
}
