pub fn escape_html_attribute(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '&' => escaped.push_str("&amp;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&#39;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            _ => escaped.push(ch),
        }
    }
    escaped
}

#[cfg(any(target_family = "wasm", test))]
pub(crate) fn safe_browser_open_url(url: &str) -> Option<String> {
    let parsed_url = url::Url::parse(url).ok()?;
    match parsed_url.scheme() {
        "http" | "https" | "mailto" | "warp" | "warppreview" | "warpdev" | "warplocal"
        | "warposs" | "warpintegration" => Some(parsed_url.to_string()),
        _ => None,
    }
}

#[cfg(test)]
#[path = "browser_tests.rs"]
mod tests;
