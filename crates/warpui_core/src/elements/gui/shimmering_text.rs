mod config;
mod glyph_index;

use std::borrow::Cow;
use std::collections::HashMap;
use std::f32::consts::PI;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use instant::Instant;
use rangemap::RangeMap;
use string_offset::CharOffset;

use crate::color::ColorU;
pub use crate::elements::shimmering_text::config::ShimmerConfig;
use crate::elements::shimmering_text::glyph_index::GlyphIndex;
use crate::elements::{Axis, Point, DEFAULT_UI_LINE_HEIGHT_RATIO};
use crate::fonts::{FamilyId, Properties};
use crate::geometry::rect::RectF;
use crate::geometry::vector::{vec2f, Vector2F};
use crate::platform::LineStyle;
use crate::text_layout::{
    ClipConfig, Line, PaintStyleOverride, StyleAndFont, TextStyle, DEFAULT_TOP_BOTTOM_RATIO,
};
use crate::{AppContext, Element, PaintContext, SizeConstraint};

/// A key to determine whether we need to re-layout text to a given invocation of #layout to this
/// element.
#[derive(PartialEq, Clone, Debug)]
struct LayoutKey {
    text: Cow<'static, str>,
    font_family: FamilyId,
    font_size: f32,
    max_width: f32,
}

struct StateInternal {
    laid_out_key: Option<LayoutKey>,
    laid_out_line: Option<Arc<Line>>,
    /// A list of the character index of every glyph in the line. In other words, index 0 contains
    /// A mapping from glyph index in the line to the character index for that glyph.
    /// In other words, key 0 contains the character index of the first glyph. We store this as a map
    /// to be resilient to ligatures: the ligature 'fi' is two characters but should only have one fade.
    /// to ligatures: the ligature "fi" is two characters but should only have one fade.
    glyph_indices_in_order: HashMap<GlyphIndex<usize>, CharOffset>,
    animation_start_time: Instant,
}

#[derive(Clone)]
pub struct ShimmeringTextStateHandle(Arc<Mutex<StateInternal>>);

impl Default for ShimmeringTextStateHandle {
    fn default() -> Self {
        Self::new()
    }
}

impl ShimmeringTextStateHandle {
    pub fn new() -> Self {
        Self(Arc::new(Mutex::new(StateInternal {
            laid_out_key: None,
            laid_out_line: None,
            glyph_indices_in_order: HashMap::default(),
            animation_start_time: Instant::now(),
        })))
    }

    fn get(&self) -> MutexGuard<'_, StateInternal> {
        self.0.lock().expect("Mutex should not be poisoned")
    }
}

/// An element that displays the given text using given `base_color` with a shimmer that animates
/// from left to right with the given `shimmer_color`.
///
/// See [`ShimmerConfig`] for adjusting configuration options, such as the duration, size, and
/// frequency of the shimmer.
pub struct ShimmeringTextElement {
    text: Cow<'static, str>,

    font_family: FamilyId,
    font_size: f32,

    base_color: ColorU,
    shimmer_color: ColorU,

    config: ShimmerConfig,

    size: Option<Vector2F>,
    origin: Option<Point>,

    handle: ShimmeringTextStateHandle,
}

impl ShimmeringTextElement {
    pub fn new(
        text: impl Into<Cow<'static, str>>,
        font_family: FamilyId,
        font_size: f32,
        base_color: ColorU,
        shimmer_color: ColorU,
        config: ShimmerConfig,
        state_handle: ShimmeringTextStateHandle,
    ) -> Self {
        Self {
            text: text.into(),
            font_family,
            font_size,
            base_color,
            shimmer_color,
            config,
            size: None,
            origin: None,
            handle: state_handle,
        }
    }

    /// Returns the center of the shimmer as a fractional glyph index along the "track".
    fn shimmer_center(&self, number_of_glyphs: usize, state: &StateInternal) -> GlyphIndex<f32> {
        if number_of_glyphs <= 1 {
            return GlyphIndex(0.0);
        }

        let period_s = self.config.period.as_secs_f32();
        let elapsed_s = state.animation_start_time.elapsed().as_secs_f32();
        // Get the percent of the way through we are of the current loop.
        let progress = (elapsed_s / period_s).fract();

        // Compute the total number of glyphs the band needs to travel.
        let span = (number_of_glyphs as f32 - 1.0) + (2.0 * self.config.padding as f32);
        // Get the fractional glyph index for the center of the band, factoring in that the center
        // can be negative (before any of the text)
        GlyphIndex((progress * span) - self.config.padding as f32)
    }

    /// Returns how strong the shimmer effect should be for a given glyph based on how far it is
    /// from the center of the shimmer.
    fn intensity_at(&self, glyph_index: GlyphIndex<usize>, center: GlyphIndex<f32>) -> f32 {
        let dist = (glyph_index.as_f32().0 - center.0).abs();
        // If the distance is greater than the size of the band, there's no intensity.
        if dist >= self.config.shimmer_radius as f32 {
            return 0.0;
        }
        // Use a cosine wave to generate the intensity otherwise and normalize it to [0,1].
        let theta = (dist / self.config.shimmer_radius as f32) * PI;
        (theta.cos() + 1.0) * 0.5
    }

    fn glyph_index_to_character_index_map(line: &Line) -> HashMap<GlyphIndex<usize>, CharOffset> {
        line.runs
            .iter()
            .flat_map(|run| run.glyphs.iter())
            .enumerate()
            .map(|(glyph_index, glyph)| (GlyphIndex(glyph_index), CharOffset::from(glyph.index)))
            .collect()
    }

    fn build_color_overrides(&self) -> PaintStyleOverride {
        let state = self.handle.get();

        let glyph_indices_in_order = &state.glyph_indices_in_order;

        let n = glyph_indices_in_order.len();
        if n == 0 {
            return PaintStyleOverride::default();
        }

        let center = self.shimmer_center(n, &state);

        let mut overrides = RangeMap::new();
        for (glyph_index, char_index) in glyph_indices_in_order.iter() {
            let intensity = self.intensity_at(*glyph_index, center);
            let color = self
                .base_color
                .to_f32()
                .lerp(self.shimmer_color.to_f32(), intensity)
                .to_u8();
            overrides.insert(char_index.as_usize()..char_index.as_usize() + 1, color);
        }

        PaintStyleOverride::default().with_color(overrides)
    }
}

impl Element for ShimmeringTextElement {
    fn layout(
        &mut self,
        constraint: SizeConstraint,
        ctx: &mut crate::LayoutContext,
        app: &AppContext,
    ) -> Vector2F {
        let mut state = self.handle.get();

        let text_len = self.text.chars().count();
        let styles = [(
            0..text_len,
            StyleAndFont::new(self.font_family, Properties::default(), TextStyle::new()),
        )];

        let max_width = constraint.max_along(Axis::Horizontal);

        let layout_key = LayoutKey {
            text: self.text.clone(),
            font_family: self.font_family,
            font_size: self.font_size,
            max_width,
        };

        // Determine whether we need to relayout the text.
        let line = match state.laid_out_line.clone() {
            Some(line) if Some(&layout_key) == state.laid_out_key.as_ref() => line,
            _ => {
                let line = ctx.text_layout_cache.layout_line(
                    self.text.as_ref(),
                    LineStyle {
                        font_size: self.font_size,
                        line_height_ratio: DEFAULT_UI_LINE_HEIGHT_RATIO,
                        baseline_ratio: DEFAULT_TOP_BOTTOM_RATIO,
                        fixed_width_tab_size: None,
                    },
                    &styles,
                    max_width,
                    ClipConfig::default(),
                    &app.font_cache().text_layout_system(),
                );

                // Restart the animation if the font or font size has changed.
                let should_restart_animation = match (&layout_key, state.laid_out_key.as_ref()) {
                    (new_layout_key, Some(old_layout_key)) => {
                        new_layout_key.font_family != old_layout_key.font_family
                            || new_layout_key.font_size != old_layout_key.font_size
                            || new_layout_key.text != old_layout_key.text
                    }
                    _ => true,
                };

                if should_restart_animation {
                    state.animation_start_time = Instant::now();
                }

                state.glyph_indices_in_order = Self::glyph_index_to_character_index_map(&line);
                state.laid_out_line = Some(line.clone());
                state.laid_out_key = Some(layout_key);

                line
            }
        };

        let size = vec2f(
            line.width.max(constraint.min.x()).min(constraint.max.x()),
            line.height(),
        );

        self.size = Some(size);
        size
    }

    fn after_layout(&mut self, _: &mut crate::AfterLayoutContext, _: &AppContext) {}

    fn paint(&mut self, origin: Vector2F, ctx: &mut PaintContext, app: &AppContext) {
        /// Duration, in ms, for which to repaint. Approximately 30fps.
        const REPAINT_DURATION: u64 = 32;

        self.origin = Some(Point::from_vec2f(origin, ctx.scene.z_index()));

        let Some(size) = self.size else {
            return;
        };

        let Some(line) = self.handle.get().laid_out_line.clone() else {
            return;
        };

        ctx.repaint_after(Duration::from_millis(REPAINT_DURATION));

        let bounds = RectF::from_points(origin, origin + size);
        let style_overrides = self.build_color_overrides();

        line.paint(
            bounds,
            &style_overrides,
            self.base_color,
            app.font_cache(),
            ctx.scene,
        );
    }

    fn size(&self) -> Option<Vector2F> {
        self.size
    }

    fn origin(&self) -> Option<Point> {
        self.origin
    }

    fn dispatch_event(
        &mut self,
        _: &crate::event::DispatchedEvent,
        _: &mut crate::EventContext,
        _: &AppContext,
    ) -> bool {
        false
    }
}
