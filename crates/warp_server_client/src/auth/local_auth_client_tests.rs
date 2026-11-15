use super::{AUTH_UNAVAILABLE, AuthClient, AuthClientImpl, LoginToken};

#[test]
fn local_client_rejects_account_requests() {
    futures::executor::block_on(async {
        let client = AuthClientImpl;
        assert!(
            client
                .fetch_user(LoginToken::SessionCookie, false)
                .await
                .err()
                .unwrap()
                .to_string()
                .contains(AUTH_UNAVAILABLE)
        );
        assert_eq!(
            client.list_api_keys(None).await.unwrap_err().to_string(),
            AUTH_UNAVAILABLE
        );
        assert_eq!(
            client
                .set_is_telemetry_enabled(true)
                .await
                .unwrap_err()
                .to_string(),
            AUTH_UNAVAILABLE
        );
    });
}
