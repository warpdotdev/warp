use crate::elements::Fill;
use crate::geometry::vector::vec2f;
use crate::image_cache::StaticImage;
use crate::{
    elements::Point,
    fonts::{FontId, GlyphId},
    rendering,
};
use ordered_float::OrderedFloat;
use pathfinder_color::ColorU;
use pathfinder_geometry::rect::RectF;
use pathfinder_geometry::vector::Vector2F;
use rstar::{primitives::Rectangle, RTree};
use std::sync::Arc;
use vec1::{vec1, Vec1};

#[derive(Clone)]
pub struct Scene {
    scale_factor: f32,
    rendering_config: rendering::Config,
    active_layer_index_stack: Vec1<ZIndex>,
    layers: Vec1<Layer>,
    overlay_layers: Vec<Layer>,
    #[cfg(debug_assertions)]
    /// Custom panic location, set with [`Scene::set_location_for_panic_logging`]
    panic_location: Option<&'static std::panic::Location<'static>>,
}

#[derive(Clone, Default)]
pub struct Layer {
    hit_map: RTree<Rectangle<[OrderedFloat<f32>; 2]>>,
    pub clip_bounds: Option<RectF>,
    pub rects: Vec<Rect>,
    pub images: Vec<Image>,
    pub glyphs: Vec<Glyph>,
    pub icons: Vec<Icon>,
    pub click_through: bool,
}

/// Clip bounds to use for a layer.
pub enum ClipBounds {
    /// Use the bounds of the active layer.
    ActiveLayer,
    /// Use the specified bounds as the bounds for the new layer.
    ///
    /// Note that this ignores any clip bounds applied to the currently-active
    /// layer.
    BoundedBy(RectF),
    /// Intersect the active layer's bounds and the provided rect
    /// to get the bounds for the new layer.
    BoundedByActiveLayerAnd(RectF),
    /// No clipping
    None,
}

impl Layer {
    fn record_hit_rect(&mut self, rect: RectF) {
        if let Some(intersected) = self
            .clip_bounds
            .map_or(Some(rect), |c| rect.intersection(c))
        {
            self.hit_map.insert(Rectangle::from_corners(
                [intersected.min_x().into(), intersected.min_y().into()],
                [intersected.max_x().into(), intersected.max_y().into()],
            ));
        }
    }
}

#[derive(Debug, Clone, Copy, Eq, Hash, PartialEq)]
pub struct GlyphKey {
    pub glyph_id: GlyphId,
    pub font_id: FontId,
    pub font_size: OrderedFloat<f32>,
}

#[derive(Debug, Copy, Clone)]
pub enum GlyphFade {
    /// A horizontal fade from alpha 1 to 0 with start and end positions in screen coordinates
    /// start - where the fade is transparent
    /// end - where the fade is most opaque
    Horizontal { start: f32, end: f32 },
}

impl GlyphFade {
    pub fn horizontal(start: f32, end: f32) -> Self {
        GlyphFade::Horizontal { start, end }
    }
}

#[derive(Clone, Debug)]
pub struct Glyph {
    pub glyph_key: GlyphKey,
    pub position: Vector2F,
    pub fade: Option<GlyphFade>,
    pub color: ColorU,
}

#[derive(Clone, Default)]
pub struct Rect {
    pub bounds: RectF,
    pub drop_shadow: Option<DropShadow>,
    pub corner_radius: CornerRadius,
    pub background: Fill,
    pub border: Border,
}

#[derive(Clone)]
pub struct Image {
    pub bounds: RectF,
    pub asset: Arc<StaticImage>,
    pub opacity: f32,
    pub corner_radius: CornerRadius,
}

#[derive(Clone)]
pub struct Icon {
    pub bounds: RectF,
    pub asset: Arc<StaticImage>,
    pub opacity: f32,
    pub color: ColorU,
}

// These were picked empirically to make the shadows look decent by
// default, but there is nothing special about them.
const DEFAULT_DROP_SHADOW_OFFSET_X: f32 = 0.;
const DEFAULT_DROP_SHADOW_OFFSET_Y: f32 = 10.;
const DEFAULT_DROP_SHADOW_BLUR_RADIUS: f32 = 10.;
const DEFAULT_DROP_SHADOW_SPREAD_RADIUS: f32 = 30.;

#[derive(Clone, Copy)]
pub struct DropShadow {
    pub color: ColorU,

    // How the shadow is offset from the target rect
    pub offset: Vector2F,

    // Controls how tightly sampled the shadow is - the larger the number
    // the more spread out the shadow.
    pub blur_radius: f32,

    // Controls how wide the shadow is outside the target.
    pub spread_radius: f32,
}

impl DropShadow {
    pub fn new_with_standard_offset_and_spread(color: ColorU) -> Self {
        Self {
            color,
            offset: vec2f(DEFAULT_DROP_SHADOW_OFFSET_X, DEFAULT_DROP_SHADOW_OFFSET_Y),
            blur_radius: DEFAULT_DROP_SHADOW_BLUR_RADIUS,
            spread_radius: DEFAULT_DROP_SHADOW_SPREAD_RADIUS,
        }
    }

    pub fn with_offset(mut self, offset: Vector2F) -> Self {
        self.offset = offset;
        self
    }
}

impl Default for DropShadow {
    fn default() -> Self {
        Self::new_with_standard_offset_and_spread(ColorU::new(0, 0, 0, 32))
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Border {
    pub width: f32,
    pub color: Fill,
    pub top: bool,
    pub left: bool,
    pub bottom: bool,
    pub right: bool,
    pub dash: Option<Dash>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Dash {
    pub dash_length: f32,
    pub gap_length: f32,

    /// If true, gaps will always be the length specified in `gap_length`.
    /// Otherwise, gap length may be adjusted slightly to guarantee that the
    /// dashed line starts and ends with a dash.
    pub force_consistent_gap_length: bool,
}

impl Border {
    pub fn top_width(&self) -> f32 {
        if self.top {
            self.width
        } else {
            0.0
        }
    }

    pub fn right_width(&self) -> f32 {
        if self.right {
            self.width
        } else {
            0.0
        }
    }

    pub fn bottom_width(&self) -> f32 {
        if self.bottom {
            self.width
        } else {
            0.0
        }
    }

    pub fn left_width(&self) -> f32 {
        if self.left {
            self.width
        } else {
            0.0
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Radius {
    /// Specify a radius in absolute pixels.
    Pixels(f32),
    /// Specify a radius as a percentage of the rectangle's smaller dimension.
    /// For example, using `Percentage(50.)` will produce a pill shape.
    Percentage(f32),
}

impl Default for Radius {
    fn default() -> Self {
        Radius::Pixels(0.)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CornerRadius {
    /// Top left corner radius
    top_left: Option<Radius>,
    /// Top right corner radius
    top_right: Option<Radius>,
    /// Bottom left corner radius
    bottom_left: Option<Radius>,
    /// Bottom right corner radius
    bottom_right: Option<Radius>,
}

impl CornerRadius {
    /// Merge this CornerRadius struct with another.
    /// `Some(r)` takes precedence over `None`.
    /// If both are present, `other`'s values, take precedence over `self`'s existing values.
    pub fn merge(&mut self, other: CornerRadius) {
        self.top_left = other.top_left.or(self.top_left);
        self.top_right = other.top_right.or(self.top_right);
        self.bottom_left = other.bottom_left.or(self.bottom_left);
        self.bottom_right = other.bottom_right.or(self.bottom_right);
    }

    pub fn get_top_left(&self) -> Radius {
        self.top_left.unwrap_or(Radius::Pixels(0.))
    }

    pub fn get_top_right(&self) -> Radius {
        self.top_right.unwrap_or(Radius::Pixels(0.))
    }

    pub fn get_bottom_left(&self) -> Radius {
        self.bottom_left.unwrap_or(Radius::Pixels(0.))
    }

    pub fn get_bottom_right(&self) -> Radius {
        self.bottom_right.unwrap_or(Radius::Pixels(0.))
    }

    pub const fn with_all(radius: Radius) -> Self {
        CornerRadius {
            top_left: Some(radius),
            top_right: Some(radius),
            bottom_left: Some(radius),
            bottom_right: Some(radius),
        }
    }
    pub const fn with_top(radius: Radius) -> Self {
        CornerRadius {
            top_left: Some(radius),
            top_right: Some(radius),
            bottom_left: None,
            bottom_right: None,
        }
    }
    pub const fn with_bottom(radius: Radius) -> Self {
        CornerRadius {
            top_left: None,
            top_right: None,
            bottom_left: Some(radius),
            bottom_right: Some(radius),
        }
    }
    pub const fn with_left(radius: Radius) -> Self {
        CornerRadius {
            top_left: Some(radius),
            top_right: None,
            bottom_left: Some(radius),
            bottom_right: None,
        }
    }
    pub const fn with_right(radius: Radius) -> Self {
        CornerRadius {
            top_left: None,
            top_right: Some(radius),
            bottom_left: None,
            bottom_right: Some(radius),
        }
    }
    pub const fn with_top_left(radius: Radius) -> Self {
        CornerRadius {
            top_left: Some(radius),
            top_right: None,
            bottom_left: None,
            bottom_right: None,
        }
    }
    pub const fn with_top_right(radius: Radius) -> Self {
        CornerRadius {
            top_left: None,
            top_right: Some(radius),
            bottom_left: None,
            bottom_right: None,
        }
    }
    pub const fn with_bottom_left(radius: Radius) -> Self {
        CornerRadius {
            top_left: None,
            top_right: None,
            bottom_left: Some(radius),
            bottom_right: None,
        }
    }
    pub const fn with_bottom_right(radius: Radius) -> Self {
        CornerRadius {
            top_left: None,
            top_right: None,
            bottom_left: None,
            bottom_right: Some(radius),
        }
    }

    /// Filters this [`CornerRadius`] to only have the top corners rounded.
    pub const fn top(self) -> Self {
        CornerRadius {
            top_left: self.top_left,
            top_right: self.top_right,
            bottom_left: None,
            bottom_right: None,
        }
    }

    /// Filters this [`CornerRadius`] to only have the bottom corners rounded.
    pub const fn bottom(self) -> Self {
        CornerRadius {
            top_left: None,
            top_right: None,
            bottom_left: self.bottom_left,
            bottom_right: self.bottom_right,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
/// Newtype to encapsulate a Z index, which actually represents a layer index in the list of layers
pub enum ZIndex {
    Normal(usize),
    Overlay(usize),
}

impl ZIndex {
    #[cfg(test)]
    pub fn new(layer: usize) -> Self {
        ZIndex::Normal(layer)
    }
}

impl Scene {
    pub fn new(scale_factor: f32, rendering_config: rendering::Config) -> Self {
        Self {
            scale_factor,
            rendering_config,
            active_layer_index_stack: vec1![ZIndex::Normal(0)],
            layers: vec1![Layer::default()],
            overlay_layers: Vec::new(),
            #[cfg(debug_assertions)]
            panic_location: None,
        }
    }

    /// Temporarily set the panic location for the scene. This is cleared
    /// during the next draw call.
    #[cfg(debug_assertions)]
    pub fn set_location_for_panic_logging(
        &mut self,
        panic_location: Option<&'static std::panic::Location<'static>>,
    ) {
        self.panic_location = panic_location;
    }

    fn active_layer(&mut self) -> &mut Layer {
        match *self.active_layer_index_stack.last() {
            ZIndex::Normal(index) => &mut self.layers[index],
            ZIndex::Overlay(index) => &mut self.overlay_layers[index],
        }
    }

    pub fn is_covered(&self, position: Point) -> bool {
        // Does any layer at a higher z-index contain this point?
        let point = [position.x().into(), position.y().into()];
        let predicate = |l: &Layer| !l.click_through && l.hit_map.locate_at_point(&point).is_some();

        match position.z_index() {
            ZIndex::Normal(index) => self
                .layers
                .get((index + 1)..)
                .into_iter()
                .flatten()
                .chain(self.overlay_layers.iter())
                .any(predicate),
            ZIndex::Overlay(index) => self
                .overlay_layers
                .get((index + 1)..)
                .into_iter()
                .flatten()
                .any(predicate),
        }
    }

    // Compute the intersection between the bound of the element and the clip bound
    // on its current layer. The intersection is then checked against the event position
    // to determine whether we should dispatch the event.
    pub fn visible_rect(&self, origin: Point, size: Vector2F) -> Option<RectF> {
        // TODO: Investigate how / when we would pass a z-index that isn't in the scene
        // This appears to be fairly common, based on adding sentry reporting to it, however it
        // doesn't seem to dramatically impact app usage. Perhaps it's something that happens on
        // a view teardown frame?
        let maybe_layer = match origin.z_index() {
            ZIndex::Normal(index) => self.layers.get(index),
            ZIndex::Overlay(index) => self.overlay_layers.get(index),
        };
        let maybe_bounds = maybe_layer.and_then(|layer| layer.clip_bounds);

        let input_rect = RectF::new(origin.xy(), size);
        match maybe_bounds {
            Some(clip_rect) => clip_rect.intersection(input_rect),
            None => Some(input_rect),
        }
    }

    /// Get the Z-Index of the currently-active layer
    pub fn z_index(&self) -> ZIndex {
        *self.active_layer_index_stack.last()
    }

    /// Get the maximum Z-Index in the active layer stack (whether Normal or Overlay).
    pub fn max_active_z_index(&self) -> ZIndex {
        match self.active_layer_index_stack.last() {
            ZIndex::Normal(_) => ZIndex::Normal(self.layers.len() - 1),
            // Safety: If the active layer is an overlay layer, then there must be at least one
            // overlay layer, so subtracting one from the length is valid.
            ZIndex::Overlay(_) => ZIndex::Overlay(self.overlay_layers.len() - 1),
        }
    }

    pub fn start_layer(&mut self, bounds: ClipBounds) {
        let layer = self.create_layer(bounds);

        match *self.active_layer_index_stack.last() {
            ZIndex::Normal(_) => self.push_normal_layer(layer),
            ZIndex::Overlay(_) => self.push_overlay_layer(layer),
        }
    }

    pub(crate) fn start_overlay_layer(&mut self, bounds: ClipBounds) {
        let layer = self.create_layer(bounds);
        self.push_overlay_layer(layer);
    }

    fn create_layer(&mut self, bounds: ClipBounds) -> Layer {
        let clip_bounds = match bounds {
            ClipBounds::ActiveLayer => self.active_layer().clip_bounds,
            ClipBounds::BoundedBy(bounds) => Some(bounds),
            ClipBounds::BoundedByActiveLayerAnd(bounds) => {
                if let Some(current_layer_bounds) = self.active_layer().clip_bounds {
                    // If the current layer has bounds, return the intersection...
                    current_layer_bounds
                        .intersection(bounds)
                        // ...or, if the regions don't overlap, an empty bounding rect.
                        .or(Some(RectF::default()))
                } else {
                    // If the current layer has no bounds, return the bounds
                    // for the new layer.
                    Some(bounds)
                }
            }
            ClipBounds::None => None,
        };

        Layer {
            clip_bounds,
            ..Default::default()
        }
    }

    fn push_normal_layer(&mut self, layer: Layer) {
        self.active_layer_index_stack
            .push(ZIndex::Normal(self.layers.len()));
        self.layers.push(layer);
    }

    fn push_overlay_layer(&mut self, layer: Layer) {
        self.active_layer_index_stack
            .push(ZIndex::Overlay(self.overlay_layers.len()));
        self.overlay_layers.push(layer);
    }

    pub fn set_active_layer_click_through(&mut self) {
        self.active_layer().click_through = true;
    }

    pub fn stop_layer(&mut self) {
        if self.active_layer_index_stack.pop().is_err() {
            panic!("popped the last layer from active_layer_index_stack");
        }
    }

    fn validate_rect(rect: &RectF, location: Option<&'static std::panic::Location<'static>>) {
        #[cfg(debug_assertions)]
        let location_info = location
            .map(|loc| {
                format!(
                    " (element created at {}:{}:{})",
                    loc.file(),
                    loc.line(),
                    loc.column()
                )
            })
            .unwrap_or_default();
        #[cfg(not(debug_assertions))]
        let location_info = "";
        debug_assert!(
            !rect.origin().y().is_infinite(),
            "!rect.origin().y().is_infinite(){location_info}"
        );
        debug_assert!(
            !rect.origin().y().is_nan(),
            "!rect.origin().y().is_nan(){location_info}"
        );

        debug_assert!(
            !rect.size().x().is_infinite(),
            "!rect.size().x().is_infinite(){location_info}"
        );
        debug_assert!(
            !rect.size().x().is_nan(),
            "!rect.size().x().is_nan(){location_info}"
        );
        debug_assert!(
            !rect.size().y().is_infinite(),
            "!rect.size().y().is_infinite(){location_info}"
        );
        debug_assert!(
            !rect.size().y().is_nan(),
            "!rect.size().y().is_nan(){location_info}"
        );
    }

    /// This method draws a rectangle without recording any information about it in the current
    /// layer. Note this should be used with caution. In most cases, what you need is
    /// `draw_rect_with_hit_recording` instead. However, in rare cases this may be useful for
    /// performance reasons when many intermediate rects are drawn. If this is called, it is up to
    /// the caller to also draw a rect (via draw_rect_with_hit_recording) that encompasses the range
    /// of the rects drawn so that layer recording for event dispatching is correctly kept
    /// up-to-date.
    pub fn draw_rect_without_hit_recording(&mut self, rect: RectF) -> &mut Rect {
        #[cfg(debug_assertions)]
        let location = self.panic_location.take();
        #[cfg(not(debug_assertions))]
        let location = None;
        let layer = self.active_layer();
        Self::validate_rect(&rect, location);

        layer.rects.push(Rect {
            bounds: rect,
            ..Default::default()
        });
        layer.rects.last_mut().unwrap()
    }

    pub fn draw_rect_with_hit_recording(&mut self, rect: RectF) -> &mut Rect {
        let layer = self.active_layer();
        layer.record_hit_rect(rect);
        self.draw_rect_without_hit_recording(rect)
    }

    pub fn draw_image(
        &mut self,
        rect: RectF,
        asset: Arc<StaticImage>,
        opacity: f32,
        corner_radius: CornerRadius,
    ) {
        #[cfg(debug_assertions)]
        let location = self.panic_location.take();
        #[cfg(not(debug_assertions))]
        let location = None;
        let layer = self.active_layer();
        Self::validate_rect(&rect, location);

        layer.images.push(Image {
            bounds: rect,
            asset,
            opacity,
            corner_radius,
        });
        layer.record_hit_rect(rect);
    }

    pub fn draw_icon(&mut self, rect: RectF, asset: Arc<StaticImage>, opacity: f32, color: ColorU) {
        #[cfg(debug_assertions)]
        let location = self.panic_location.take();
        #[cfg(not(debug_assertions))]
        let location = None;
        let layer = self.active_layer();
        Self::validate_rect(&rect, location);

        layer.icons.push(Icon {
            bounds: rect,
            asset,
            opacity,
            color,
        });
        layer.record_hit_rect(rect);
    }

    /// Adds a glyph that should be drawn in the scene.
    ///
    /// `position` is the point at which the glyph's left edge meets the
    /// baseline.
    pub fn draw_glyph(
        &mut self,
        position: Vector2F,
        glyph_id: GlyphId,
        font_id: FontId,
        font_size: f32,
        color: ColorU,
    ) -> &mut Glyph {
        // TODO: Support hit testing on glyphs?
        let layer = self.active_layer();
        layer.glyphs.push(Glyph {
            glyph_key: GlyphKey {
                glyph_id,
                font_id,
                font_size: font_size.into(),
            },
            position,
            color,
            fade: None,
        });
        layer.glyphs.last_mut().unwrap()
    }

    /// Get an iterator over all layers in order, from bottom to top
    pub fn layers(&self) -> impl Iterator<Item = &Layer> {
        self.layers.iter().chain(self.overlay_layers.iter())
    }

    /// Get the total number of layers
    #[cfg(test)]
    pub fn layer_count(&self) -> usize {
        self.layers.len() + self.overlay_layers.len()
    }

    pub fn scale_factor(&self) -> f32 {
        self.scale_factor
    }

    pub fn rendering_config(&self) -> &rendering::Config {
        &self.rendering_config
    }
}

impl Rect {
    pub fn with_corner_radius(&mut self, radius: CornerRadius) -> &mut Self {
        self.corner_radius.merge(radius);
        self
    }

    pub fn with_border(&mut self, border: Border) -> &mut Self {
        self.border = border;
        self
    }

    pub fn with_background<F>(&mut self, background: F) -> &mut Self
    where
        F: Into<Fill>,
    {
        self.background = background.into();
        self
    }

    pub fn with_drop_shadow(&mut self, drop_shadow: DropShadow) -> &mut Self {
        self.drop_shadow = Some(drop_shadow);
        self
    }
}

impl Glyph {
    pub fn with_fade(&mut self, fade: Option<GlyphFade>) -> &mut Self {
        self.fade = fade;
        self
    }
}

#[cfg(test)]
#[path = "scene_tests.rs"]
mod tests;
