use anyhow::{anyhow, Result};
use url::Url;
use uuid::Uuid;
#[cfg(target_family = "wasm")]
use warp_core::context_flag::ContextFlag;

#[cfg(target_family = "wasm")]
use crate::uri::browser_url_handler::parse_current_url;
use crate::ChannelState;

#[derive(Debug)]
/// Represents an intent parsed from a web url
pub enum WebIntent {
    SessionView(Url),
    ConversationView(Url),
    DriveObject(Url),
    SettingsView(Url),
    Home(Url),
    CloudAgentHome(Url),
    Action(Url),
}

impl WebIntent {
    pub fn try_from_url(url: &Url) -> Result<Self> {
        // Only handle URLs that point at the current channel's web server.
        let server_root = ChannelState::server_root_url();
        let server_root_url = Url::parse(&server_root)?;
        if url.scheme() != server_root_url.scheme()
            || url.domain() != server_root_url.domain()
            || url.port_or_known_default() != server_root_url.port_or_known_default()
        {
            return Err(anyhow!("Attempting to parse invalid url: {}", url));
        }

        let segments = url
            .path_segments()
            .map(|segments| segments.collect::<Vec<_>>());
        if let Some(segments) = segments {
            let url_scheme = ChannelState::url_scheme();
            if segments.is_empty() {
                return Ok(WebIntent::Home(Url::parse(&format!(
                    "{url_scheme}://home"
                ))?));
            } else {
                match segments[0] {
                    "app" => {
                        return Ok(WebIntent::CloudAgentHome(Url::parse(&format!(
                            "{url_scheme}://action/new_cloud_agent_conversation?source=web_home"
                        ))?));
                    }
                    // For sessions, we expect the URL to be in the format: {scheme}/session/{session_id}
                    "session" => {
                        if segments.len() != 2 {
                            return Err(anyhow!("Attempting to parse invalid url: {}", url));
                        }

                        let session_id = segments[1];

                        // Validate that the session ID is a UUID. If it's not, this isn't a
                        // valid shared-session URL and we should return an error so the
                        // caller can ignore it.
                        if Uuid::parse_str(session_id).is_err() {
                            return Err(anyhow!("Attempting to parse invalid url: {}", url));
                        }

                        let mut session_intent = Url::parse(
                            format!("{url_scheme}://shared_session/{session_id}").as_str(),
                        )
                        .map_err(|_| anyhow!("Attempting to parse invalid url: {}", url))?;

                        // Preserve any query parameters (e.g. pwd, preview) from the original URL.
                        if let Some(query) = url.query() {
                            session_intent.set_query(Some(query));
                        }

                        return Ok(WebIntent::SessionView(session_intent));
                    }
                    // For conversations, we expect the URL to be in the format: {scheme}/conversation/{conversation_id}
                    "conversation" => {
                        if segments.len() != 2 {
                            return Err(anyhow!("Attempting to parse invalid url: {}", url));
                        }

                        let conversation_id = segments[1];
                        let conversation_intent = Url::parse(
                            format!("{url_scheme}://conversation/{conversation_id}").as_str(),
                        )
                        .map_err(|_| anyhow!("Attempting to parse invalid url: {}", url))?;

                        return Ok(WebIntent::ConversationView(conversation_intent));
                    }
                    // For drive objects, we expect the URL to be of the format: {scheme}/drive/{object-type}/{object-name}-{object-id}?focused_folder_id={focused_folder_id}
                    // The focused_folder_id is optional, and if it is not provided, we will not include it in the intent url.
                    "drive" => {
                        if segments.len() != 3 {
                            return Err(anyhow!("Attempting to parse invalid url: {}", url));
                        }
                        let id_and_name: Vec<&str> =
                            segments[segments.len() - 1].split('-').collect();
                        let id = id_and_name[id_and_name.len() - 1];
                        let object_type = segments[segments.len() - 2];
                        if let Ok(mut drive_intent) =
                            Url::parse(format!("{url_scheme}://drive/{object_type}").as_str())
                        {
                            drive_intent.set_query(url.query());
                            drive_intent.query_pairs_mut().append_pair("id", id);
                            return Ok(WebIntent::DriveObject(drive_intent));
                        }
                    }
                    "settings" => {
                        // For the settings links, we expect the URL to be of the format: {scheme}/settings/{sub_section}?{query_str}
                        if segments.len() != 2 {
                            return Err(anyhow!("Attempting to parse invalid url: {}", url));
                        }
                        let sub_section = segments[segments.len() - 1];
                        let query_str = url.query().unwrap_or_default();
                        if query_str.is_empty() {
                            return Err(anyhow!("Attempting to parse invalid url: {}", url));
                        }
                        if let Ok(settings_intent) = Url::parse(
                            format!("{url_scheme}://settings/{sub_section}?{query_str}").as_str(),
                        ) {
                            return Ok(WebIntent::SettingsView(settings_intent));
                        }
                    }
                    "action" => {
                        if segments.len() != 2 {
                            return Err(anyhow!("Attempting to parse invalid url: {}", url));
                        }
                        let action_type = segments[1];
                        // Allowlist of valid actions,
                        // since we shouldn't expose all Warp actions as web URLs.
                        const ALLOWED_ACTIONS: &[&str] = &["open-repo", "focus_cloud_mode"];
                        if !ALLOWED_ACTIONS.contains(&action_type) {
                            return Err(anyhow!("Unknown action type in url: {}", action_type));
                        }
                        if let Ok(action_intent) =
                            Url::parse(format!("{url_scheme}://action/{action_type}").as_str())
                        {
                            return Ok(WebIntent::Action(action_intent));
                        }
                    }
                    _ => return Err(anyhow!("Attempting to parse invalid url: {}", url)),
                }
            }
        }
        Err(anyhow!("Attempting to parse invalid url: {}", url))
    }

    /// Convert this web intent into the underlying native desktop URL.
    pub fn into_intent_url(self) -> Url {
        match self {
            WebIntent::SessionView(url) => url,
            WebIntent::ConversationView(url) => url,
            WebIntent::DriveObject(url) => url,
            WebIntent::SettingsView(url) => url,
            WebIntent::Home(url) => url,
            WebIntent::CloudAgentHome(url) => url,
            WebIntent::Action(url) => url,
        }
    }
}

/// Attempts to rewrite a Warp web URL into a native desktop intent URL (warp://...).
/// Returns `None` if the URL is not a recognized Warp web intent.
pub fn maybe_rewrite_web_url_to_intent(url: &Url) -> Option<Url> {
    WebIntent::try_from_url(url)
        .ok()
        .map(WebIntent::into_intent_url)
}

/// On WASM warp, fires an event to try and open the given link on the desktop app.
#[cfg(target_family = "wasm")]
pub fn open_url_on_desktop(url: &Url) {
    match WebIntent::try_from_url(url) {
        Ok(WebIntent::ConversationView(intent))
        | Ok(WebIntent::DriveObject(intent))
        | Ok(WebIntent::SessionView(intent))
        | Ok(WebIntent::CloudAgentHome(intent))
        | Ok(WebIntent::Action(intent)) => {
            crate::platform::wasm::emit_event(crate::platform::wasm::WarpEvent::OpenOnNative {
                url: intent.into(),
            });
        }
        _ => {
            log::warn!("Attempting to open invalid url on desktop app:{url}");
        }
    };
}

#[cfg(target_family = "wasm")]
fn set_context_flags_from_url(url: Url) {
    match WebIntent::try_from_url(&url) {
        Ok(WebIntent::SessionView(_)) => ContextFlag::set_shared_session_only(),
        Ok(WebIntent::ConversationView(_)) => ContextFlag::set_conversation_only(),
        Ok(WebIntent::DriveObject(_)) => ContextFlag::set_warp_drive_link_only(),
        Ok(WebIntent::SettingsView(_)) => ContextFlag::set_settings_link_only(),
        Ok(WebIntent::Home(_)) => ContextFlag::set_warp_home_link_only(),
        Ok(WebIntent::CloudAgentHome(_)) => {}
        Ok(WebIntent::Action(_)) => {} // No special context flag for actions
        _ => {}
    }

    // Allow directly setting flags through query params in dogfood.
    if ChannelState::channel().is_dogfood() {
        for (param, value) in url.query_pairs() {
            let Ok(flag) = param.parse::<ContextFlag>() else {
                continue;
            };
            let Ok(bool_value) = value.parse::<bool>() else {
                continue;
            };

            flag.set(bool_value);
        }
    }
}

/// Looks at the current URL and converts it into an app intent.
#[cfg(target_family = "wasm")]
pub fn current_web_intent() -> Option<WebIntent> {
    let Some(current_url) = parse_current_url() else {
        log::warn!("Unable to parse the current url");
        return None;
    };

    WebIntent::try_from_url(&current_url).ok()
}

// Looks at the current url and converts it into an app intent.
// NOTE: This is only intended for use with target_family = "wasm"
#[cfg(target_family = "wasm")]
pub fn parse_web_intent_from_current_url() -> Option<Url> {
    current_web_intent().map(WebIntent::into_intent_url)
}

#[cfg(target_family = "wasm")]
pub fn set_context_flags_from_current_url() {
    let Some(current_url) = parse_current_url() else {
        log::warn!("Unable to parse the current url");
        return;
    };

    set_context_flags_from_url(current_url);
}
