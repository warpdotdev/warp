//! A small, transient HUD that briefly displays the current UI zoom percentage
//! after a user-initiated zoom action. It is intentionally not built on top of
//! `DismissibleToastStack` so it stays single-state, has no close affordance,
//! and uses a shorter timeout than general toasts.

use std::time::Duration;

use pathfinder_color::ColorU;
use warpui::elements::{
    Border, Container, CornerRadius, CrossAxisAlignment, Element, Flex, MainAxisAlignment,
    MainAxisSize, ParentElement, Radius, Text,
};
use warpui::r#async::{SpawnedFutureHandle, Timer};
use warpui::{AppContext, Entity, SingletonEntity, View, ViewContext};

use crate::appearance::Appearance;
use crate::themes::theme::Fill as ThemeFill;

/// Time the HUD remains visible after the most recent zoom action.
pub const ZOOM_LEVEL_HUD_TIMEOUT: Duration = Duration::from_millis(1000);

const HORIZONTAL_PADDING: f32 = 14.;
const VERTICAL_PADDING: f32 = 8.;
const CORNER_RADIUS: f32 = 6.;
const FONT_SIZE_MULTIPLIER: f32 = 1.4;

/// View that renders a single transient zoom-percentage indicator in the workspace.
///
/// Calling [`ZoomLevelHud::show_zoom_level`] sets the visible value and (re)starts
/// a one-shot timer. When the timer fires, the value is cleared and the view
/// renders nothing. Calling `show_zoom_level` again before the timer fires
/// replaces the value and restarts the timer instead of stacking.
pub struct ZoomLevelHud {
    visible_zoom_level: Option<u16>,
    dismiss_handle: Option<SpawnedFutureHandle>,
    timeout: Duration,
}

impl ZoomLevelHud {
    pub fn new() -> Self {
        Self {
            visible_zoom_level: None,
            dismiss_handle: None,
            timeout: ZOOM_LEVEL_HUD_TIMEOUT,
        }
    }

    /// The currently displayed zoom level, if any. Used in tests and assertions.
    #[cfg(test)]
    pub fn visible_zoom_level(&self) -> Option<u16> {
        self.visible_zoom_level
    }

    /// Show the given zoom percentage in the HUD and (re)start the dismissal timer.
    pub fn show_zoom_level(&mut self, zoom_level: u16, ctx: &mut ViewContext<Self>) {
        self.visible_zoom_level = Some(zoom_level);

        if let Some(previous) = self.dismiss_handle.take() {
            previous.abort();
        }

        let abort_handle = ctx.spawn_abortable(
            Timer::after(self.timeout),
            |view, _, ctx| {
                view.dismiss_handle = None;
                view.visible_zoom_level = None;
                ctx.notify();
            },
            |_, _| {},
        );
        self.dismiss_handle = Some(abort_handle);

        ctx.notify();
    }

    fn render_pill(&self, zoom_level: u16, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        let background = theme.surface_3();
        let text_color: ColorU = theme.main_text_color(background).into();
        let border_fill: ThemeFill = theme.outline();
        let font_size = appearance.ui_font_size() * FONT_SIZE_MULTIPLIER;

        let percentage = Text::new(
            format!("{zoom_level}%"),
            appearance.ui_font_family(),
            font_size,
        )
        .with_color(text_color)
        .finish();

        let row = Flex::row()
            .with_main_axis_size(MainAxisSize::Min)
            .with_main_axis_alignment(MainAxisAlignment::Center)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(percentage)
            .finish();

        Container::new(row)
            .with_horizontal_padding(HORIZONTAL_PADDING)
            .with_vertical_padding(VERTICAL_PADDING)
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(CORNER_RADIUS)))
            .with_border(Border::all(1.).with_border_fill(border_fill))
            .with_background(background)
            .finish()
    }
}

impl Default for ZoomLevelHud {
    fn default() -> Self {
        Self::new()
    }
}

impl Entity for ZoomLevelHud {
    type Event = ();
}

impl View for ZoomLevelHud {
    fn ui_name() -> &'static str {
        "ZoomLevelHud"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        match self.visible_zoom_level {
            Some(zoom_level) => self.render_pill(zoom_level, app),
            None => Flex::row()
                .with_main_axis_size(MainAxisSize::Min)
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .finish(),
        }
    }
}
