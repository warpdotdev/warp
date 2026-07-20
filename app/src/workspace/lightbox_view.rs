use std::sync::Arc;

pub use lightbox::LightboxImage;
use pathfinder_geometry::vector::Vector2F;
use ui_components::{Component as _, lightbox};
use warpui::assets::asset_cache::{AssetCache, AssetSource, AssetState};
use warpui::image_cache::ImageType;
use warpui::keymap::{FixedBinding, Keystroke};
use warpui::prelude::*;
use warpui::{AppContext, BlurContext, Element, Entity, SingletonEntity, View, ViewContext};

use crate::appearance::Appearance;

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
}

/// A view that renders a full-window lightbox overlay.
pub struct LightboxView {
    params: LightboxParams,
    current_index: usize,
    lightbox: lightbox::Lightbox,
}

impl LightboxView {
    pub fn new(params: LightboxParams, ctx: &mut ViewContext<Self>) -> Self {
        let initial_index = params
            .initial_index
            .min(params.images.len().saturating_sub(1));
        let view = Self {
            params,
            current_index: initial_index,
            lightbox: lightbox::Lightbox::default(),
        };
        view.start_asset_loads(ctx);
        view
    }

    /// Replace the images and navigate to the given initial index.
    pub fn update_params(&mut self, params: LightboxParams, ctx: &mut ViewContext<Self>) {
        let initial_index = params
            .initial_index
            .min(params.images.len().saturating_sub(1));
        self.params = params;
        self.current_index = initial_index;
        self.start_asset_loads(ctx);
    }

    /// Update a single image at the given index without replacing the full list.
    pub fn update_image_at(
        &mut self,
        index: usize,
        image: LightboxImage,
        ctx: &mut ViewContext<Self>,
    ) {
        if let Some(slot) = self.params.images.get_mut(index) {
            if let lightbox::LightboxImageSource::Resolved { ref asset_source } = image.source {
                Self::start_asset_load(asset_source, ctx);
            }
            *slot = image;
        }
    }

    /// Kick off asset loads for all `Resolved` images and schedule re-renders.
    fn start_asset_loads(&self, ctx: &mut ViewContext<Self>) {
        for img in &self.params.images {
            if let lightbox::LightboxImageSource::Resolved { ref asset_source } = img.source {
                Self::start_asset_load(asset_source, ctx);
            }
        }
    }

    /// Eagerly load a single asset and schedule a `ctx.notify()` when the fetch
    /// completes so the lightbox re-renders with the loaded image.
    fn start_asset_load(asset_source: &AssetSource, ctx: &mut ViewContext<Self>) {
        let asset_cache = AssetCache::as_ref(ctx);
        if let AssetState::Loading { handle } =
            asset_cache.load_asset::<ImageType>(asset_source.clone())
            && let Some(future) = handle.when_loaded(asset_cache)
        {
            ctx.spawn(future, |_me, (), ctx| {
                ctx.notify();
            });
        }
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
                    ctx.notify();
                }
            }
            LightboxViewAction::NavigateNext => {
                if self.current_index + 1 < self.params.images.len() {
                    self.current_index += 1;
                    ctx.notify();
                }
            }
        }
    }
}
