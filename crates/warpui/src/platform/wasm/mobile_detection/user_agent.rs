/// Determines if a user agent string indicates a mobile device.
pub fn is_mobile_user_agent(user_agent: &str) -> bool {
    let ua_lower = user_agent.to_lowercase();

    // iOS devices
    if ua_lower.contains("iphone") || ua_lower.contains("ipad") || ua_lower.contains("ipod") {
        return true;
    }

    // Android devices (phones and tablets)
    if ua_lower.contains("android") && !ua_lower.contains("windows") {
        return true;
    }

    // Other mobile platforms
    if ua_lower.contains("webos")
        || ua_lower.contains("blackberry")
        || ua_lower.contains("bb10") // BlackBerry 10 devices
        || ua_lower.contains("opera mini")
        || ua_lower.contains("opera mobi")
        || ua_lower.contains("iemobile")
        || ua_lower.contains("windows phone")
    {
        return true;
    }

    false
}

#[cfg(test)]
#[path = "user_agent_tests.rs"]
mod user_agent_tests;
