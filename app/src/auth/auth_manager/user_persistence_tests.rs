use chrono::DateTime;

use crate::auth::{
    user::{FirebaseAuthTokens, PersonalObjectLimits, UserMetadata},
    UserUid,
};

use super::PersistedUser;

/// Verifies that the JSON blob format as of March 6, 2026 can be deserialized correctly.
///
/// We must ALWAYS be backwards-compatible with the format here. The inlined JSON string can never change - it represents
/// data serialized on user devices.
#[test]
#[allow(deprecated)]
fn test_deserialize_2026_03_06_persisted_user() {
    const BLOB: &str = r#"{"id_token":{"id_token":"test-id-token","refresh_token":"test-refresh-token","expiration_time":"2099-01-01T00:00:00Z"},"refresh_token":"","local_id":"test-uid","email":"test@example.com","display_name":"Test User","photo_url":"https://example.com/photo.jpg","is_onboarded":true,"needs_sso_link":false,"anonymous_user_type":null,"linked_at":null,"personal_object_limits":null,"is_on_work_domain":false}"#;

    let user: PersistedUser =
        serde_json::from_str(BLOB).expect("2026-03-06 JSON should deserialize");

    assert_eq!(user.auth_tokens.id_token, "test-id-token");
    assert_eq!(user.auth_tokens.refresh_token, "test-refresh-token");
    assert_eq!(user.refresh_token, "");
    assert_eq!(user.local_id.as_str(), "test-uid");
    assert_eq!(user.metadata.email, "test@example.com");
    assert_eq!(user.metadata.display_name.as_deref(), Some("Test User"));
    assert_eq!(
        user.metadata.photo_url.as_deref(),
        Some("https://example.com/photo.jpg")
    );
    assert!(user.is_onboarded);
    assert!(!user.needs_sso_link);
    assert_eq!(user.anonymous_user_type, None);
    assert_eq!(user.linked_at, None);
    assert!(user.personal_object_limits.is_none());
    assert!(!user.is_on_work_domain);
}

/// Verifies that serializing a PersistedUser produces the expected JSON string.
///
/// If this test fails, it means the serialization format has changed.
/// You should:
/// 1. Add a new dated deserialization test (see [`test_deserialize_2026_03_06_persisted_user`])
/// 2. Update the serialization test to match the new format
#[test]
#[allow(deprecated)]
fn test_serialize_persisted_user() {
    const EXPECTED_BLOB: &str = r#"{"id_token":{"id_token":"test-id-token","refresh_token":"test-refresh-token","expiration_time":"2099-01-01T00:00:00Z"},"refresh_token":"","local_id":"test-uid","email":"test@example.com","display_name":"Test User","photo_url":"https://example.com/photo.jpg","is_onboarded":true,"needs_sso_link":false,"anonymous_user_type":null,"linked_at":null,"personal_object_limits":{"env_var_limit":10,"notebook_limit":20,"workflow_limit":30},"is_on_work_domain":false}"#;

    let expiration_time = DateTime::parse_from_rfc3339("2099-01-01T00:00:00+00:00")
        .expect("should parse expiration datetime");

    let user = PersistedUser {
        auth_tokens: FirebaseAuthTokens {
            id_token: "test-id-token".to_string(),
            refresh_token: "test-refresh-token".to_string(),
            expiration_time,
        },
        refresh_token: String::new(),
        local_id: UserUid::new("test-uid"),
        metadata: UserMetadata {
            email: "test@example.com".to_string(),
            display_name: Some("Test User".to_string()),
            photo_url: Some("https://example.com/photo.jpg".to_string()),
        },
        is_onboarded: true,
        needs_sso_link: false,
        anonymous_user_type: None,
        linked_at: None,
        personal_object_limits: Some(PersonalObjectLimits {
            env_var_limit: 10,
            notebook_limit: 20,
            workflow_limit: 30,
        }),
        is_on_work_domain: false,
    };

    let serialized = serde_json::to_string(&user).expect("serialization should succeed");
    assert_eq!(serialized, EXPECTED_BLOB);
}

/// Test serializing and deserializing persisted user data.
/// See warpui_extras::secure_storage::linux_test.rs for Linux-specific tests.
#[cfg(target_os = "windows")]
#[cfg_attr(windows, ignore = "passes locally but not in CI on Windows")]
#[test]
#[allow(deprecated)]
fn test_windows_user_persistence() {
    use crate::auth::{AuthManager, AuthStateProvider};
    use crate::server::{
        datetime_ext::DateTimeExt, telemetry::context_provider::AppTelemetryContextProvider,
    };
    use crate::ServerApiProvider;
    use chrono::DateTime;
    use warp_core::channel::ChannelState;
    use warpui::{App, SingletonEntity};
    use warpui_extras::secure_storage;

    App::test((), |mut app| async move {
        app.add_singleton_model(|_ctx| ServerApiProvider::new_for_test());
        app.add_singleton_model(|_| AuthStateProvider::new_for_test());
        app.add_singleton_model(AppTelemetryContextProvider::new_context_provider);
        app.add_singleton_model(|ctx| {
            secure_storage::register_with_dir(
                ChannelState::data_domain().as_str(),
                warp_core::paths::state_dir(),
                ctx,
            );
            AuthManager::new_for_test(ctx)
        });

        let tokens = FirebaseAuthTokens {
            id_token: String::from("This is an ID token."),
            refresh_token: String::from("This is a refresh token."),
            expiration_time: DateTime::now() + chrono::Duration::days(365),
        };
        let persisted_user = PersistedUser {
            auth_tokens: tokens.clone(),
            refresh_token: String::new(),
            local_id: UserUid::new("test_uid"),
            metadata: UserMetadata {
                email: "test@test.com".to_string(),
                display_name: Some(String::from("abcdef")),
                photo_url: Some(String::from("some-photo-url")),
            },
            is_onboarded: true,
            needs_sso_link: false,
            anonymous_user_type: None,
            linked_at: None,
            personal_object_limits: None,
            is_on_work_domain: false,
        };

        AuthManager::handle(&app).update(&mut app, |_auth_manager, ctx| {
            // Write the test user to secure storage.
            let write = persisted_user.write_to_secure_storage(ctx);
            match &write {
                Ok(()) => {}
                Err(err) => {
                    println!("{err:?}");
                }
            }
            assert!(write.is_ok());

            // Read the persisted user back and ensure the fields match.
            let stored = PersistedUser::from_secure_storage(ctx).unwrap();
            assert_eq!(stored.auth_tokens.id_token, tokens.id_token);
            assert_eq!(stored.auth_tokens.refresh_token, tokens.refresh_token);
            assert_eq!(stored.auth_tokens.expiration_time, tokens.expiration_time);
            assert_eq!(
                stored.metadata.display_name,
                persisted_user.metadata.display_name
            );
            assert_eq!(stored.metadata.email, persisted_user.metadata.email);
            assert_eq!(stored.metadata.photo_url, persisted_user.metadata.photo_url);

            // Remove the user from secure storage.
            assert!(PersistedUser::remove_from_secure_storage(ctx).is_ok());

            // Attempt to read a user back, which should fail.
            let empty_user = PersistedUser::from_secure_storage(ctx);
            assert!(empty_user.is_err());
        })
    });
}
