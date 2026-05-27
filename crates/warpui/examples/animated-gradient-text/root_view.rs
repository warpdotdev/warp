use warpui::color::ColorU;
use warpui::elements::shimmering_text::{
    ShimmerConfig, ShimmeringTextElement, ShimmeringTextStateHandle,
};
use warpui::elements::{Align, ConstrainedBox, ParentElement, Rect, Stack};
use warpui::fonts::FamilyId;
use warpui::{
    AppContext, Element, Entity, SingletonEntity as _, TypedActionView, View, ViewContext,
};

pub struct RootView {
    text: String,
    font_family: FamilyId,
    font_size: f32,
    start: ColorU,
    end: ColorU,
    config: ShimmerConfig,

    // Persist the animation/layout state across renders.
    shimmering_text_handle: ShimmeringTextStateHandle,
}

impl RootView {
    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        let font_family = warpui::fonts::Cache::handle(ctx).update(ctx, |cache, _| {
            cache
                .load_system_font("Times")
                .or_else(|_| cache.load_system_font("Arial"))
                .expect("Should load a system font")
        });

        // Treat start/end as the dim → bright endpoints.
        let start = ColorU::new(160, 160, 160, 255);
        let end = ColorU::new(255, 255, 255, 255);

        Self {
            text: "Warp shimmer: 👩‍💻with ligatures — fi fl 🇺🇸".to_string(),
            font_family,
            font_size: 28.0,
            start,
            end,
            config: ShimmerConfig::default(),
            shimmering_text_handle: ShimmeringTextStateHandle::new(),
        }
    }
}

impl Entity for RootView {
    type Event = ();
}

impl View for RootView {
    fn ui_name() -> &'static str {
        "AnimatedGradientText"
    }

    fn render(&self, _: &AppContext) -> Box<dyn Element> {
        let shimmering = ShimmeringTextElement::new(
            self.text.clone(),
            self.font_family,
            self.font_size,
            self.start,
            self.end,
            self.config,
            self.shimmering_text_handle.clone(),
        )
        .finish();

        Stack::new()
            .with_child(Rect::new().with_background_color(ColorU::black()).finish())
            .with_child(
                Align::new(
                    ConstrainedBox::new(shimmering)
                        .with_max_width(900.)
                        .finish(),
                )
                .finish(),
            )
            .finish()
    }
}

impl TypedActionView for RootView {
    type Action = ();
}
