//! Small presentation helpers for the `warp-tui` front-end's TUI views.
use std::time::Duration;

use warpui_core::AppContext;
use warpui_core::elements::CrossAxisAlignment;
use warpui_core::elements::animation::AnimationClock;
use warpui_core::elements::tui::{
    Modifier, TuiConstrainedBox, TuiConstraint, TuiElement, TuiFlex, TuiLayoutContext,
    TuiPaintContext, TuiPaintSurface, TuiScreenPosition, TuiSize, TuiStyle, TuiText, text_width,
    truncate_with_ellipsis,
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

/// Shortens a path for the one-line session footer so it fits within `budget`
/// display columns, eliding as little as possible.
///
/// The home prefix is always abbreviated to `~`. When the abbreviated path fits
/// the budget it is returned in full — the footer only elides when the row
/// genuinely runs out of room. When it does not fit, interior components are
/// dropped progressively (widest fitting form wins), preserving the root/home
/// prefix and basename where possible (`~/projects/research` →
/// `~/…/research`). If even `…/basename` cannot fit, the basename itself is
/// grapheme-truncated with an ellipsis as a last resort. All measurements are
/// display-cell widths, so multi-byte and wide characters never panic or
/// overflow the row.
pub(crate) fn elide_footer_path(path: &str, budget: u16) -> String {
    let path = abbreviate_home_prefix(path);
    if text_width(&path) <= budget {
        return path;
    }

    let separator = if path.contains('\\') && !path.contains('/') {
        '\\'
    } else {
        '/'
    };
    let separator_str = separator.to_string();
    let leading = path.starts_with(separator);
    let components: Vec<&str> = path
        .split(separator)
        .filter(|component| !component.is_empty())
        .collect();
    let last = components.last().copied().unwrap_or_default();

    // With two or fewer components there is no interior to elide, so fall back
    // to grapheme truncation of the whole abbreviated path.
    if components.len() <= 2 {
        return truncate_with_ellipsis(&path, usize::from(budget));
    }

    let count = components.len();
    let first = components[0];
    // Prefix-preserving candidates: keep the root/first component and a growing
    // suffix of trailing components, eliding the interior with `…`.
    let mut prefix_candidates: Vec<String> = Vec::new();
    for kept in (1..=count - 2).rev() {
        let suffix = components[count - kept..].join(&separator_str);
        prefix_candidates.push(if leading {
            format!("{separator}{first}{separator}…{separator}{suffix}")
        } else {
            format!("{first}{separator}…{separator}{suffix}")
        });
    }
    // Fallback candidates: drop the leading component too, keeping only `…` plus
    // a trailing suffix. These sacrifice the root/home prefix, so they are only
    // considered when no prefix-preserving form fits.
    let mut fallback_candidates: Vec<String> = Vec::new();
    for kept in (1..=count - 1).rev() {
        let suffix = components[count - kept..].join(&separator_str);
        fallback_candidates.push(if leading {
            format!("{separator}…{separator}{suffix}")
        } else {
            format!("…{separator}{suffix}")
        });
    }

    // Prefer the widest prefix-preserving candidate that fits, so a useful
    // root/home prefix is retained whenever possible. Only when no
    // prefix-preserving form fits do we fall back to the leading-component-free
    // forms; and only when nothing fits at all do we grapheme-truncate the
    // basename as a last resort.
    let widest_fitting = |candidates: Vec<String>| -> Option<String> {
        candidates
            .into_iter()
            .filter(|candidate| text_width(candidate) <= budget)
            .max_by_key(|candidate| text_width(candidate))
    };

    widest_fitting(prefix_candidates)
        .or_else(|| widest_fitting(fallback_candidates))
        .unwrap_or_else(|| truncate_with_ellipsis(last, usize::from(budget)))
}

/// The footer's working-directory segment: a width-aware label that defers path
/// elision to layout. Unlike a plain [`TuiText`], it consults the width it is
/// actually granted (minus `reserved` columns held for the segments rendered
/// after it) so the cwd only shortens when the row genuinely runs out of room.
pub(crate) struct WorkingDirectoryLabel {
    /// The raw working directory; home abbreviation and elision happen at
    /// layout time against the granted width.
    path: String,
    style: TuiStyle,
    /// Columns to leave for the segments rendered after this one, so eliding
    /// the cwd never crowds out the git branch, usage, etc.
    reserved: u16,
    /// The elided text resolved during the most recent layout.
    text: Option<TuiText>,
    size: Option<TuiSize>,
}

impl WorkingDirectoryLabel {
    pub(crate) fn new(path: String, style: TuiStyle, reserved: u16) -> Self {
        Self {
            path,
            style,
            reserved,
            text: None,
            size: None,
        }
    }
}

impl TuiElement for WorkingDirectoryLabel {
    fn layout(
        &mut self,
        constraint: TuiConstraint,
        ctx: &mut TuiLayoutContext,
        app: &AppContext,
    ) -> TuiSize {
        let budget = constraint.max.width.saturating_sub(self.reserved);
        let mut text = TuiText::new(elide_footer_path(&self.path, budget))
            .with_style(self.style)
            .truncate();
        let size = text.layout(constraint, ctx, app);
        self.text = Some(text);
        self.size = Some(size);
        size
    }

    fn render(
        &mut self,
        origin: TuiScreenPosition,
        surface: &mut TuiPaintSurface<'_>,
        ctx: &mut TuiPaintContext,
    ) {
        if let Some(text) = &mut self.text {
            text.render(origin, surface, ctx);
        }
    }

    fn size(&self) -> Option<TuiSize> {
        self.size
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
pub(crate) fn centered_in_viewport(content: Box<dyn TuiElement>) -> Box<dyn TuiElement> {
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
                TuiText::new("Requesting a sign-in link…")
                    .with_style(dim)
                    .truncate()
                    .finish(),
            );
        }
    }
    centered_in_viewport(
        content
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .finish(),
    )
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
