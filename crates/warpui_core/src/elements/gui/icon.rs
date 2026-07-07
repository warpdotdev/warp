use pathfinder_color::ColorU;
use pathfinder_geometry::rect::RectF;
use pathfinder_geometry::vector::Vector2F;

use super::{Element, Point};
use crate::assets::asset_cache::{AssetCache, AssetSource, AssetState};
use crate::event::DispatchedEvent;
use crate::image_cache::{AnimatedImageBehavior, CacheOption, FitType, Image, ImageCache};
use crate::{
    AfterLayoutContext, AppContext, EventContext, LayoutContext, PaintContext, SingletonEntity,
    SizeConstraint,
};

/// An element that renders a monochrome icon. This differs from `Svg` in that it sets the color dynamically
/// instead of statically from the SVG itself.
#[derive(Clone, Copy)]
pub struct Icon {
    path: &'static str,
    opacity: f32,
    size: Option<Vector2F>,
    origin: Option<Point>,
    color: ColorU,
    #[cfg(debug_assertions)]
    /// Captures the location of the constructor call site. This is used for debugging purposes.
    constructor_location: Option<&'static std::panic::Location<'static>>,
}

impl Icon {
    #[cfg_attr(debug_assertions, track_caller)]
    pub fn new(path: &'static str, color: impl Into<ColorU>) -> Self {
        Self {
            path,
            opacity: 1.,
            size: None,
            color: color.into(),
            origin: None,
            #[cfg(debug_assertions)]
            constructor_location: Some(std::panic::Location::caller()),
        }
    }
    pub fn with_opacity(mut self, opacity: f32) -> Self {
        self.opacity = opacity;
        self
    }
    pub fn with_color(mut self, color: impl Into<ColorU>) -> Self {
        self.color = color.into();
        self
    }
}

impl Element for Icon {
    fn layout(
        &mut self,
        constraint: SizeConstraint,
        _: &mut LayoutContext,
        _: &AppContext,
    ) -> Vector2F {
        let size = constraint.max;
        self.size = Some(size);
        size
    }

    fn after_layout(&mut self, _: &mut AfterLayoutContext, _: &AppContext) {}

    fn paint(&mut self, origin: Vector2F, ctx: &mut PaintContext, app: &AppContext) {
        let bounds = (self.size.unwrap() * ctx.scene.scale_factor()).to_i32();

        // If the x or y bounds are less than or equal to 0, don't attempt to paint the icon.
        if bounds.x() <= 0 || bounds.y() <= 0 {
            return;
        }

        let asset_cache = AssetCache::as_ref(app);
        match ImageCache::as_ref(app).image(
            // Right now, the location of SVG files is hard-coded to be the app bundle. In the future,
            // to make icons a fetch-able asset, we should modify the API of Icon to accept an AssetSource,
            // exactly how Image does.
            AssetSource::Bundled { path: self.path },
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
                    let origin = origin + ((self.size().unwrap() - logical_image_size) / 2.0);
                    self.origin = Some(Point::from_vec2f(origin, ctx.scene.z_index()));

                    #[cfg(debug_assertions)]
                    ctx.scene
                        .set_location_for_panic_logging(self.constructor_location);

                    ctx.scene.draw_icon(
                        RectF::new(origin, logical_image_size),
                        image.clone(),
                        self.opacity,
                        self.color,
                    );
                }
                Image::Animated(_image) => {
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

    fn size(&self) -> Option<Vector2F> {
        self.size
    }

    fn dispatch_event(
        &mut self,
        _: &DispatchedEvent,
        _: &mut EventContext,
        _: &AppContext,
    ) -> bool {
        false
    }

    fn origin(&self) -> Option<Point> {
        self.origin
    }
}
