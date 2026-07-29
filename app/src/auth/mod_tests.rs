use warp_core::channel::ChannelState;

use super::web_logout_url;

#[test]
fn web_logout_url_uses_configured_server_root() {
    let server_root_url = ChannelState::server_root_url();
    assert_eq!(
        web_logout_url(),
        format!("{}/logout", server_root_url.trim_end_matches('/'))
    );
}
