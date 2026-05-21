use crate::auth::credentials::{FirebaseToken, RefreshToken};
#[cfg(feature = "skip_login")]
use crate::server::server_api::ServerApi;
use anyhow::Result;

#[test]
fn test_firebase_token_urls() -> Result<()> {
    let custom_token = FirebaseToken::Custom("ct".to_string());
    let refresh_token = FirebaseToken::Refresh(RefreshToken::new("rt".to_string()));

    assert_eq!(
        custom_token.access_token_url("api_key"),
        "https://identitytoolkit.googleapis.com/v1/accounts:signInWithCustomToken?key=api_key"
    );
    assert_eq!(
        refresh_token.access_token_url("api_key"),
        "https://securetoken.googleapis.com/v1/token?key=api_key"
    );

    assert_eq!(
        custom_token.access_token_request_body(),
        vec![("returnSecureToken", "true"), ("token", "ct")]
    );
    assert_eq!(
        refresh_token.access_token_request_body(),
        vec![("grant_type", "refresh_token"), ("refresh_token", "rt")],
    );

    assert_eq!(
        custom_token.proxy_url("https://staging.warp.dev", "api_key"),
        "https://staging.warp.dev/proxy/customToken?key=api_key"
    );
    assert_eq!(
        refresh_token.proxy_url("https://staging.warp.dev", "api_key"),
        "https://staging.warp.dev/proxy/token?key=api_key"
    );
    Ok(())
}

#[cfg(feature = "skip_login")]
#[test]
fn access_token_skip_login_rejects_bearer_token() {
    let (event_sender, _) = async_channel::unbounded();
    let server_api =
        ServerApi::new_for_test_with_bearer_token(Some("daemon-token".to_string()), event_sender);

    let error = futures::executor::block_on(server_api.access_token()).unwrap_err();

    assert_eq!(
        error.to_string(),
        "skip_login enabled; failing all authenticated requests"
    );
}
