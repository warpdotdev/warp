use pathfinder_geometry::vector::vec2f;
use warp_core::ui::appearance::Appearance;
use warpui::elements::{Flex, ParentElement};
use warpui::platform::WindowStyle;
use warpui::{
    App, Element, Entity, Presenter, SingletonEntity, TypedActionView, View, WindowInvalidation,
};

use super::render_router_error_card;

/// Root view that lays a custom-router error card inside a bare
/// `Flex::column()`. This mirrors the Custom Routers settings section, which
/// renders its cards inside a vertically-scrollable container that passes an
/// **unbounded** (infinite) vertical constraint down to each card: a
/// `Flex::column()` lays out its non-flexible child with an infinite main-axis
/// constraint, exactly like the real settings page.
struct ErrorCardTestView;

impl Entity for ErrorCardTestView {
    type Event = ();
}

impl View for ErrorCardTestView {
    fn ui_name() -> &'static str {
        "CustomRouterErrorCardTestView"
    }

    fn render(&self, app: &warpui::AppContext) -> Box<dyn warpui::Element> {
        let appearance = Appearance::as_ref(app);
        Flex::column()
            .with_child(render_router_error_card(
                "broken_router.yaml",
                "`My Router`: complexity type requires a `default` model",
                appearance,
            ))
            .finish()
    }
}

impl TypedActionView for ErrorCardTestView {
    type Action = ();
}

/// Regression test for the Custom Routers crash: deleting every model name in a
/// router `.yaml` makes it fail to parse, so the settings page renders an error
/// card. Before the fix, that card wrapped its message in a `Shrinkable`
/// (a *flexible* flex child) inside a `Flex::column()`, so laying it out under
/// the settings page's unbounded vertical constraint panicked in flex layout
/// with "flex contains flexible children but has an infinite constraint along
/// the flex axis". Building the scene here must not panic.
#[test]
fn error_card_lays_out_under_unbounded_vertical_constraint_without_panicking() {
    App::test((), |mut app| async move {
        let app = &mut app;
        app.add_singleton_model(|_| Appearance::mock());

        let (window_id, _view) = app.add_window(WindowStyle::NotStealFocus, |_| ErrorCardTestView);
        let root_view_id = app
            .root_view_id(window_id)
            .expect("window should have a root view");

        let mut presenter = Presenter::new(window_id);
        let invalidation = WindowInvalidation {
            updated: [root_view_id].into_iter().collect(),
            ..Default::default()
        };

        app.update(move |ctx| {
            presenter.invalidate(invalidation, ctx);
            // Panicked here before the fix.
            presenter.build_scene(vec2f(400., 400.), 1., None, ctx);
        });
    });
}
