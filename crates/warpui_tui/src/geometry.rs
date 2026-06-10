use warpui_core::geometry::vector::Vector2F;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct TuiSize {
    pub width: u16,
    pub height: u16,
}

impl TuiSize {
    pub const fn new(width: u16, height: u16) -> Self {
        Self { width, height }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct TuiRect {
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
}

impl TuiRect {
    pub const fn new(x: u16, y: u16, width: u16, height: u16) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    pub fn right(self) -> u16 {
        self.x.saturating_add(self.width)
    }

    pub fn bottom(self) -> u16 {
        self.y.saturating_add(self.height)
    }

    pub fn is_empty(self) -> bool {
        self.width == 0 || self.height == 0
    }

    pub fn inset(self, inset: u16) -> Self {
        let inset_width = inset.saturating_mul(2);
        Self {
            x: self.x.saturating_add(inset),
            y: self.y.saturating_add(inset),
            width: self.width.saturating_sub(inset_width),
            height: self.height.saturating_sub(inset_width),
        }
    }

    pub fn contains_position(self, position: Vector2F) -> bool {
        position.x() >= f32::from(self.x)
            && position.x() < f32::from(self.right())
            && position.y() >= f32::from(self.y)
            && position.y() < f32::from(self.bottom())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TuiConstraint {
    pub min: TuiSize,
    pub max: TuiSize,
}

impl TuiConstraint {
    pub const fn new(min: TuiSize, max: TuiSize) -> Self {
        Self { min, max }
    }

    pub const fn tight(size: TuiSize) -> Self {
        Self {
            min: size,
            max: size,
        }
    }
}
