use warpui::color::ColorU;
use warpui::elements::{
    Align, Container, CornerRadius, CrossAxisAlignment, Fill, Flex, ParentElement, Radius, Text,
};
use warpui::fonts::FamilyId;
use warpui::presenter::ChildView;
use warpui::ui_components::components::{Coords, UiComponentStyles};
use warpui::ui_components::segmented_control::{
    LabelConfig, RenderableOptionConfig, SegmentedControl, SegmentedControlEvent,
};
use warpui::{
    AppContext, Element, Entity, SingletonEntity as _, TypedActionView, View, ViewContext,
    ViewHandle,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisplayMode {
    Rendered,
    Raw,
}

pub struct RootView {
    font_family: FamilyId,
    segmented_control: ViewHandle<SegmentedControl<DisplayMode>>,
    selected_mode: DisplayMode,
}

impl RootView {
    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        let font_family = warpui::fonts::Cache::handle(ctx)
            .update(ctx, |cache, _| cache.load_system_font("Arial").unwrap());

        let segmented_control = ctx.add_typed_action_view(move |_ctx| {
            SegmentedControl::new(
                vec![DisplayMode::Rendered, DisplayMode::Raw],
                move |mode, is_selected, _app| {
                    Some(RenderableOptionConfig {
                        icon_path: "",
                        icon_color: ColorU::white(),
                        label: Some(LabelConfig {
                            label: match mode {
                                DisplayMode::Rendered => "Rendered".into(),
                                DisplayMode::Raw => "Raw".into(),
                            },
                            width_override: Some(55.0),
                            color: if is_selected {
                                ColorU::new(100, 200, 255, 255) // accent color
                            } else {
                                ColorU::white()
                            },
                        }),
                        tooltip: None,
                        background: if is_selected {
                            Fill::Solid(ColorU::new(60, 60, 60, 255)) // surface_3
                        } else {
                            Fill::None
                        },
                    })
                },
                DisplayMode::Rendered,
                segmented_control_styles(font_family),
            )
        });

        ctx.subscribe_to_view(&segmented_control, |me, _, event, ctx| {
            let SegmentedControlEvent::OptionSelected(mode) = event;
            me.selected_mode = *mode;
            ctx.notify();
        });

        Self {
            font_family,
            segmented_control,
            selected_mode: DisplayMode::Rendered,
        }
    }
}

fn segmented_control_styles(font_family: FamilyId) -> UiComponentStyles {
    UiComponentStyles {
        font_family_id: Some(font_family),
        font_size: Some(12.0),
        border_radius: Some(CornerRadius::with_all(Radius::Pixels(4.0))),
        border_width: Some(1.0),
        border_color: Some(Fill::Solid(ColorU::new(60, 60, 60, 255))), // surface_3
        background: Some(Fill::Solid(ColorU::new(30, 30, 30, 255))),   // background
        height: Some(20.0),
        padding: Some(Coords::uniform(2.0)),
        margin: Some(Coords {
            top: 0.0,
            bottom: 0.0,
            left: 0.0,
            right: 8.0,
        }),
        ..Default::default()
    }
}

impl Entity for RootView {
    type Event = ();
}

impl View for RootView {
    fn ui_name() -> &'static str {
        "RootView"
    }

    fn render(&self, _app: &AppContext) -> Box<dyn Element> {
        let status_text = format!("Selected: {:?}", self.selected_mode);

        let content = Flex::column()
            .with_spacing(20.0)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(
                Text::new_inline("Segmented Control Example", self.font_family, 20.0)
                    .with_color(ColorU::white())
                    .finish(),
            )
            .with_child(ChildView::new(&self.segmented_control).finish())
            .with_child(
                Text::new_inline(status_text, self.font_family, 14.0)
                    .with_color(ColorU::new(150, 150, 150, 255))
                    .finish(),
            )
            .finish();

        Container::new(Align::new(content).finish())
            .with_background_color(ColorU::new(20, 20, 20, 255))
            .finish()
    }
}

impl TypedActionView for RootView {
    type Action = ();
}
