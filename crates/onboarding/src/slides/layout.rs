use pathfinder_geometry::vector::{Vector2F, vec2f};
use warpui_core::assets::asset_cache::AssetSource;
use warpui_core::elements::{
    Align, CacheOption, Clipped, ConstrainedBox, Container, CrossAxisAlignment, Empty, Expanded,
    Flex, Image, MainAxisSize, ParentElement, Point, Shrinkable, SizeConstraintCondition,
    SizeConstraintSwitch, Stack,
};
use warpui_core::event::DispatchedEvent;
use warpui_core::{
    AfterLayoutContext, AppContext, Element, EventContext, LayoutContext, PaintContext,
    SizeConstraint,
};

// Onboarding images live under `app/assets/async/` so they are excluded from the WASM
// binary (RustEmbed excludes `async/**` on wasm targets). They are still bundled normally
// on native builds. Unlike other `async/` assets these are NOT wired up with
// `bundled_or_fetched_asset!`, so they cannot be fetched remotely on web. We can't use
// that macro here because it resolves paths relative to CARGO_MANIFEST_DIR (i.e.
// `crates/onboarding/`), but the assets live under `app/assets/`. Onboarding is not
// shown on web, so this is fine.
// TODO(APP-3934): support the macro outside the app crate
pub const ONBOARDING_BG_PATH: &str = "async/png/onboarding/onboarding_bg.png";

const LEFT_COLUMN_WIDTH: f32 = 580.;
const LEFT_COLUMN_CONTENT_MAX_WIDTH: f32 = 800.;
const MIN_RIGHT_COLUMN_WIDTH: f32 = 540.;

/// The minimum window width at which the two-column layout is shown (left content + right panel).
/// Below this width, `static_left` collapses to a single left-only column.
pub const TWO_COLUMN_MIN_WIDTH: f32 = LEFT_COLUMN_WIDTH + MIN_RIGHT_COLUMN_WIDTH;

/// Creates a two-column layout with a fixed-width left column and flexible right column.
///
/// The left column's *content* is constrained to [`LEFT_COLUMN_CONTENT_MAX_WIDTH`] and is always
/// horizontally centered within the left panel.
///
/// If the available width is too narrow for the right column to have at least
/// [`MIN_RIGHT_COLUMN_WIDTH`], we instead render only the left column and center it.
///
/// # Arguments
/// * `left` - Builder for the element to display in the left column ([`LEFT_COLUMN_WIDTH`] px)
/// * `right` - Builder for the element to display in the right column (flexible width)
///
/// # Returns
/// A `Box<dyn Element>` containing the responsive layout
pub fn static_left(
    left: impl Fn() -> Box<dyn Element>,
    right: impl FnOnce() -> Box<dyn Element>,
) -> Box<dyn Element> {
    let max_width_for_two_columns = LEFT_COLUMN_WIDTH + MIN_RIGHT_COLUMN_WIDTH;

    let left_constrained = || {
        ConstrainedBox::new(left())
            .with_max_width(LEFT_COLUMN_CONTENT_MAX_WIDTH)
            .finish()
    };

    // Narrow layout: show only the left section, centered.
    // Use Align instead of a max-sized Flex so we can safely center even when the incoming
    // height constraint is unbounded.
    let left_only = Align::new(left_constrained()).finish();

    // Default layout: fixed-width left + flexible right.
    // Use Align instead of a max-sized Flex so we can safely center even when the incoming
    // height constraint is unbounded.
    let left_centered = Align::new(left_constrained()).finish();

    let left_fixed_width = Container::new(
        ConstrainedBox::new(left_centered)
            .with_width(LEFT_COLUMN_WIDTH)
            .finish(),
    )
    .finish();

    let right_flexible = Shrinkable::new(1., Container::new(right()).finish()).finish();

    let two_column_layout = Container::new(
        Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_child(left_fixed_width)
            .with_child(right_flexible)
            .finish(),
    )
    .finish();

    SizeConstraintSwitch::new(
        two_column_layout,
        vec![(
            SizeConstraintCondition::WidthLessThan(max_width_for_two_columns),
            left_only,
        )],
    )
    .finish()
}

/// How horizontal space around the foreground image is applied.
#[derive(Clone, Copy)]
pub enum HPadding {
    /// Symmetric fixed padding on both sides (in pixels). Used for the default layout.
    Fixed(f32),
    /// Symmetric padding on both sides scales with panel width.
    /// Value is a fraction of panel width per side (e.g. `39. / 640.`).
    ProportionalBoth(f32),
    /// Left padding scales with panel width; image fills the rest to the right edge.
    /// Value is a fraction of panel width (e.g. `39. / 640.`).
    ProportionalLeft(f32),
    /// Right padding scales with panel width; image fills the rest to the left edge.
    /// `ratio` is a fraction of panel width (e.g. `69. / 640.`).
    /// `left_offset` is a fixed pixel adjustment to the image's left edge; use a negative
    /// value to shift left when the asset has built-in visual padding on that side.
    ProportionalRight { ratio: f32, left_offset: f32 },
}

/// How the vertical top offset for the foreground image is determined.
#[derive(Clone, Copy)]
pub enum TopMode {
    /// Fixed fraction of panel height: `top = panel_height × ratio`.
    Ratio(f32),
    /// Dynamic: centers a specific fraction of the image's rendered height in the panel.
    ///
    /// For `cover()` width-limited: `image_height = panel_width × (1 − 2×h_ratio) × inv_aspect`
    /// `top = (panel_height × 0.5 − image_height × frac).max(0.)`
    ///
    /// Adapts to both panel dimensions so the chosen image fraction stays
    /// centered even when the panel size changes or the image overflows at the bottom.
    CenterFraction {
        /// Each side’s proportion of panel width used for horizontal spacing.
        h_ratio: f32,
        /// `natural_height / natural_width` of the image asset.
        inv_aspect: f32,
        /// The fraction of image height to center in the panel (e.g. `0.25`).
        frac: f32,
    },
}

/// How the foreground image is fitted within its slot.
#[derive(Clone, Copy)]
pub enum ForegroundFit {
    /// `cover().top_aligned()`: image fills the slot width; overflows at the bottom.
    CoverTopAligned,
    /// `contain().top_aligned()`: full image visible, centered horizontally, pinned to top.
    ContainTopAligned,
    /// `contain().right_aligned()`: full image visible, right edge flush, space on the left.
    ContainRightAligned,
}

/// Sizing and positioning for a foreground image on the onboarding right panel.
///
/// `top_mode` controls vertical positioning; `h_padding` controls horizontal padding.
#[derive(Clone, Copy)]
pub struct ForegroundLayout {
    /// Vertical positioning mode.
    pub top_mode: TopMode,
    /// Horizontal padding mode.
    pub h_padding: HPadding,
    /// Image fit and alignment within its slot.
    pub fit: ForegroundFit,
}

/// Layout for welcome/intention/theme-picker slides.
/// Proportional symmetric horizontal padding; cover() fills the slot width.
/// Dynamic top: centers the 25%-from-top image point in the panel, adapting to panel size.
pub const FOREGROUND_LAYOUT_DEFAULT: ForegroundLayout = ForegroundLayout {
    top_mode: TopMode::CenterFraction {
        h_ratio: 39. / 640.,
        inv_aspect: 612. / 561.,
        frac: 0.25,
    },
    h_padding: HPadding::ProportionalBoth(39. / 640.),
    fit: ForegroundFit::CoverTopAligned,
};

/// Layout for customize slides.
/// Proportional left padding; image fills to the right edge via cover.
/// We cover because the most important part is the top left of the image.
pub const FOREGROUND_LAYOUT_WIDE: ForegroundLayout = ForegroundLayout {
    top_mode: TopMode::Ratio(118. / 800.),
    h_padding: HPadding::ProportionalLeft(39. / 640.),
    fit: ForegroundFit::CoverTopAligned,
};

/// Layout for code-review customize images.
/// Proportional right padding; portrait image fills to the left edge and overflows bottom.
/// We cover because the most important part is the top right of the image.
pub const FOREGROUND_LAYOUT_CODE_REVIEW: ForegroundLayout = ForegroundLayout {
    top_mode: TopMode::Ratio(118. / 800.),
    h_padding: HPadding::ProportionalRight {
        ratio: 69. / 640.,
        left_offset: -16.,
    },
    fit: ForegroundFit::CoverTopAligned,
};

/// Layout for third-party slides.
/// No horizontal padding; contain() shows the full image; right-aligned so any
/// leftover horizontal space appears on the left.
/// We contain because the most important part is the bottom of the image which needs to be visible.
pub const FOREGROUND_LAYOUT_THIRD_PARTY: ForegroundLayout = ForegroundLayout {
    top_mode: TopMode::Ratio(118. / 800.),
    h_padding: HPadding::Fixed(0.),
    fit: ForegroundFit::ContainRightAligned,
};

/// Wraps an image slot with a dynamic top offset that keeps a chosen fraction
/// of the image height centered in the panel.
///
/// The image slot is given the remaining panel height after the computed top offset.
/// The slot's paint overflows downward (visible up to the panel `Clipped` boundary).
struct CenterFractionTopWrapper {
    h_ratio: f32,
    inv_aspect: f32,
    frac: f32,
    inner: Box<dyn Element>,
    panel_size: Option<Vector2F>,
    top: f32,
    origin: Option<Point>,
}

impl CenterFractionTopWrapper {
    fn new(h_ratio: f32, inv_aspect: f32, frac: f32, inner: Box<dyn Element>) -> Self {
        Self {
            h_ratio,
            inv_aspect,
            frac,
            inner,
            panel_size: None,
            top: 0.,
            origin: None,
        }
    }
}

impl Element for CenterFractionTopWrapper {
    fn layout(
        &mut self,
        constraint: SizeConstraint,
        ctx: &mut LayoutContext,
        app: &AppContext,
    ) -> Vector2F {
        let panel = constraint.max;
        self.panel_size = Some(panel);
        // image_height for cover() width-limited: slot_width × inv_aspect
        let slot_w = panel.x() * (1. - 2. * self.h_ratio);
        let image_h = slot_w * self.inv_aspect;
        self.top = (panel.y() * 0.5 - image_h * self.frac).max(0.);
        let slot_h = (panel.y() - self.top).max(1.);
        let slot = vec2f(panel.x(), slot_h);
        self.inner.layout(SizeConstraint::new(slot, slot), ctx, app);
        panel
    }

    fn after_layout(&mut self, ctx: &mut AfterLayoutContext, app: &AppContext) {
        self.inner.after_layout(ctx, app);
    }

    fn paint(&mut self, origin: Vector2F, ctx: &mut PaintContext, app: &AppContext) {
        self.origin = Some(Point::from_vec2f(origin, ctx.scene.z_index()));
        self.inner.paint(origin + vec2f(0., self.top), ctx, app);
    }

    fn dispatch_event(
        &mut self,
        _event: &DispatchedEvent,
        _ctx: &mut EventContext,
        _app: &AppContext,
    ) -> bool {
        false
    }

    fn size(&self) -> Option<Vector2F> {
        self.panel_size
    }

    fn origin(&self) -> Option<Point> {
        self.origin
    }
}

/// Wraps a slide's foreground visual with the shared onboarding background image.
///
/// For `TopMode::Ratio`: uses a `Flex::column` with a proportional `Expanded` top spacer.
/// For `TopMode::CenterFraction`: uses `CenterFractionTopWrapper` which computes the
/// top offset dynamically at layout time based on the actual panel dimensions.
pub fn onboarding_right_panel_with_bg(
    path: &'static str,
    layout: ForegroundLayout,
) -> Box<dyn Element> {
    let background = Image::new(
        AssetSource::Bundled {
            path: ONBOARDING_BG_PATH,
        },
        CacheOption::Original,
    )
    .stretch()
    .finish();

    let image = match layout.fit {
        ForegroundFit::CoverTopAligned => {
            Image::new(AssetSource::Bundled { path }, CacheOption::Original)
                .cover()
                .top_aligned()
                .finish()
        }
        ForegroundFit::ContainTopAligned => {
            Image::new(AssetSource::Bundled { path }, CacheOption::Original)
                .contain()
                .top_aligned()
                .finish()
        }
        ForegroundFit::ContainRightAligned => {
            Image::new(AssetSource::Bundled { path }, CacheOption::Original)
                .contain()
                .right_aligned()
                .finish()
        }
    };

    // Apply horizontal padding: fixed pixels (DEFAULT) or proportional to panel width.
    let image_slot: Box<dyn Element> = match layout.h_padding {
        HPadding::Fixed(px) => Container::new(image)
            .with_padding_left(px)
            .with_padding_right(px)
            .finish(),
        HPadding::ProportionalBoth(ratio) => Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_child(Box::new(Expanded::new(ratio, Box::new(Empty::new()))))
            .with_child(Box::new(Expanded::new(1. - 2. * ratio, image)))
            .with_child(Box::new(Expanded::new(ratio, Box::new(Empty::new()))))
            .finish(),
        HPadding::ProportionalLeft(ratio) => Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_child(Box::new(Expanded::new(ratio, Box::new(Empty::new()))))
            .with_child(Box::new(Expanded::new(1. - ratio, image)))
            .finish(),
        HPadding::ProportionalRight { ratio, left_offset } => {
            let adjusted = if left_offset != 0. {
                Container::new(image)
                    .with_padding_left(left_offset)
                    .finish()
            } else {
                image
            };
            Flex::row()
                .with_main_axis_size(MainAxisSize::Max)
                .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                .with_child(Box::new(Expanded::new(1. - ratio, adjusted)))
                .with_child(Box::new(Expanded::new(ratio, Box::new(Empty::new()))))
                .finish()
        }
    };

    // Build the foreground with either a fixed or dynamic top offset.
    let foreground: Box<dyn Element> = match layout.top_mode {
        TopMode::Ratio(ratio) => {
            let remaining = 1. - ratio;
            Flex::column()
                .with_main_axis_size(MainAxisSize::Max)
                .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                .with_child(Box::new(Expanded::new(ratio, Box::new(Empty::new()))))
                .with_child(Box::new(Expanded::new(remaining, image_slot)))
                .finish()
        }
        TopMode::CenterFraction {
            h_ratio,
            inv_aspect,
            frac,
        } => Box::new(CenterFractionTopWrapper::new(
            h_ratio, inv_aspect, frac, image_slot,
        )),
    };

    let mut stack = Stack::new();
    stack.extend(Some(background));
    stack.extend(Some(foreground));

    Clipped::new(stack.finish()).finish()
}
