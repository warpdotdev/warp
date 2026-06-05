use std::sync::LazyLock;

use warp_core::ui::appearance::Appearance;
use warpui_core::elements::{MouseStateHandle, Rect};
use warpui_core::prelude::stack::*;
use warpui_core::prelude::*;

use crate::{Component, MouseEventHandler, Renderable};

/// The color of the switch's track when it is unchecked.
static TRACK_COLOR: LazyLock<ColorU> = LazyLock::new(|| ColorU::new(170, 170, 170, 255));
/// The drop shadow to apply to the switch's thumb when it is hovered.
static DROP_SHADOW: LazyLock<DropShadow> = LazyLock::new(|| DropShadow {
    color: ColorU::black(),
    offset: vec2f(-0.5, 2.),
    blur_radius: 20.,
    spread_radius: 0.,
});

#[derive(Default)]
pub struct Switch {
    component_mouse_state: MouseStateHandle,
    thumb_mouse_state: MouseStateHandle,
}

pub struct Params<'a> {
    pub checked: bool,
    pub on_click: Option<MouseEventHandler>,
    pub options: Options<'a>,
}

impl<'a> crate::Params for Params<'a> {
    type Options<'o> = Options<'a>;
}

#[derive(Default)]
pub struct Options<'a> {
    pub disabled: bool,
    pub height: f32,
    /// Optional label for the switch that is rendered within the switch's click target.
    pub label: Option<Box<dyn Renderable<'a>>>,
    pub hover_border_size: Option<f32>,
}

impl crate::Options for Options<'_> {
    fn default(_appearance: &Appearance) -> Self {
        const DEFAULT_THUMB_HEIGHT: f32 = 18.;

        Self {
            disabled: false,
            height: DEFAULT_THUMB_HEIGHT,
            label: None,
            hover_border_size: None,
        }
    }
}

impl Component for Switch {
    type Params<'a> = Params<'a>;

    fn render<'a>(&self, appearance: &Appearance, params: Self::Params<'a>) -> Box<dyn Element> {
        let disabled = params.options.disabled;

        let switch = self.render_switch(appearance, &params);

        let mut hoverable = Hoverable::new(self.component_mouse_state.clone(), |_state| {
            if let Some(label) = params.options.label {
                let label = label.render(appearance);

                Flex::row()
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .with_child(label)
                    .with_child(Container::new(switch).with_margin_left(8.).finish())
                    .finish()
            } else {
                switch
            }
        });

        if !disabled && let Some(mut on_click) = params.on_click {
            hoverable = hoverable.on_click(move |ctx, app, pos| {
                on_click(ctx, app, pos);
            });
        }

        if !disabled {
            hoverable = hoverable.with_cursor(Cursor::PointingHand);
        }

        hoverable.finish()
    }
}

impl Switch {
    fn render_switch(&self, appearance: &Appearance, params: &Params) -> Box<dyn Element> {
        let thumb_height = params.options.height;

        let track = Container::new(
            ConstrainedBox::new(Empty::new().finish())
                .with_width(thumb_height * 2.)
                .with_height(thumb_height)
                .finish(),
        )
        .with_corner_radius(CornerRadius::with_all(Radius::Percentage(50.)));

        let background_color = if params.checked {
            appearance.theme().accent().into()
        } else {
            Fill::Solid(*TRACK_COLOR)
        };

        Stack::new()
            .with_child(track.with_background(background_color).finish())
            .with_positioned_child(
                self.render_thumb(params),
                Self::thumb_positioning(params.checked),
            )
            .finish()
    }

    fn thumb_positioning(checked: bool) -> OffsetPositioning {
        // If checked, right-align the thumb. If unchecked, left-align the thumb.
        let (parent_anchor, child_anchor) = if checked {
            (ParentAnchor::TopRight, ChildAnchor::TopRight)
        } else {
            (ParentAnchor::TopLeft, ChildAnchor::TopLeft)
        };
        OffsetPositioning::offset_from_parent(
            vec2f(0., 0.),
            ParentOffsetBounds::Unbounded,
            parent_anchor,
            child_anchor,
        )
    }

    // Renders the thumb. The thumb needs its own hoverable to render a border around itself when
    // hovered.
    fn render_thumb(&self, params: &Params<'_>) -> Box<dyn Element> {
        let thumb_height = params.options.height;
        let is_disabled = params.options.disabled;
        let thumb_color = Fill::Solid(ColorU::white());
        Hoverable::new(self.thumb_mouse_state.clone(), |state| {
            let thumb = Container::new(
                ConstrainedBox::new(
                    Rect::new()
                        .with_background(thumb_color)
                        .with_corner_radius(CornerRadius::with_all(Radius::Percentage(50.)))
                        .with_drop_shadow(*DROP_SHADOW)
                        .finish(),
                )
                .with_width(thumb_height)
                .with_height(thumb_height)
                .finish(),
            )
            .finish();
            let mut stack = Stack::new();

            // If a border is specified and the mouse is over the element,
            // render a circle behind the thumb with the border color.
            if let Some(border_size) = params.options.hover_border_size
                && !is_disabled
                && state.is_mouse_over_element()
            {
                Self::add_thumb_hover(&mut stack, thumb_height, border_size);
            }

            stack.add_child(thumb);
            stack.finish()
        })
        .finish()
    }

    /// Adds the hovered thumb border to the given stack.
    fn add_thumb_hover(stack: &mut Stack, thumb_height: f32, border_size: f32) {
        let mut hover_background = *TRACK_COLOR;
        hover_background.a = 100;

        let hover_size = thumb_height + border_size;

        let thumb_hover = ConstrainedBox::new(
            Rect::new()
                .with_background_color(hover_background)
                .with_corner_radius(CornerRadius::with_all(Radius::Percentage(50.)))
                .finish(),
        )
        .with_width(hover_size)
        .with_height(hover_size)
        .finish();

        // Compute the difference in radii between the hover and the thumb.
        let radius_diff = (hover_size - thumb_height) / 2.;
        // Offset the hover so that it's centered around the thumb.
        let offset = OffsetType::Pixel(-radius_diff);
        stack.add_positioned_child(
            thumb_hover,
            OffsetPositioning::from_axes(
                PositioningAxis::relative_to_parent(
                    ParentOffsetBounds::Unbounded,
                    offset,
                    AnchorPair::new(XAxisAnchor::Left, XAxisAnchor::Left),
                ),
                PositioningAxis::relative_to_parent(
                    ParentOffsetBounds::Unbounded,
                    offset,
                    AnchorPair::new(YAxisAnchor::Top, YAxisAnchor::Top),
                ),
            ),
        );
    }
}
