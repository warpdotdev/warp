//! Small presentation helpers for the `warp-tui` front-end's TUI views.
use std::sync::Arc;
use std::time::Duration;

use warpui_core::AppContext;
use warpui_core::elements::animation::AnimationClock;
use warpui_core::elements::tui::{
    Color, Modifier, TuiConstrainedBox, TuiContainer, TuiElement, TuiEventContext, TuiEventHandler,
    TuiFlex, TuiHoverable, TuiStack, TuiStyle, TuiText,
};
use warpui_core::elements::{CrossAxisAlignment, MouseStateHandle};

use crate::transient_hint::TransientHintTone;
use crate::tui_builder::TuiUiBuilder;
use crate::warping_indicator::render_spinner;
use crate::zero_state_animation::{
    WarpLogoStyles, ZeroStateAnimationConfig, ZeroStateAnimationElement, ZeroStateStarfieldElement,
};

const AUTH_COPY_COLS: u16 = 48;
const AUTH_ANIMATION_COLS: u16 = 32;

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

/// Centers `content` horizontally within its available row.
pub(crate) fn horizontally_centered(content: Box<dyn TuiElement>) -> Box<dyn TuiElement> {
    TuiFlex::row()
        .flex_child(TuiFlex::row().finish())
        .child(content)
        .flex_child(TuiFlex::row().finish())
        .finish()
}

/// Centers `content` horizontally and vertically within the viewport.
pub(crate) fn centered_in_viewport(content: Box<dyn TuiElement>) -> Box<dyn TuiElement> {
    TuiFlex::column()
        .flex_child(TuiFlex::column().finish())
        .child(horizontally_centered(content))
        .flex_child(TuiFlex::column().finish())
        .finish()
}

/// Signed-out welcome shown before browser device authorization begins.
pub(crate) fn signed_out_welcome(
    clock: AnimationClock,
    animation_config: Arc<ZeroStateAnimationConfig>,
    app: &AppContext,
    mut on_login: impl FnMut(&mut TuiEventContext, &AppContext) + 'static,
) -> Box<dyn TuiElement> {
    let builder = TuiUiBuilder::from_app(app);
    let primary = builder.primary_text_style();
    let muted = builder.muted_text_style();
    let title = builder
        .credential_entry_accent_style()
        .add_modifier(Modifier::BOLD);
    let success = builder.success_glyph_style();
    let content = TuiFlex::column()
        .child(
            TuiText::new("Welcome to Warp")
                .with_style(title)
                .truncate()
                .finish(),
        )
        .child(
            TuiText::from_spans([
                ("> ".to_owned(), success),
                ("Press ".to_owned(), muted),
                ("enter".to_owned(), success.add_modifier(Modifier::BOLD)),
                (" to get started".to_owned(), muted),
            ])
            .finish(),
        )
        .child(blank_row())
        .child(blank_row())
        .child(
            TuiText::new("What’s different about Warp")
                .with_style(muted)
                .finish(),
        )
        .child(capability_row(
            "⟡",
            "Prompts or shell commands autodetected",
            builder.credential_entry_accent_style(),
            primary,
        ))
        .child(capability_row(
            "⊹",
            "Set up custom model routers",
            builder.link_text_style(),
            primary,
        ))
        .child(capability_row(
            "✶",
            "Orchestrate fleets of agents",
            success,
            primary,
        ))
        .child(capability_row(
            "*",
            "Run full-screen terminal apps",
            builder.credential_entry_accent_style(),
            primary,
        ))
        .child(capability_row(
            "◊",
            "Persist sessions through state changes",
            builder.attention_glyph_style(),
            primary,
        ))
        .finish();
    TuiEventHandler::new(auth_layout(clock, animation_config, content, &builder))
        .on_key("enter", move |_, event_ctx, app| {
            on_login(event_ctx, app);
        })
        .finish()
}

/// Browser interaction state for the waiting login screen.
pub(crate) struct LoginWaitingParams<'a> {
    pub(crate) browser_url: Option<&'a str>,
    pub(crate) login_mouse: MouseStateHandle,
    pub(crate) copy_mouse: MouseStateHandle,
    pub(crate) copy_feedback: Option<(&'a str, TransientHintTone)>,
}

/// Waiting state shown after device authorization starts.
pub(crate) fn login_waiting(
    clock: AnimationClock,
    animation_config: Arc<ZeroStateAnimationConfig>,
    params: LoginWaitingParams<'_>,
    app: &AppContext,
    on_open: impl FnMut(&mut TuiEventContext, &AppContext) + 'static,
    on_copy: impl FnMut(&mut TuiEventContext, &AppContext) + Clone + 'static,
) -> Box<dyn TuiElement> {
    let LoginWaitingParams {
        browser_url,
        login_mouse,
        copy_mouse,
        copy_feedback,
    } = params;
    let builder = TuiUiBuilder::from_app(app);
    let primary = builder.primary_text_style();
    let muted = builder.muted_text_style();
    let title = builder
        .credential_entry_accent_style()
        .add_modifier(Modifier::BOLD);
    let mut content = TuiFlex::column()
        .child(
            TuiText::new("Welcome to Warp")
                .with_style(title)
                .truncate()
                .finish(),
        )
        .child(
            TuiText::from_spans([
                ("● ".to_owned(), builder.attention_glyph_style()),
                ("Waiting for login...".to_owned(), primary),
            ])
            .finish(),
        )
        .child(blank_row())
        .child(blank_row());

    let has_browser_url = browser_url.is_some();
    let mut on_copy_key = on_copy.clone();
    if let Some(browser_url) = browser_url {
        let link_style = if login_mouse.lock().is_ok_and(|state| state.is_hovered()) {
            primary
                .add_modifier(Modifier::BOLD)
                .add_modifier(Modifier::UNDERLINED)
        } else {
            primary.add_modifier(Modifier::UNDERLINED)
        };
        content = content.child(
            TuiHoverable::new(
                login_mouse,
                TuiText::from_spans([
                    ("Visit ".to_owned(), muted),
                    (browser_url.to_owned(), link_style),
                    (" to get started, then come back here.".to_owned(), muted),
                ])
                .finish(),
            )
            .on_click(on_open)
            .finish(),
        );
        let copy_hovered = copy_mouse.lock().is_ok_and(|state| state.is_hovered());
        let (copy_label, copy_style) = match copy_feedback {
            Some((message, TransientHintTone::Muted)) => (message.to_owned(), muted),
            Some((message, TransientHintTone::Success)) => {
                (message.to_owned(), builder.success_glyph_style())
            }
            Some((message, TransientHintTone::Error)) => {
                (message.to_owned(), builder.error_text_style())
            }
            None => {
                let style = if copy_hovered {
                    primary.add_modifier(Modifier::BOLD)
                } else {
                    builder.link_text_style()
                };
                ("Copy URL (c)".to_owned(), style)
            }
        };
        content = content.child(
            TuiHoverable::new(
                copy_mouse,
                TuiText::new(copy_label)
                    .with_style(copy_style)
                    .truncate()
                    .finish(),
            )
            .on_click(on_copy)
            .finish(),
        );
    } else {
        content = content.child(
            TuiText::new("Requesting a secure sign-in link...")
                .with_style(muted)
                .finish(),
        );
    }

    let content = auth_layout(clock, animation_config, content.finish(), &builder);
    if has_browser_url {
        TuiEventHandler::new(content)
            .on_key("c", move |_, event_ctx, app| {
                on_copy_key(event_ctx, app);
            })
            .finish()
    } else {
        content
    }
}

fn capability_row(
    glyph: &str,
    label: &str,
    glyph_style: TuiStyle,
    text_style: TuiStyle,
) -> Box<dyn TuiElement> {
    TuiText::from_spans([
        (format!("{glyph} "), glyph_style),
        (label.to_owned(), text_style),
    ])
    .finish()
}

fn auth_layout(
    clock: AnimationClock,
    animation_config: Arc<ZeroStateAnimationConfig>,
    content: Box<dyn TuiElement>,
    builder: &TuiUiBuilder,
) -> Box<dyn TuiElement> {
    let starfield = ZeroStateStarfieldElement::new(
        clock,
        builder.muted_text_style(),
        AUTH_COPY_COLS,
        AUTH_ANIMATION_COLS,
    )
    .finish();
    let animation = ZeroStateAnimationElement::new(
        clock,
        animation_config,
        WarpLogoStyles {
            front: builder.accent_text_style(),
            back: builder.primary_text_style(),
            side: builder.dim_text_style(),
            background: builder.muted_text_style(),
        },
    )
    .without_background_stars()
    .finish();
    let copy_reservation = TuiConstrainedBox::new(TuiText::new("").finish())
        .with_min_cols(AUTH_COPY_COLS)
        .with_max_cols(AUTH_COPY_COLS)
        .finish();
    let animation = TuiConstrainedBox::new(animation)
        .with_max_cols(AUTH_ANIMATION_COLS)
        .finish();
    let animation_region = TuiFlex::row()
        .flex_child(TuiText::new("").finish())
        .child(animation)
        .flex_child(TuiText::new("").finish())
        .finish();
    let animation_layer = TuiFlex::row()
        .child(copy_reservation)
        .flex_child(animation_region)
        .finish();

    let content = TuiContainer::new(content)
        .with_padding_left(3)
        .with_background(Color::Reset)
        .finish();
    let content = TuiConstrainedBox::new(content)
        .with_min_cols(AUTH_COPY_COLS)
        .with_max_cols(AUTH_COPY_COLS)
        .finish();
    let content_layer = TuiFlex::column()
        .flex_child(TuiText::new("").finish())
        .child(content)
        .flex_child(TuiText::new("").finish())
        .finish();

    TuiStack::new()
        .child(starfield)
        .child(animation_layer)
        .child(content_layer)
        .finish()
}

fn blank_row() -> Box<dyn TuiElement> {
    TuiText::new(" ").truncate().finish()
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
