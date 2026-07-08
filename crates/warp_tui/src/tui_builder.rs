//! [`TuiUiBuilder`]: the TUI counterpart of the GUI's `UiBuilder`
//! (`warp_core::ui::builder`). It owns the theme→style recipes so TUI views
//! ask for semantic styles ("primary text", "muted text") or ready-styled
//! components instead of hand-deriving [`TuiStyle`]s from the theme.
//! Composition and layout stay with the views and the element library; the
//! builder only owns styles.

use warp::tui_export::Appearance;
use warp_core::ui::color::blend::Blend;
use warp_core::ui::theme::{Fill as ThemeFill, WarpTheme};
use warpui::SingletonEntity;
use warpui_core::elements::tui::{
    tui_collapsible, Color, Modifier, TuiElement, TuiEventContext, TuiStyle,
};
use warpui_core::elements::{Fill as CoreFill, MouseStateHandle};
use warpui_core::AppContext;

/// Theme-derived styles and components for the TUI, mirroring the GUI's
/// `UiBuilder` (minus fonts, which terminal cells don't have). Cheap to
/// construct per render via [`TuiUiBuilder::from_app`].
#[derive(Clone, Debug)]
pub(crate) struct TuiUiBuilder {
    warp_theme: WarpTheme,
}

impl TuiUiBuilder {
    /// Creates a builder from the current [`Appearance`] theme.
    pub(crate) fn from_app(app: &AppContext) -> Self {
        Self {
            warp_theme: Appearance::as_ref(app).theme().clone(),
        }
    }

    /// Style for primary response/body text.
    pub(crate) fn primary_text_style(&self) -> TuiStyle {
        TuiStyle::default().fg(cell_color(ThemeFill::from(
            self.warp_theme.terminal_colors().normal.white,
        )))
    }

    /// Style for muted secondary text (e.g. thinking headers and bodies).
    pub(crate) fn muted_text_style(&self) -> TuiStyle {
        TuiStyle::default().fg(cell_color(ThemeFill::from(
            self.warp_theme.terminal_colors().bright.black,
        )))
    }

    /// Muted and dimmed: de-emphasized status rows (e.g. tool-call stubs).
    pub(crate) fn dim_text_style(&self) -> TuiStyle {
        self.muted_text_style().add_modifier(Modifier::DIM)
    }

    /// Style for error text (e.g. failed tool-call glyphs).
    pub(crate) fn error_text_style(&self) -> TuiStyle {
        TuiStyle::default().fg(cell_color(ThemeFill::from(
            self.warp_theme.terminal_colors().normal.red,
        )))
    }

    /// Green success glyph (e.g. ✓ on completed tool calls), mirroring the
    /// GUI's `green_check_icon`.
    pub(crate) fn success_glyph_style(&self) -> TuiStyle {
        TuiStyle::default().fg(cell_color(ThemeFill::from(
            self.warp_theme.terminal_colors().normal.green,
        )))
    }

    /// Yellow attention glyph for executing or approval-blocked tool calls,
    /// mirroring the GUI's `yellow_running_icon` / `yellow_stop_icon`.
    pub(crate) fn attention_glyph_style(&self) -> TuiStyle {
        TuiStyle::default().fg(cell_color(ThemeFill::from(
            self.warp_theme.terminal_colors().normal.yellow,
        )))
    }

    /// Bold foreground over the accent-tinted input background; pair with
    /// [`Self::input_background`] on the enclosing container.
    pub(crate) fn input_text_style(&self) -> TuiStyle {
        TuiStyle::default()
            .fg(cell_color(self.warp_theme.foreground()))
            .bg(self.input_background())
            .add_modifier(Modifier::BOLD)
    }

    /// The accent-tinted background behind the user-input section.
    pub(crate) fn input_background(&self) -> Color {
        let accent = ThemeFill::from(self.warp_theme.terminal_colors().normal.cyan);
        cell_color(self.warp_theme.background().blend(&accent.with_opacity(20)))
    }

    /// Accent-colored border style for focused/primary containers.
    pub(crate) fn accent_border_style(&self) -> TuiStyle {
        TuiStyle::default().fg(cell_color(ThemeFill::from(
            self.warp_theme.terminal_colors().normal.cyan,
        )))
    }

    /// Collapsible-header style while the pointer hovers it.
    fn hovered_header_style(&self) -> TuiStyle {
        self.primary_text_style().add_modifier(Modifier::BOLD)
    }

    /// Themed [`tui_collapsible`]: a muted header that brightens to bold
    /// primary text while hovered, over the caller's body element.
    pub(crate) fn collapsible(
        &self,
        collapsed: bool,
        label: impl Into<String>,
        mouse_state: MouseStateHandle,
        body: Box<dyn TuiElement>,
        on_toggle: impl FnMut(&mut TuiEventContext, &AppContext) + 'static,
    ) -> Box<dyn TuiElement> {
        tui_collapsible(
            collapsed,
            label,
            self.muted_text_style(),
            self.hovered_header_style(),
            mouse_state,
            body,
            on_toggle,
        )
    }
}

/// Converts a theme fill into a terminal-cell color.
fn cell_color(fill: ThemeFill) -> Color {
    CoreFill::from(fill).into()
}
