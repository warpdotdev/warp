use super::sentry_client_options_for_dsn;

#[test]
fn sentry_client_options_disable_auto_session_tracking() {
    let options = sentry_client_options_for_dsn("https://public@example.com/1");

    assert!(!options.auto_session_tracking);
}
