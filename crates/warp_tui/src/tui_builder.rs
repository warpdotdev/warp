//! [`TuiUiBuilder`]: the TUI counterpart of the GUI's `UiBuilder`
//! (`warp_core::ui::builder`). It owns the theme→style recipes so TUI views
//! ask for semantic styles ("primary text", "muted text") or ready-styled
//! components instead of hand-deriving [`TuiStyle`]s from the theme.
//! Composition and layout stay with the views and the element library; the
//! builder only owns styles.

use pathfinder_color::ColorU;
use warp::tui_export::Appearance;
use warp_core::ui::color::blend::Blend;
use warp_core::ui::color::Opacity;
use warp_core::ui::theme::{Fill as ThemeFill, WarpTheme};
use warpui::SingletonEntity;
use warpui_core::elements::tui::{
    tui_collapsible, Color, Modifier, TuiElement, TuiEventContext, TuiStyle,
};
use warpui_core::elements::{Fill as CoreFill, MouseStateHandle};
use warpui_core::AppContext;

use crate::terminal_background::probed_colors;

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

    /// Style for primary response/body text: the theme foreground at the
    /// theme's main-text strength (the GUI's `text_main` recipe). This remains
    /// readable on light and custom themes where the ANSI white slot would
    /// wash out.
    pub(crate) fn primary_text_style(&self) -> TuiStyle {
        TuiStyle::default()
            .fg(self.foreground_text_color(self.warp_theme.details().main_text_opacity))
    }

    /// The theme foreground over the transcript's base background at
    /// `opacity` percent. Pre-blended to a solid because terminal cells drop
    /// the alpha channel that the GUI's text tokens rely on.
    fn foreground_text_color(&self, opacity: Opacity) -> Color {
        cell_color(
            self.base_background()
                .blend(&self.warp_theme.foreground().with_opacity(opacity)),
        )
    }

    /// Style for muted secondary text (e.g. thinking headers, bodies, and
    /// footer metadata): the theme foreground at the theme's sub-text
    /// strength. This remains readable across dark, light, and custom themes.
    pub(crate) fn muted_text_style(&self) -> TuiStyle {
        TuiStyle::default()
            .fg(self.foreground_text_color(self.warp_theme.details().sub_text_opacity))
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

    /// Style for added diff lines and `+n` counts (theme green).
    pub(crate) fn diff_added_style(&self) -> TuiStyle {
        TuiStyle::default().fg(cell_color(ThemeFill::from(
            self.warp_theme.terminal_colors().normal.green,
        )))
    }

    /// Style for removed diff lines and `−n` counts (theme red).
    pub(crate) fn diff_removed_style(&self) -> TuiStyle {
        TuiStyle::default().fg(cell_color(ThemeFill::from(
            self.warp_theme.terminal_colors().normal.red,
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

    /// Full-strength accent text, distinct from translucent accent borders.
    pub(crate) fn accent_text_style(&self) -> TuiStyle {
        TuiStyle::default().fg(cell_color(ThemeFill::from(
            self.warp_theme.terminal_colors().normal.cyan,
        )))
    }

    /// Blue command-name text used by the slash-command menu and recognized
    /// slash-command prefixes in the input.
    pub(crate) fn slash_command_text_style(&self) -> TuiStyle {
        TuiStyle::default().fg(cell_color(ThemeFill::Solid(self.warp_theme.ansi_fg_blue())))
    }

    /// Solid cyan selection background used by the slash-command menu.
    pub(crate) fn slash_command_selection_background(&self) -> Color {
        cell_color(ThemeFill::from(
            self.warp_theme.terminal_colors().normal.cyan,
        ))
    }

    /// Bold, contrast-derived text over the slash-command selection color.
    pub(crate) fn slash_command_selection_text_style(&self) -> TuiStyle {
        let background_fill = ThemeFill::from(self.warp_theme.terminal_colors().normal.cyan);
        let foreground = self.warp_theme.font_color(background_fill.into_solid());
        TuiStyle::default()
            .fg(cell_color(foreground))
            .bg(cell_color(background_fill))
            .add_modifier(Modifier::BOLD)
    }
    /// Bold accent prompt marker over the submitted-input background.
    pub(crate) fn input_prefix_style(&self) -> TuiStyle {
        self.accent_text_style()
            .bg(self.input_background())
            .add_modifier(Modifier::BOLD)
    }

    /// The accent-tinted background behind the user-input section.
    pub(crate) fn input_background(&self) -> Color {
        let accent = ThemeFill::from(self.warp_theme.terminal_colors().normal.cyan);
        cell_color(
            self.base_background()
                .blend(&accent.with_opacity(10))
                .blend(&accent.with_opacity(10)),
        )
    }

    /// Blue-overlay background for inline plan bodies, matching the TUI
    /// design's `blue_overlay_1` treatment.
    pub(crate) fn plan_background(&self) -> Color {
        let blue = ThemeFill::Solid(self.warp_theme.ansi_fg_blue());
        cell_color(self.base_background().blend(&blue.with_opacity(10)))
    }

    /// The background the transcript actually renders over: default cells
    /// stay bg-unset, so it is the terminal's *own* background when the
    /// startup probe captured it, else the theme background as the closest
    /// approximation.
    fn base_background(&self) -> ThemeFill {
        match probed_colors().bg {
            Some(bg) => ThemeFill::Solid(ColorU::new(bg.r, bg.g, bg.b, u8::MAX)),
            None => self.warp_theme.background(),
        }
    }

    /// Accent-colored border style for focused/primary containers. The design
    /// uses the cyan token at 50%; pre-blend it because terminal cells do not
    /// preserve alpha.
    pub(crate) fn accent_border_style(&self) -> TuiStyle {
        let accent = ThemeFill::from(self.warp_theme.terminal_colors().normal.cyan);
        TuiStyle::default().fg(cell_color(
            self.base_background().blend(&accent.with_opacity(50)),
        ))
    }

    /// Style in the shell-mode accent color (the same blue the GUI uses for
    /// `!` shell mode).
    pub(crate) fn shell_mode_accent_style(&self) -> TuiStyle {
        TuiStyle::default().fg(cell_color(ThemeFill::Solid(self.warp_theme.ansi_fg_blue())))
    }

    /// The warping indicator's base fill: the terminal palette's bright
    /// magenta, corresponding to the design's Lilac-200.
    fn warping_base_fill(&self) -> ThemeFill {
        ThemeFill::from(self.warp_theme.terminal_colors().bright.magenta)
    }

    /// The warping indicator's base color as a solid color, for per-glyph
    /// shimmer lerping.
    pub(crate) fn warping_base_color(&self) -> ColorU {
        self.warping_base_fill().into_solid()
    }

    /// The peak color the "Warping" shimmer band lerps toward: a theme text
    /// color selected for contrast against the resolved terminal background.
    pub(crate) fn warping_shimmer_color(&self) -> ColorU {
        self.warp_theme
            .font_color(self.base_background())
            .into_solid()
    }

    /// Style for the warping indicator's spinner glyph.
    pub(crate) fn warping_spinner_style(&self) -> TuiStyle {
        TuiStyle::default().fg(cell_color(self.warping_base_fill()))
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
        let style = if mouse_state.lock().unwrap().is_hovered() {
            self.hovered_header_style()
        } else {
            self.muted_text_style()
        };
        tui_collapsible(
            collapsed,
            [(label.into(), style)],
            style,
            mouse_state,
            move || body,
            on_toggle,
        )
    }

    /// Prominent [`tui_collapsible`] variant: a bold primary-text header of
    /// a leading `glyph` and a `label` (e.g. the task-list header, which the
    /// design renders bold white). Since the header is already bold, hover
    /// signals with an underline instead of the muted collapsible's
    /// brighten-on-hover — applied to the label only, so the decorative
    /// glyph and the chevron don't pick up a clashing underline.
    pub(crate) fn prominent_collapsible(
        &self,
        collapsed: bool,
        glyph: impl Into<String>,
        label: impl Into<String>,
        mouse_state: MouseStateHandle,
        body: Box<dyn TuiElement>,
        on_toggle: impl FnMut(&mut TuiEventContext, &AppContext) + 'static,
    ) -> Box<dyn TuiElement> {
        let header_style = self.primary_text_style().add_modifier(Modifier::BOLD);
        let label_style = if mouse_state.lock().unwrap().is_hovered() {
            header_style.add_modifier(Modifier::UNDERLINED)
        } else {
            header_style
        };
        tui_collapsible(
            collapsed,
            [
                (format!("{} ", glyph.into()), header_style),
                (label.into(), label_style),
            ],
            header_style,
            mouse_state,
            move || body,
            on_toggle,
        )
    }
}

/// Converts a theme fill into a terminal-cell color.
fn cell_color(fill: ThemeFill) -> Color {
    CoreFill::from(fill).into()
}

#[cfg(test)]
#[path = "tui_builder_tests.rs"]
mod tests;
