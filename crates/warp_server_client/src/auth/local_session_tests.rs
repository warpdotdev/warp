use super::{AUTH_UNAVAILABLE, AuthSession, LoginToken};

#[test]
fn local_session_refuses_credentials_and_device_login() {
    futures::executor::block_on(async {
        let session = AuthSession;
        assert!(!session.allowed_to_refresh_token());
        assert_eq!(
            session
                .get_or_refresh_access_token()
                .await
                .unwrap_err()
                .to_string(),
            AUTH_UNAVAILABLE
        );
        assert!(
            session
                .exchange_credentials(LoginToken::SessionCookie)
                .await
                .is_err()
        );
        assert!(session.request_device_code().await.is_err());
    });
}
