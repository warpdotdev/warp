use pathfinder_color::ColorU;
use warp_core::ui::Icon;
use warpui_core::assets::asset_cache::{AssetCache, AssetSource, AssetState};
use warpui_core::elements::{CornerRadius, Fill, Point, Radius};
use warpui_core::event::DispatchedEvent;
use warpui_core::geometry::rect::RectF;
use warpui_core::geometry::vector::{vec2f, Vector2F};
use warpui_core::image_cache::{AnimatedImageBehavior, CacheOption, FitType, Image, ImageCache};
use warpui_core::{
    AfterLayoutContext, AppContext, Element, EventContext, LayoutContext, PaintContext,
    SingletonEntity as _, SizeConstraint,
};

#[derive(Debug, Clone, Copy)]
pub(crate) struct RectPct {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl RectPct {
    pub const fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self { x, y, w, h }
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct Pill {
    pub rect: RectPct,
    pub color: ColorU,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct Rect {
    pub rect: RectPct,
    pub color: ColorU,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct IconPct {
    pub icon: Icon,
    pub color: ColorU,
    pub center_x: f32,
    pub center_y: f32,
    pub width_pct: f32,
}

pub(crate) struct OnboardingVisual {
    panel_background: ColorU,
    pills: Vec<Pill>,
    rects: Vec<Rect>,
    icons: Vec<IconPct>,
    terminal_box: bool,

    size: Option<Vector2F>,
    origin: Option<Point>,
}

impl OnboardingVisual {
    // Padding scale parameters (based on max(x, y) in pixels).
    const MIN_PADDING_SIZE_PX: f32 = 500.0;
    const MAX_PADDING_SIZE_PX: f32 = 2000.0;
    const MIN_PADDING_PCT: f32 = 0.10;
    const MAX_PADDING_PCT: f32 = 0.30;

    pub fn new(panel_background: ColorU, pills: Vec<Pill>, terminal_box: bool) -> Self {
        Self::new_internal(panel_background, pills, terminal_box)
    }

    fn new_internal(panel_background: ColorU, pills: Vec<Pill>, terminal_box: bool) -> Self {
        Self {
            panel_background,
            pills,
            icons: Vec::new(),
            rects: Vec::new(),
            terminal_box,
            size: None,
            origin: None,
        }
    }

    pub fn with_icons(mut self, icons: Vec<IconPct>) -> Self {
        self.icons = icons;
        self
    }

    pub fn with_rects(mut self, rects: Vec<Rect>) -> Self {
        self.rects = rects;
        self
    }

    pub fn compute_contained_size(constraint: SizeConstraint) -> Vector2F {
        // Maintain the old image’s aspect ratio so the right-side layout stays stable.
        const ASPECT_RATIO: f32 = 1.5;

        let max = constraint.max;

        if max.x().is_infinite() && max.y().is_infinite() {
            return vec2f(constraint.min.x().max(0.), constraint.min.y().max(0.));
        }

        let mut width = if max.x().is_finite() {
            max.x().max(0.)
        } else {
            (max.y().max(0.)) * ASPECT_RATIO
        };

        let mut height = if max.y().is_finite() {
            max.y().max(0.)
        } else {
            width / ASPECT_RATIO
        };

        // Contain within both axes when both are finite.
        if max.x().is_finite() && max.y().is_finite() {
            height = width / ASPECT_RATIO;
            if height > max.y() {
                height = max.y();
                width = height * ASPECT_RATIO;
            }
        }

        vec2f(width.max(1.), height.max(1.))
    }

    fn feature_radius_px(inner_height: f32) -> f32 {
        // All features use the same radius.
        inner_height * 0.02
    }

    fn draw_pill(
        &self,
        rect: RectF,
        color: ColorU,
        feature_radius_px: f32,
        ctx: &mut PaintContext,
    ) {
        ctx.scene
            .draw_rect_with_hit_recording(rect)
            .with_background(Fill::Solid(color))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(feature_radius_px)));
    }

    fn draw_rect(&self, rect: RectF, color: ColorU, ctx: &mut PaintContext) {
        ctx.scene
            .draw_rect_with_hit_recording(rect)
            .with_background(Fill::Solid(color));
    }

    fn rect_from_pct(inner_origin: Vector2F, inner_size: Vector2F, rect: RectPct) -> RectF {
        RectF::new(
            vec2f(
                inner_origin.x() + inner_size.x() * rect.x,
                inner_origin.y() + inner_size.y() * rect.y,
            ),
            vec2f(inner_size.x() * rect.w, inner_size.y() * rect.h),
        )
    }

    fn icon_rect_from_pct(
        inner_origin: Vector2F,
        inner_size: Vector2F,
        center_x: f32,
        center_y: f32,
        width_pct: f32,
    ) -> RectF {
        let center = vec2f(
            inner_origin.x() + inner_size.x() * center_x,
            inner_origin.y() + inner_size.y() * center_y,
        );
        let width_px = inner_size.x() * width_pct;
        RectF::new(
            center - vec2f(width_px / 2.0, width_px / 2.0),
            vec2f(width_px, width_px),
        )
    }

    fn draw_icon(
        &self,
        rect: RectF,
        icon: Icon,
        color: ColorU,
        ctx: &mut PaintContext,
        app: &AppContext,
    ) {
        let bounds = (rect.size() * ctx.scene.scale_factor()).to_i32();
        if bounds.x() <= 0 || bounds.y() <= 0 {
            return;
        }

        let path: &'static str = icon.into();

        let asset_cache = AssetCache::as_ref(app);
        match ImageCache::as_ref(app).image(
            AssetSource::Bundled { path },
            bounds,
            FitType::Contain,
            AnimatedImageBehavior::FullAnimation,
            CacheOption::BySize,
            ctx.max_texture_dimension_2d,
            asset_cache,
        ) {
            AssetState::Loaded { data } => match data.as_ref() {
                Image::Static(image) => {
                    let logical_image_size = image.size().to_f32() / ctx.scene.scale_factor();
                    let origin = rect.origin() + ((rect.size() - logical_image_size) / 2.0);
                    ctx.scene.draw_icon(
                        RectF::new(origin, logical_image_size),
                        image.clone(),
                        1.0,
                        color,
                    );
                }
                Image::Animated(_) => {
                    log::info!("Animated icons are currently not supported");
                }
            },
            AssetState::Loading { handle } => {
                ctx.repaint_after_load(handle);
            }
            AssetState::Evicted => {
                log::warn!("Unable to render svg because it was evicted");
            }
            AssetState::FailedToLoad(err) => {
                log::warn!("Unable to render svg: {err:#}");
            }
        }
    }
}

impl Element for OnboardingVisual {
    fn layout(
        &mut self,
        constraint: SizeConstraint,
        _ctx: &mut LayoutContext,
        _app: &AppContext,
    ) -> Vector2F {
        let size = Self::compute_contained_size(constraint);
        self.size = Some(size);
        size
    }

    fn after_layout(&mut self, _ctx: &mut AfterLayoutContext, _app: &AppContext) {}

    fn paint(&mut self, origin: Vector2F, ctx: &mut PaintContext, _app: &AppContext) {
        let Some(size) = self.size else {
            return;
        };

        self.origin = Some(Point::from_vec2f(origin, ctx.scene.z_index()));

        let max_dimension = size.x().max(size.y());

        // Scale padding linearly with size.
        let padding_pct = if max_dimension <= Self::MIN_PADDING_SIZE_PX {
            Self::MIN_PADDING_PCT
        } else if max_dimension >= Self::MAX_PADDING_SIZE_PX {
            Self::MAX_PADDING_PCT
        } else {
            let t = (max_dimension - Self::MIN_PADDING_SIZE_PX)
                / (Self::MAX_PADDING_SIZE_PX - Self::MIN_PADDING_SIZE_PX);
            Self::MIN_PADDING_PCT + t * (Self::MAX_PADDING_PCT - Self::MIN_PADDING_PCT)
        };

        let padding_x: f32 = padding_pct * size.x();
        let padding_y: f32 = padding_pct * size.y();

        let inner_origin = origin + vec2f(padding_x, padding_y);
        let inner_size = vec2f(
            (size.x() - 2.0 * padding_x).max(1.),
            (size.y() - 2.0 * padding_y).max(1.),
        );

        let feature_radius_px = Self::feature_radius_px(inner_size.y());

        // Background panel.
        ctx.scene
            .draw_rect_with_hit_recording(RectF::new(inner_origin, inner_size))
            .with_background(Fill::Solid(self.panel_background))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(feature_radius_px)));

        if self.terminal_box {
            // Bottom container behind the final rows.
            const TERMINAL_BOX_Y_PCT: f32 = 0.80;
            const TERMINAL_BOX_H_PCT: f32 = 0.25;

            let min_x = self
                .pills
                .iter()
                .map(|pill| pill.rect.x)
                .fold(0.0_f32, f32::min);
            let max_x = self
                .pills
                .iter()
                .map(|pill| pill.rect.x + pill.rect.w)
                .fold(1.0_f32, f32::max);

            let box_rect = Self::rect_from_pct(
                inner_origin,
                inner_size,
                RectPct::new(min_x, TERMINAL_BOX_Y_PCT, max_x - min_x, TERMINAL_BOX_H_PCT),
            );

            let box_color = self
                .pills
                .first()
                .map(|pill| pill.color)
                .unwrap_or(self.panel_background);

            ctx.scene
                .draw_rect_with_hit_recording(box_rect)
                .with_background(Fill::Solid(box_color))
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(feature_radius_px)));
        }

        // Rects.
        for rect in &self.rects {
            let rect_px = Self::rect_from_pct(inner_origin, inner_size, rect.rect);
            self.draw_rect(rect_px, rect.color, ctx);
        }

        // Pills.
        for pill in &self.pills {
            let rect = Self::rect_from_pct(inner_origin, inner_size, pill.rect);
            self.draw_pill(rect, pill.color, feature_radius_px, ctx);
        }

        // Icons.
        for icon in &self.icons {
            let rect = Self::icon_rect_from_pct(
                inner_origin,
                inner_size,
                icon.center_x,
                icon.center_y,
                icon.width_pct,
            );
            self.draw_icon(rect, icon.icon, icon.color, ctx, _app);
        }
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
        self.size
    }

    fn origin(&self) -> Option<Point> {
        self.origin
    }
}
