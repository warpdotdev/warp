//! Small presentation helpers for the `warp-tui` front-end's TUI views.

use warpui_core::elements::tui::{Modifier, TuiElement, TuiFlex, TuiStyle, TuiText};

/// Abbreviates a leading home-directory prefix of `path` to `~`.
pub(crate) fn abbreviate_home_prefix(path: &str) -> String {
    if let Some(home) = dirs::home_dir() {
        let home = home.to_string_lossy();
        if let Some(rest) = path.strip_prefix(&*home) {
            if rest.is_empty() || rest.starts_with('/') || rest.starts_with('\\') {
                return format!("~{rest}");
            }
        }
    }
    path.to_owned()
}

/// Compacts a path for the one-line session footer while preserving its root
/// (or first relative component) and basename.
pub(crate) fn compact_footer_path(path: &str) -> String {
    let path = abbreviate_home_prefix(path);
    let separator = if path.contains('\\') && !path.contains('/') {
        '\\'
    } else {
        '/'
    };
    let components: Vec<_> = path
        .split(separator)
        .filter(|component| !component.is_empty())
        .collect();
    if components.len() <= 2 {
        return path;
    }

    let last = components
        .last()
        .expect("path has more than two components");
    if path.starts_with(separator) {
        format!("{separator}…{separator}{last}")
    } else {
        format!(
            "{}{separator}…{separator}{last}",
            components.first().expect("path has components")
        )
    }
}

/// Vertically centers `content` by padding above and below with flex spacers.
pub(crate) fn centered(content: TuiFlex) -> Box<dyn TuiElement> {
    TuiFlex::column()
        .flex_child(TuiFlex::column().finish())
        .child(content.finish())
        .flex_child(TuiFlex::column().finish())
        .finish()
}

/// Placeholder shown while the user completes device-authorization login. The
/// verification URL/code are surfaced once known (the browser also auto-opens).
pub(crate) fn login_placeholder(
    verification_uri: Option<&str>,
    user_code: Option<&str>,
) -> Box<dyn TuiElement> {
    let dim = TuiStyle::default().add_modifier(Modifier::DIM);
    let mut content =
        TuiFlex::column().child(TuiText::new("Sign in to continue").truncate().finish());
    match (verification_uri, user_code) {
        (Some(uri), Some(code)) => {
            content = content
                .child(
                    TuiText::new(format!("Open {uri} in your browser"))
                        .with_style(dim)
                        .truncate()
                        .finish(),
                )
                .child(
                    TuiText::new(format!("and enter code: {code}"))
                        .with_style(dim)
                        .truncate()
                        .finish(),
                );
        }
        (Some(uri), None) => {
            content = content.child(
                TuiText::new(format!("Open {uri} in your browser"))
                    .with_style(dim)
                    .truncate()
                    .finish(),
            );
        }
        _ => {
            content = content.child(
                TuiText::new("Opening your browser…")
                    .with_style(dim)
                    .truncate()
                    .finish(),
            );
        }
    }
    centered(content)
}

/// Placeholder shown between login completion and terminal session creation.
pub(crate) fn terminal_starting() -> Box<dyn TuiElement> {
    let dim = TuiStyle::default().add_modifier(Modifier::DIM);
    centered(
        TuiFlex::column().child(
            TuiText::new("Starting terminal…")
                .with_style(dim)
                .truncate()
                .finish(),
        ),
    )
}

/// Placeholder shown when login fails; the user can quit with `Ctrl-C`.
pub(crate) fn login_failed(message: &str) -> Box<dyn TuiElement> {
    let dim = TuiStyle::default().add_modifier(Modifier::DIM);
    let content = TuiFlex::column()
        .child(
            TuiText::new(format!("Login failed: {message}"))
                .truncate()
                .finish(),
        )
        .child(
            TuiText::new("Press Ctrl-C to exit.")
                .with_style(dim)
                .truncate()
                .finish(),
        );
    centered(content)
}

#[cfg(test)]
#[path = "ui_tests.rs"]
mod tests;
