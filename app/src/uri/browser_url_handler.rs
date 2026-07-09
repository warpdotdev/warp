use url::Url;

use super::web_intent_parser::WebIntent;
use crate::report_error;

const DEFAULT_TITLE: &str = "Warp";
const BASE_APP_PATH: &str = "/app";

pub fn update_browser_url(url: Option<Url>, force_redirect: bool) {
    let mut new_url = url;
    if new_url.is_none() {
        new_url = get_base_app_url()
    }

    if let Some(unwrapped_url) = new_url.and_then(safe_browser_navigation_url) {
        let window = gloo::utils::window();
        if force_redirect {
            let _ = window.location().set_href(unwrapped_url.as_str());
        } else if let Ok(history) = window.history() {
            history
                .replace_state_with_url(
                    &wasm_bindgen::JsValue::null(),
                    DEFAULT_TITLE,
                    Some(unwrapped_url.as_str()),
                )
                .unwrap_or_else(|_| {
                    report_error!("Failed to replace browser state");
                    crate::platform::wasm::emit_event(
                        crate::platform::wasm::WarpEvent::ErrorLogged {
                            error: String::from("Failed to replace browser state"),
                        },
                    );
                });
        } else {
            report_error!("Failed to get gloo history while trying to update browser url");
        }
    } else {
        report_error!("Failed to get new url to update browser with");
    }
}

fn safe_browser_navigation_url(url: Url) -> Option<Url> {
    match url.scheme() {
        "http" | "https" => Some(url),
        _ => {
            log::warn!("Skipping browser URL update for invalid or unsafe URL");
            None
        }
    }
}

pub fn parse_current_url() -> Option<Url> {
    let loc = gloo::utils::document().location();
    let unwrapped_loc = loc.as_ref()?;

    let the_href = unwrapped_loc.href();
    if the_href.is_err() {
        return None;
    }

    if let Ok(parsed_url) = Url::parse(the_href.expect("Invalid href parsed from url").as_str()) {
        return Some(parsed_url);
    }

    None
}

fn get_base_app_url() -> Option<Url> {
    if let Some(current_url) = parse_current_url() {
        if should_preserve_current_url_on_base_fallback(&current_url) {
            return Some(current_url);
        }
        let mut new_url = current_url.clone();
        new_url.set_path(BASE_APP_PATH);
        new_url.set_query(None);
        return Some(new_url);
    }
    report_error!("Failed to get the base url");
    None
}

fn should_preserve_current_url_on_base_fallback(url: &Url) -> bool {
    matches!(
        WebIntent::try_from_url(url),
        Ok(WebIntent::ConversationView(_) | WebIntent::SessionView(_))
    )
}
