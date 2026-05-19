use std::sync::Arc;

use instant::Instant;
use pathfinder_geometry::vector::{Vector2F, vec2f};
use ui_components::{lightbox, Component as _};
use warpui::assets::asset_cache::{AssetCache, AssetSource, AssetState};
use warpui::image_cache::ImageType;
use warpui::keymap::{FixedBinding, Keystroke};
use warpui::prelude::*;
use warpui::elements::ClippedScrollStateHandle;
use warpui::units::Pixels;
use warpui::{AppContext, BlurContext, Element, Entity, SingletonEntity, View, ViewContext};

use crate::appearance::Appearance;

pub use lightbox::LightboxImage;

pub fn init(app: &mut AppContext) {
    use warpui::keymap::macros::*;
    let view_id = id!(LightboxView::ui_name());
    app.register_fixed_bindings([
        FixedBinding::new("escape", LightboxViewAction::Dismiss, view_id.clone()),
        FixedBinding::new(
            "left",
            LightboxViewAction::NavigatePrevious,
            view_id.clone(),
        ),
        FixedBinding::new("right", LightboxViewAction::NavigateNext, view_id),
        // GH9729 §698 / t2-12: zoom is GUI-only.
        //
        // t2-7 originally registered bare `=` / `-` / `0` keybindings;
        // t2-11 rebound them to `cmdorctrl-=` / `--` / `-0` after a
        // manual test surfaced that bare character keys never dispatch
        // (they route to the terminal stdin layer). The t2-11 reroute
        // ALSO failed in practice: while R1's theoretical analysis
        // claimed view-scope shadowing would beat the workspace-level
        // font-zoom binding, in reality pressing `cmd-=` with the
        // lightbox open zooms the terminal font behind the scrim
        // (likely because `LightboxView` doesn't actually claim
        // keyboard focus on open — escape/left/right work via a
        // different routing path that modifier-prefixed keys don't
        // take).
        //
        // Zoom now lives entirely in mouse-driven UI: three toolbar
        // buttons in `crates/ui_components/src/lightbox.rs` plus
        // cmd+scroll-wheel on the image. The action dispatch is
        // identical (`LightboxViewAction::ZoomIn`/`ZoomOut`/`ZoomReset`);
        // only the trigger surface changed.
    ]);
}

/// Parameters needed to open a lightbox.
#[derive(Clone, Debug)]
pub struct LightboxParams {
    /// The images to display in the lightbox.
    pub images: Vec<LightboxImage>,
    /// The index of the image to display initially.
    pub initial_index: usize,
}

/// Events emitted by the `LightboxView` to its parent.
pub enum LightboxViewEvent {
    /// The user explicitly dismissed the lightbox (Escape, close button, or scrim click).
    Close,
    /// Focus left the lightbox subtree (e.g. the user switched tabs).
    FocusLost,
}

impl Entity for LightboxView {
    type Event = LightboxViewEvent;
}

/// Actions dispatched within the `LightboxView`.
#[derive(Debug)]
pub enum LightboxViewAction {
    /// Dismiss the lightbox (triggered by clicking outside, close button, or Escape).
    Dismiss,
    /// Navigate to the previous image.
    NavigatePrevious,
    /// Navigate to the next image.
    NavigateNext,
    /// GH9729 §698: zoom the current image in by one step.
    ZoomIn,
    /// GH9729 §698: zoom the current image out by one step.
    ZoomOut,
    /// GH9729 §698: reset the current image to native size (`zoom_factor = 1.0`).
    ZoomReset,
    /// GH9729 §698 / t2-19: pan the currently-displayed image to a new
    /// pixel offset. Fields are floats (not Vector2F) so the action
    /// implements `Debug` cleanly without an extra impl on the wrapping
    /// type.
    Pan { offset_x: f32, offset_y: f32 },
    /// GH9729 §698 / t2-21: zoom in (or back to native) and re-center
    /// on the supplied tap location. `tap_offset_from_center_*` is the
    /// click position relative to viewport center. Combined into one
    /// action (vs separate ZoomIn + Pan) so the pan calculation runs
    /// against the same `zoom_factor` it's being applied with — splitting
    /// it would compute pan against the post-zoom value and miscenter.
    DoubleTapZoom {
        tap_offset_from_center_x: f32,
        tap_offset_from_center_y: f32,
    },
    /// GH9729 (post-tier2): jump to the image at `index` in the current
    /// `params.images` list. Dispatched by clicks on the optional
    /// vertical thumbnail rail. No-op if the index is the current one
    /// or out of bounds. Resets per-image state on success so the
    /// freshly-displayed image starts at native zoom / no pan / frame 0.
    SelectImage(usize),
}

/// A view that renders a full-window lightbox overlay.
pub struct LightboxView {
    params: LightboxParams,
    current_index: usize,
    lightbox: lightbox::Lightbox,
    /// GH9729 §697: timeline anchor for the currently-displayed image's
    /// animated GIF/WebP playback. Reset to `Instant::now()` whenever
    /// the displayed image changes (construction, params replacement,
    /// arrow-key navigation) so each entry's loop starts from frame 0.
    /// Static images ignore this; the Image element only consults
    /// `started_at` when there's an animated payload.
    animation_start_time: Instant,
    /// GH9729 §698: current zoom factor applied to the displayed image.
    /// `1.0` is native size; `>1` zooms in, `<1` shrinks. Always within
    /// `[lightbox::MIN_ZOOM_FACTOR, lightbox::MAX_ZOOM_FACTOR]` after
    /// any mutation. Reset to `1.0` whenever the displayed image
    /// changes (construction, params replacement, arrow-key navigation)
    /// so a freshly-shown image is never inherited at an unexpected
    /// zoom level.
    zoom_factor: f32,
    /// GH9729 §698 / t2-19: drag-to-pan offset for the currently-
    /// displayed image (pixels). Reset to `Vector2F::zero()` on every
    /// image change AND on every zoom action — at native zoom the
    /// image always fits viewport so pan would be meaningless. The
    /// lightbox component clamps the actual paint offset so the user
    /// can't drag the visible edge past viewport center.
    pan_offset: Vector2F,
    /// GH9729 (post-tier2): scroll position for the optional vertical
    /// thumbnail rail. Owned here on the persistent view so position
    /// survives the rail's per-`render()` rebuild (same lifetime
    /// reasoning as `drag_state` for `PanClippedImage`). Reset to top
    /// + scrolled to current on `update_params`; survives clicks
    /// inside the rail so the user's hand-driven scroll doesn't snap
    /// back when they pick a thumbnail near the bottom.
    rail_scroll_state: ClippedScrollStateHandle,
}

impl LightboxView {
    pub fn new(params: LightboxParams, ctx: &mut ViewContext<Self>) -> Self {
        let initial_index = params
            .initial_index
            .min(params.images.len().saturating_sub(1));
        let image_count = params.images.len();
        let rail_scroll_state = ClippedScrollStateHandle::new();
        let mut view = Self {
            params,
            current_index: initial_index,
            lightbox: lightbox::Lightbox::default(),
            animation_start_time: Instant::now(),
            zoom_factor: 1.0,
            pan_offset: Vector2F::zero(),
            rail_scroll_state,
        };
        view.start_asset_loads(ctx);
        // GH9729 (post-tier2): scroll the rail so the initially-selected
        // thumbnail is visible. We don't know the rail's viewport height
        // at this point so we approximate with a half-window of rows
        // above the current entry. This is a one-shot scroll on open;
        // after that the user's manual scrolling is respected.
        if image_count > 1 {
            view.rail_scroll_state
                .scroll_to(initial_rail_scroll_target(initial_index));
        }
        view
    }

    /// Replace the images and navigate to the given initial index.
    pub fn update_params(&mut self, params: LightboxParams, ctx: &mut ViewContext<Self>) {
        let initial_index = params
            .initial_index
            .min(params.images.len().saturating_sub(1));
        let image_count = params.images.len();
        self.params = params;
        self.current_index = initial_index;
        self.reset_per_image_state();
        self.start_asset_loads(ctx);
        // GH9729 (post-tier2): re-anchor the rail scroll on a fresh
        // params update (different image clicked while the lightbox
        // was already open). Same rationale as `new`.
        if image_count > 1 {
            self.rail_scroll_state
                .scroll_to(initial_rail_scroll_target(initial_index));
        }
    }

    /// GH9729 §697 + §698 + §698/t2-19: reset the per-image transient
    /// state (animation timeline anchor, zoom factor, pan offset) so
    /// the next render starts the image from frame 0 at native size,
    /// centered. Called from every site that changes which image is
    /// currently displayed.
    fn reset_per_image_state(&mut self) {
        self.animation_start_time = Instant::now();
        self.zoom_factor = 1.0;
        self.pan_offset = Vector2F::zero();
    }

    /// Update a single image at the given index without replacing the full list.
    pub fn update_image_at(
        &mut self,
        index: usize,
        image: LightboxImage,
        ctx: &mut ViewContext<Self>,
    ) {
        if index >= self.params.images.len() {
            return;
        }
        let asset_source = match &image.source {
            lightbox::LightboxImageSource::Resolved { asset_source } => Some(asset_source.clone()),
            _ => None,
        };
        self.params.images[index] = image;
        if let Some(asset_source) = asset_source {
            self.start_asset_load(index, asset_source, ctx);
        }
    }

    /// Kick off asset loads for all `Resolved` images and schedule re-renders.
    fn start_asset_loads(&mut self, ctx: &mut ViewContext<Self>) {
        // Collect first (immutable borrow) so the per-entry call below can
        // take `&mut self`.
        let to_load: Vec<(usize, AssetSource)> = self
            .params
            .images
            .iter()
            .enumerate()
            .filter_map(|(i, img)| match &img.source {
                lightbox::LightboxImageSource::Resolved { asset_source } => {
                    Some((i, asset_source.clone()))
                }
                _ => None,
            })
            .collect();
        for (index, asset_source) in to_load {
            self.start_asset_load(index, asset_source, ctx);
        }
    }

    /// Eagerly load a single asset and schedule a `ctx.notify()` when the
    /// fetch completes so the lightbox re-renders with the loaded image.
    ///
    /// `rewrite_image_for_load_state` is applied against the asset cache's
    /// returned state on both the synchronous and asynchronous code paths:
    ///
    /// * **Synchronous path** (GH9729 §695 + t2-10): a tiny mislabeled file
    ///   like a `.png` containing tarball bytes is so small the asset
    ///   cache can deliver `FailedToLoad` inline on the first
    ///   `load_asset` call — the §695 `Err("could not detect image
    ///   format")` from `ImageType::try_from_bytes` is reached before
    ///   any executor turn. The `when_loaded` future never fires for an
    ///   already-failed entry, so without the inline rewrite the
    ///   lightbox would render a permanent spinner. Apply the rewrite
    ///   immediately and skip the spawn.
    ///
    /// * **Asynchronous path**: the load is still pending; spawn a
    ///   callback that re-queries the cache once the future completes
    ///   and applies the same rewrite. The callback also calls
    ///   `ctx.notify()` so the lightbox repaints with the loaded image
    ///   (or the rewritten Error variant).
    ///
    /// See specs/GH9729/tech.md §182 and §695.
    fn start_asset_load(
        &mut self,
        index: usize,
        asset_source: AssetSource,
        ctx: &mut ViewContext<Self>,
    ) {
        let asset_cache = AssetCache::as_ref(ctx);
        let state = asset_cache.load_asset::<ImageType>(asset_source.clone());

        // GH9729 §695 / t2-10: apply the rewrite inline for any state
        // the asset cache resolves synchronously. `rewrite_image_for_load_state`
        // returns `None` for `Loading` and (post-§695) `Loaded`, so this
        // is a no-op for the common path — it only fires for
        // synchronously-`FailedToLoad`, which is the case the
        // `Loading`-only spawn below cannot rescue.
        if apply_rewrite_to_slot(&mut self.params.images, index, &state) {
            return;
        }

        if let AssetState::Loading { handle } = state {
            if let Some(future) = handle.when_loaded(asset_cache) {
                ctx.spawn(future, move |me, (), ctx| {
                    ctx.notify();
                    let asset_cache = AssetCache::as_ref(ctx);
                    let state = asset_cache.load_asset::<ImageType>(asset_source.clone());
                    apply_rewrite_to_slot(&mut me.params.images, index, &state);
                });
            }
        }
    }
}

/// GH9729 §698 / t2-21: compute the `(zoom_factor, pan_offset)` to
/// apply when the user double-taps on the image.
///
/// **Toggle behaviour** — macOS Preview / iOS Photos convention:
///   * If currently zoomed in (`zoom > 1.0`): zoom back to `1.0` and
///     reset pan to centred. Tap position is irrelevant.
///   * Otherwise: zoom to `DOUBLE_TAP_TARGET_ZOOM` (2.0×) and shift
///     the pan so the tapped point lands at viewport centre.
///
/// **Centering math** — for the tap at viewport-center-relative
/// offset `tap`, the image-coordinate of the tapped point is
/// `(tap - pan_old) / s_old`. After zooming to `s_new`, the tapped
/// point's screen offset from the new image centre is
/// `(tap - pan_old) * (s_new / s_old)`. To land at viewport centre,
/// the image centre must be at `-((tap - pan_old) * (s_new / s_old))`
/// from viewport centre:
///
/// ```text
///     pan_new = -tap * (s_new / s_old) + pan_old * (s_new / s_old)
/// ```
///
/// Visible image pan is clamped by `lightbox::PanClippedImage` at
/// paint time; this helper returns the unclamped value so the
/// stored model state stays accurate for subsequent drag deltas.
fn double_tap_zoom_target(
    zoom_old: f32,
    pan_old: pathfinder_geometry::vector::Vector2F,
    tap_offset_from_center: pathfinder_geometry::vector::Vector2F,
) -> (f32, pathfinder_geometry::vector::Vector2F) {
    use pathfinder_geometry::vector::Vector2F;
    if !zoom_old.is_finite() || zoom_old <= 0.0 {
        return (1.0, Vector2F::zero());
    }
    // Toggle: any non-native zoom returns to native; native zooms in.
    if zoom_old > 1.0 {
        return (1.0, Vector2F::zero());
    }
    let zoom_new = lightbox::DOUBLE_TAP_TARGET_ZOOM;
    let scale_ratio = zoom_new / zoom_old;
    let pan_new = -tap_offset_from_center * scale_ratio + pan_old * scale_ratio;
    (zoom_new, pan_new)
}

/// GH9729 §699 + t2-11: format the lightbox status-footer string for an
/// image with the given intrinsic size and current zoom factor.
///
/// At zoom 1.0 returns `"<W> × <H> px"`. At any other (clamped) zoom
/// returns `"<W> × <H> px · <Z>%"` so the user sees zoom-level feedback
/// even when the visible image size is capped by the parent constraint
/// (the t2-7-r1 gotcha for large images).
///
/// Non-finite `zoom_factor` collapses to 100% so a NaN-poisoned input
/// doesn't surface in the user-visible string; the view's `step_zoom`
/// already guards against this at the input edge but the renderer-side
/// public `Params::zoom_factor` is technically NaN-poisonable from
/// external callers, so defend here too.
fn format_metadata_line(size: pathfinder_geometry::vector::Vector2F, zoom_factor: f32) -> String {
    let w = size.x().round() as i32;
    let h = size.y().round() as i32;
    let zoom_pct = if zoom_factor.is_finite() {
        (zoom_factor * 100.0).round() as i32
    } else {
        100
    };
    if zoom_pct == 100 {
        format!("{w} × {h} px")
    } else {
        format!("{w} × {h} px · {zoom_pct}%")
    }
}

/// GH9729 §695 / t2-10: apply `rewrite_image_for_load_state` to the
/// image at `index`, replacing its `source` if a rewrite is warranted.
/// Returns `true` when the slot was mutated, `false` otherwise. Out-of-
/// bounds indices are tolerated (return `false`) so callers don't have
/// to pre-check the slice length.
fn apply_rewrite_to_slot(
    images: &mut [LightboxImage],
    index: usize,
    state: &AssetState<ImageType>,
) -> bool {
    let Some(new_source) = rewrite_image_for_load_state(state) else {
        return false;
    };
    let Some(slot) = images.get_mut(index) else {
        return false;
    };
    slot.source = new_source;
    true
}

/// GH9729 §698: direction for a single zoom-step keystroke.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ZoomDirection {
    In,
    Out,
}

/// GH9729 §698: compute the next zoom factor for a `ZoomIn` or `ZoomOut`
/// keystroke. Multiplies (or divides) `current` by `lightbox::ZOOM_STEP`
/// and clamps the result to
/// `[lightbox::MIN_ZOOM_FACTOR, lightbox::MAX_ZOOM_FACTOR]`. A non-finite
/// input collapses to `1.0` (the safe default) so a poisoned float
/// cannot escape into the `ConstrainedBox` size.
fn step_zoom(current: f32, direction: ZoomDirection) -> f32 {
    if !current.is_finite() {
        return 1.0;
    }
    let raw = match direction {
        ZoomDirection::In => current * lightbox::ZOOM_STEP,
        ZoomDirection::Out => current / lightbox::ZOOM_STEP,
    };
    raw.clamp(lightbox::MIN_ZOOM_FACTOR, lightbox::MAX_ZOOM_FACTOR)
}

/// Sanitize a per-asset-cache load error into a small set of categorical
/// strings that never interpolate raw OS errors or filesystem paths. The
/// underlying error is logged via `log::warn!` for the operator.
/// See specs/GH9729/tech.md §182 and §695.
fn sanitize_load_error(err: &anyhow::Error) -> &'static str {
    let s = format!("{err}").to_lowercase();
    if s.contains("too large") || s.contains("exceeds") {
        "image is too large to preview"
    } else if s.contains("could not detect") {
        // GH9729 §695: `ImageType::try_from_bytes` returns this exact
        // string for unrecognized-format bytes (e.g. a `.png` containing
        // tarball bytes). Preserve the user-visible "detect" wording
        // rather than collapsing it into the generic "decode" bucket.
        "could not detect image format"
    } else if s.contains("decode") || s.contains("format") {
        "could not decode image"
    } else {
        "could not read image"
    }
}

/// Inspect an `AssetState` for an image entry and decide whether the
/// `LightboxImage::source` should be rewritten to the `Error` variant.
/// Returns `Some(new_source)` if a rewrite is warranted, or `None` to
/// leave the entry unchanged.
///
/// Per `tech.md` §182 + §695, only `FailedToLoad` triggers a rewrite:
///   * After the §695 refactor, every `Loaded { data }` carries a
///     successfully decoded image (`Svg` / `StaticBitmap` /
///     `AnimatedBitmap`). Mislabeled-or-unsupported bytes now surface as
///     `try_from_bytes` returning `Err`, which the asset cache stores as
///     `FailedToLoad` — handled here by `sanitize_load_error`.
fn rewrite_image_for_load_state(
    state: &AssetState<ImageType>,
) -> Option<lightbox::LightboxImageSource> {
    match state {
        AssetState::FailedToLoad(err) => {
            log::warn!("GH9729: image preview load failed: {}", err);
            Some(lightbox::LightboxImageSource::Error {
                message: sanitize_load_error(err).to_string(),
            })
        }
        _ => None,
    }
}

impl View for LightboxView {
    fn ui_name() -> &'static str {
        "LightboxView"
    }

    fn on_blur(&mut self, _blur_ctx: &BlurContext, ctx: &mut ViewContext<Self>) {
        // Only dismiss if focus has left the entire lightbox subtree.
        if !ctx.is_self_or_child_focused() {
            ctx.emit(LightboxViewEvent::FocusLost);
        }
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);

        // Determine the native pixel size of the current image by querying the
        // asset cache. This will be `Some` once the image bytes have been fully
        // loaded and decoded.
        let current_image_native_size =
            self.params
                .images
                .get(self.current_index)
                .and_then(|img| match &img.source {
                    lightbox::LightboxImageSource::Resolved { asset_source } => {
                        let asset_cache = AssetCache::as_ref(app);
                        match asset_cache.load_asset::<ImageType>(asset_source.clone()) {
                            AssetState::Loaded { data } => data
                                .image_size()
                                .map(|size| Vector2F::new(size.x() as f32, size.y() as f32)),
                            _ => None,
                        }
                    }
                    lightbox::LightboxImageSource::Loading => None,
                    // Error entries have no native size; the existing render
                    // logic tolerates `None` here. See specs/GH9729/tech.md §182.
                    lightbox::LightboxImageSource::Error { .. } => None,
                });

        self.lightbox.render(
            appearance,
            lightbox::Params {
                images: &self.params.images,
                current_index: self.current_index,
                on_dismiss: Arc::new(|ctx, _| {
                    ctx.dispatch_typed_action(LightboxViewAction::Dismiss);
                }),
                current_image_native_size,
                animation_start_time: Some(self.animation_start_time),
                zoom_factor: self.zoom_factor,
                pan_offset: self.pan_offset,
                // GH9729 §699 + t2-11: surface the loaded image's intrinsic
                // dimensions plus the current zoom percentage when zoom is
                // not 1.0 (so the user gets visual feedback even when the
                // image is window-capped and a zoom keystroke produces no
                // visible size change — the t2-7-r1 gotcha). Format string
                // and file size are deferred — see `TIER2_TODO::t2-8-r2`.
                metadata_line: current_image_native_size
                    .map(|size| format_metadata_line(size, self.zoom_factor)),
                // GH9729 (post-tier2): expose the vertical thumbnail
                // rail when there's more than one image to navigate.
                // Click on a thumbnail dispatches `SelectImage(idx)`,
                // which handles the model update and per-image reset.
                // For single-image lightboxes (the artifacts/screenshots
                // call sites, and file-tree clicks where the directory
                // has exactly one supported image) the rail is `None`
                // and the existing centered-image layout is unchanged.
                thumbnail_rail: if self.params.images.len() > 1 {
                    Some(lightbox::ThumbnailRail {
                        on_select: Arc::new(|index, ctx, _| {
                            ctx.dispatch_typed_action(LightboxViewAction::SelectImage(index));
                        }),
                        scroll_state: self.rail_scroll_state.clone(),
                    })
                } else {
                    None
                },
                options: lightbox::Options {
                    dismiss_keystroke: Keystroke::parse("escape").ok(),
                    on_navigate: Some(Arc::new(|direction, ctx, _| match direction {
                        lightbox::NavigationDirection::Previous => {
                            ctx.dispatch_typed_action(LightboxViewAction::NavigatePrevious);
                        }
                        lightbox::NavigationDirection::Next => {
                            ctx.dispatch_typed_action(LightboxViewAction::NavigateNext);
                        }
                    })),
                    // GH9729 §698 / t2-12: route toolbar button clicks
                    // and cmd+scroll-wheel events from the Lightbox
                    // component back into our action dispatch.
                    on_zoom: Some(Arc::new(|direction, ctx, _| match direction {
                        lightbox::ZoomDirection::In => {
                            ctx.dispatch_typed_action(LightboxViewAction::ZoomIn);
                        }
                        lightbox::ZoomDirection::Out => {
                            ctx.dispatch_typed_action(LightboxViewAction::ZoomOut);
                        }
                        lightbox::ZoomDirection::Reset => {
                            ctx.dispatch_typed_action(LightboxViewAction::ZoomReset);
                        }
                    })),
                    // GH9729 §698 / t2-19: route the lightbox's pan
                    // gesture (drag inside viewport) into a typed
                    // action so the view owns the canonical offset.
                    on_pan: Some(Arc::new(|new_offset, ctx, _| {
                        ctx.dispatch_typed_action(LightboxViewAction::Pan {
                            offset_x: new_offset.x(),
                            offset_y: new_offset.y(),
                        });
                    })),
                    // GH9729 §698 / t2-21: route double-tap-on-image
                    // through the same action dispatch so the zoom +
                    // pan transition is atomic at the model layer.
                    on_double_tap_zoom: Some(Arc::new(|tap_offset, ctx, _| {
                        ctx.dispatch_typed_action(LightboxViewAction::DoubleTapZoom {
                            tap_offset_from_center_x: tap_offset.x(),
                            tap_offset_from_center_y: tap_offset.y(),
                        });
                    })),
                },
            },
        )
    }
}

impl TypedActionView for LightboxView {
    type Action = LightboxViewAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            LightboxViewAction::Dismiss => {
                ctx.emit(LightboxViewEvent::Close);
            }
            LightboxViewAction::NavigatePrevious => {
                if self.current_index > 0 {
                    self.current_index -= 1;
                    self.reset_per_image_state();
                    ctx.notify();
                }
            }
            LightboxViewAction::NavigateNext => {
                if self.current_index + 1 < self.params.images.len() {
                    self.current_index += 1;
                    self.reset_per_image_state();
                    ctx.notify();
                }
            }
            LightboxViewAction::ZoomIn => {
                let next = step_zoom(self.zoom_factor, ZoomDirection::In);
                if next != self.zoom_factor {
                    self.zoom_factor = next;
                    ctx.notify();
                }
            }
            LightboxViewAction::ZoomOut => {
                let next = step_zoom(self.zoom_factor, ZoomDirection::Out);
                if next != self.zoom_factor {
                    self.zoom_factor = next;
                    ctx.notify();
                }
            }
            LightboxViewAction::ZoomReset => {
                if self.zoom_factor != 1.0 {
                    self.zoom_factor = 1.0;
                    // GH9729 §698 / t2-19: dropping back to native
                    // zoom invalidates any pan offset — at 1.0 the
                    // image fits viewport and pan is meaningless.
                    self.pan_offset = Vector2F::zero();
                    ctx.notify();
                }
            }
            LightboxViewAction::Pan { offset_x, offset_y } => {
                // GH9729 §698 / t2-19: the Lightbox component clamps
                // the visible paint offset, but we still want the
                // model state to mirror the clamped value so
                // subsequent drag deltas accumulate sanely. NaN-
                // sanitise defensively — the on_pan handler receives
                // arbitrary float input from mouse-position math.
                let next = vec2f(*offset_x, *offset_y);
                if next.x().is_finite() && next.y().is_finite() && next != self.pan_offset {
                    self.pan_offset = next;
                    ctx.notify();
                }
            }
            LightboxViewAction::DoubleTapZoom {
                tap_offset_from_center_x,
                tap_offset_from_center_y,
            } => {
                let tap = vec2f(*tap_offset_from_center_x, *tap_offset_from_center_y);
                if !tap.x().is_finite() || !tap.y().is_finite() {
                    return;
                }
                let (next_zoom, next_pan) =
                    double_tap_zoom_target(self.zoom_factor, self.pan_offset, tap);
                if next_zoom != self.zoom_factor || next_pan != self.pan_offset {
                    self.zoom_factor = next_zoom;
                    self.pan_offset = next_pan;
                    ctx.notify();
                }
            }
            LightboxViewAction::SelectImage(index) => {
                if *index < self.params.images.len() && *index != self.current_index {
                    self.current_index = *index;
                    self.reset_per_image_state();
                    // Note: deliberately do NOT call
                    // `rail_scroll_state.scroll_to(...)` here. The user
                    // just chose a thumbnail that was already visible
                    // by definition (they clicked on it). Re-anchoring
                    // would yank the rail under their cursor.
                    ctx.notify();
                }
            }
        }
    }
}

/// GH9729 (post-tier2): compute the initial vertical scroll offset (in
/// pixels) for the thumbnail rail so the entry at `index` appears
/// roughly centered in the rail viewport.
///
/// Because we don't know the rail viewport height at construction time,
/// we approximate "centered" as ~3 rows above the target — large enough
/// to give spatial context, small enough that it doesn't over-scroll
/// past the top when the current entry sits near the start of a short
/// list. The scrollable element clamps to `[0, max_scroll]` so passing
/// a negative-ish value here lands at top (which is what we want for
/// the first few entries anyway).
fn initial_rail_scroll_target(index: usize) -> Pixels {
    // 3 rows of headroom. RAIL_ROW_PITCH lives on the component so the
    // view side and the renderer agree on row geometry.
    const HEADROOM_ROWS: f32 = 3.0;
    let offset = (index as f32 - HEADROOM_ROWS).max(0.0) * lightbox::RAIL_ROW_PITCH;
    Pixels::new(offset)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::rc::Rc;

    #[test]
    fn post_load_callback_rewrites_failed_to_load_to_error() {
        // Simulate the asset cache reporting a load failure for an image
        // entry. The rewrite helper must produce an Error variant whose
        // message is one of the sanitized categorical strings, never the
        // raw error string.
        let err = anyhow::anyhow!(
            "io error: failed to read /home/secret/path/to/image.png: permission denied"
        );
        let state: AssetState<ImageType> = AssetState::FailedToLoad(Rc::new(err));
        let rewritten = rewrite_image_for_load_state(&state).expect("FailedToLoad must rewrite");
        match rewritten {
            lightbox::LightboxImageSource::Error { message } => {
                assert_eq!(
                    message, "could not read image",
                    "expected sanitized 'could not read image' for io/permission errors",
                );
                assert!(
                    !message.contains("/home/"),
                    "sanitized message must not leak the absolute path",
                );
                assert!(
                    !message.contains("permission denied"),
                    "sanitized message must not leak the OS error string",
                );
            }
            other => panic!("expected Error variant, got {other:?}"),
        }
    }

    #[test]
    fn post_load_callback_rewrites_unrecognized_to_error() {
        // GH9729 §695: mislabeled / unsupported bytes (e.g. a `.png`
        // containing tarball bytes) now surface as `try_from_bytes`
        // returning `Err("could not detect image format")`, which the
        // asset cache stores as `FailedToLoad`. The rewrite helper must
        // surface the specific "detect" category, not collapse it into
        // the generic "could not decode image" bucket — `sanitize_load_error`
        // matches the "could not detect" prefix before the generic
        // "decode/format" branch.
        let err = anyhow::anyhow!("could not detect image format");
        let state: AssetState<ImageType> = AssetState::FailedToLoad(Rc::new(err));
        let rewritten = rewrite_image_for_load_state(&state)
            .expect("FailedToLoad on unrecognized format must rewrite");
        match rewritten {
            lightbox::LightboxImageSource::Error { message } => {
                assert_eq!(message, "could not detect image format");
            }
            other => panic!("expected Error variant, got {other:?}"),
        }
    }

    #[test]
    fn sanitize_load_error_picks_too_large_category() {
        let err = anyhow::anyhow!("local asset exceeds size cap");
        assert_eq!(
            super::sanitize_load_error(&err),
            "image is too large to preview"
        );
    }

    #[test]
    fn step_zoom_in_multiplies_by_step() {
        let result = super::step_zoom(1.0, super::ZoomDirection::In);
        assert!((result - lightbox::ZOOM_STEP).abs() < f32::EPSILON);
    }

    #[test]
    fn step_zoom_out_divides_by_step() {
        let result = super::step_zoom(1.0, super::ZoomDirection::Out);
        assert!((result - 1.0 / lightbox::ZOOM_STEP).abs() < f32::EPSILON);
    }

    #[test]
    fn step_zoom_in_clamps_to_max() {
        // GH9729 §698: zooming in repeatedly must saturate, not run away.
        let mut z = 1.0;
        for _ in 0..50 {
            z = super::step_zoom(z, super::ZoomDirection::In);
        }
        assert_eq!(z, lightbox::MAX_ZOOM_FACTOR);
    }

    #[test]
    fn step_zoom_out_clamps_to_min() {
        let mut z = 1.0;
        for _ in 0..50 {
            z = super::step_zoom(z, super::ZoomDirection::Out);
        }
        assert_eq!(z, lightbox::MIN_ZOOM_FACTOR);
    }

    #[test]
    fn double_tap_zoom_from_native_targets_2x_and_centers_on_tap() {
        // GH9729 t2-21: at native zoom, double-tap should zoom to
        // DOUBLE_TAP_TARGET_ZOOM (2.0) and re-center on the tap.
        // Tap 100 px left of viewport center → image must shift +200
        // (rightward) so the tapped point lands at viewport center.
        let (zoom, pan) = super::double_tap_zoom_target(
            1.0,
            pathfinder_geometry::vector::Vector2F::zero(),
            pathfinder_geometry::vector::vec2f(-100.0, 0.0),
        );
        assert_eq!(zoom, lightbox::DOUBLE_TAP_TARGET_ZOOM);
        assert!((pan.x() - 200.0).abs() < f32::EPSILON);
        assert!(pan.y().abs() < f32::EPSILON);
    }

    #[test]
    fn double_tap_zoom_returns_to_native_when_zoomed_in() {
        // Toggle behaviour: any zoom > 1.0 → back to 1.0 with pan
        // reset to zero, tap location ignored (at native zoom, pan is
        // meaningless because image fits viewport).
        let (zoom, pan) = super::double_tap_zoom_target(
            3.0,
            pathfinder_geometry::vector::vec2f(50.0, 75.0),
            pathfinder_geometry::vector::vec2f(123.0, 456.0),
        );
        assert_eq!(zoom, 1.0);
        assert_eq!(pan, pathfinder_geometry::vector::Vector2F::zero());
    }

    #[test]
    fn double_tap_zoom_preserves_existing_pan_under_scale() {
        // Already at 0.5x with pan (40, -20). Double-tap from a
        // non-native shrunk state should zoom to TARGET (2.0) and
        // apply the centering math against the OLD zoom.
        let (zoom, pan) = super::double_tap_zoom_target(
            0.5,
            pathfinder_geometry::vector::vec2f(40.0, -20.0),
            pathfinder_geometry::vector::vec2f(10.0, 10.0),
        );
        assert_eq!(zoom, lightbox::DOUBLE_TAP_TARGET_ZOOM);
        // scale_ratio = 2.0 / 0.5 = 4.0.
        // pan_new = -tap * 4 + pan_old * 4 = (-10, -10)*4 + (40, -20)*4
        //         = (-40, -40) + (160, -80) = (120, -120).
        assert!((pan.x() - 120.0).abs() < f32::EPSILON);
        assert!((pan.y() - (-120.0)).abs() < f32::EPSILON);
    }

    #[test]
    fn double_tap_zoom_sanitises_non_positive_zoom() {
        // Defensive: pathological zoom_old (≤ 0 or non-finite) must
        // collapse to the safe default (native, no pan) rather than
        // dividing by zero or propagating NaN into pan_offset.
        for bad in [0.0f32, -1.0, f32::NAN, f32::INFINITY] {
            let (zoom, pan) = super::double_tap_zoom_target(
                bad,
                pathfinder_geometry::vector::Vector2F::zero(),
                pathfinder_geometry::vector::vec2f(50.0, 50.0),
            );
            assert_eq!(zoom, 1.0, "bad zoom_old={bad} should reset to 1.0");
            assert_eq!(pan, pathfinder_geometry::vector::Vector2F::zero());
        }
    }

    #[test]
    fn format_metadata_line_at_native_zoom_omits_percentage() {
        // GH9729 t2-11: the percentage suffix is a visual signal that
        // zoom has changed. At 1.0 (native) the footer reads exactly
        // like pre-t2-11 so the common case stays uncluttered.
        let size = pathfinder_geometry::vector::Vector2F::new(1024., 768.);
        assert_eq!(super::format_metadata_line(size, 1.0), "1024 × 768 px");
    }

    #[test]
    fn format_metadata_line_appends_zoom_percentage_when_not_native() {
        // GH9729 t2-11 / t2-7-r1: when the image is window-capped the
        // zoom keystroke produces no visible size change, so the
        // footer is the only signal that the action fired. Verify it
        // formats as a percentage with no decimals.
        let size = pathfinder_geometry::vector::Vector2F::new(200., 200.);
        assert_eq!(super::format_metadata_line(size, 1.5), "200 × 200 px · 150%");
        assert_eq!(super::format_metadata_line(size, 2.0), "200 × 200 px · 200%");
        assert_eq!(super::format_metadata_line(size, 0.5), "200 × 200 px · 50%");
    }

    #[test]
    fn format_metadata_line_rounds_zoom_to_integer() {
        // ZOOM_STEP = 1.5 → after one zoom-in from 1.0 the factor is
        // 1.5 exactly, but accumulated multiplications produce
        // irrational-looking values (1.5 * 1.5 = 2.25, so "225%").
        // Confirm rounding behaves at a boundary.
        let size = pathfinder_geometry::vector::Vector2F::new(100., 100.);
        // 0.6667 * 100 = 66.67 → rounds to 67.
        assert_eq!(
            super::format_metadata_line(size, 1.0 / 1.5),
            "100 × 100 px · 67%"
        );
    }

    #[test]
    fn format_metadata_line_sanitises_non_finite_zoom() {
        // A NaN-poisoned `Params::zoom_factor` from an external caller
        // must not produce "NaN%" in the user-visible footer. Match
        // the same posture as the renderer-side clamp guard.
        let size = pathfinder_geometry::vector::Vector2F::new(100., 100.);
        assert_eq!(
            super::format_metadata_line(size, f32::NAN),
            "100 × 100 px",
            "NaN must collapse to native (100%) and drop the suffix"
        );
        assert_eq!(
            super::format_metadata_line(size, f32::INFINITY),
            "100 × 100 px",
        );
    }

    #[test]
    fn format_metadata_line_rounds_fractional_dimensions() {
        // GH9729 t2-8-r2 / t2-11: SVGs can have fractional intrinsic
        // sizes. Use `.round()` rather than truncating-toward-zero
        // `as i32` cast so e.g. 199.7px doesn't render as "199 px".
        let size = pathfinder_geometry::vector::Vector2F::new(199.7, 200.3);
        assert_eq!(super::format_metadata_line(size, 1.0), "200 × 200 px");
    }

    #[test]
    fn apply_rewrite_to_slot_rewrites_synchronous_failed_to_load() {
        // GH9729 t2-10: a synchronously-resolved `FailedToLoad` (reachable
        // for tiny mislabeled files after t2-4 made `try_from_bytes`
        // return `Err` immediately) must be rewritten to the `Error`
        // variant inline — the `when_loaded` future never fires for an
        // already-failed entry, so without this branch the lightbox
        // would render a permanent spinner.
        let mut images = vec![LightboxImage {
            source: lightbox::LightboxImageSource::Resolved {
                asset_source: warpui::assets::asset_cache::AssetSource::Bundled {
                    path: "fake/bundled/path.png",
                },
            },
            description: Some("filename.png".to_string()),
        }];
        let state: AssetState<ImageType> = AssetState::FailedToLoad(Rc::new(
            anyhow::anyhow!("could not detect image format"),
        ));

        let mutated = super::apply_rewrite_to_slot(&mut images, 0, &state);

        assert!(mutated, "synchronous FailedToLoad must mutate the slot");
        match &images[0].source {
            lightbox::LightboxImageSource::Error { message } => {
                assert_eq!(message, "could not detect image format");
            }
            other => panic!("expected Error variant, got {other:?}"),
        }
    }

    #[test]
    fn apply_rewrite_to_slot_leaves_loading_state_alone() {
        // The asynchronous-loading path must NOT trigger an inline
        // rewrite — the spawn callback will run when the load completes
        // and that's what's responsible for the post-load state. This
        // test pins the contract so a future tweak to
        // `rewrite_image_for_load_state` (e.g. adding a `Loading` arm)
        // can't silently break the lightbox by mutating the slot
        // mid-load.
        let mut images = vec![LightboxImage {
            source: lightbox::LightboxImageSource::Resolved {
                asset_source: warpui::assets::asset_cache::AssetSource::Bundled {
                    path: "fake/bundled/path.png",
                },
            },
            description: None,
        }];
        // `AssetState::Evicted` is the cheapest non-Loading,
        // non-FailedToLoad variant to construct in a unit test — it
        // exercises the same "no rewrite" branch as `Loading`, which
        // requires a real `LoadHandle`.
        let state: AssetState<ImageType> = AssetState::Evicted;

        let mutated = super::apply_rewrite_to_slot(&mut images, 0, &state);

        assert!(!mutated, "non-failure states must leave the slot unchanged");
        assert!(matches!(
            images[0].source,
            lightbox::LightboxImageSource::Resolved { .. }
        ));
    }

    #[test]
    fn apply_rewrite_to_slot_tolerates_out_of_bounds_index() {
        // The caller in `start_asset_load` passes a captured index that
        // could in principle be stale by the time the spawn callback
        // fires (e.g., `update_params` shrinks the image list). The
        // helper must not panic.
        let mut images: Vec<LightboxImage> = vec![];
        let state: AssetState<ImageType> = AssetState::FailedToLoad(Rc::new(
            anyhow::anyhow!("could not detect image format"),
        ));

        let mutated = super::apply_rewrite_to_slot(&mut images, 5, &state);

        assert!(!mutated, "out-of-bounds index must be a no-op, not a panic");
    }

    #[test]
    fn step_zoom_recovers_from_non_finite_input() {
        // A NaN-poisoned zoom factor would NaN-poison the ConstrainedBox
        // size and corrupt the layout. The step helper must squelch that
        // back to the safe default of 1.0.
        assert_eq!(
            super::step_zoom(f32::NAN, super::ZoomDirection::In),
            1.0,
            "NaN must be sanitized to 1.0"
        );
        assert_eq!(
            super::step_zoom(f32::INFINITY, super::ZoomDirection::Out),
            1.0,
            "infinity must be sanitized to 1.0"
        );
    }

    #[test]
    fn sanitize_load_error_picks_decode_category() {
        let err = anyhow::anyhow!("png decode error: invalid IHDR chunk");
        assert_eq!(super::sanitize_load_error(&err), "could not decode image");
    }

    #[test]
    fn sanitize_load_error_falls_back_to_read_category() {
        let err = anyhow::anyhow!("io error: connection reset");
        assert_eq!(super::sanitize_load_error(&err), "could not read image");
    }
}
