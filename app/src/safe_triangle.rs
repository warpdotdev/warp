use pathfinder_geometry::rect::RectF;
use pathfinder_geometry::vector::Vector2F;

/// Vertical padding (in points) above and below the cursor that limits the triangle's
/// vertical extent. This prevents the triangle from being overly aggressive when the
/// target rect (sidecar panel) is much taller than the menu items.
const VERTICAL_PADDING: f32 = 80.0;

/// Minimum squared distance the mouse must move before the safe triangle can suppress.
/// Prevents false suppression from sub-pixel jitter between consecutive events
/// (e.g., hover-out/hover-in/mouse-in firing for the same logical mouse move).
const MIN_MOVEMENT_SQUARED: f32 = 4.0;

/// Implements a "safe triangle" pattern for menu sidecar hover.
///
/// When the user hovers a menu item that opens a sidecar panel, moving the mouse
/// diagonally toward the sidecar would normally trigger hovers on intermediate items.
/// SafeTriangle suppresses those intermediate hovers by defining a triangle between
/// the last mouse position and two points on the near-side edge of the target rect,
/// clamped to a vertical band around the cursor. Any new position inside this triangle
/// is considered "in transit" to the sidecar.
pub struct SafeTriangle {
    target_rect: Option<RectF>,
    last_position: Option<Vector2F>,
    is_suppressing: bool,
}

impl SafeTriangle {
    pub fn new() -> Self {
        Self {
            target_rect: None,
            last_position: None,
            is_suppressing: false,
        }
    }

    pub fn set_target_rect(&mut self, rect: Option<RectF>) {
        self.target_rect = rect;
        if rect.is_none() {
            self.last_position = None;
            self.is_suppressing = false;
        }
    }

    pub fn is_suppressing(&self) -> bool {
        self.is_suppressing
    }

    /// Returns true if the hover should be suppressed
    /// (i.e. the mouse is moving toward the target rect through the safe triangle).
    pub fn should_suppress_hover(&mut self, new_position: Vector2F) -> bool {
        let (Some(target), Some(last_pos)) = (self.target_rect, self.last_position) else {
            self.is_suppressing = false;
            return false;
        };

        // Once the cursor has already entered the target panel, subsequent motion should no
        // longer count as "in transit" toward it. Without this, exiting from inside the sidecar
        // can look like another safe-triangle move and keep hover-driven sidecars visible too
        // long.
        if target.contains_point(last_pos) {
            self.is_suppressing = false;
            return false;
        }

        // If the mouse hasn't moved meaningfully, don't suppress. This handles
        // multiple events firing for the same mouse move (hover-out from old item,
        // hover-in for new item, mouse-in) where sub-pixel jitter between events
        // could otherwise trigger false suppression near the triangle apex.
        let diff = new_position - last_pos;
        if diff.x() * diff.x() + diff.y() * diff.y() < MIN_MOVEMENT_SQUARED {
            return self.is_suppressing;
        }

        // Determine which side of the target rect faces the menu.
        let target_center_x = (target.min_x() + target.max_x()) / 2.0;
        let near_x = if last_pos.x() < target_center_x {
            target.min_x()
        } else {
            target.max_x()
        };

        // Clamp the triangle's vertical extent to a band around the cursor.
        // Without this, a tall sidecar panel creates a triangle covering nearly
        // the entire menu, blocking straight up/down movement.
        let corner_a = Vector2F::new(near_x, last_pos.y() - VERTICAL_PADDING);
        let corner_b = Vector2F::new(near_x, last_pos.y() + VERTICAL_PADDING);

        let suppressing = point_in_triangle(new_position, last_pos, corner_a, corner_b);
        self.is_suppressing = suppressing;
        suppressing
    }

    pub fn update_position(&mut self, position: Vector2F) {
        self.last_position = Some(position);
    }
}

/// Returns true if point P lies inside triangle (A, B, C) using the sign-of-cross-product method.
fn point_in_triangle(p: Vector2F, a: Vector2F, b: Vector2F, c: Vector2F) -> bool {
    let d1 = cross_sign(p, a, b);
    let d2 = cross_sign(p, b, c);
    let d3 = cross_sign(p, c, a);

    let has_neg = (d1 < 0.0) || (d2 < 0.0) || (d3 < 0.0);
    let has_pos = (d1 > 0.0) || (d2 > 0.0) || (d3 > 0.0);

    !(has_neg && has_pos)
}

/// Returns the z-component of the cross product of vectors (v2 - v1) and (p - v1).
fn cross_sign(p: Vector2F, v1: Vector2F, v2: Vector2F) -> f32 {
    (v2.x() - v1.x()) * (p.y() - v1.y()) - (v2.y() - v1.y()) * (p.x() - v1.x())
}

#[cfg(test)]
#[path = "safe_triangle_tests.rs"]
mod tests;
