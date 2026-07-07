use pathfinder_geometry::vector::vec2f;
use warpui::color::ColorU;
use warpui::elements::{
    Align, ConstrainedBox, Container, CornerRadius, DropShadow, Flex, ParentElement, Radius, Rect,
    Shrinkable,
};
use warpui::{AppContext, Element, Entity, TypedActionView, View};

pub struct RootView;

impl Entity for RootView {
    type Event = ();
}

fn rect_with_shadow(shadow: DropShadow, corner_radius: CornerRadius) -> Box<dyn Element> {
    Shrinkable::new(
        1.,
        Container::new(
            ConstrainedBox::new(
                Rect::new()
                    .with_background_color(ColorU::new(255, 255, 255, 255))
                    .with_corner_radius(corner_radius)
                    .with_drop_shadow(shadow)
                    .finish(),
            )
            .with_width(200.)
            .with_height(100.)
            .finish(),
        )
        .with_uniform_margin(30.)
        .finish(),
    )
    .finish()
}

impl View for RootView {
    fn ui_name() -> &'static str {
        "RootView"
    }

    fn render(&self, _: &AppContext) -> Box<dyn Element> {
        Container::new(
            Align::new(
                Flex::column()
                    .with_children([
                        rect_with_shadow(
                            DropShadow {
                                color: ColorU::black(),
                                offset: vec2f(0., 10.),
                                blur_radius: 10.,
                                spread_radius: 30.,
                            },
                            CornerRadius::default(),
                        ),
                        rect_with_shadow(
                            DropShadow {
                                color: ColorU::new(255, 0, 0, 255),
                                offset: vec2f(10., 10.),
                                blur_radius: 5.,
                                spread_radius: 20.,
                            },
                            CornerRadius::with_all(Radius::Pixels(8.)),
                        ),
                        rect_with_shadow(
                            DropShadow {
                                color: ColorU::new(0, 255, 0, 255),
                                offset: vec2f(-10., -20.),
                                blur_radius: 20.,
                                spread_radius: 10.,
                            },
                            CornerRadius::with_all(Radius::Percentage(30.)),
                        ),
                        rect_with_shadow(
                            DropShadow {
                                color: ColorU::new(0, 0, 255, 255),
                                offset: vec2f(30., 0.),
                                blur_radius: 30.,
                                spread_radius: 40.,
                            },
                            CornerRadius::with_right(Radius::Pixels(40.)),
                        ),
                        ConstrainedBox::new(
                            Rect::new()
                                .with_background_color(ColorU::white())
                                .with_corner_radius(CornerRadius::with_all(Radius::Percentage(50.)))
                                .with_drop_shadow(DropShadow {
                                    color: ColorU::black(),
                                    offset: vec2f(-0.5, 2.),
                                    blur_radius: 20.,
                                    spread_radius: 0.,
                                })
                                .finish(),
                        )
                        .with_width(30.)
                        .with_height(30.)
                        .finish(),
                        ConstrainedBox::new(
                            Rect::new()
                                .with_background_color(ColorU::white())
                                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(10.)))
                                .with_drop_shadow(DropShadow {
                                    color: ColorU::black(),
                                    offset: vec2f(50.5, 2.),
                                    blur_radius: 2.,
                                    spread_radius: 0.,
                                })
                                .finish(),
                        )
                        .with_width(30.)
                        .with_height(30.)
                        .finish(),
                    ])
                    .finish(),
            )
            .finish(),
        )
        .with_background_color(ColorU::new(128, 128, 128, 255))
        .finish()
    }
}

impl TypedActionView for RootView {
    type Action = ();
}
