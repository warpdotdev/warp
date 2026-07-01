//! Small presentation helpers for the `warp-tui` front-end's TUI views.

use warpui_core::elements::tui::{Modifier, TuiColumn, TuiElement, TuiStyle, TuiText};

/// Vertically centers `content` by padding above and below with flex spacers.
pub(crate) fn centered(content: TuiColumn) -> Box<dyn TuiElement> {
    TuiColumn::new()
        .flex_child(TuiColumn::new())
        .child(content)
        .flex_child(TuiColumn::new())
        .finish()
}

/// Placeholder shown while the user completes device-authorization login. The
/// verification URL/code are surfaced once known (the browser also auto-opens).
pub(crate) fn login_placeholder(
    verification_uri: Option<&str>,
    user_code: Option<&str>,
) -> Box<dyn TuiElement> {
    let dim = TuiStyle::default().add_modifier(Modifier::DIM);
    let mut content = TuiColumn::new().child(TuiText::new("Sign in to continue").truncate());
    match (verification_uri, user_code) {
        (Some(uri), Some(code)) => {
            content = content
                .child(
                    TuiText::new(format!("Open {uri} in your browser"))
                        .with_style(dim)
                        .truncate(),
                )
                .child(
                    TuiText::new(format!("and enter code: {code}"))
                        .with_style(dim)
                        .truncate(),
                );
        }
        (Some(uri), None) => {
            content = content.child(
                TuiText::new(format!("Open {uri} in your browser"))
                    .with_style(dim)
                    .truncate(),
            );
        }
        _ => {
            content = content.child(
                TuiText::new("Opening your browser…")
                    .with_style(dim)
                    .truncate(),
            );
        }
    }
    centered(content)
}

/// Placeholder shown between login completion and terminal session creation.
pub(crate) fn terminal_starting() -> Box<dyn TuiElement> {
    let dim = TuiStyle::default().add_modifier(Modifier::DIM);
    centered(
        TuiColumn::new().child(
            TuiText::new("Starting terminal…")
                .with_style(dim)
                .truncate(),
        ),
    )
}

/// Placeholder shown when login fails; the user can quit with `Ctrl-C`.
pub(crate) fn login_failed(message: &str) -> Box<dyn TuiElement> {
    let dim = TuiStyle::default().add_modifier(Modifier::DIM);
    let content = TuiColumn::new()
        .child(TuiText::new(format!("Login failed: {message}")).truncate())
        .child(
            TuiText::new("Press Ctrl-C to exit.")
                .with_style(dim)
                .truncate(),
        );
    centered(content)
}
