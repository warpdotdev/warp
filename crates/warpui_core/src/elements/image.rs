use super::{CornerRadius, Element, Point};
use crate::{
    assets::asset_cache::{AssetCache, AssetSource, AssetState},
    event::DispatchedEvent,
    image_cache::{AnimatedImage, AnimatedImageBehavior, FitType, ImageCache, StaticImage},
    AfterLayoutContext, AppContext, EventContext, LayoutContext, PaintContext, SingletonEntity,
    SizeConstraint,
};

pub use crate::image_cache::CacheOption;
use instant::Instant;
use lazy_static::lazy_static;
use parking_lot::Mutex;
use pathfinder_geometry::rect::RectF;
use pathfinder_geometry::vector::{vec2f, Vector2F, Vector2I};
use std::time::Duration;
use std::{
    collections::HashMap,
    hash::{DefaultHasher, Hash, Hasher},
    sync::Arc,
};

lazy_static! {
    static ref IMAGE_LOAD_TIMEOUT_STARTED_AT: Mutex<HashMap<u64, Instant>> =
        Mutex::new(HashMap::new());
}
struct LoadTimeout {
    timeout: Duration,
    element: Box<dyn Element>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BackupElementKind {
    BeforeLoad,
    FailedToLoad,
    LoadTimeout,
}

pub struct Image {
    source: AssetSource,
    opacity: f32,
    size: Option<Vector2F>,
    origin: Option<Point>,
    fit_type: FitType,
    animated_image_behavior: AnimatedImageBehavior,
    cache_option: CacheOption,
    started_at: Option<Instant>,
    corner_radius: CornerRadius,
    top_aligned: bool,
    right_aligned: bool,

    /// The "back up" element to render when an asset is not ready or encountered an error.
    /// This could be None in two situations: (1) the caller does not provide a before_load_element
    /// or (2) the caller provided one but it's no longer needed due to the image having loaded.
    before_load_element: Option<Box<dyn Element>>,
    failed_to_load_element: Option<Box<dyn Element>>,
    load_timeout: Option<LoadTimeout>,

    /// To avoid duplicating delayed repaint, we store whether or not we've requested a
    /// repaint on behalf of this element.
    ///
    /// Note: we use this for asset loading but not for animation repaint. An animated image
    /// may request several repaints in its lifetime.
    requested_repaint_after_load: bool,
    #[cfg(debug_assertions)]
    /// Captures the location of the constructor call site. This is used for debugging purposes.
    constructor_location: Option<&'static std::panic::Location<'static>>,
}

impl Image {
    /// Creates an image element with an explicit [`CacheOption`].
    ///
    /// Use [`CacheOption::BySize`] for images rendered at a fixed size (icons, thumbnails);
    /// a CPU-resized copy is cached per size.
    /// Use [`CacheOption::Original`] for images whose display size changes continuously
    /// (e.g. background images); only the original asset is cached and the GPU scales it.
    #[cfg_attr(debug_assertions, track_caller)]
    pub fn new(source: AssetSource, cache_option: CacheOption) -> Self {
        Self {
            source,
            opacity: 1.,
            size: None,
            origin: None,
            fit_type: FitType::Contain,
            animated_image_behavior: AnimatedImageBehavior::default(),
            cache_option,
            started_at: None,
            corner_radius: CornerRadius::default(),
            top_aligned: false,
            right_aligned: false,
            before_load_element: None,
            failed_to_load_element: None,
            load_timeout: None,
            requested_repaint_after_load: false,
            #[cfg(debug_assertions)]
            constructor_location: Some(std::panic::Location::caller()),
        }
    }

    pub fn with_corner_radius(mut self, radius: CornerRadius) -> Self {
        self.corner_radius = radius;
        self
    }

    pub fn with_opacity(mut self, opacity: f32) -> Self {
        self.opacity = opacity;
        self
    }

    pub fn cover(mut self) -> Self {
        self.fit_type = FitType::Cover;
        self
    }

    pub fn contain(mut self) -> Self {
        self.fit_type = FitType::Contain;
        self
    }

    /// Stretches the image to fill the element bounds without preserving the aspect ratio.
    pub fn stretch(mut self) -> Self {
        self.fit_type = FitType::Stretch;
        self
    }

    /// Renders animated image sources as a static preview of their first frame.
    pub fn first_frame_preview(mut self) -> Self {
        self.animated_image_behavior = AnimatedImageBehavior::FirstFramePreview;
        self
    }

    /// Aligns the image to the top of the element bounds instead of centering vertically.
    /// Useful for cover-fit images where the bottom should be clipped rather than
    /// cropping equally from top and bottom.
    pub fn top_aligned(mut self) -> Self {
        self.top_aligned = true;
        self
    }

    /// Aligns the image to the right of the element bounds and pins it to the top.
    /// Useful for contain-fit images where the image is narrower than the element
    /// and the empty space should appear on the left rather than being split on both sides.
    pub fn right_aligned(mut self) -> Self {
        self.right_aligned = true;
        self
    }

    /// Enables animated images for the current image element. The start time indicates
    /// the timestamp at which the animated image started rendering. The element uses
    /// this timestamp to calculate which frame of the animation to display at a given
    /// moment.
    /// Animations are still fairly experimental, so you should do extensive testing to
    /// make sure there's no performance degradation from using an animation.
    pub fn enable_animation_with_start_time(mut self, started_at: Instant) -> Self {
        self.started_at = Some(started_at);
        self
    }

    pub fn before_load(mut self, element: Box<dyn Element>) -> Self {
        self.before_load_element = Some(element);
        self
    }
    pub fn on_load_failure(mut self, element: Box<dyn Element>) -> Self {
        self.failed_to_load_element = Some(element);
        self
    }

    pub fn on_load_timeout(mut self, timeout: Duration, element: Box<dyn Element>) -> Self {
        self.load_timeout = Some(LoadTimeout { timeout, element });
        self
    }

    fn load_timeout_key(&self) -> u64 {
        let mut hasher = DefaultHasher::new();
        self.source.hash(&mut hasher);
        hasher.finish()
    }

    fn load_started_at(&self, now: Instant) -> Instant {
        *IMAGE_LOAD_TIMEOUT_STARTED_AT
            .lock()
            .entry(self.load_timeout_key())
            .or_insert(now)
    }

    fn clear_load_timeout_started_at(&self) {
        IMAGE_LOAD_TIMEOUT_STARTED_AT
            .lock()
            .remove(&self.load_timeout_key());
    }

    fn loading_backup_element_kind(
        &mut self,
        now: Instant,
    ) -> (Option<BackupElementKind>, Option<Duration>) {
        let Some(load_timeout) = self.load_timeout.as_ref() else {
            return (
                self.before_load_element
                    .as_ref()
                    .map(|_| BackupElementKind::BeforeLoad),
                None,
            );
        };
        let started_at = self.load_started_at(now);
        let elapsed = now.duration_since(started_at);
        if elapsed >= load_timeout.timeout {
            return (Some(BackupElementKind::LoadTimeout), None);
        }

        (
            self.before_load_element
                .as_ref()
                .map(|_| BackupElementKind::BeforeLoad),
            Some(load_timeout.timeout - elapsed),
        )
    }

    fn failed_to_load_backup_element_kind(&self) -> Option<BackupElementKind> {
        if self.failed_to_load_element.is_some() {
            Some(BackupElementKind::FailedToLoad)
        } else if self.before_load_element.is_some() {
            Some(BackupElementKind::BeforeLoad)
        } else {
            None
        }
    }

    fn paint_backup_element(
        &mut self,
        kind: BackupElementKind,
        origin: Vector2F,
        ctx: &mut PaintContext,
        app: &AppContext,
    ) {
        match kind {
            BackupElementKind::BeforeLoad => {
                if let Some(before_load_element) = self.before_load_element.as_mut() {
                    before_load_element.paint(origin, ctx, app);
                }
            }
            BackupElementKind::FailedToLoad => {
                if let Some(failed_to_load_element) = self.failed_to_load_element.as_mut() {
                    failed_to_load_element.paint(origin, ctx, app);
                }
            }
            BackupElementKind::LoadTimeout => {
                if let Some(load_timeout) = self.load_timeout.as_mut() {
                    load_timeout.element.paint(origin, ctx, app);
                }
            }
        }
    }

    fn paint_static_image(
        &mut self,
        image: Arc<StaticImage>,
        size: Vector2F,
        origin: Vector2F,
        bounds: Vector2I,
        ctx: &mut PaintContext,
    ) {
        let desired_image_size = match self.cache_option {
            CacheOption::Original => {
                dimensions(image.size().to_f32(), bounds.to_f32(), self.fit_type)
            }
            _ => image.size().to_f32(),
        };
        let logical_image_size = desired_image_size / ctx.scene.scale_factor();
        let Some(rect) = image_rect(
            size,
            origin,
            logical_image_size,
            self.top_aligned,
            self.right_aligned,
        ) else {
            self.origin = None;
            log::error!(
                "invalid image rect before draw_image source={:?} element_size=({}, {}) image_size=({}, {}) desired_image_size=({}, {}) logical_image_size=({}, {}) origin=({}, {}) bounds=({}, {}) fit_type={:?} cache_option={:?}",
                self.source,
                size.x(),
                size.y(),
                image.width(),
                image.height(),
                desired_image_size.x(),
                desired_image_size.y(),
                logical_image_size.x(),
                logical_image_size.y(),
                origin.x(),
                origin.y(),
                bounds.x(),
                bounds.y(),
                self.fit_type,
                self.cache_option,
            );
            return;
        };

        self.origin = Some(Point::from_vec2f(rect.origin(), ctx.scene.z_index()));

        #[cfg(debug_assertions)]
        ctx.scene
            .set_location_for_panic_logging(self.constructor_location);

        ctx.scene
            .draw_image(rect, image, self.opacity, self.corner_radius);
    }

    fn paint_animated_image(
        &mut self,
        animated_image: Arc<AnimatedImage>,
        size: Vector2F,
        origin: Vector2F,
        bounds: Vector2I,
        ctx: &mut PaintContext,
    ) {
        // If self.started_at is not provided, we set it to current time
        // so only the first frame is shown.
        let started_at = self.started_at.unwrap_or_else(Instant::now);
        let elapsed_time = started_at.elapsed().as_millis();
        // After about ~50 days, casting `elapsed_time` to a u32 will
        // silently overflow. The gif may jump and start playing from a
        // different frame.
        match animated_image.get_current_frame(elapsed_time as u32) {
            Ok((frame, remaining_delay)) => {
                self.paint_static_image(frame.clone(), size, origin, bounds, ctx);
                // Only repaint if self.started_at is set. Otherwise
                // enable_animation_with_start_time has not been called
                // and we shouldn't animate.
                if self.started_at.is_some() {
                    ctx.repaint_after(Duration::from_millis(remaining_delay as u64));
                }
            }
            Err(e) => {
                log::error!("Unable to retrieve current frame from image: {e:?}");
            }
        }
    }
}

fn image_rect(
    size: Vector2F,
    origin: Vector2F,
    logical_image_size: Vector2F,
    top_aligned: bool,
    right_aligned: bool,
) -> Option<RectF> {
    let offset = if right_aligned {
        vec2f(size.x() - logical_image_size.x(), 0.0)
    } else if top_aligned {
        vec2f((size.x() - logical_image_size.x()) / 2.0, 0.0)
    } else {
        (size - logical_image_size) / 2.0
    };
    let origin = origin + offset;
    let rect = RectF::new(origin, logical_image_size);
    if rect.origin().x().is_finite()
        && rect.origin().y().is_finite()
        && rect.size().x().is_finite()
        && rect.size().y().is_finite()
    {
        Some(rect)
    } else {
        None
    }
}

/// Returns desired dimensions of the image given the original size (x, y), desired container size
/// (dest_x, dest_y) and fit_type.
/// Returns a vector with new dimensions maintaining the original aspect ratio,
/// unless the FitType is `Stretch`.
fn dimensions(original: Vector2F, dest: Vector2F, fit_type: FitType) -> Vector2F {
    let ratio_x = dest.x() / original.x();
    let ratio_y = dest.y() / original.y();

    let ratio = match fit_type {
        FitType::Contain => ratio_x.min(ratio_y),
        FitType::Cover => ratio_x.max(ratio_y),
        FitType::Stretch => {
            // Stretch doesn't maintain aspect ratio
            return dest;
        }
    };

    let x = original.x() * ratio;
    let y = original.y() * ratio;

    vec2f(x.max(1.), y.max(1.)).round()
}

impl Element for Image {
    fn layout(
        &mut self,
        constraint: SizeConstraint,
        ctx: &mut LayoutContext,
        app: &AppContext,
    ) -> Vector2F {
        let size = constraint.max;
        self.size = Some(size);

        if let Some(before_load_element) = self.before_load_element.as_mut() {
            before_load_element.layout(constraint, ctx, app);
        }
        if let Some(failed_to_load_element) = self.failed_to_load_element.as_mut() {
            failed_to_load_element.layout(constraint, ctx, app);
        }
        if let Some(load_timeout) = self.load_timeout.as_mut() {
            load_timeout.element.layout(constraint, ctx, app);
        }

        size
    }

    fn after_layout(&mut self, ctx: &mut AfterLayoutContext, app: &AppContext) {
        if let Some(before_load_element) = self.before_load_element.as_mut() {
            before_load_element.after_layout(ctx, app);
        }
        if let Some(failed_to_load_element) = self.failed_to_load_element.as_mut() {
            failed_to_load_element.after_layout(ctx, app);
        }
        if let Some(load_timeout) = self.load_timeout.as_mut() {
            load_timeout.element.after_layout(ctx, app);
        }
    }

    fn paint(&mut self, origin: Vector2F, ctx: &mut PaintContext, app: &AppContext) {
        let Some(size) = self.size else {
            return;
        };

        let bounds = (size * ctx.scene.scale_factor()).to_i32();
        if !size.x().is_finite()
            || !size.y().is_finite()
            || size.x() <= 0.0
            || size.y() <= 0.0
            || bounds.x() <= 0
            || bounds.y() <= 0
        {
            log::warn!(
                "image paint with suspicious size source={:?} element_size=({}, {}) bounds=({}, {}) fit_type={:?} cache_option={:?}",
                self.source,
                size.x(),
                size.y(),
                bounds.x(),
                bounds.y(),
                self.fit_type,
                self.cache_option,
            );
        }
        let assert_cache = AssetCache::as_ref(app);
        let image = ImageCache::as_ref(app).image(
            self.source.clone(),
            bounds,
            self.fit_type,
            self.animated_image_behavior,
            self.cache_option,
            ctx.max_texture_dimension_2d,
            assert_cache,
        );

        match image {
            AssetState::Loading { handle } => {
                if !self.requested_repaint_after_load {
                    ctx.repaint_after_load(handle);
                    self.requested_repaint_after_load = true;
                }
                let (backup_element_kind, repaint_after) =
                    self.loading_backup_element_kind(Instant::now());
                if let Some(repaint_after) = repaint_after {
                    ctx.repaint_after(repaint_after);
                }
                if let Some(kind) = backup_element_kind {
                    self.paint_backup_element(kind, origin, ctx, app);
                }
            }
            AssetState::Evicted => {
                self.clear_load_timeout_started_at();
                if let Some(before_load_element) = self.before_load_element.as_mut() {
                    before_load_element.paint(origin, ctx, app);
                }
            }
            AssetState::FailedToLoad(_) => {
                self.clear_load_timeout_started_at();
                if let Some(kind) = self.failed_to_load_backup_element_kind() {
                    self.paint_backup_element(kind, origin, ctx, app);
                }
            }
            AssetState::Loaded { data } => {
                // Don't waste time calling layout() and after_layout() on the backup element once the main
                // one has loaded.
                self.clear_load_timeout_started_at();
                self.before_load_element = None;
                self.failed_to_load_element = None;
                self.load_timeout = None;

                match data.as_ref() {
                    crate::image_cache::Image::Static(static_image) => {
                        self.paint_static_image(static_image.clone(), size, origin, bounds, ctx)
                    }
                    crate::image_cache::Image::Animated(animated_image) => {
                        self.paint_animated_image(animated_image.clone(), size, origin, bounds, ctx)
                    }
                }
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

#[cfg(test)]
#[path = "image_tests.rs"]
mod tests;
