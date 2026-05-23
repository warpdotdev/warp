use std::sync::OnceLock;

use woothee::parser::{Parser, WootheeResult};

use crate::platform::OperatingSystem;

static PARSED_USER_AGENT: OnceLock<Option<ParsedUserAgent>> = OnceLock::new();
static PLATFORM: OnceLock<OperatingSystem> = OnceLock::new();

#[derive(Debug)]
struct ParsedUserAgent {
    os: String,
    /// For macOS, the version number is probably incorrect as it is currently
    /// capped at 10.15. See: https://bugs.webkit.org/show_bug.cgi?id=216593.
    /// It's possible to get the correct version using the Client Hints API, but
    /// this is currently only supported by Chrome: https://developer.mozilla.org/en-US/docs/Web/API/User-Agent_Client_Hints_API.
    os_version: String,
    browser: String,
    browser_version: String,
}

impl ParsedUserAgent {
    /// Converts the result we get from parsing the user agent into a struct
    /// with owned values.
    fn from_woothee_result(result: &WootheeResult) -> Self {
        ParsedUserAgent {
            os: result.os.to_string(),
            os_version: result.os_version.to_string(),
            browser: result.name.to_string(),
            browser_version: result.version.to_string(),
        }
    }
}

fn parsed_user_agent() -> Option<&'static ParsedUserAgent> {
    PARSED_USER_AGENT
        .get_or_init(|| {
            let Ok(user_agent) = gloo::utils::window().navigator().user_agent() else {
                return None;
            };

            let parser = Parser::new();
            parser
                .parse(user_agent.as_str())
                .map(|result| ParsedUserAgent::from_woothee_result(&result))
        })
        .as_ref()
}

/// Returns the current operating system by reading the user agent. If the user agent was not able
/// to be read, [`OperatingSystem::Other`] is returned.
///
/// # Panics
/// Panics if called before the app was attached to the DOM.
pub(super) fn current_platform() -> OperatingSystem {
    *PLATFORM.get_or_init(|| {
        let Some(parsed_user_agent) = parsed_user_agent() else {
            return OperatingSystem::Other(None);
        };

        // Try to parse the user agent to determine the OS. _heavily_ inspired by
        // https://github.com/mozilla-services/contile/blob/61da2719fa4586fc0b15fe7f47ebbc1586f28a47/src/web/user_agent.rs#L95-L105.
        let os = parsed_user_agent.os.to_lowercase();
        match os.as_str() {
            _ if os.starts_with("windows") => OperatingSystem::Windows,
            "mac osx" => OperatingSystem::Mac,
            "linux" => OperatingSystem::Linux,
            _ => OperatingSystem::Other(Some(&parsed_user_agent.os)),
        }
    })
}

/// Returns the user agent provided by the browser. If the user agent was
/// unable to be read, returns None.
pub fn user_agent() -> Option<String> {
    gloo::utils::window().navigator().user_agent().ok()
}

/// Returns the version of the current operating system, parsed from the user
/// agent. If the user agent was not able to be read, returns None.
///
/// Also returns None if the current operating system is macOS. The version
/// reported to the user agent is capped at 10.15, meaning it is probably
/// incorrect in most cases: https://bugs.webkit.org/show_bug.cgi?id=216593.
pub fn current_os_version() -> Option<&'static str> {
    if matches!(current_platform(), OperatingSystem::Mac) {
        return None;
    };

    parsed_user_agent().map(|ua| ua.os_version.as_str())
}

/// Returns the name of the browser, parsed from the user agent. If the user
/// agent was not able to be read, returns None.
pub fn current_browser() -> Option<&'static str> {
    parsed_user_agent().map(|ua| ua.browser.as_str())
}

/// Returns the version of the current browser, parsed from the user agent.
/// If the user agent was not able to be read, returns None.
pub fn current_browser_version() -> Option<&'static str> {
    parsed_user_agent().map(|ua| ua.browser_version.as_str())
}
