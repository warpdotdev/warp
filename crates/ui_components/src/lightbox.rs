use std::sync::{Arc, Mutex};

use instant::Instant;
use pathfinder_geometry::vector::{Vector2F, vec2f};
use warp_core::ui::{Icon, appearance::Appearance};
use warpui::{
    AfterLayoutContext, AppContext, ClipBounds, Element, EventContext, LayoutContext, PaintContext,
    SizeConstraint,
    assets::asset_cache::AssetSource,
    elements::{
        CacheOption, ClippedScrollStateHandle, ClippedScrollable, ConstrainedBox, Container,
        DispatchEventResult, Dismiss, EventHandler, Fill, Image, Point, ScrollbarWidth,
        Shrinkable,
    },
    event::{DispatchedEvent, Event, ModifiersState},
    keymap::Keystroke,
    prelude::{stack::*, *},
    scene::{Border, CornerRadius, Radius},
};

use crate::{Component, Options as _, button};

/// Padding between the scrim edge and the image.
const SCRIM_PADDING: f32 = 48.;

/// GH9729 §698 / t2-13: inset for buttons anchored to a scrim
/// corner. The close button still uses this for its top-right offset;
/// the zoom toolbar moved to a bottom-center pill (see ZOOM_PILL_*
/// constants below) so it no longer reads from this value.
const SCRIM_BUTTON_INSET: f32 = 12.;

/// GH9729 §698 / t2-12: scroll-delta magnitude below which a
/// cmd+scroll event is treated as a no-op. Stops trackpad jitter at
/// rest from triggering rapid-fire zoom steps. Empirically: macOS
/// continuous-touch scroll events report values well below 1.0 at
/// rest and above 1.0 during deliberate gestures.
const SCROLL_ZOOM_DEAD_ZONE: f32 = 1.0;

/// GH9729 (post-tier2): vertical thumbnail-rail constants. Side rail
/// shown when the lightbox is opened with > 1 sibling images so the
/// user can hop between them by clicking; aligns with Warp's vertical-
/// tabs UX direction.
///
/// Sizing decisions (UX revision after first manual review):
/// * Rail is **flush to the left edge of the scrim** — the scrim's
///   uniform padding only applies inside the right cell so the image
///   stays nicely inset, but the rail itself hugs the viewport edge
///   the way macOS Preview / Mail.app sidebars do.
/// * 64 px thumbs (down from 72) give a tighter, denser list — more
///   siblings visible per viewport without scrolling.
/// * 6 px inter-thumb spacing (down from 8) — native sidebars feel
///   tight; loose spacing reads as "this is not a list".
/// * 12 px horizontal padding inside the rail, computed so the
///   centering slack on either side of a 64-px thumb in an 88-px-wide
///   rail is exactly 12 px — symmetric without a separate centering
///   gap on top of the padding.
const RAIL_WIDTH: f32 = 88.;
const RAIL_THUMB_SIZE: f32 = 64.;
const RAIL_THUMB_SPACING: f32 = 6.;
const RAIL_OUTER_PADDING_VERTICAL: f32 = 16.;
/// Width of the soft-fill background behind the currently-selected
/// thumbnail; sits 4 px wider on each side than the thumb itself so
/// the highlight reads as a "pill behind the thumb" (macOS Mail
/// convention) rather than a hard outline.
const RAIL_HIGHLIGHT_INSET: f32 = 4.;
const RAIL_HIGHLIGHT_CORNER_RADIUS: f32 = 8.;
const RAIL_HIGHLIGHT_RING_WIDTH: f32 = 2.;
/// 1 px hairline on the rail's right edge, separating it from the
/// image canvas without being a heavy visual line.
const RAIL_DIVIDER_WIDTH: f32 = 1.;
/// Single rail row's effective height: thumb + spacing between rows.
/// Used by the view-side scroll-to-current helper.
pub const RAIL_ROW_PITCH: f32 = RAIL_THUMB_SIZE + RAIL_THUMB_SPACING;

/// GH9729 (post-tier2): zoom-pill constants. Replaces the previous
/// separate-button toolbar that sat in the bottom-left corner under
/// the rail. The pill is a single rounded translucent container in
/// the bottom-center of the scrim, holding the zoom-out, optional
/// 100%, and zoom-in buttons in a tight Flex::row.
///
/// Bottom-center placement is the Photos.app / Preview multi-image
/// convention; it sidesteps the bottom-LEFT crowding under the rail
/// AND the bottom-RIGHT crowding next to the close button.
const ZOOM_PILL_HORIZONTAL_PADDING: f32 = 8.;
const ZOOM_PILL_VERTICAL_PADDING: f32 = 4.;
const ZOOM_PILL_CORNER_RADIUS: f32 = 14.;
const ZOOM_PILL_BUTTON_GAP: f32 = 4.;
const ZOOM_PILL_BOTTOM_INSET: f32 = 16.;

/// GH9729 §698 / t2-19: a viewport that lets its child render at any
/// size (potentially larger than the viewport itself), centers it, and
/// clips paint to viewport bounds. Also tracks drag-to-pan and routes
/// the user's drag into a caller-supplied `on_pan` callback.
///
/// This is the answer to the long-standing t2-7-r1 gotcha: the
/// framework's `ConstrainedBox::layout` tightens its child's max by
/// parent's max, so images can't grow past viewport via the normal
/// layout path. `PanClippedImage` short-circuits that by forcing its
/// child's layout constraint to `SizeConstraint::strict(desired_size)`
/// — the child renders at exactly the size we ask for, regardless of
/// our own bounds. Then we paint it inside a clipped layer at the
/// viewport rect, optionally offset by `pan_offset`.
///
/// Mouse-down + drag + mouse-up are tracked here so the lightbox view
/// owns the canonical `pan_offset` state and we get a single point of
/// truth for clamping. cmd+scroll-wheel is also captured here (it used
/// to live on the per-image `EventHandler`) so all viewport-scoped
/// gestures share the same hit-test bounds.
struct PanClippedImage {
    child: Box<dyn Element>,
    desired_size: Vector2F,
    pan_offset: Vector2F,
    on_pan: Option<Arc<dyn Fn(Vector2F, &mut EventContext, &AppContext)>>,
    on_zoom: Option<ZoomHandler>,
    on_double_tap_zoom:
        Option<Arc<dyn Fn(Vector2F, &mut EventContext, &AppContext)>>,

    origin: Option<Point>,
    viewport_size: Option<Vector2F>,
    /// GH9729 t2-20: drag state held in the persistent `Lightbox`
    /// struct via `Arc<Mutex<…>>` so it survives the `Pan` action's
    /// `ctx.notify()` re-render. A plain struct field on
    /// `PanClippedImage` would be reset every render and the user's
    /// drag would freeze after the first delta.
    drag_state: Arc<Mutex<Option<Vector2F>>>,
}

impl PanClippedImage {
    fn new(
        child: Box<dyn Element>,
        desired_size: Vector2F,
        pan_offset: Vector2F,
        on_pan: Option<Arc<dyn Fn(Vector2F, &mut EventContext, &AppContext)>>,
        on_zoom: Option<ZoomHandler>,
        on_double_tap_zoom: Option<
            Arc<dyn Fn(Vector2F, &mut EventContext, &AppContext)>,
        >,
        drag_state: Arc<Mutex<Option<Vector2F>>>,
    ) -> Self {
        Self {
            child,
            desired_size,
            pan_offset,
            on_pan,
            on_zoom,
            on_double_tap_zoom,
            origin: None,
            viewport_size: None,
            drag_state,
        }
    }

    /// Compute the maximum half-extent of pan in each axis: half of the
    /// overflow of child past viewport. Zero when child fits in viewport.
    fn max_pan(child_size: Vector2F, viewport: Vector2F) -> Vector2F {
        vec2f(
            ((child_size.x() - viewport.x()) / 2.0).max(0.0),
            ((child_size.y() - viewport.y()) / 2.0).max(0.0),
        )
    }

    /// Clamp `pan_offset` so the user can't drag the image so far that
    /// the visible edge moves past viewport center (which would reveal
    /// the scrim and feel sloppy).
    fn clamp_pan(pan: Vector2F, child_size: Vector2F, viewport: Vector2F) -> Vector2F {
        let max = Self::max_pan(child_size, viewport);
        vec2f(
            pan.x().clamp(-max.x(), max.x()),
            pan.y().clamp(-max.y(), max.y()),
        )
    }

    fn point_in_viewport(&self, position: Vector2F) -> bool {
        let Some(origin) = self.origin else {
            return false;
        };
        let Some(viewport) = self.viewport_size else {
            return false;
        };
        let origin_xy = origin.xy();
        position.x() >= origin_xy.x()
            && position.x() <= origin_xy.x() + viewport.x()
            && position.y() >= origin_xy.y()
            && position.y() <= origin_xy.y() + viewport.y()
    }
}

impl Element for PanClippedImage {
    fn layout(
        &mut self,
        constraint: SizeConstraint,
        ctx: &mut LayoutContext,
        app: &AppContext,
    ) -> Vector2F {
        // Fill parent's max (the lightbox content area). This becomes
        // the viewport — clip bounds and pan-clamp boundary.
        let viewport = constraint.max;
        self.viewport_size = Some(viewport);

        // Force child to render at exactly `desired_size`, bypassing
        // parent-max binding. This is the load-bearing line — without
        // it, t2-7-r1 returns.
        self.child
            .layout(SizeConstraint::strict(self.desired_size), ctx, app);

        viewport
    }

    fn after_layout(&mut self, ctx: &mut AfterLayoutContext, app: &AppContext) {
        self.child.after_layout(ctx, app);
    }

    fn paint(&mut self, origin: Vector2F, ctx: &mut PaintContext, app: &AppContext) {
        let origin_point = Point::from_vec2f(origin, ctx.scene.z_index());
        self.origin = Some(origin_point);

        let viewport = self
            .viewport_size
            .expect("layout must run before paint");
        let child_size = self.child.size().unwrap_or(self.desired_size);

        // Center child within viewport, apply clamped pan offset.
        let pan = Self::clamp_pan(self.pan_offset, child_size, viewport);
        let child_origin = vec2f(
            origin.x() + (viewport.x() - child_size.x()) / 2.0 + pan.x(),
            origin.y() + (viewport.y() - child_size.y()) / 2.0 + pan.y(),
        );

        // Paint inside a clipped layer at viewport bounds so the
        // oversized child doesn't bleed past the lightbox scrim.
        if let Some(visible) = ctx.scene.visible_rect(origin_point, viewport) {
            ctx.scene.start_layer(ClipBounds::BoundedBy(visible));
            self.child.paint(child_origin, ctx, app);
            ctx.scene.stop_layer();
        }
    }

    fn dispatch_event(
        &mut self,
        event: &DispatchedEvent,
        ctx: &mut EventContext,
        app: &AppContext,
    ) -> bool {
        match event.raw_event() {
            Event::LeftMouseDown {
                click_count,
                position,
                ..
            } if self.point_in_viewport(*position) => {
                // GH9729 t2-21: double-click = zoom-and-center on tap.
                // Skip drag-tracking so the second-click's drag delta
                // doesn't pan the image during the gesture.
                if *click_count >= 2 {
                    if let (Some(origin), Some(viewport)) =
                        (self.origin, self.viewport_size)
                    {
                        let viewport_center = origin.xy() + viewport * 0.5;
                        let tap_offset = *position - viewport_center;
                        if let Some(cb) = self.on_double_tap_zoom.as_ref() {
                            cb(tap_offset, ctx, app);
                        }
                    }
                    if let Ok(mut state) = self.drag_state.lock() {
                        *state = None;
                    }
                    return true;
                }
                // Single click: start tracking drag in shared state;
                // consume so the scrim's Dismiss doesn't fire on the
                // same down event.
                if let Ok(mut state) = self.drag_state.lock() {
                    *state = Some(*position);
                }
                true
            }
            Event::LeftMouseDragged { position, .. } => {
                let last = self
                    .drag_state
                    .lock()
                    .ok()
                    .and_then(|state| *state);
                if let Some(last) = last {
                    let delta = *position - last;
                    if let Some(on_pan) = self.on_pan.as_ref() {
                        on_pan(self.pan_offset + delta, ctx, app);
                    }
                    if let Ok(mut state) = self.drag_state.lock() {
                        *state = Some(*position);
                    }
                    return true;
                }
                false
            }
            Event::LeftMouseUp { .. } => {
                let was_dragging = self
                    .drag_state
                    .lock()
                    .ok()
                    .map(|state| state.is_some())
                    .unwrap_or(false);
                if was_dragging {
                    if let Ok(mut state) = self.drag_state.lock() {
                        *state = None;
                    }
                    return true;
                }
                false
            }
            Event::ScrollWheel {
                position,
                delta,
                modifiers,
                ..
            } if self.point_in_viewport(*position) => {
                // cmd+scroll = zoom (preserved from t2-12). Plain
                // scroll falls through so parent surfaces can use it.
                let cmd_or_ctrl = modifiers_have_cmd_or_ctrl(modifiers);
                if !cmd_or_ctrl {
                    return false;
                }
                let dy = delta.y();
                if dy.abs() < SCROLL_ZOOM_DEAD_ZONE {
                    return true;
                }
                if let Some(on_zoom) = self.on_zoom.as_ref() {
                    let direction = if dy > 0.0 {
                        ZoomDirection::In
                    } else {
                        ZoomDirection::Out
                    };
                    on_zoom(direction, ctx, app);
                }
                true
            }
            _ => false,
        }
    }

    fn size(&self) -> Option<Vector2F> {
        self.viewport_size
    }

    fn origin(&self) -> Option<Point> {
        self.origin
    }
}

fn modifiers_have_cmd_or_ctrl(modifiers: &ModifiersState) -> bool {
    modifiers.cmd || modifiers.ctrl
}

/// GH9729 §699: how much smaller the metadata strip is than the
/// description. The metadata strip carries secondary information
/// (dimensions, format, size) and should read as supporting detail,
/// not a peer of the description.
const METADATA_TEXT_SIZE_REDUCTION: f32 = 2.;

/// GH9729 §699: alpha for the metadata strip's foreground colour.
/// 0.7 keeps it legible while making clear it's secondary.
const METADATA_TEXT_ALPHA: u8 = 178; // 255 * 0.7

fn metadata_text_size(appearance: &Appearance) -> f32 {
    (lightbox_text_size(appearance) - METADATA_TEXT_SIZE_REDUCTION).max(8.0)
}

fn metadata_text_color() -> ColorU {
    ColorU::new(255, 255, 255, METADATA_TEXT_ALPHA)
}

/// GH9729 §698: minimum zoom factor — below this the image becomes too
/// small to be useful and the controls feel runaway.
pub const MIN_ZOOM_FACTOR: f32 = 0.25;

/// GH9729 §698: maximum zoom factor — above this typical raster decode
/// pixels exceed any reasonable display, and the per-paint resampling
/// cost grows quadratically.
pub const MAX_ZOOM_FACTOR: f32 = 8.0;

/// GH9729 §698 / t2-21: multiplicative step applied per zoom-in /
/// zoom-out button click. `1.25` matches the macOS Preview /
/// Safari / Chrome cmd-+ convention. At this step,
/// `MAX_ZOOM_FACTOR = 8.0` is reached in ~10 `+` clicks and
/// `MIN_ZOOM_FACTOR = 0.25` in ~7 `-` clicks from the `1.0` default.
pub const ZOOM_STEP: f32 = 1.25;

/// GH9729 §698 / t2-21: target zoom for the double-tap-to-zoom
/// gesture. macOS Preview and iOS Photos use the same convention —
/// double-tap zooms to a fixed multiple of native size, second
/// double-tap returns to native. `2.0` is the established target.
pub const DOUBLE_TAP_TARGET_ZOOM: f32 = 2.0;

/// Spacing between the image/loading area and the description text.
const DESCRIPTION_SPACING: f32 = 12.;
const LIGHTBOX_TEXT_SIZE_DELTA: f32 = 4.;

/// GH9729 t2-14: nearly-opaque black background colour for the scrim.
///
/// v1 shipped with alpha=230 (90% opacity), which let bright text in
/// the underlying view (new-tab page, terminal output) show through
/// and competed with the lightbox's description / dimensions text for
/// legibility. 250 (~98% opacity) suppresses bleed-through while
/// keeping a faint hint of the underlying surface for spatial
/// context. Set to 255 if a fully opaque modal is preferred.
fn scrim_color() -> ColorU {
    ColorU::new(0, 0, 0, 250)
}

/// The loading state of a lightbox image.
#[derive(Clone, Debug)]
pub enum LightboxImageSource {
    /// The image metadata is still being fetched.
    Loading,
    /// The image source has been resolved.
    /// Note: the actual image bytes may still be loading via the `AssetCache`.
    Resolved { asset_source: AssetSource },
    /// The image could not be loaded or decoded. The Lightbox renders a
    /// non-blocking error panel inline (see `Lightbox::render`) and dismissal
    /// (Escape, scrim click, ×) continues to work.
    ///
    /// `message` is intended to be one of a small set of sanitized,
    /// human-readable categorical strings. Underlying OS errors and absolute
    /// filesystem paths must NOT be interpolated here; log the original via
    /// `log::warn!` for the operator instead. See `specs/GH9729/tech.md` §182.
    Error { message: String },
}

/// A single image entry in the lightbox.
#[derive(Clone, Debug)]
pub struct LightboxImage {
    /// The loading/loaded state of this image.
    pub source: LightboxImageSource,
    /// Optional description displayed below the image.
    pub description: Option<String>,
}

/// Direction for navigating between images.
#[derive(Clone, Copy, Debug)]
pub enum NavigationDirection {
    Previous,
    Next,
}

/// A handler invoked when the user navigates between images.
pub type NavigateHandler = Arc<dyn Fn(NavigationDirection, &mut EventContext, &AppContext)>;

/// GH9729 (post-tier2): handler invoked when the user clicks a
/// thumbnail in the optional vertical rail. Receives the index of the
/// clicked image in `Params::images`.
pub type ThumbnailSelectHandler = Arc<dyn Fn(usize, &mut EventContext, &AppContext)>;

/// GH9729 (post-tier2): configuration for the optional vertical
/// thumbnail rail displayed on the left edge of the lightbox.
///
/// The rail renders one square thumbnail per `LightboxImage` in
/// `Params::images`, highlights `current_index`, and dispatches
/// `on_select` when the user clicks a thumbnail. The scroll position is
/// owned by the caller via `scroll_state` so it survives re-renders
/// (the rail is rebuilt every `render()`; per-element state would be
/// lost).
///
/// Sibling discovery (which files end up in `Params::images`) and
/// auto-scroll-to-current (callers invoke `scroll_state.scroll_to(..)`
/// after computing the right pixel offset) both live on the caller's
/// side — the component just renders what it's handed.
pub struct ThumbnailRail {
    pub on_select: ThumbnailSelectHandler,
    pub scroll_state: ClippedScrollStateHandle,
}

/// A lightbox component for displaying images in a full-window overlay.
///
/// The lightbox displays one or more images centered on screen with a semi-transparent scrim
/// background. It supports navigating between images via arrow buttons and can be dismissed by
/// clicking outside the image, clicking the close button, or pressing Escape.
#[derive(Default)]
pub struct Lightbox {
    close_button: button::Button,
    prev_button: button::Button,
    next_button: button::Button,
    /// GH9729 §698 / t2-12: zoom-out button rendered in the lightbox
    /// toolbar. Replaces the t2-11 keyboard bindings which didn't
    /// dispatch in a Warp terminal context.
    zoom_out_button: button::Button,
    /// GH9729 §698 / t2-12: zoom-in button.
    zoom_in_button: button::Button,
    /// GH9729 §698 / t2-12: zoom-reset (100%) button.
    zoom_reset_button: button::Button,
    /// GH9729 §698 / t2-20: shared drag-tracking state for
    /// `PanClippedImage`. Lives on the persistent `Lightbox` struct so
    /// it survives re-renders (the rendered tree is rebuilt every
    /// `render()`, including the `PanClippedImage`; transient state on
    /// the element itself would be lost on every `ctx.notify()`).
    /// Holds the cursor position from the most recent unhandled
    /// `LeftMouseDown` / `LeftMouseDragged`; `None` when not currently
    /// dragging.
    drag_state: Arc<Mutex<Option<Vector2F>>>,
}

/// GH9729 §698 / t2-12: direction for a single zoom step in the
/// lightbox. Mirrors `NavigationDirection` in shape.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ZoomDirection {
    In,
    Out,
    Reset,
}

/// GH9729 §698 / t2-12: handler invoked when the user requests a zoom
/// step via a toolbar button or scroll-wheel gesture.
pub type ZoomHandler = Arc<dyn Fn(ZoomDirection, &mut EventContext, &AppContext)>;

pub struct Params<'a> {
    /// The list of images to display.
    pub images: &'a [LightboxImage],

    /// The index of the currently displayed image.
    pub current_index: usize,

    /// Handler to invoke when the lightbox is dismissed.
    pub on_dismiss: DismissHandler,

    /// The native pixel dimensions of the currently displayed image, if known.
    /// When `Some`, the image is fully loaded and the lightbox renders it with a
    /// `ConstrainedBox` plus description. When `None`, the lightbox shows a loading
    /// indicator instead.
    pub current_image_native_size: Option<Vector2F>,

    /// GH9729 §697: timeline anchor for animated GIF/WebP frame progression.
    /// When `Some`, the rendered `Image` is wired with
    /// `enable_animation_with_start_time`, driving continuous playback via
    /// the implicit per-frame `ctx.repaint_after` loop in
    /// `Image::paint_animated_image`. When `None`, the image renders the
    /// first frame only (legacy behaviour, kept for callers that don't
    /// want animation, e.g. inert example/test surfaces).
    ///
    /// Static images ignore this field entirely.
    pub animation_start_time: Option<Instant>,

    /// GH9729 §699: optional pre-formatted metadata strip rendered
    /// below the description in a smaller, dimmer font (filename
    /// already lives in `description`, so the strip typically carries
    /// `"<width>×<height>"` plus format / size when the caller knows
    /// them). `None` hides the strip entirely. The string is rendered
    /// verbatim — the caller is responsible for sanitising and
    /// localising it.
    pub metadata_line: Option<String>,

    /// GH9729 §698 / t2-19: drag-to-pan offset (pixels) for the
    /// currently-displayed image. `Vector2F::zero()` for centered.
    /// Caller is responsible for resetting on zoom-to-native and on
    /// navigation. The lightbox component clamps the offset internally
    /// so the user can't drag the image past viewport center.
    pub pan_offset: Vector2F,

    /// GH9729 §698: zoom factor applied to the image's bounding box.
    /// `1.0` renders at native size (the v1 default). Values `> 1.0`
    /// scale the `ConstrainedBox` linearly so the image renders larger
    /// (the surrounding `Align` keeps it centred; oversized images
    /// overflow the centred stack — the lightbox scrim is full-window
    /// so they stay inside the window). `< 1.0` shrinks below native
    /// size for very-large images.
    ///
    /// The companion drag-to-pan deliverable from §698 is deferred —
    /// see `TIER2_TODO.md::t2-7-pan` — because this GPUI fork has no
    /// `Translate`/`Offset` primitive that lets us shift an element
    /// during paint without an upstream addition.
    pub zoom_factor: f32,

    /// GH9729 (post-tier2): optional vertical thumbnail rail rendered
    /// on the left edge. `None` matches v1 behaviour (no rail); `Some`
    /// renders the rail iff `images.len() > 1`. When the rail is
    /// active the existing prev/next arrow buttons are suppressed (the
    /// rail is the primary navigation surface in that mode); the
    /// keyboard arrow bindings registered by `LightboxView::init` keep
    /// working as a secondary affordance.
    pub thumbnail_rail: Option<ThumbnailRail>,

    /// Optional configuration for the lightbox.
    pub options: Options,
}

impl crate::Params for Params<'_> {
    type Options<'a> = Options;
}

/// A function that handles dismiss events.
pub type DismissHandler = Arc<dyn Fn(&mut EventContext, &AppContext)>;

pub struct Options {
    /// Optional keystroke associated with the dismiss action. This will be rendered alongside
    /// the dismiss button in the dialog, but the caller is responsible for adding a keybinding.
    pub dismiss_keystroke: Option<Keystroke>,

    /// Handler to invoke when the user navigates between images.
    /// If `None`, navigation buttons are not shown.
    pub on_navigate: Option<NavigateHandler>,

    /// GH9729 §698 / t2-12: handler invoked when the user clicks a
    /// zoom button or scrolls (with cmd held) over the image. If
    /// `None`, the zoom toolbar is not rendered and scroll-wheel
    /// events fall through.
    pub on_zoom: Option<ZoomHandler>,

    /// GH9729 §698 / t2-19: handler invoked when the user drags the
    /// image to pan. Called with the new desired pan offset (before
    /// clamping — caller can clamp or ignore as needed). If `None`,
    /// drag-to-pan is disabled and mouse-down on the image just stops
    /// scrim-dismiss as before.
    pub on_pan: Option<Arc<dyn Fn(Vector2F, &mut EventContext, &AppContext)>>,

    /// GH9729 §698 / t2-21: handler invoked when the user double-taps
    /// inside the image. Called with the tap position **relative to
    /// viewport center** (so the caller can compute a re-centering
    /// pan_offset against any zoom factor). If `None`, double-tap
    /// is ignored and the second click just resumes drag tracking.
    pub on_double_tap_zoom:
        Option<Arc<dyn Fn(Vector2F, &mut EventContext, &AppContext)>>,
}

impl crate::Options for Options {
    fn default(_appearance: &Appearance) -> Self {
        Self {
            dismiss_keystroke: None,
            on_navigate: None,
            on_zoom: None,
            on_pan: None,
            on_double_tap_zoom: None,
        }
    }
}

impl Component for Lightbox {
    type Params<'a> = Params<'a>;

    fn render<'a>(&self, appearance: &Appearance, params: Self::Params<'a>) -> Box<dyn Element> {
        let on_dismiss_for_button = params.on_dismiss.clone();
        let on_dismiss = params.on_dismiss;
        let image_count = params.images.len();
        let current_index = params.current_index;

        // Extract current image data via direct indexing.
        let current_image = params.images.get(current_index);
        let current_source = current_image.map(|img| &img.source);
        let current_description = current_image.and_then(|img| img.description.clone());
        let text_size = lightbox_text_size(appearance);

        // Close button in the top-right corner.
        let close_button = self.close_button.render(
            appearance,
            button::Params {
                content: button::Content::Icon(Icon::X),
                theme: &button::themes::Secondary,
                options: button::Options {
                    size: button::Size::Small,
                    on_click: Some(Box::new(move |ctx, app, _| {
                        on_dismiss_for_button(ctx, app);
                    })),
                    keystroke: params.options.dismiss_keystroke,
                    ..button::Options::default(appearance)
                },
            },
        );

        // Build the central content based on the image source and whether the
        // native size is known (i.e. the image data has been loaded).
        let central_content: Box<dyn Element> =
            match (current_source, params.current_image_native_size) {
                // Image source resolved AND native size known → render the image.
                (Some(LightboxImageSource::Resolved { asset_source }), Some(native_size)) => {
                    // GH9729 §697: opt into continuous animated playback when
                    // the caller supplied a timeline anchor.
                    let mut image_builder =
                        Image::new(asset_source.clone(), CacheOption::Original).contain();
                    if let Some(start) = params.animation_start_time {
                        image_builder = image_builder.enable_animation_with_start_time(start);
                    }
                    // GH9729 §698 / t2-19: feed the desired (zoom*native)
                    // size into PanClippedImage, which forces the child
                    // to render at exactly that size regardless of
                    // parent's available area. Clamp first so a poisoned
                    // float can't blow up the layout.
                    let zoom = params.zoom_factor.clamp(MIN_ZOOM_FACTOR, MAX_ZOOM_FACTOR);
                    let desired_size = vec2f(native_size.x() * zoom, native_size.y() * zoom);

                    let image_element = image_builder
                        .before_load(Align::new(loading_element(appearance)).finish())
                        .finish();

                    // PanClippedImage handles its own mouse-down (stops
                    // scrim dismiss), drag (pan), and cmd+scroll (zoom),
                    // so the previous `EventHandler` wrapping is no
                    // longer needed.
                    Box::new(PanClippedImage::new(
                        image_element,
                        desired_size,
                        params.pan_offset,
                        params.options.on_pan.clone(),
                        params.options.on_zoom.clone(),
                        params.options.on_double_tap_zoom.clone(),
                        self.drag_state.clone(),
                    ))
                }
                // Per-image load/decode failure: render a non-blocking error
                // panel with the filename (description) on one line and the
                // sanitized message on the next. Dismissal (Escape, scrim, ×)
                // continues to work; this panel never traps focus. See
                // specs/GH9729/tech.md §182.
                (Some(LightboxImageSource::Error { message }), _) => {
                    let mut column = Flex::column()
                        .with_cross_axis_alignment(CrossAxisAlignment::Center)
                        .with_spacing(DESCRIPTION_SPACING);
                    if let Some(description) = current_description.clone() {
                        column = column.with_child(
                            Text::new(description, appearance.ui_font_family(), text_size)
                                .with_color(ColorU::white())
                                .finish(),
                        );
                    }
                    column = column.with_child(
                        Text::new(message.clone(), appearance.ui_font_family(), text_size)
                            .with_color(ColorU::white())
                            .finish(),
                    );
                    column.finish()
                }
                // No images provided at all.
                _ if image_count == 0 => {
                    Text::new("No images", appearance.ui_font_family(), text_size)
                        .with_color(ColorU::white())
                        .finish()
                }
                // Still loading (either metadata or image bytes).
                _ => loading_element(appearance),
            };

        // Show the description only when the image is fully loaded (native size known).
        // GH9729 §699: also append the optional metadata strip (smaller,
        // dimmer text) under the description on the same gating, so the
        // footer never appears next to a half-loaded spinner.
        let content_with_description = if let (Some(description), Some(_)) =
            (current_description, params.current_image_native_size)
        {
            let description_text = Text::new(description, appearance.ui_font_family(), text_size)
                .with_color(ColorU::white())
                .finish();

            let mut column = Flex::column()
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_spacing(DESCRIPTION_SPACING)
                .with_child(Shrinkable::new(1.0, central_content).finish())
                .with_child(description_text);

            if let Some(metadata_line) = params.metadata_line {
                let metadata_text = Text::new(
                    metadata_line,
                    appearance.ui_font_family(),
                    metadata_text_size(appearance),
                )
                .with_color(metadata_text_color())
                .finish();
                column = column.with_child(metadata_text);
            }

            column.finish()
        } else {
            central_content
        };

        let centered_content = Align::new(content_with_description).finish();

        // GH9729 (post-tier2): when the rail is shown, the scrim is laid
        // out as a Flex::row [rail | padded image area]:
        //
        //   * the rail hugs the LEFT EDGE of the scrim with no padding
        //     before it (the standard sidebar-flush-left pattern from
        //     macOS Preview / Mail / Finder column view)
        //   * the right cell carries the full SCRIM_PADDING so the
        //     image stays nicely inset, the metadata strip doesn't
        //     touch the rail's divider, and the close button still
        //     anchors to its natural corner
        //
        // Without the rail, the layout is identical to v1 — uniform
        // SCRIM_PADDING and a centred image — so the artifacts /
        // screenshots Lightbox call sites are unchanged.
        let show_rail = params.thumbnail_rail.is_some() && image_count > 1;
        let (scrim_content, scrim_uniform_padding): (Box<dyn Element>, f32) =
            match (params.thumbnail_rail, show_rail) {
                (Some(rail), true) => {
                    let rail_element = build_thumbnail_rail(
                        appearance,
                        params.images,
                        current_index,
                        params.animation_start_time,
                        rail,
                    );
                    let image_cell = Container::new(centered_content)
                        .with_uniform_padding(SCRIM_PADDING)
                        .finish();
                    let row = Flex::row()
                        .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                        .with_child(rail_element)
                        .with_child(Shrinkable::new(1.0, image_cell).finish())
                        .finish();
                    // Rail provides its own internal padding; the scrim
                    // container needs ZERO outer padding so the rail
                    // can hug the viewport edge.
                    (row, 0.0)
                }
                _ => (centered_content, SCRIM_PADDING),
            };

        let scrim = Container::new(
            Dismiss::new(scrim_content)
                .prevent_interaction_with_other_elements()
                .on_dismiss(move |ctx, app| on_dismiss(ctx, app))
                .finish(),
        )
        .with_background_color(scrim_color())
        .with_uniform_padding(scrim_uniform_padding)
        .finish();

        // Stack the scrim, close button, and optional navigation arrows.
        let mut content = Stack::new().with_child(scrim);
        content.add_positioned_child(
            close_button,
            OffsetPositioning::offset_from_parent(
                vec2f(-SCRIM_BUTTON_INSET, SCRIM_BUTTON_INSET),
                ParentOffsetBounds::Unbounded,
                ParentAnchor::TopRight,
                ChildAnchor::TopRight,
            ),
        );

        // Navigation arrows (only shown when there are multiple images,
        // a navigate handler is supplied, AND the thumbnail rail is NOT
        // active — see GH9729 (post-tier2): the rail is the primary
        // navigation surface in that mode and the arrows would be
        // redundant).
        if image_count > 1
            && !show_rail
            && let Some(on_navigate) = params.options.on_navigate
        {
            // Previous button (hidden on first image).
            if current_index > 0 {
                let on_nav = on_navigate.clone();
                let prev_button = self.prev_button.render(
                    appearance,
                    button::Params {
                        content: button::Content::Icon(Icon::ChevronLeft),
                        theme: &button::themes::Secondary,
                        options: button::Options {
                            size: button::Size::Small,
                            on_click: Some(Box::new(move |ctx, app, _| {
                                on_nav(NavigationDirection::Previous, ctx, app);
                            })),
                            ..button::Options::default(appearance)
                        },
                    },
                );
                content.add_positioned_child(
                    prev_button,
                    OffsetPositioning::offset_from_parent(
                        vec2f(12., 0.),
                        ParentOffsetBounds::Unbounded,
                        ParentAnchor::MiddleLeft,
                        ChildAnchor::MiddleLeft,
                    ),
                );
            }

            // Next button (hidden on last image).
            if current_index < image_count - 1 {
                let on_nav = on_navigate;
                let next_button = self.next_button.render(
                    appearance,
                    button::Params {
                        content: button::Content::Icon(Icon::ChevronRight),
                        theme: &button::themes::Secondary,
                        options: button::Options {
                            size: button::Size::Small,
                            on_click: Some(Box::new(move |ctx, app, _| {
                                on_nav(NavigationDirection::Next, ctx, app);
                            })),
                            ..button::Options::default(appearance)
                        },
                    },
                );
                content.add_positioned_child(
                    next_button,
                    OffsetPositioning::offset_from_parent(
                        vec2f(-12., 0.),
                        ParentOffsetBounds::Unbounded,
                        ParentAnchor::MiddleRight,
                        ChildAnchor::MiddleRight,
                    ),
                );
            }
        }

        // GH9729 (post-tier2): zoom controls live in a single rounded
        // pill anchored to the bottom-CENTER of the scrim. Replaces
        // the previous bottom-LEFT cluster of separately-positioned
        // buttons (which competed visually with the rail and felt
        // chunky per UX review).
        //
        // The pill contains `[−]  [+]` and conditionally `[100%]`
        // when zoom != 1.0, all inside one Container with a soft
        // dark fill and rounded corners. Photos.app and Preview both
        // use a centered bottom toolbar for this kind of transient
        // control.
        //
        // The Flex::row inside the pill still partitions hit-test
        // space cleanly between its children, preserving the
        // overlap-resistance that the t2-16 refactor introduced.
        if let Some(on_zoom) = params.options.on_zoom {
            let zoom = params.zoom_factor;
            let on_zoom_out = on_zoom.clone();
            let zoom_out_button = self.zoom_out_button.render(
                appearance,
                button::Params {
                    content: button::Content::Icon(Icon::Minus),
                    theme: &button::themes::Secondary,
                    options: button::Options {
                        size: button::Size::Small,
                        on_click: Some(Box::new(move |ctx, app, _| {
                            on_zoom_out(ZoomDirection::Out, ctx, app);
                        })),
                        ..button::Options::default(appearance)
                    },
                },
            );
            let on_zoom_in = on_zoom.clone();
            let zoom_in_button = self.zoom_in_button.render(
                appearance,
                button::Params {
                    content: button::Content::Icon(Icon::Plus),
                    theme: &button::themes::Secondary,
                    options: button::Options {
                        size: button::Size::Small,
                        on_click: Some(Box::new(move |ctx, app, _| {
                            on_zoom_in(ZoomDirection::In, ctx, app);
                        })),
                        ..button::Options::default(appearance)
                    },
                },
            );

            // Build the Flex::row with [−][optional 100%][+]. The
            // reset button only appears when the image is NOT at
            // native zoom (the t2-15 "hide when no-op" decision).
            let mut pill_row = Flex::row()
                .with_spacing(ZOOM_PILL_BUTTON_GAP)
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_child(zoom_out_button);
            if zoom != 1.0 {
                let on_zoom_reset = on_zoom.clone();
                let zoom_reset_button = self.zoom_reset_button.render(
                    appearance,
                    button::Params {
                        content: button::Content::Label("100%".into()),
                        theme: &button::themes::Secondary,
                        options: button::Options {
                            size: button::Size::Small,
                            on_click: Some(Box::new(move |ctx, app, _| {
                                on_zoom_reset(ZoomDirection::Reset, ctx, app);
                            })),
                            ..button::Options::default(appearance)
                        },
                    },
                );
                pill_row = pill_row.with_child(zoom_reset_button);
            }
            pill_row = pill_row.with_child(zoom_in_button);

            let pill = Container::new(pill_row.finish())
                .with_background_color(zoom_pill_background_color())
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(
                    ZOOM_PILL_CORNER_RADIUS,
                )))
                .with_horizontal_padding(ZOOM_PILL_HORIZONTAL_PADDING)
                .with_vertical_padding(ZOOM_PILL_VERTICAL_PADDING)
                .finish();

            // GH9729 (post-tier2): when the rail is shown, the Stack's
            // bottom-middle anchor is the centre of the *workspace*,
            // not the centre of the *image area* (which starts
            // RAIL_WIDTH px to the right of the workspace's left
            // edge). Offset the pill right by half the rail width so
            // it sits centred under the actual image — the user
            // expects "the zoom control for THIS image" to live
            // under THIS image, not under the union of rail + image.
            let pill_x_offset = if show_rail { RAIL_WIDTH / 2. } else { 0. };
            content.add_positioned_child(
                pill,
                OffsetPositioning::offset_from_parent(
                    vec2f(pill_x_offset, -ZOOM_PILL_BOTTOM_INSET),
                    ParentOffsetBounds::Unbounded,
                    ParentAnchor::BottomMiddle,
                    ChildAnchor::BottomMiddle,
                ),
            );
        }

        content.finish()
    }
}

/// Builds the shared "Loading..." text element used in both the `Loading` state
/// and as the `before_load` fallback while the `AssetCache` fetches image bytes.
fn loading_element(appearance: &Appearance) -> Box<dyn Element> {
    Text::new(
        "Loading...",
        appearance.ui_font_family(),
        lightbox_text_size(appearance),
    )
    .with_color(ColorU::white())
    .finish()
}

/// GH9729 (post-tier2): construct the vertical thumbnail rail rendered
/// on the left of the lightbox. One square thumbnail per
/// `LightboxImage`; current entry gets a coloured highlight border; the
/// whole column is wrapped in a vertical `ClippedScrollable` so a
/// directory with many siblings scrolls naturally.
///
/// Click on a thumbnail dispatches `rail.on_select(index, …)`. The
/// thumbnail bytes share `AssetCache` with the main display, so opening
/// the rail does not double-decode the currently-shown image.
///
/// Memory caveat: each thumbnail asks the image cache for an
/// `Original`-quality decode (the cache currently has no thumbnail-
/// downsample variant). For very large sibling sets this can push
/// memory hard — sibling discovery on the caller's side caps the list
/// at a centered window for that reason. A proper thumbnail cache is
/// tracked under `tech.md` §706 "Disk-backed thumbnail cache".
fn build_thumbnail_rail(
    appearance: &Appearance,
    images: &[LightboxImage],
    current_index: usize,
    animation_start_time: Option<Instant>,
    rail: ThumbnailRail,
) -> Box<dyn Element> {
    let mut column = Flex::column()
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .with_spacing(RAIL_THUMB_SPACING);

    for (index, image) in images.iter().enumerate() {
        let thumbnail = build_thumbnail_cell(
            appearance,
            image,
            index == current_index,
            index,
            animation_start_time,
            rail.on_select.clone(),
        );
        column = column.with_child(thumbnail);
    }

    // Vertical breathing room inside the rail so the first and last
    // thumb don't touch the top/bottom edges of the scrim.
    let padded_column = Container::new(column.finish())
        .with_vertical_padding(RAIL_OUTER_PADDING_VERTICAL)
        .finish();

    // Wrap in a vertical scrollable so long sibling lists scroll
    // inside the rail rather than overflowing the scrim. The
    // caller-supplied scroll-state handle survives the per-render
    // rebuild of this element (rail is rebuilt every `render()`; any
    // per-element scroll state would reset on `ctx.notify()`).
    //
    // Scrollbar VISIBILITY: `ScrollbarWidth::None` hides the
    // scrollbar entirely. Three reasons:
    //   * The default `Auto` (8 px gutter on the right) made
    //     `CrossAxisAlignment::Center` centre the thumbnails in
    //     `RAIL_WIDTH − 8` instead of full `RAIL_WIDTH`, so they
    //     read as visually off-centre in the rail.
    //   * A native-look scrollbar competes with Warp's overall
    //     UI style.
    //   * Modern sidebars (macOS Photos, VSCode file tree, Finder
    //     column view) all hide the scrollbar by default — users
    //     scroll with trackpad / mouse wheel.
    //
    // Hiding the scrollbar does NOT disable scrolling — the
    // `ClippedScrollable` still consumes scroll events and the
    // `rail_scroll_state` handle still drives the position; only
    // the visual scrollbar widget is suppressed. Track / thumb fills
    // are set to `Fill::None` so they have no effect even if the
    // width were ever flipped back on.
    let scrollable = ClippedScrollable::vertical(
        rail.scroll_state,
        padded_column,
        ScrollbarWidth::None,
        Fill::None,
        Fill::None,
        Fill::None,
    )
    .finish();

    // The rail itself: a faintly-lighter background than the scrim
    // (so the panel reads as a distinct region without competing
    // with the image) plus a 1 px hairline on the right edge
    // separating rail from canvas — the macOS Mail / Finder
    // column-view convention.
    let panel = Container::new(scrollable)
        .with_background_color(rail_background_color())
        .with_border(Border {
            width: RAIL_DIVIDER_WIDTH,
            color: Fill::Solid(rail_divider_color()),
            top: false,
            left: false,
            bottom: false,
            right: true,
            dash: None,
        })
        .finish();

    // Fixed width — the rail isn't allowed to grow into the image
    // canvas, and the centred-content Flex cell to its right takes
    // all remaining space.
    ConstrainedBox::new(panel)
        .with_max_width(RAIL_WIDTH)
        .with_min_width(RAIL_WIDTH)
        .finish()
}

/// GH9729 (post-tier2): build one clickable thumbnail cell. The image
/// is rendered through the standard `Image::contain()` path so the
/// existing decode + size caps + animation behaviour apply.
///
/// When the caller supplies `animation_start_time` (i.e. when the
/// containing Lightbox has animated playback enabled for the main
/// image), the rail thumbnails also animate — using the SAME timeline
/// anchor as the main image so all animated siblings appear visually
/// synchronised on open, which makes the rail useful as a live
/// directory preview rather than a wall of arbitrary first frames.
///
/// Current-entry highlight (UX revision after first manual review):
/// a soft accent-blue **filled rounded-rect background** sitting 4 px
/// wider than the thumb on every side (the macOS Mail "pill behind
/// the selected row" convention), PLUS a 2 px accent-blue ring around
/// the thumb itself for a crisp definition edge. Non-current entries
/// reserve the same outer space at 0 alpha so layout doesn't shift
/// on selection change.
fn build_thumbnail_cell(
    appearance: &Appearance,
    image: &LightboxImage,
    is_current: bool,
    index: usize,
    animation_start_time: Option<Instant>,
    on_select: ThumbnailSelectHandler,
) -> Box<dyn Element> {
    // The thumbnail surface depends on the image source state. Loading /
    // error entries render as a placeholder rather than a broken-image
    // glyph; clicking still selects them so the user gets the same
    // main-area error or spinner they'd see by other means.
    let inner: Box<dyn Element> = match &image.source {
        LightboxImageSource::Resolved { asset_source } => {
            let mut builder = Image::new(asset_source.clone(), CacheOption::Original).contain();
            if let Some(start) = animation_start_time {
                builder = builder.enable_animation_with_start_time(start);
            }
            builder
                .before_load(Align::new(loading_element(appearance)).finish())
                .finish()
        }
        LightboxImageSource::Loading => Align::new(loading_element(appearance)).finish(),
        LightboxImageSource::Error { .. } => {
            // A tiny "!" placeholder keeps the rail visually tidy
            // without leaking the underlying error string into the rail
            // (the error is surfaced in the main image area when the
            // entry is selected).
            Align::new(
                Text::new(
                    "!".to_string(),
                    appearance.ui_font_family(),
                    lightbox_text_size(appearance),
                )
                .with_color(ColorU::new(255, 255, 255, 178))
                .finish(),
            )
            .finish()
        }
    };

    // 64x64 square for the thumb itself, with optional 2px accent ring
    // drawn AS the Container's border — traces the thumb's exact
    // edges (the pill below is intentionally wider).
    let ring = Border {
        width: RAIL_HIGHLIGHT_RING_WIDTH,
        color: Fill::Solid(if is_current {
            rail_accent_color()
        } else {
            ColorU::new(0, 0, 0, 0)
        }),
        top: true,
        left: true,
        bottom: true,
        right: true,
        dash: None,
    };
    let ringed_thumb = ConstrainedBox::new(Container::new(inner).with_border(ring).finish())
        .with_max_width(RAIL_THUMB_SIZE)
        .with_min_width(RAIL_THUMB_SIZE)
        .with_max_height(RAIL_THUMB_SIZE)
        .with_min_height(RAIL_THUMB_SIZE)
        .finish();

    // Soft "pill behind the thumb" highlight. Using `Container` with
    // `with_uniform_padding(RAIL_HIGHLIGHT_INSET)` is the deterministic
    // way to make the pill exactly `RAIL_HIGHLIGHT_INSET` px wider
    // than its child on every side — Container sizes to
    // `child + padding`, the background_color fills that bounding box,
    // and the rounded-corner mask is applied at that exact rect.
    //
    // (Earlier draft wrapped in `Align` + outer ConstrainedBox; that
    // pattern leaked the parent's available width through Align and
    // the pill rendered at the full rail width on selected entries.)
    //
    // Non-current entries reserve the same outer rect with a
    // transparent fill so the cell's bounding box never changes
    // between selection states — layout doesn't shift.
    let pill_fill = if is_current {
        rail_accent_fill_color()
    } else {
        ColorU::new(0, 0, 0, 0)
    };
    let pill = Container::new(ringed_thumb)
        .with_background_color(pill_fill)
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(
            RAIL_HIGHLIGHT_CORNER_RADIUS,
        )))
        .with_uniform_padding(RAIL_HIGHLIGHT_INSET)
        .finish();

    Box::new(
        EventHandler::new(pill).on_left_mouse_down(move |ctx, app, _position| {
            on_select(index, ctx, app);
            DispatchEventResult::StopPropagation
        }),
    )
}

/// GH9729 (post-tier2): solid dark surface for the rail panel.
///
/// Earlier draft used a translucent white tint (alpha 16) on top of
/// the scrim. Two problems with that: (1) the scrim itself is
/// intentionally 98 %-opaque (t2-14 chose alpha 250 for "spatial
/// context bleed"), so a ~6 % white overlay barely changed the net
/// opacity and the file-tree text behind the workspace bled
/// through the rail; (2) it didn't read as a panel — just a
/// slightly-lighter zone of the scrim. Switching to a fully opaque
/// near-black surface gives the rail a Material-Design "elevated
/// surface" feel: it sits ON the scrim instead of being part of it.
fn rail_background_color() -> ColorU {
    ColorU::new(24, 24, 24, 255)
}

/// GH9729 (post-tier2): 1 px hairline divider on the rail's right
/// edge. ~16 % opacity white — visible against the rail's dark
/// surface but not loud.
fn rail_divider_color() -> ColorU {
    ColorU::new(255, 255, 255, 40)
}

/// GH9729 (post-tier2): selection accent — macOS-system-blue-ish.
/// Used at full alpha for the 2 px ring around the current thumb.
fn rail_accent_color() -> ColorU {
    ColorU::new(80, 160, 255, 235)
}

/// GH9729 (post-tier2): selection accent at low alpha for the
/// rounded-rect fill BEHIND the current thumb. The mail-style
/// "pill behind the row" treatment.
fn rail_accent_fill_color() -> ColorU {
    ColorU::new(80, 160, 255, 60)
}

/// GH9729 (post-tier2): semi-transparent dark background for the
/// compact zoom pill at the bottom-center of the scrim. Subtly
/// darker than the rail panel so the pill reads as a focused
/// control surface, not part of the rail family.
fn zoom_pill_background_color() -> ColorU {
    ColorU::new(0, 0, 0, 140)
}

fn lightbox_text_size(appearance: &Appearance) -> f32 {
    appearance.ui_font_size() + LIGHTBOX_TEXT_SIZE_DELTA
}
