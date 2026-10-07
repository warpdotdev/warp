#[cfg(any(target_family = "wasm", test))]
use url::Url;

#[cfg(any(target_family = "wasm", test))]
use super::web_intent_parser::WebIntent;

#[cfg(any(target_family = "wasm", test))]
const BASE_APP_PATH: &str = "/app";

/// Resolves the URL that belongs on the browser address bar. Pure, so the
/// decision is testable apart from the DOM-writing wrapper in
/// `browser_url_handler`.
///
/// A `ConversationView`/`SessionView` URL is never replaced by a non-forced
/// request: that route anchors the web session viewer and must hold
/// regardless of which pane inside it is focused.
#[cfg(any(target_family = "wasm", test))]
pub(crate) fn resolve_browser_url(
    current_url: Option<Url>,
    requested_url: Option<Url>,
    force_redirect: bool,
) -> Option<Url> {
    // force_redirect is a full navigation (e.g. login/signup) that must
    // always take effect, not be held back by the viewer guard below.
    if !force_redirect
        && let Some(current) = current_url.clone()
        && WebIntent::is_conversation_or_session_view(&current)
    {
        return Some(current);
    }

    requested_url.or_else(|| base_app_url(current_url))
}

#[cfg(any(target_family = "wasm", test))]
fn base_app_url(current_url: Option<Url>) -> Option<Url> {
    let mut new_url = current_url?;
    new_url.set_path(BASE_APP_PATH);
    new_url.set_query(None);
    Some(new_url)
}
