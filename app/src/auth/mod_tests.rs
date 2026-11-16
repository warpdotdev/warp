use warp_core::channel::ChannelState;

use super::web_logout_url;
#[cfg(feature = "tui")]
use super::web_logout_url_with_continue;

#[test]
fn web_logout_url_uses_configured_server_root() {
    let server_root_url = ChannelState::server_root_url();
    assert_eq!(
        web_logout_url(),
        format!("{}/logout", server_root_url.trim_end_matches('/'))
    );
}

#[test]
#[cfg(feature = "tui")]
fn web_logout_url_rejects_unsafe_device_auth_continuations() {
    let server_root_url = ChannelState::server_root_url();
    for continue_url in [
        "https://example.com/device?user_code=ABCD-EFGH&source=warp-agent-cli".to_owned(),
        format!(
            "{}/login?user_code=ABCD-EFGH&source=warp-agent-cli",
            server_root_url.trim_end_matches('/')
        ),
        format!(
            "{}/device?source=warp-agent-cli",
            server_root_url.trim_end_matches('/')
        ),
        format!(
            "{}/device?user_code=ABCD-EFGH",
            server_root_url.trim_end_matches('/')
        ),
    ] {
        assert_eq!(web_logout_url_with_continue(&continue_url), None);
    }
}

#[test]
#[cfg(feature = "tui")]
fn web_logout_url_encodes_device_auth_continuation() {
    let continue_url = format!(
        "{}/device?user_code=ABCD-EFGH&source=warp-agent-cli",
        ChannelState::server_root_url().trim_end_matches('/')
    );
    let logout_url =
        url::Url::parse(&web_logout_url_with_continue(&continue_url).unwrap()).unwrap();

    assert_eq!(logout_url.path(), "/logout");
    assert_eq!(
        logout_url
            .query_pairs()
            .find(|(key, _)| key == "continue")
            .map(|(_, value)| value.into_owned()),
        Some(continue_url)
    );
}
