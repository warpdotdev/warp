#[cfg(any(target_family = "wasm", test))]
use url::Url;
#[cfg(any(target_family = "wasm", test))]
use super::web_intent_parser::WebIntent;

#[cfg(any(target_family = "wasm", test))]
const BASE_APP_PATH: &str = "/app";

/// Decides which URL belongs on the browser address bar, given the URL
/// currently committed there and the URL a pane focus or link-update event
/// is requesting. Contains no browser I/O, so the decision can be tested
/// directly rather than only through the DOM-writing wrapper in
/// `browser_url_handler`.
///
/// A `ConversationView`/`SessionView` URL currently on the address bar is
/// never replaced by a non-forced request, such as a pane focus change:
/// that route anchors the web session viewer and must stay there no matter
/// which pane inside it is focused. Otherwise, the requested URL is used
/// when present, falling back to the base app URL derived from the current
/// one when it is not.
#[cfg(any(target_family = "wasm", test))]
pub(crate) fn resolve_browser_url(
    current_url: Option<Url>,
    requested_url: Option<Url>,
    force_redirect: bool,
) -> Option<Url> {
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
