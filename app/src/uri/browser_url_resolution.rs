#[cfg(any(target_family = "wasm", test))]
use url::Url;

#[cfg(any(target_family = "wasm", test))]
use super::web_intent_parser::WebIntent;

#[cfg(any(target_family = "wasm", test))]
const BASE_APP_PATH: &str = "/app";
#[cfg(any(target_family = "wasm", test))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BrowserNavigationOrigin {
    Incidental,
    AutomaticRootRouteChange,
    ChangedSelection,
    BrowserHistory,
    InitialAnchorRestoration,
    #[cfg_attr(not(target_family = "wasm"), allow(dead_code))]
    InvalidAnchorCleanup,
    ColdChildCanonicalization,
    Forced,
}

#[cfg(any(target_family = "wasm", test))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BrowserHistoryWrite {
    None,
    Push,
    Replace,
    NavigateReplace,
    Navigate,
}

#[cfg(any(target_family = "wasm", test))]
pub(crate) struct BrowserNavigation {
    pub url: Option<Url>,
    pub write: BrowserHistoryWrite,
}

#[cfg(any(target_family = "wasm", test))]
pub(crate) fn resolve_browser_url(
    current_url: Option<Url>,
    requested_url: Option<Url>,
    origin: BrowserNavigationOrigin,
) -> BrowserNavigation {
    if origin == BrowserNavigationOrigin::BrowserHistory
        || origin == BrowserNavigationOrigin::InitialAnchorRestoration
    {
        return BrowserNavigation {
            url: current_url,
            write: BrowserHistoryWrite::None,
        };
    }

    if origin == BrowserNavigationOrigin::Incidental
        && let Some(current) = current_url.clone()
        && WebIntent::is_conversation_or_session_view(&current)
    {
        return BrowserNavigation {
            url: Some(current),
            write: BrowserHistoryWrite::None,
        };
    }

    let url = if origin == BrowserNavigationOrigin::AutomaticRootRouteChange {
        requested_url
            .map(|url| preserve_viewer_state(current_url.as_ref(), url))
            .or_else(|| base_app_url(current_url.clone()))
    } else {
        requested_url.or_else(|| base_app_url(current_url.clone()))
    };
    let write = match origin {
        BrowserNavigationOrigin::Incidental => BrowserHistoryWrite::Replace,
        BrowserNavigationOrigin::AutomaticRootRouteChange => {
            if url == current_url {
                BrowserHistoryWrite::None
            } else {
                BrowserHistoryWrite::Replace
            }
        }
        BrowserNavigationOrigin::ChangedSelection => {
            if url == current_url {
                BrowserHistoryWrite::None
            } else {
                BrowserHistoryWrite::Push
            }
        }
        BrowserNavigationOrigin::InvalidAnchorCleanup => BrowserHistoryWrite::Replace,
        BrowserNavigationOrigin::ColdChildCanonicalization => BrowserHistoryWrite::NavigateReplace,
        BrowserNavigationOrigin::Forced => BrowserHistoryWrite::Navigate,
        BrowserNavigationOrigin::BrowserHistory
        | BrowserNavigationOrigin::InitialAnchorRestoration => BrowserHistoryWrite::None,
    };
    BrowserNavigation { url, write }
}

#[cfg(any(target_family = "wasm", test))]
fn preserve_viewer_state(current_url: Option<&Url>, mut requested_url: Url) -> Url {
    let Some(current_url) = current_url else {
        return requested_url;
    };
    let Some(location) = super::viewer_location::ViewerLocation::parse(current_url) else {
        return requested_url;
    };
    if !WebIntent::is_conversation_or_session_view(&requested_url) {
        return requested_url;
    }
    if location.standalone
        && !requested_url
            .query_pairs()
            .any(|(key, value)| key == "view" && value == "standalone")
    {
        requested_url
            .query_pairs_mut()
            .append_pair("view", "standalone");
    }
    requested_url.set_fragment(current_url.fragment());
    requested_url
}

#[cfg(any(target_family = "wasm", test))]
fn base_app_url(current_url: Option<Url>) -> Option<Url> {
    let mut new_url = current_url?;
    new_url.set_path(BASE_APP_PATH);
    new_url.set_query(None);
    Some(new_url)
}
