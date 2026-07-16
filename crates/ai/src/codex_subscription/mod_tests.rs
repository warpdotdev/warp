use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use warpui_core::App;

use super::*;

fn id_token(account_id: &str) -> String {
    let header = URL_SAFE_NO_PAD.encode(br#"{"alg":"none"}"#);
    let claims = serde_json::json!({
        "https://api.openai.com/auth": {
            "chatgpt_account_id": account_id
        }
    });
    let payload = URL_SAFE_NO_PAD.encode(serde_json::to_vec(&claims).unwrap());
    format!("{header}.{payload}.signature")
}

#[test]
fn known_expiry_refreshes_five_minutes_early() {
    assert_eq!(refresh_delay(Some(60 * 60)), Duration::from_secs(55 * 60));
}

#[test]
fn near_expiry_refreshes_immediately() {
    assert_eq!(refresh_delay(Some(60)), Duration::ZERO);
}

#[test]
fn missing_expiry_refreshes_after_twenty_four_hours() {
    assert_eq!(refresh_delay(None), Duration::from_secs(24 * 60 * 60));
}

#[test]
fn token_response_builds_codex_tokens_and_extracts_account_id() {
    let token = id_token("account-new");
    let stored = codex_tokens_from_response(
        TokenResponse {
            id_token: Some(token.clone()),
            access_token: "access-new".into(),
            refresh_token: Some("refresh-new".into()),
            expires_in: Some(3600),
        },
        None,
    )
    .unwrap();

    assert_eq!(stored.access_token, "access-new");
    assert_eq!(stored.refresh_token.as_deref(), Some("refresh-new"));
    assert_eq!(stored.id_token.as_deref(), Some(token.as_str()));
    assert_eq!(stored.chatgpt_account_id, "account-new");
    assert!(stored.expires_at.is_some());
    assert!(stored.connected_at.is_some());
}

#[test]
fn refresh_response_carries_forward_optional_identity_and_refresh_fields() {
    let connected_at = SystemTime::now() - Duration::from_secs(60);
    let previous = CodexTokens {
        access_token: "access-old".into(),
        refresh_token: Some("refresh-old".into()),
        id_token: Some(id_token("account-old")),
        chatgpt_account_id: "account-old".into(),
        expires_at: None,
        connected_at: Some(connected_at),
    };
    let stored = codex_tokens_from_response(
        TokenResponse {
            id_token: None,
            access_token: "access-new".into(),
            refresh_token: None,
            expires_in: None,
        },
        Some(&previous),
    )
    .unwrap();

    assert_eq!(stored.refresh_token.as_deref(), Some("refresh-old"));
    assert_eq!(stored.id_token, previous.id_token);
    assert_eq!(stored.chatgpt_account_id, "account-old");
    assert_eq!(stored.connected_at, Some(connected_at));
    assert_eq!(stored.expires_at, None);
}

#[cfg(not(target_family = "wasm"))]
#[test]
fn stale_refresh_success_cannot_restore_or_overwrite_tokens() {
    for replacement in [
        None,
        Some(CodexTokens {
            access_token: "replacement-access".into(),
            refresh_token: Some("replacement-refresh".into()),
            id_token: Some(id_token("replacement-account")),
            chatgpt_account_id: "replacement-account".into(),
            expires_at: None,
            connected_at: None,
        }),
    ] {
        App::test((), |mut app| async move {
            app.update(|ctx| {
                warpui_extras::secure_storage::register_noop("test", ctx);
            });
            let manager = app.add_singleton_model(ApiKeyManager::new);
            let request_refresh_token = "request-refresh".to_string();
            let (response_sender, response_receiver) =
                oneshot::channel::<anyhow::Result<TokenResponse>>();
            let first_waiter = manager.update(&mut app, |manager, ctx| {
                manager.set_codex_tokens(
                    Some(CodexTokens {
                        access_token: "refreshing-access".into(),
                        refresh_token: Some(request_refresh_token.clone()),
                        id_token: Some(id_token("request-account")),
                        chatgpt_account_id: "request-account".into(),
                        expires_at: None,
                        connected_at: None,
                    }),
                    ctx,
                );
                let (waiter_sender, waiter_receiver) = oneshot::channel();
                spawn_codex_refresh_with(
                    manager,
                    request_refresh_token.clone(),
                    vec![waiter_sender],
                    async move {
                        response_receiver
                            .await
                            .expect("test refresh response sender dropped")
                    },
                    ctx,
                );
                waiter_receiver
            });
            let second_waiter = manager.update(&mut app, |manager, _| {
                let (waiter_sender, waiter_receiver) = oneshot::channel();
                assert!(!register_codex_refresh(
                    manager,
                    &request_refresh_token,
                    vec![waiter_sender],
                ));
                waiter_receiver
            });

            manager.update(&mut app, |manager, ctx| {
                manager.set_codex_tokens(replacement.clone(), ctx);
            });
            response_sender
                .send(Ok(TokenResponse {
                    id_token: None,
                    access_token: "stale-refreshed-access".into(),
                    refresh_token: Some("stale-rotated-refresh".into()),
                    expires_in: Some(3600),
                }))
                .expect("refresh task dropped response receiver");

            assert_eq!(first_waiter.await.unwrap(), CodexRefreshOutcome::Failed);
            assert_eq!(second_waiter.await.unwrap(), CodexRefreshOutcome::Failed);
            manager.read(&app, |manager, _| {
                assert_eq!(manager.codex_tokens(), replacement.as_ref());
                assert!(manager.codex_refresh_state.is_none());
            });
        });
    }
}

#[cfg(not(target_family = "wasm"))]
#[test]
fn replacement_token_waiters_are_dispatched_after_stale_flight_finishes() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            warpui_extras::secure_storage::register_noop("test", ctx);
        });
        let manager = app.add_singleton_model(ApiKeyManager::new);
        let (stale_sender, stale_receiver) = oneshot::channel();
        let (current_sender, current_receiver) = oneshot::channel();

        let pending = manager.update(&mut app, |manager, ctx| {
            manager.set_codex_tokens(
                Some(CodexTokens {
                    access_token: "stale-access".into(),
                    refresh_token: Some("stale-refresh".into()),
                    id_token: Some(id_token("account")),
                    chatgpt_account_id: "account".into(),
                    expires_at: None,
                    connected_at: None,
                }),
                ctx,
            );
            assert!(register_codex_refresh(
                manager,
                "stale-refresh",
                vec![stale_sender],
            ));

            manager.set_codex_tokens(
                Some(CodexTokens {
                    access_token: "current-access".into(),
                    refresh_token: Some("current-refresh".into()),
                    id_token: Some(id_token("account")),
                    chatgpt_account_id: "account".into(),
                    expires_at: None,
                    connected_at: None,
                }),
                ctx,
            );
            assert!(!register_codex_refresh(
                manager,
                "current-refresh",
                vec![current_sender],
            ));
            finish_codex_refresh(manager, "stale-refresh", CodexRefreshOutcome::Failed)
                .expect("current-token waiters must be dispatched to a new flight")
        });

        assert_eq!(stale_receiver.await.unwrap(), CodexRefreshOutcome::Failed);
        assert_eq!(pending.refresh_token, "current-refresh");
        manager.update(&mut app, |manager, _| {
            assert!(register_codex_refresh(
                manager,
                &pending.refresh_token,
                pending.waiters,
            ));
            assert!(
                finish_codex_refresh(manager, "stale-refresh", CodexRefreshOutcome::Failed)
                    .is_none()
            );
            assert_eq!(
                manager
                    .codex_refresh_state
                    .as_ref()
                    .map(|flight| flight.refresh_token.as_str()),
                Some("current-refresh")
            );
            assert!(finish_codex_refresh(
                manager,
                "current-refresh",
                CodexRefreshOutcome::Refreshed,
            )
            .is_none());
        });
        assert_eq!(
            current_receiver.await.unwrap(),
            CodexRefreshOutcome::Refreshed
        );
    });
}

#[cfg(not(target_family = "wasm"))]
#[test]
fn current_refresh_success_still_applies_and_wakes_waiter() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            warpui_extras::secure_storage::register_noop("test", ctx);
        });
        let manager = app.add_singleton_model(ApiKeyManager::new);
        let waiter = manager.update(&mut app, |manager, ctx| {
            manager.set_codex_tokens(
                Some(CodexTokens {
                    access_token: "old-access".into(),
                    refresh_token: Some("current-refresh".into()),
                    id_token: Some(id_token("current-account")),
                    chatgpt_account_id: "current-account".into(),
                    expires_at: None,
                    connected_at: None,
                }),
                ctx,
            );
            let (waiter_sender, waiter_receiver) = oneshot::channel();
            spawn_codex_refresh_with(
                manager,
                "current-refresh".into(),
                vec![waiter_sender],
                async {
                    Ok(TokenResponse {
                        id_token: None,
                        access_token: "fresh-access".into(),
                        refresh_token: Some("rotated-refresh".into()),
                        expires_in: Some(3600),
                    })
                },
                ctx,
            );
            waiter_receiver
        });

        assert_eq!(waiter.await.unwrap(), CodexRefreshOutcome::Refreshed);
        manager.read(&app, |manager, _| {
            let tokens = manager.codex_tokens().unwrap();
            assert_eq!(tokens.access_token, "fresh-access");
            assert_eq!(tokens.refresh_token.as_deref(), Some("rotated-refresh"));
            assert_eq!(tokens.chatgpt_account_id, "current-account");
            assert!(manager.codex_refresh_state.is_none());
        });
    });
}

#[cfg(not(target_family = "wasm"))]
#[test]
fn test_set_codex_refresh_allowed() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            warpui_extras::secure_storage::register_noop("test", ctx);
        });
        let manager = app.add_singleton_model(ApiKeyManager::new);

        // Prepare valid tokens with a future expiry.
        let test_tokens = CodexTokens {
            access_token: "test-access".into(),
            refresh_token: Some("test-refresh".into()),
            id_token: Some(id_token("test-account")),
            chatgpt_account_id: "test-account".into(),
            expires_at: Some(SystemTime::now() + Duration::from_secs(3600)),
            connected_at: Some(SystemTime::now()),
        };

        {
            // Case 1: Feature flag OFF -> does not schedule, allowed is always false
            let _guard =
                warp_core::features::FeatureFlag::CodexSubscription.override_enabled(false);

            // Set tokens with feature disabled.
            manager.update(&mut app, |manager, ctx| {
                manager.set_codex_tokens(Some(test_tokens.clone()), ctx);
                manager.set_codex_refresh_allowed(true, ctx);
            });
            manager.read(&app, |manager, _| {
                assert!(!manager.codex_refresh_allowed);
                assert_eq!(manager.codex_refresh_scheduled_count, 0);
            });

            // Set allowed false with feature disabled.
            manager.update(&mut app, |manager, ctx| {
                manager.set_codex_refresh_allowed(false, ctx);
            });
            manager.read(&app, |manager, _| {
                assert!(!manager.codex_refresh_allowed);
                assert_eq!(manager.codex_refresh_scheduled_count, 0);
            });
        }

        // Feature flag is now ON because the OFF guard was dropped.
        let _guard = warp_core::features::FeatureFlag::CodexSubscription.override_enabled(true);

        {
            // Case 2: Feature flag ON but no tokens installed -> does not schedule, but allowed is true
            manager.update(&mut app, |manager, ctx| {
                manager.set_codex_tokens(None, ctx);
                manager.set_codex_refresh_allowed(true, ctx);
            });
            manager.read(&app, |manager, _| {
                assert!(manager.codex_refresh_allowed);
                assert_eq!(manager.codex_refresh_scheduled_count, 0);
            });

            // Reset allowed to false
            manager.update(&mut app, |manager, ctx| {
                manager.set_codex_refresh_allowed(false, ctx);
            });
        }

        {
            // Case 3: Feature flag ON but tokens installed have no refresh_token -> does not schedule
            let tokens_no_refresh = CodexTokens {
                access_token: "test-access".into(),
                refresh_token: None,
                id_token: Some(id_token("test-account")),
                chatgpt_account_id: "test-account".into(),
                expires_at: Some(SystemTime::now() + Duration::from_secs(3600)),
                connected_at: Some(SystemTime::now()),
            };
            manager.update(&mut app, |manager, ctx| {
                manager.set_codex_tokens(Some(tokens_no_refresh), ctx);
                manager.set_codex_refresh_allowed(true, ctx);
            });
            manager.read(&app, |manager, _| {
                assert!(manager.codex_refresh_allowed);
                assert_eq!(manager.codex_refresh_scheduled_count, 0);
            });

            // Reset allowed to false
            manager.update(&mut app, |manager, ctx| {
                manager.set_codex_refresh_allowed(false, ctx);
            });
        }

        {
            // Case 4: Enabling the feature/allowed path with tokens -> schedules exactly once
            manager.update(&mut app, |manager, ctx| {
                manager.set_codex_tokens(Some(test_tokens.clone()), ctx);
                assert_eq!(manager.codex_refresh_scheduled_count, 0);
                manager.set_codex_refresh_allowed(true, ctx);
            });
            manager.read(&app, |manager, _| {
                assert!(manager.codex_refresh_allowed);
                assert_eq!(manager.codex_refresh_scheduled_count, 1);
            });

            // Case 5: Repeated enable -> no duplicate scheduling
            manager.update(&mut app, |manager, ctx| {
                manager.set_codex_refresh_allowed(true, ctx);
            });
            manager.read(&app, |manager, _| {
                assert!(manager.codex_refresh_allowed);
                assert_eq!(manager.codex_refresh_scheduled_count, 1);
            });

            // Case 6: Transition allowed to false -> stops scheduling
            manager.update(&mut app, |manager, ctx| {
                manager.set_codex_refresh_allowed(false, ctx);
            });
            manager.read(&app, |manager, _| {
                assert!(!manager.codex_refresh_allowed);
                assert_eq!(manager.codex_refresh_scheduled_count, 1);
            });

            // Case 7: Re-enabling allowed -> schedules again
            manager.update(&mut app, |manager, ctx| {
                manager.set_codex_refresh_allowed(true, ctx);
            });
            manager.read(&app, |manager, _| {
                assert!(manager.codex_refresh_allowed);
                assert_eq!(manager.codex_refresh_scheduled_count, 2);
            });
        }
    });
}
