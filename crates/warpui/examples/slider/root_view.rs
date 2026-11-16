use pathfinder_color::ColorU;
use warpui::elements::{Align, Container};
use warpui::presenter::ChildView;
use warpui::ui_components::components::{UiComponent, UiComponentStyles};
use warpui::ui_components::slider::{Slider, SliderStateHandle};
use warpui::{AppContext, Element, Entity, TypedActionView, View, ViewContext, ViewHandle};

/// Renders a center-aligned slider component against a black background. When the slider is
/// dragged, the updated value is printed to stdout.
#[derive(Default)]
pub struct SliderExample {
    slider_state: SliderStateHandle,
}

impl SliderExample {
    pub fn new() -> Self {
        Self {
            slider_state: Default::default(),
        }
    }
}

impl View for SliderExample {
    fn ui_name() -> &'static str {
        "SliderExample"
    }

    fn render(&self, _: &AppContext) -> Box<dyn Element> {
        Slider::new(self.slider_state.clone())
            .on_drag(|event_ctx, _app, new_value| {
                event_ctx.dispatch_typed_action(SliderExampleAction::OnSliderDrag(new_value))
            })
            .on_change(|event_ctx, _app, new_value| {
                event_ctx.dispatch_typed_action(SliderExampleAction::OnSliderValueChange(new_value))
            })
            // Set a custom value range.
            .with_range(0.0..100.)
            .with_style(UiComponentStyles {
                width: Some(400.),
                ..Default::default()
            })
            .build()
            .finish()
    }
}

impl Entity for SliderExample {
    type Event = ();
}

#[derive(Debug)]
pub enum SliderExampleAction {
    OnSliderDrag(f32),
    OnSliderValueChange(f32),
}

impl TypedActionView for SliderExample {
    type Action = SliderExampleAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            SliderExampleAction::OnSliderDrag(new_value) => {
                println!("Slider dragged: {new_value:?}");
                ctx.notify();
            }
            SliderExampleAction::OnSliderValueChange(new_value) => {
                println!("Slider dropped: {new_value:?}");
                ctx.notify();
            }
        }
    }
}

/// Create a wrapper view so [`SliderExample`] can be added as a [`TypedActionView`].
pub struct RootView {
    slider_example_view: ViewHandle<SliderExample>,
}

impl RootView {
    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        let slider_example_view = ctx.add_typed_action_view(|_| SliderExample::new());
        Self {
            slider_example_view,
        }
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
        Container::new(Align::new(ChildView::new(&self.slider_example_view).finish()).finish())
            .with_background_color(ColorU::black())
            .finish()
    }
}

impl TypedActionView for RootView {
    type Action = ();
}
