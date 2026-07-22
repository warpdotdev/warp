//! Small presentation helpers for the `warp-tui` front-end's TUI views.
use std::time::Duration;

use warpui_core::AppContext;
use warpui_core::elements::CrossAxisAlignment;
use warpui_core::elements::animation::AnimationClock;
use warpui_core::elements::tui::{
    Modifier, TuiConstrainedBox, TuiElement, TuiFlex, TuiStyle, TuiText,
};

use crate::tui_builder::TuiUiBuilder;
use crate::warping_indicator::render_spinner;

/// Abbreviates a leading home-directory prefix of `path` to `~`.
pub(crate) fn abbreviate_home_prefix(path: &str) -> String {
    if let Some(home) = dirs::home_dir() {
        let home = home.to_string_lossy();
        if let Some(rest) = path.strip_prefix(&*home)
            && (rest.is_empty() || rest.starts_with('/') || rest.starts_with('\\'))
        {
            return format!("~{rest}");
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

/// Placeholder shown while a requested conversation is restored.
pub(crate) fn conversation_restoring(app: &AppContext) -> Box<dyn TuiElement> {
    let muted = TuiUiBuilder::from_app(app).muted_text_style();
    let hint = "Esc or Ctrl-C to cancel and start a new session";

    centered_in_viewport(
        TuiConstrainedBox::new(
            TuiFlex::column()
                .child(render_spinner(
                    AnimationClock::starting_at(Duration::ZERO),
                    muted,
                ))
                .child(
                    TuiText::new("Loading session...")
                        .with_style(muted)
                        .truncate()
                        .finish(),
                )
                .child(TuiText::new(hint).with_style(muted).truncate().finish())
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .finish(),
        )
        .with_max_cols(hint.len() as u16)
        .finish(),
    )
}

/// Placeholder shown when a requested conversation cannot be restored.
pub(crate) fn conversation_restore_failed(message: &str) -> Box<dyn TuiElement> {
    let dim = TuiStyle::default().add_modifier(Modifier::DIM);
    vertically_centered(
        TuiFlex::column()
            .child(
                TuiText::new(format!("Could not restore conversation: {message}"))
                    .truncate()
                    .finish(),
            )
            .child(
                TuiText::new("Press Ctrl-C to exit.")
                    .with_style(dim)
                    .truncate()
                    .finish(),
            ),
    )
}

/// Vertically centers `content` with its existing horizontal alignment.
fn vertically_centered(content: TuiFlex) -> Box<dyn TuiElement> {
    TuiFlex::column()
        .flex_child(TuiFlex::column().finish())
        .child(content.finish())
        .flex_child(TuiFlex::column().finish())
        .finish()
}

/// Centers `content` horizontally and vertically within the viewport.
fn centered_in_viewport(content: Box<dyn TuiElement>) -> Box<dyn TuiElement> {
    let centered_row = TuiFlex::row()
        .flex_child(TuiFlex::row().finish())
        .child(content)
        .flex_child(TuiFlex::row().finish());
    TuiFlex::column()
        .flex_child(TuiFlex::column().finish())
        .child(centered_row.finish())
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
    vertically_centered(content)
}

/// Placeholder shown between login completion and terminal session creation.
pub(crate) fn terminal_starting() -> Box<dyn TuiElement> {
    let dim = TuiStyle::default().add_modifier(Modifier::DIM);
    vertically_centered(
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
    vertically_centered(content)
}

#[cfg(test)]
#[path = "ui_tests.rs"]
mod tests;
