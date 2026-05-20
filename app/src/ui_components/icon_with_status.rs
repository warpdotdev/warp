use pathfinder_color::ColorU;
use pathfinder_geometry::vector::vec2f;
use warp_core::ui::icons::Icon as WarpIcon;
use warp_core::ui::theme::color::internal_colors;
use warp_core::ui::theme::{Fill as WarpThemeFill, WarpTheme};
use warpui::elements::{
    ChildAnchor, ConstrainedBox, Container, CornerRadius, Element, OffsetPositioning, ParentAnchor,
    ParentElement, ParentOffsetBounds, Radius, Stack,
};

use crate::ai::agent::conversation::{ConversationStatus, StatusColorStyle};
use crate::terminal::CLIAgent;
use crate::themes::theme::Fill as ThemeFill;

/// Background color used for the Oz agent's circle when it is running in an ambient (cloud)
/// run. Matches the Oz brand purple used in the cloud-mode design spec.
const OZ_AMBIENT_BACKGROUND_COLOR: ColorU = ColorU {
    r: 203,
    g: 176,
    b: 247,
    a: 255,
};

// Sub-component size ratios, expressed as fractions of `total_size`. The brand circle is
// ~76% wide and the status badge is ~57% wide, with the badge's bottom-right anchored at
// the box's bottom-right corner. With these ratios the badge center sits *inside* the
// brand circle (not on its edge). `CIRCLE_RATIO` is `pub(crate)` so callers that
// pre-render their own avatar can size it consistently with the other variants.
pub(crate) const CIRCLE_RATIO: f32 = 0.76;
const ICON_RATIO: f32 = 0.43;
const BADGE_RATIO: f32 = 0.57;
const BADGE_ICON_RATIO: f32 = 0.34;
const CLOUD_RATIO: f32 = 0.57;
const STATUS_IN_CLOUD_RATIO: f32 = 0.285;

// Neutral variants have no overlay, so they fill the full `total_size` bounding box. The
// inner glyph occupies `NEUTRAL_GLYPH_RATIO * total_size`, matching the old sizing where
// a 24px container held a 16px glyph (16/24 ≈ 0.667).
const NEUTRAL_GLYPH_RATIO: f32 = 16.0 / 24.0;

/// Returns the brand-circle diameter for a given `total_size`.
pub(crate) fn circle_size(total: f32) -> f32 {
    total * CIRCLE_RATIO
}

fn icon_size(total: f32) -> f32 {
    total * ICON_RATIO
}

fn circle_padding(total: f32) -> f32 {
    (circle_size(total) - icon_size(total)) / 2.
}

fn badge_size(total: f32) -> f32 {
    total * BADGE_RATIO
}

fn badge_icon_size(total: f32) -> f32 {
    total * BADGE_ICON_RATIO
}

fn badge_padding(total: f32) -> f32 {
    (badge_size(total) - badge_icon_size(total)) / 4.
}

fn cloud_icon_size(total: f32) -> f32 {
    total * CLOUD_RATIO
}

fn status_in_cloud_size(total: f32) -> f32 {
    total * STATUS_IN_CLOUD_RATIO
}

/// Default overhang of the overlay's BR past the circle's BR edge (toward the box's
/// BR), as a fraction of `total_size`. Baked into `corner_overlay_offset` so most
/// surfaces can just pass `0.0` for their `overlay_extra_overhang_ratio`.
const DEFAULT_OVERLAY_OVERHANG_PAST_CIRCLE_EDGE: f32 = 0.19;

/// Returns the pixel offset applied to the overlay's `BottomRight → BottomRight`
/// anchor.
/// The offset is measured from the bounding box's BR corner, so the returned value is
/// negative whenever the overlay sits up-and-left of the box's BR (which is the only
/// case we render).
///
/// `overlay_extra_overhang_ratio` is a signed fraction of `total` added to
/// `DEFAULT_OVERLAY_OVERHANG_PAST_CIRCLE_EDGE`:
/// * `0.0` — overlay BR sits `DEFAULT_OVERLAY_OVERHANG_PAST_CIRCLE_EDGE * total` past
///   the circle's BR (the position most surfaces want).
/// * Positive — overlay BR pushed further toward the box's BR. A value of
///   `1 - CIRCLE_RATIO - DEFAULT_OVERLAY_OVERHANG_PAST_CIRCLE_EDGE` (= 0.05) lands
///   exactly on the box's BR — the Figma-natural overhang.
/// * Negative — overlay BR pulled inward toward the circle's center.
fn corner_overlay_offset(total: f32, overlay_extra_overhang_ratio: f32) -> f32 {
    let total_overhang = DEFAULT_OVERLAY_OVERHANG_PAST_CIRCLE_EDGE + overlay_extra_overhang_ratio;
    -((1.0 - CIRCLE_RATIO) - total_overhang) * total
}

/// What to render inside the circle.
pub(crate) enum IconWithStatusVariant {
    /// A generic icon with a given color on an overlay background.
    Neutral {
        icon: WarpIcon,
        icon_color: WarpThemeFill,
    },
    /// A pre-built icon element on an overlay background.
    NeutralElement { icon_element: Box<dyn Element> },
    /// An Oz agent icon on the theme background.
    OzAgent {
        status: Option<ConversationStatus>,
        is_ambient: bool,
    },
    /// A CLI agent icon on the agent's brand color background.
    CLIAgent {
        agent: CLIAgent,
        status: Option<ConversationStatus>,
        is_ambient: bool,
    },
    /// A pre-rendered avatar with an optional status overlay (cloud lobe when
    /// ambient). Caller must size `avatar` to `circle_size(total_size)` so the
    /// overlay's overhang matches the other variants.
    CustomAvatar {
        avatar: Box<dyn Element>,
        status: Option<ConversationStatus>,
        is_ambient: bool,
    },
}

/// Renders an icon-with-status component sized entirely from a single `total_size`. All
/// sub-components (brand circle, status badge, cloud lobe) are derived proportionally,
/// so callers only need to pick the size they want.
///
/// `overlay_extra_overhang_ratio` is a signed fraction of `total_size` added to the
/// default overlay overhang past the circle's BR edge. Most surfaces pass `0.0` to
/// get the default position; positive values push the overlay further toward the box's
/// BR (more overhang) and negative values pull it inward toward the circle's center.
///
/// When `is_ambient` is set on an agent variant, the status badge is replaced by a
/// cloud (filled with `status_container_background`) containing the status icon.
pub(crate) fn render_icon_with_status(
    variant: IconWithStatusVariant,
    total_size: f32,
    overlay_extra_overhang_ratio: f32,
    theme: &WarpTheme,
    status_container_background: WarpThemeFill,
) -> Box<dyn Element> {
    let sub_text = theme.sub_text_color(theme.background());

    match variant {
        IconWithStatusVariant::Neutral { icon, icon_color } => render_neutral_circle(
            icon.to_warpui_icon(icon_color).finish(),
            internal_colors::fg_overlay_2(theme),
            total_size,
        ),
        IconWithStatusVariant::NeutralElement { icon_element } => render_neutral_circle(
            icon_element,
            internal_colors::fg_overlay_2(theme),
            total_size,
        ),
        IconWithStatusVariant::OzAgent { status, is_ambient } => {
            let circle_background = if is_ambient {
                ThemeFill::Solid(OZ_AMBIENT_BACKGROUND_COLOR)
            } else {
                theme.background()
            };
            // In ambient/cloud mode use the combined `OzCloud` silhouette (Oz + cloud),
            // matching the treatment used in the agent view header. Non-ambient runs
            // continue to use the plain `Oz` glyph.
            let oz_glyph = if is_ambient {
                WarpIcon::OzCloud
            } else {
                WarpIcon::Oz
            };
            // Cloud (ambient) runs use a black glyph on the light-purple background
            // for consistency with the web app; local runs keep the theme text color.
            let glyph_color = if is_ambient {
                WarpThemeFill::Solid(ColorU::black())
            } else {
                theme.main_text_color(theme.background())
            };
            let circle = render_circle(
                oz_glyph.to_warpui_icon(glyph_color).finish(),
                circle_background,
                total_size,
            );
            attach_status_overlay(
                circle,
                status.as_ref(),
                is_ambient,
                total_size,
                overlay_extra_overhang_ratio,
                theme,
                status_container_background,
            )
        }
        IconWithStatusVariant::CLIAgent {
            agent,
            status,
            is_ambient,
        } => {
            let brand_color = agent
                .brand_color()
                .unwrap_or(ColorU::new(100, 100, 100, 255));
            let icon_color = agent.brand_icon_color();
            let icon_element = agent
                .icon()
                .map(|icon| {
                    icon.to_warpui_icon(WarpThemeFill::Solid(icon_color))
                        .finish()
                })
                .unwrap_or_else(|| WarpIcon::Terminal.to_warpui_icon(sub_text).finish());
            let circle = render_circle(icon_element, ThemeFill::Solid(brand_color), total_size);
            attach_status_overlay(
                circle,
                status.as_ref(),
                is_ambient,
                total_size,
                overlay_extra_overhang_ratio,
                theme,
                status_container_background,
            )
        }
        IconWithStatusVariant::CustomAvatar {
            avatar,
            status,
            is_ambient,
        } => attach_status_overlay(
            avatar,
            status.as_ref(),
            is_ambient,
            total_size,
            overlay_extra_overhang_ratio,
            theme,
            status_container_background,
        ),
    }
}

/// Builds the brand-circle container around `icon_element`. The circle's diameter is
/// `circle_size(total)` and the icon glyph is `icon_size(total)`, with the rest going
/// to symmetric padding around the glyph.
/// The returned element is `circle_size(total)` wide; agent callers wrap it via
/// `attach_status_overlay` to occupy the full `total_size` footprint.
fn render_circle(
    icon_element: Box<dyn Element>,
    background: WarpThemeFill,
    total_size: f32,
) -> Box<dyn Element> {
    let icon = icon_size(total_size);
    let padding = circle_padding(total_size);
    let inner = ConstrainedBox::new(icon_element)
        .with_width(icon)
        .with_height(icon)
        .finish();
    Container::new(inner)
        .with_uniform_padding(padding)
        .with_background(background)
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(
            circle_size(total_size) / 2.,
        )))
        .finish()
}

/// Builds the neutral circle: a full-`total_size` container with the glyph at
/// `NEUTRAL_GLYPH_RATIO * total_size`. Used for non-agent surfaces (plain terminal,
/// code, file tabs, etc.) which have no status overlay and therefore should fill the
/// requested bounding box rather than shrinking to `circle_size(total)`.
fn render_neutral_circle(
    icon_element: Box<dyn Element>,
    background: WarpThemeFill,
    total_size: f32,
) -> Box<dyn Element> {
    let glyph = total_size * NEUTRAL_GLYPH_RATIO;
    let padding = (total_size - glyph) / 2.;
    let inner = ConstrainedBox::new(icon_element)
        .with_width(glyph)
        .with_height(glyph)
        .finish();
    Container::new(inner)
        .with_uniform_padding(padding)
        .with_background(background)
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(total_size / 2.)))
        .finish()
}

/// Wraps a brand circle with the appropriate status overlay (badge for non-ambient runs,
/// cloud lobe for ambient runs). Both overlays are derived from `total_size`.
fn attach_status_overlay(
    circle: Box<dyn Element>,
    status: Option<&ConversationStatus>,
    is_ambient: bool,
    total_size: f32,
    overlay_extra_overhang_ratio: f32,
    theme: &WarpTheme,
    status_container_background: WarpThemeFill,
) -> Box<dyn Element> {
    if is_ambient {
        render_with_cloud_status_badge(
            circle,
            status,
            total_size,
            overlay_extra_overhang_ratio,
            theme,
        )
    } else {
        render_with_optional_status_badge(
            circle,
            status,
            total_size,
            overlay_extra_overhang_ratio,
            theme,
            status_container_background,
        )
    }
}

/// Overlays a cloud (with the conversation status icon centered inside, if any) at
/// the bottom-right of the base circle. Used for agents running in ambient/cloud mode.
fn render_with_cloud_status_badge(
    circle: Box<dyn Element>,
    status: Option<&ConversationStatus>,
    total_size: f32,
    overlay_extra_overhang_ratio: f32,
    theme: &WarpTheme,
) -> Box<dyn Element> {
    let cloud_diameter = cloud_icon_size(total_size);
    let cloud = ConstrainedBox::new(
        WarpIcon::CloudFilled
            .to_warpui_icon(theme.foreground())
            .finish(),
    )
    .with_width(cloud_diameter)
    .with_height(cloud_diameter)
    .finish();

    let cloud_with_status: Box<dyn Element> = match status {
        Some(status) => {
            let (icon, color) = status.status_icon_and_color(theme, StatusColorStyle::Cloud);
            let inner = status_in_cloud_size(total_size);
            let status_icon =
                ConstrainedBox::new(icon.to_warpui_icon(WarpThemeFill::Solid(color)).finish())
                    .with_width(inner)
                    .with_height(inner)
                    .finish();
            let mut stack = Stack::new().with_child(cloud);
            // The CloudFilled SVG's visual center of mass sits below the container's
            // geometric center (the cloud is wider at the bottom than the top), so we
            // nudge the status icon down to look optically centered inside the cloud
            // shape rather than the bounding box.
            stack.add_positioned_child(
                status_icon,
                OffsetPositioning::offset_from_parent(
                    vec2f(0., 1.),
                    ParentOffsetBounds::Unbounded,
                    ParentAnchor::Center,
                    ChildAnchor::Center,
                ),
            );
            stack.finish()
        }
        None => cloud,
    };

    let cloud_offset = corner_overlay_offset(total_size, overlay_extra_overhang_ratio);
    let mut stack = Stack::new().with_child(
        ConstrainedBox::new(circle)
            .with_width(total_size)
            .with_height(total_size)
            .finish(),
    );
    stack.add_positioned_child(
        cloud_with_status,
        OffsetPositioning::offset_from_parent(
            vec2f(cloud_offset, cloud_offset),
            ParentOffsetBounds::Unbounded,
            ParentAnchor::BottomRight,
            ChildAnchor::BottomRight,
        ),
    );
    ConstrainedBox::new(stack.finish())
        .with_width(total_size)
        .with_height(total_size)
        .finish()
}

/// Adds a status badge with a cutout ring to the bottom-right of the circle.
fn render_with_optional_status_badge(
    circle: Box<dyn Element>,
    status: Option<&ConversationStatus>,
    total_size: f32,
    overlay_extra_overhang_ratio: f32,
    theme: &WarpTheme,
    status_container_background: WarpThemeFill,
) -> Box<dyn Element> {
    let Some(status) = status else {
        // No status badge: still occupy the full `total_size` footprint so the agent
        // circle (which is only `circle_size(total)` wide) sits centered in the box
        // the caller reserved.
        return ConstrainedBox::new(circle)
            .with_width(total_size)
            .with_height(total_size)
            .finish();
    };
    let (icon, color) = status.status_icon_and_color(theme, StatusColorStyle::Standard);
    let badge_icon_diameter = badge_icon_size(total_size);
    let pad = badge_padding(total_size);
    let badge_icon = ConstrainedBox::new(icon.to_warpui_icon(WarpThemeFill::Solid(color)).finish())
        .with_width(badge_icon_diameter)
        .with_height(badge_icon_diameter)
        .finish();
    let badge = Container::new(badge_icon)
        .with_uniform_padding(pad)
        .with_corner_radius(CornerRadius::with_all(Radius::Percentage(50.)))
        .finish();
    // Cutout ring that visually separates the badge from the circle.
    let badge_with_ring = Container::new(badge)
        .with_uniform_padding(pad)
        .with_background(status_container_background)
        .with_corner_radius(CornerRadius::with_all(Radius::Percentage(50.)))
        .finish();

    let badge_corner_offset = corner_overlay_offset(total_size, overlay_extra_overhang_ratio);
    let mut stack = Stack::new().with_child(
        ConstrainedBox::new(circle)
            .with_width(total_size)
            .with_height(total_size)
            .finish(),
    );
    stack.add_positioned_child(
        badge_with_ring,
        OffsetPositioning::offset_from_parent(
            vec2f(badge_corner_offset, badge_corner_offset),
            ParentOffsetBounds::Unbounded,
            ParentAnchor::BottomRight,
            ChildAnchor::BottomRight,
        ),
    );
    ConstrainedBox::new(stack.finish())
        .with_width(total_size)
        .with_height(total_size)
        .finish()
}
