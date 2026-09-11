use url::Url;
use warp_errors::report_error;
use wasm_bindgen::JsCast;

use super::browser_url_resolution::{
    BrowserHistoryWrite, BrowserNavigationOrigin, resolve_browser_url,
};
use super::viewer_location::ViewerLocation;

const DEFAULT_TITLE: &str = "Warp";

pub(crate) fn install_viewer_history_listener() {
    let callback = wasm_bindgen::closure::Closure::<dyn FnMut(web_sys::Event)>::new(move |_| {
        let Some(current_url) = parse_current_url() else {
            return;
        };
        if ViewerLocation::parse(&current_url).is_some() {
            let _ = gloo::utils::window().location().reload();
        }
    });
    let result = gloo::utils::window()
        .add_event_listener_with_callback("popstate", callback.as_ref().unchecked_ref());
    if result.is_ok() {
        callback.forget();
    } else {
        report_error!("Failed to install browser history listener");
    }
}

pub fn update_browser_url(url: Option<Url>, force_redirect: bool) {
    let origin = if force_redirect {
        BrowserNavigationOrigin::Forced
    } else {
        BrowserNavigationOrigin::Incidental
    };
    update_browser_url_from_origin(url, origin);
}

pub(crate) fn update_browser_url_from_origin(url: Option<Url>, origin: BrowserNavigationOrigin) {
    let current_url = parse_current_url();
    if url.is_none() && current_url.is_none() {
        report_error!("Failed to get the base url");
    }
    let navigation = resolve_browser_url(current_url, url, origin);

    if let Some(unwrapped_url) = navigation.url.and_then(safe_browser_navigation_url) {
        let window = gloo::utils::window();
        if navigation.write == BrowserHistoryWrite::Navigate {
            let _ = window.location().set_href(unwrapped_url.as_str());
        } else if navigation.write == BrowserHistoryWrite::NavigateReplace {
            let _ = window.location().replace(unwrapped_url.as_str());
        } else if let Ok(history) = window.history() {
            let result = match navigation.write {
                BrowserHistoryWrite::Push => history.push_state_with_url(
                    &wasm_bindgen::JsValue::null(),
                    DEFAULT_TITLE,
                    Some(unwrapped_url.as_str()),
                ),
                BrowserHistoryWrite::Replace => history.replace_state_with_url(
                    &wasm_bindgen::JsValue::null(),
                    DEFAULT_TITLE,
                    Some(unwrapped_url.as_str()),
                ),
                BrowserHistoryWrite::None => return,
                BrowserHistoryWrite::Navigate | BrowserHistoryWrite::NavigateReplace => {
                    unreachable!()
                }
            };
            result.unwrap_or_else(|_| {
                report_error!("Failed to replace browser state");
                crate::platform::wasm::emit_event(crate::platform::wasm::WarpEvent::ErrorLogged {
                    error: String::from("Failed to update browser state"),
                });
            });
        } else {
            report_error!("Failed to get gloo history while trying to update browser url");
        }
    } else {
        report_error!("Failed to get new url to update browser with");
    }
}

pub(crate) fn update_viewer_selection(
    selected_child_run_id: Option<crate::ai::ambient_agents::AmbientAgentTaskId>,
    origin: BrowserNavigationOrigin,
) {
    let Some(current_url) = parse_current_url() else {
        return;
    };
    let Some(location) = ViewerLocation::parse(&current_url) else {
        return;
    };
    let requested_url = selected_child_run_id
        .map(|task_id| location.with_child(task_id))
        .unwrap_or(location.root_url);
    update_browser_url_from_origin(Some(requested_url), origin);
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
