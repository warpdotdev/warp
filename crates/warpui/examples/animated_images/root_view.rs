use instant::Instant;
use warpui::assets::asset_cache::AssetSource;
use warpui::elements::{
    CacheOption, ConstrainedBox, CrossAxisAlignment, Flex, Image, ParentElement, Shrinkable, Stack,
};
use warpui::{AppContext, Element, Entity, TypedActionView, View};

pub struct RootView {
    animation_start_time: Instant,
}

impl RootView {
    pub fn new() -> Self {
        println!(
            "WARN: This example is slow to start up due to the huge GIF. Compiling with --release \
            helps."
        );
        RootView {
            animation_start_time: Instant::now(),
        }
    }
}

impl Default for RootView {
    fn default() -> Self {
        Self::new()
    }
}

impl Entity for RootView {
    type Event = ();
}
impl View for RootView {
    fn ui_name() -> &'static str {
        "RootView"
    }

    fn render(&self, _: &AppContext) -> Box<dyn Element> {
        Stack::new()
            .with_child(
                Shrinkable::new(
                    1.,
                    Image::new(
                        AssetSource::Bundled {
                            path: "rustyrain.gif",
                        },
                        CacheOption::Original,
                    )
                    .enable_animation_with_start_time(self.animation_start_time)
                    .cover()
                    .finish(),
                )
                .finish(),
            )
            .with_child(
                Flex::row()
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .with_child(
                        ConstrainedBox::new(
                            Image::new(
                                AssetSource::Bundled {
                                    path: "numbers-1000ms.gif",
                                },
                                CacheOption::BySize,
                            )
                            .enable_animation_with_start_time(self.animation_start_time)
                            .finish(),
                        )
                        .with_height(350.)
                        .with_width(350.)
                        .finish(),
                    )
                    .with_child(
                        ConstrainedBox::new(
                            Image::new(
                                AssetSource::Bundled {
                                    path: "numbers-750ms.gif",
                                },
                                CacheOption::BySize,
                            )
                            .enable_animation_with_start_time(self.animation_start_time)
                            .finish(),
                        )
                        .with_height(350.)
                        .with_width(350.)
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
