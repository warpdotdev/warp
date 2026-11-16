//! Retained screen geometry used by TUI input dispatch.

use super::{TuiPoint, TuiSize};
/// A signed absolute position in terminal screen space.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TuiScreenPosition {
    pub x: i32,
    pub y: i32,
}

impl TuiScreenPosition {
    /// Creates an absolute terminal-space position.
    pub const fn new(x: i32, y: i32) -> Self {
        Self { x, y }
    }

    /// Returns this position translated by the signed cell offset.
    pub fn offset(self, x: i32, y: i32) -> Self {
        Self {
            x: self.x.saturating_add(x),
            y: self.y.saturating_add(y),
        }
    }
}

/// Paint-order index for a normal or overlay TUI scene layer.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum TuiZIndex {
    Normal(usize),
    Overlay(usize),
}

/// A signed point relative to an element's retained screen origin.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TuiLocalPoint {
    pub x: i32,
    pub y: i32,
}

impl TuiLocalPoint {
    /// Creates an element-local point.
    pub const fn new(x: i32, y: i32) -> Self {
        Self { x, y }
    }
}

/// A signed terminal-space point associated with a painted scene layer.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TuiScreenPoint {
    pub x: i32,
    pub y: i32,
    pub z_index: TuiZIndex,
}

impl TuiScreenPoint {
    /// Creates a terminal-space point on `z_index`.
    pub const fn new(x: i32, y: i32, z_index: TuiZIndex) -> Self {
        Self { x, y, z_index }
    }
    /// Attaches `z_index` to an absolute terminal-space position.
    pub const fn from_position(position: TuiScreenPosition, z_index: TuiZIndex) -> Self {
        Self::new(position.x, position.y, z_index)
    }

    /// Returns this point's absolute position without its scene layer.
    pub const fn position(self) -> TuiScreenPosition {
        TuiScreenPosition::new(self.x, self.y)
    }

    /// Returns the same position on a different scene layer.
    fn with_z_index(self, z_index: TuiZIndex) -> Self {
        Self { z_index, ..self }
    }
}

/// A signed terminal-space rectangle.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TuiScreenRect {
    pub origin: TuiScreenPoint,
    pub size: TuiSize,
}

impl TuiScreenRect {
    /// Creates a screen rectangle from retained element geometry.
    pub const fn new(origin: TuiScreenPoint, size: TuiSize) -> Self {
        Self { origin, size }
    }

    /// Returns the rectangle's exclusive right edge.
    pub fn right(self) -> i32 {
        self.origin.x.saturating_add(i32::from(self.size.width))
    }

    /// Returns the rectangle's exclusive bottom edge.
    pub fn bottom(self) -> i32 {
        self.origin.y.saturating_add(i32::from(self.size.height))
    }

    /// Returns whether `position` lies inside the half-open rectangle.
    pub fn contains(self, position: TuiPoint) -> bool {
        self.contains_xy(i32::from(position.x), i32::from(position.y))
    }

    /// Returns whether a signed screen coordinate lies inside the rectangle.
    pub fn contains_xy(self, x: i32, y: i32) -> bool {
        x >= self.origin.x && x < self.right() && y >= self.origin.y && y < self.bottom()
    }

    /// Returns the intersection of two screen rectangles.
    pub fn intersection(self, other: Self) -> Option<Self> {
        let left = self.origin.x.max(other.origin.x);
        let top = self.origin.y.max(other.origin.y);
        let right = self.right().min(other.right());
        let bottom = self.bottom().min(other.bottom());
        if left >= right || top >= bottom {
            return None;
        }
        Some(Self::new(
            TuiScreenPoint::new(left, top, self.origin.z_index),
            TuiSize::new(
                u16::try_from(right - left).unwrap_or(u16::MAX),
                u16::try_from(bottom - top).unwrap_or(u16::MAX),
            ),
        ))
    }

    /// Returns the same rectangle on a different scene layer.
    fn with_z_index(self, z_index: TuiZIndex) -> Self {
        Self {
            origin: self.origin.with_z_index(z_index),
            ..self
        }
    }
}

/// Clip bounds to apply to a new TUI scene layer.
#[derive(Clone, Copy, Debug)]
pub enum TuiClipBounds {
    ActiveLayer,
    BoundedBy(TuiScreenRect),
    BoundedByActiveLayerAnd(TuiScreenRect),
    None,
}

#[derive(Clone, Default)]
struct TuiSceneLayer {
    hit_rects: Vec<TuiScreenRect>,
    clip_bounds: Option<TuiScreenRect>,
    click_through: bool,
}

/// Retained clip and occlusion data from the last TUI paint.
#[derive(Clone)]
pub struct TuiScene {
    active_layers: Vec<TuiZIndex>,
    layers: Vec<TuiSceneLayer>,
    overlay_layers: Vec<TuiSceneLayer>,
}

impl Default for TuiScene {
    fn default() -> Self {
        Self {
            active_layers: vec![TuiZIndex::Normal(0)],
            layers: vec![TuiSceneLayer::default()],
            overlay_layers: Vec::new(),
        }
    }
}

impl TuiScene {
    /// Returns the active paint layer.
    pub fn z_index(&self) -> TuiZIndex {
        *self
            .active_layers
            .last()
            .expect("the TUI scene always has an active root layer")
    }

    /// Returns the highest layer created in the active layer family.
    pub fn max_active_z_index(&self) -> TuiZIndex {
        match self.z_index() {
            TuiZIndex::Normal(_) => TuiZIndex::Normal(self.layers.len() - 1),
            TuiZIndex::Overlay(_) => TuiZIndex::Overlay(self.overlay_layers.len() - 1),
        }
    }

    /// Starts a normal layer nested under the active layer.
    pub fn start_layer(&mut self, bounds: TuiClipBounds) {
        let layer = self.layer_for_bounds(bounds);
        match self.z_index() {
            TuiZIndex::Normal(_) => {
                let z_index = TuiZIndex::Normal(self.layers.len());
                self.layers.push(layer);
                self.active_layers.push(z_index);
            }
            TuiZIndex::Overlay(_) => {
                let z_index = TuiZIndex::Overlay(self.overlay_layers.len());
                self.overlay_layers.push(layer);
                self.active_layers.push(z_index);
            }
        }
    }

    /// Starts an overlay layer above every normal layer.
    pub(crate) fn start_overlay_layer(&mut self, bounds: TuiClipBounds) {
        let layer = self.layer_for_bounds(bounds);
        self.overlay_layers.push(layer);
        self.active_layers
            .push(TuiZIndex::Overlay(self.overlay_layers.len() - 1));
    }

    /// Makes the active layer transparent to hit testing.
    pub fn set_active_layer_click_through(&mut self) {
        self.active_layer_mut().click_through = true;
    }

    /// Finishes the active layer.
    pub fn stop_layer(&mut self) {
        assert!(
            self.active_layers.len() > 1,
            "the TUI scene root layer cannot be stopped"
        );
        self.active_layers.pop();
    }

    /// Records a painted hit rectangle on the active layer.
    pub fn record_hit_rect(&mut self, rect: TuiScreenRect) {
        let z_index = self.z_index();
        let rect = rect.with_z_index(z_index);
        let layer = self.active_layer_mut();
        if let Some(rect) = layer
            .clip_bounds
            .map_or(Some(rect), |clip| rect.intersection(clip))
        {
            layer.hit_rects.push(rect);
        }
    }

    /// Returns the visible portion of retained element bounds.
    pub fn visible_rect(&self, origin: TuiScreenPoint, size: TuiSize) -> Option<TuiScreenRect> {
        let rect = TuiScreenRect::new(origin, size);
        self.layer(origin.z_index).and_then(|layer| {
            layer
                .clip_bounds
                .map_or(Some(rect), |clip| rect.intersection(clip))
        })
    }

    /// Returns whether a higher opaque layer covers `point`.
    pub fn is_covered(&self, point: TuiScreenPoint) -> bool {
        let contains = |layer: &TuiSceneLayer| {
            !layer.click_through
                && layer
                    .hit_rects
                    .iter()
                    .any(|rect| rect.contains_xy(point.x, point.y))
        };
        match point.z_index {
            TuiZIndex::Normal(index) => self
                .layers
                .get(index.saturating_add(1)..)
                .into_iter()
                .flatten()
                .chain(self.overlay_layers.iter())
                .any(contains),
            TuiZIndex::Overlay(index) => self
                .overlay_layers
                .get(index.saturating_add(1)..)
                .into_iter()
                .flatten()
                .any(contains),
        }
    }

    /// Returns whether all nested layers have been closed.
    pub(crate) fn is_at_root_layer(&self) -> bool {
        self.active_layers.len() == 1
    }

    fn layer_for_bounds(&self, bounds: TuiClipBounds) -> TuiSceneLayer {
        let active_clip = self.active_layer().clip_bounds;
        let clip_bounds = match bounds {
            TuiClipBounds::ActiveLayer => active_clip,
            TuiClipBounds::BoundedBy(bounds) => Some(bounds),
            TuiClipBounds::BoundedByActiveLayerAnd(bounds) => {
                active_clip.map_or(Some(bounds), |clip| {
                    Some(
                        bounds
                            .intersection(clip)
                            .unwrap_or_else(|| TuiScreenRect::new(bounds.origin, TuiSize::ZERO)),
                    )
                })
            }
            TuiClipBounds::None => None,
        };
        TuiSceneLayer {
            clip_bounds,
            ..Default::default()
        }
    }

    fn active_layer(&self) -> &TuiSceneLayer {
        self.layer(self.z_index())
            .expect("the active TUI scene layer exists")
    }

    fn active_layer_mut(&mut self) -> &mut TuiSceneLayer {
        match self.z_index() {
            TuiZIndex::Normal(index) => &mut self.layers[index],
            TuiZIndex::Overlay(index) => &mut self.overlay_layers[index],
        }
    }

    fn layer(&self, z_index: TuiZIndex) -> Option<&TuiSceneLayer> {
        match z_index {
            TuiZIndex::Normal(index) => self.layers.get(index),
            TuiZIndex::Overlay(index) => self.overlay_layers.get(index),
        }
    }
}

#[cfg(test)]
#[path = "scene_tests.rs"]
mod tests;
