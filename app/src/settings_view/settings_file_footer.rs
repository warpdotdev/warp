//! Footer rendered at the bottom of the settings nav rail.
//!
//! The footer is the user's always-visible entrypoint into `settings.toml`.
//! It takes one of three forms:
//! * Hidden when the `SettingsFile` feature flag is disabled.
//! * An inline yellow error alert (mirroring the workspace-level banner in
//!   `Workspace::render_settings_error_banner`) when the settings file has an
//!   error *and* the user has dismissed the workspace banner.
//! * Otherwise, a plain bordered "Open settings file" button.
use pathfinder_color::ColorU;
use warp_core::ui::color::coloru_with_opacity;
use warp_core::ui::theme::Fill;
use warpui::elements::{
    Border, Clipped, ClippedScrollStateHandle, ClippedScrollable, ConstrainedBox, Container,
    CornerRadius, CrossAxisAlignment, Element, Empty, Expanded, Flex, Highlight, Hoverable,
    MainAxisAlignment, MainAxisSize, MouseStateHandle, ParentElement, Radius, ScrollbarWidth, Text,
    Wrap,
};
use warpui::fonts::{FamilyId, Properties, Weight};
use warpui::platform::Cursor;

use crate::appearance::Appearance;
use crate::settings::SettingsFileError;
use crate::ui_components::icons::Icon;
use crate::WorkspaceAction;

/// Horizontal + vertical padding applied to the footer inside the sidebar.
const FOOTER_PADDING: f32 = 12.;
/// Font size used for the button label and the alert copy; matches the
/// Figma spec for both designs.
const FOOTER_FONT_SIZE: f32 = 12.;
/// Height of the plain "Open settings file" button.
const OPEN_BUTTON_HEIGHT: f32 = 32.;
/// Height of action buttons inside the error alert.
const ALERT_ACTION_BUTTON_HEIGHT: f32 = 24.;
/// Size of the leading icons (search-sm, code-02, alert-circle, oz).
const FOOTER_ICON_SIZE: f32 = 16.;
/// Size of the Oz brand mark inside the "Fix with Oz" button. Matches the
/// Figma spec and the workspace banner's secondary-button icon sizing.
const ALERT_OZ_ICON_SIZE: f32 = 14.;
/// Horizontal padding inside the "Open file" / "Fix with Oz" action buttons.
/// Matches the workspace banner's secondary button pad.
const ALERT_BUTTON_HORIZONTAL_PADDING: f32 = 8.;
/// Spacing between the two action buttons when they fit on one row.
const ALERT_BUTTON_SPACING: f32 = 4.;
/// Maximum height of the scrollable text region inside the error alert. If
/// the settings error's description exceeds this, the text scrolls within
/// the alert so the footer doesn't balloon to fill the sidebar. The action
/// buttons below the text always remain visible.
const ALERT_TEXT_MAX_HEIGHT: f32 = 140.;

/// Which variant of the footer should be shown.
///
/// Extracted as a pure enum so the decision logic can be unit-tested without
/// rendering a full `SettingsView`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsFooterKind {
    /// Footer is not rendered at all (feature flag off).
    Hidden,
    /// Render the plain "Open settings file" button.
    OpenButton,
    /// Render the inline yellow error alert.
    ErrorAlert,
}

impl SettingsFooterKind {
    /// Decide which footer variant to render based on the three inputs that
    /// gate the footer: the `SettingsFile` feature flag, the current settings
    /// file error (if any), and whether the user has dismissed the workspace
    /// banner for invalid settings.
    pub fn choose(feature_enabled: bool, has_error: bool, banner_dismissed: bool) -> Self {
        if !feature_enabled {
            Self::Hidden
        } else if has_error && banner_dismissed {
            Self::ErrorAlert
        } else {
            Self::OpenButton
        }
    }
}

/// Per-render-persistent handles for the footer. The `MouseStateHandle`s
/// back the three clickable surfaces and must be created once and reused
/// across renders per `WARP.md`; the scroll state handle serves the same
/// purpose for the error alert's scrollable text region.
#[derive(Clone, Default)]
pub struct SettingsFooterMouseStates {
    pub open_settings_file_button: MouseStateHandle,
    pub alert_open_file_button: MouseStateHandle,
    pub alert_fix_with_oz_button: MouseStateHandle,
    /// Scroll state for the error alert's text region (heading +
    /// description), so scroll position survives renders.
    pub alert_text_scroll_state: ClippedScrollStateHandle,
}

/// Renders the plain "Open settings file" button shown in the default state.
///
/// Visual spec (Figma `5655:62575`): 32px tall, full-width, 1px outlined,
/// 4px rounded corners, `code-02` leading icon, semibold label.
pub fn render_open_settings_file_button(
    appearance: &Appearance,
    mouse_state: MouseStateHandle,
) -> Box<dyn Element> {
    let theme = appearance.theme();
    let text_fill = theme.nonactive_ui_text_color();
    let text_color = text_fill.into_solid();
    let border_color = theme.outline().into_solid();
    let ui_font_family = appearance.ui_font_family();

    Hoverable::new(mouse_state, move |state| {
        let icon = ConstrainedBox::new(Icon::Code2.to_warpui_icon(text_fill).finish())
            .with_width(FOOTER_ICON_SIZE)
            .with_height(FOOTER_ICON_SIZE)
            .finish();

        let label = Text::new_inline("Open settings file", ui_font_family, FOOTER_FONT_SIZE)
            .with_color(text_color)
            .with_style(Properties {
                weight: Weight::Semibold,
                ..Default::default()
            })
            .finish();

        // Use `MainAxisSize::Max` so the row (and its surrounding bordered
        // container) expands to fill the full sidebar width. The icon + text
        // are then centered inside that full-width row.
        let row = Flex::row()
            .with_main_axis_alignment(MainAxisAlignment::Center)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_size(MainAxisSize::Max)
            .with_child(Container::new(icon).with_margin_right(4.).finish())
            .with_child(label)
            .finish();

        // Clip the row so the icon + label stay inside the bordered button
        // when the sidebar (which is `Shrinkable` inside the workspace row)
        // is resized narrower than the row's natural content width.
        let mut container = Container::new(Clipped::new(row).finish())
            .with_border(Border::all(1.).with_border_color(border_color))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)));
        if state.is_hovered() {
            container = container.with_background_color(coloru_with_opacity(text_color, 10));
        }

        ConstrainedBox::new(container.finish())
            .with_height(OPEN_BUTTON_HEIGHT)
            .finish()
    })
    .with_cursor(Cursor::PointingHand)
    .on_click(|ctx, _, _| ctx.dispatch_typed_action(WorkspaceAction::OpenSettingsFile))
    .finish()
}

/// Renders the inline yellow alert shown when the settings file has an error
/// and the workspace banner has been dismissed. Mirrors the workspace banner
/// messaging and actions.
pub fn render_settings_error_alert(
    appearance: &Appearance,
    error: &SettingsFileError,
    ai_enabled: bool,
    mouse_states: &SettingsFooterMouseStates,
) -> Box<dyn Element> {
    let theme = appearance.theme();
    // Warning banner colors: yellow background, contrast-safe text on top of
    // the yellow. Same pattern as `Workspace::render_workspace_banner`.
    let bg_color = theme.ansi_fg_yellow();
    let text_color = theme.main_text_color(Fill::Solid(bg_color)).into_solid();
    let ui_font_family = appearance.ui_font_family();

    // ── Heading + description ────────────────────────────────────────────
    // Copy is shared with `Workspace::render_settings_error_banner` via
    // `SettingsFileError::heading_and_description` so the two UIs can't
    // drift out of sync.
    let (heading, description) = error.heading_and_description();
    let heading_char_count = heading.chars().count();
    let combined_text = format!("{heading} {description}");
    // Soft-wrap (the `Text::new` default) is appropriate here since the
    // alert's vertical space grows to fit the text.
    let mut text_widget =
        Text::new(combined_text, ui_font_family, FOOTER_FONT_SIZE).with_color(text_color);
    if heading_char_count > 0 {
        text_widget = text_widget.with_single_highlight(
            Highlight::new().with_properties(Properties::default().weight(Weight::Semibold)),
            (0..heading_char_count).collect(),
        );
    }

    let alert_icon = ConstrainedBox::new(
        Icon::AlertCircle
            .to_warpui_icon(Fill::Solid(text_color))
            .finish(),
    )
    .with_width(FOOTER_ICON_SIZE)
    .with_height(FOOTER_ICON_SIZE)
    .finish();

    let text_row = Flex::row()
        .with_cross_axis_alignment(CrossAxisAlignment::Start)
        .with_main_axis_size(MainAxisSize::Max)
        .with_child(Container::new(alert_icon).with_margin_right(8.).finish())
        .with_child(Expanded::new(1., text_widget.finish()).finish())
        .finish();

    // Cap the text region's height so verbose settings errors don't balloon
    // the alert to fill the sidebar. Overflow scrolls within the alert; the
    // action buttons below stay fixed and always actionable. Scrollbar thumb
    // colors are derived from `text_color` (which already contrasts against
    // the yellow alert background) so they remain visible in both themes.
    // `ClippedScrollable` wants `warpui::elements::Fill` (not the theme
    // `Fill` used elsewhere in this file), so the three fills below are
    // fully qualified to avoid an import alias.
    let scrollable_text = ConstrainedBox::new(
        ClippedScrollable::vertical(
            mouse_states.alert_text_scroll_state.clone(),
            text_row,
            ScrollbarWidth::Auto,
            warpui::elements::Fill::Solid(coloru_with_opacity(text_color, 30)),
            warpui::elements::Fill::Solid(coloru_with_opacity(text_color, 60)),
            warpui::elements::Fill::None,
        )
        .finish(),
    )
    .with_max_height(ALERT_TEXT_MAX_HEIGHT)
    .finish();

    // ── Action buttons ───────────────────────────────────────────────────
    let open_file_button = render_alert_action_button(
        ui_font_family,
        text_color,
        mouse_states.alert_open_file_button.clone(),
        "Open file",
        /*icon=*/ None,
        /*bordered=*/ true,
        WorkspaceAction::OpenSettingsFile,
    );

    // Use a `Wrap` flex as a graceful fallback: if the sidebar is narrower
    // than the buttons' combined natural width, they wrap onto a second
    // row instead of pushing the alert container wider than the sidebar.
    let mut buttons_row = Wrap::row()
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .with_main_axis_size(MainAxisSize::Min)
        .with_spacing(ALERT_BUTTON_SPACING)
        .with_run_spacing(ALERT_BUTTON_SPACING)
        .with_child(open_file_button);

    if ai_enabled {
        let error_description = error.to_string();
        let fix_with_oz_button = render_alert_action_button(
            ui_font_family,
            text_color,
            mouse_states.alert_fix_with_oz_button.clone(),
            "Fix with Oz",
            Some(Icon::Oz),
            /*bordered=*/ false,
            WorkspaceAction::FixSettingsWithOz { error_description },
        );
        buttons_row.add_child(fix_with_oz_button);
    }

    // ── Assemble ─────────────────────────────────────────────────────────
    // Left-align the buttons with the start of the text (past the icon + gap).
    let buttons_indented = Container::new(buttons_row.finish())
        .with_margin_left(FOOTER_ICON_SIZE + 8.)
        .with_margin_top(8.)
        .finish();

    // `CrossAxisAlignment::Stretch` tightens each child's cross-axis (width)
    // constraint to the column's available width so the alert container
    // doesn't end up sized by buttons overflowing their allotment.
    let alert_body = Flex::column()
        .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
        .with_child(scrollable_text)
        .with_child(buttons_indented)
        .finish();

    Container::new(alert_body)
        .with_background_color(bg_color)
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)))
        .with_uniform_padding(12.)
        .finish()
}

/// Wraps one of the three footer branches with the correct outer padding so
/// it aligns with the nav items above.
pub fn render_footer(
    kind: SettingsFooterKind,
    appearance: &Appearance,
    error: Option<&SettingsFileError>,
    ai_enabled: bool,
    mouse_states: &SettingsFooterMouseStates,
) -> Box<dyn Element> {
    let inner: Box<dyn Element> = match kind {
        SettingsFooterKind::Hidden => return Empty::new().finish(),
        SettingsFooterKind::OpenButton => render_open_settings_file_button(
            appearance,
            mouse_states.open_settings_file_button.clone(),
        ),
        SettingsFooterKind::ErrorAlert => match error {
            Some(error) => render_settings_error_alert(appearance, error, ai_enabled, mouse_states),
            // Defensive fallback: if the error disappears between `choose` and
            // `render_footer`, fall back to the plain button rather than
            // rendering an empty alert shell.
            None => render_open_settings_file_button(
                appearance,
                mouse_states.open_settings_file_button.clone(),
            ),
        },
    };

    Container::new(inner)
        .with_uniform_padding(FOOTER_PADDING)
        .finish()
}

/// Renders a single action button inside the inline alert. Mirrors
/// `Workspace::render_banner_action_button` so the styling stays consistent
/// with the workspace-level banner.
fn render_alert_action_button(
    ui_font_family: FamilyId,
    text_color: ColorU,
    mouse_state: MouseStateHandle,
    text: &'static str,
    icon: Option<Icon>,
    bordered: bool,
    action: WorkspaceAction,
) -> Box<dyn Element> {
    Hoverable::new(mouse_state, move |state| {
        let mut row = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_main_axis_alignment(MainAxisAlignment::Center)
            .with_main_axis_size(MainAxisSize::Min);
        if let Some(icon) = icon {
            row.add_child(
                Container::new(
                    ConstrainedBox::new(icon.to_warpui_icon(Fill::Solid(text_color)).finish())
                        .with_width(ALERT_OZ_ICON_SIZE)
                        .with_height(ALERT_OZ_ICON_SIZE)
                        .finish(),
                )
                .with_margin_right(4.)
                .finish(),
            );
        }
        row.add_child(
            Text::new_inline(text.to_owned(), ui_font_family, FOOTER_FONT_SIZE)
                .with_color(text_color)
                .with_style(Properties {
                    weight: Weight::Semibold,
                    ..Default::default()
                })
                .finish(),
        );

        let mut container = Container::new(row.finish())
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)))
            .with_horizontal_padding(ALERT_BUTTON_HORIZONTAL_PADDING);
        if bordered {
            container = container.with_border(Border::all(1.).with_border_color(text_color));
        }
        if state.is_hovered() {
            container = container.with_background_color(coloru_with_opacity(text_color, 20));
        }

        ConstrainedBox::new(container.finish())
            .with_height(ALERT_ACTION_BUTTON_HEIGHT)
            .finish()
    })
    .with_cursor(Cursor::PointingHand)
    .on_click(move |ctx, _, _| ctx.dispatch_typed_action(action.clone()))
    .finish()
}

#[cfg(test)]
#[path = "settings_file_footer_tests.rs"]
mod tests;
