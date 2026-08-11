use warp_core::ui::theme::Fill;

use super::{
    CIRCLE_RATIO, IconWithStatusVariant, OZ_AMBIENT_BACKGROUND_COLOR, circle_size,
    render_icon_with_status, warp_agent_circle_colors,
};
use crate::themes::default_themes::{dark_theme, light_theme};

#[test]
fn local_warp_agent_circle_uses_white_glyph_on_black_for_dark_themes() {
    assert_eq!(
        warp_agent_circle_colors(&dark_theme(), false),
        (Fill::black(), Fill::white())
    );
}

#[test]
fn local_warp_agent_circle_uses_black_glyph_on_white_for_light_themes() {
    assert_eq!(
        warp_agent_circle_colors(&light_theme(), false),
        (Fill::white(), Fill::black())
    );
}

#[test]
fn ambient_warp_agent_circle_keeps_purple_background_in_all_themes() {
    let expected = (Fill::Solid(OZ_AMBIENT_BACKGROUND_COLOR), Fill::black());

    assert_eq!(warp_agent_circle_colors(&dark_theme(), true), expected);
    assert_eq!(warp_agent_circle_colors(&light_theme(), true), expected);
}

/// The brand circle covers only `CIRCLE_RATIO` of the footprint the component reserves, and
/// `corner_overlay_offset` positions the status badge against that footprint's bottom-right
/// corner. Both only line up while the circle is centered in the footprint: left-aligned, the
/// badge lands clear of the circle and the caller's trailing gap grows by the slack.
#[test]
fn brand_circle_is_centered_in_the_reserved_footprint() {
    use pathfinder_geometry::vector::vec2f;
    use warpui::platform::WindowStyle;
    use warpui::{
        App, AppContext, Element, Entity, Presenter, TypedActionView, View, ViewContext,
        WindowInvalidation,
    };

    use crate::ai::agent::conversation::ConversationStatus;

    const TOTAL_SIZE: f32 = 24.;
    const EPSILON: f32 = 0.01;

    struct AgentIconTestView;

    impl AgentIconTestView {
        fn new(_ctx: &mut ViewContext<Self>) -> Self {
            Self
        }
    }

    impl Entity for AgentIconTestView {
        type Event = ();
    }

    impl View for AgentIconTestView {
        fn ui_name() -> &'static str {
            "AgentIconTestView"
        }

        fn render(&self, _app: &AppContext) -> Box<dyn Element> {
            let theme = dark_theme();
            render_icon_with_status(
                IconWithStatusVariant::OzAgent {
                    status: Some(ConversationStatus::Success),
                    is_ambient: false,
                },
                TOTAL_SIZE,
                0.,
                &theme,
                theme.background(),
            )
        }
    }

    impl TypedActionView for AgentIconTestView {
        type Action = ();
    }

    App::test((), |mut app| async move {
        let (window_id, _view) = app.add_window(WindowStyle::NotStealFocus, AgentIconTestView::new);
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
            let scene = presenter.build_scene(vec2f(400., 300.), 1., None, ctx);

            let expected_diameter = circle_size(TOTAL_SIZE);
            let expected_inset = TOTAL_SIZE * (1. - CIRCLE_RATIO) / 2.;
            let circle_origins: Vec<_> = scene
                .layers()
                .flat_map(|layer| layer.rects.iter())
                .filter(|rect| {
                    (rect.bounds.width() - expected_diameter).abs() < EPSILON
                        && (rect.bounds.height() - expected_diameter).abs() < EPSILON
                })
                .map(|rect| rect.bounds.origin())
                .collect();

            let [circle_origin] = circle_origins.as_slice() else {
                panic!(
                    "expected exactly one {expected_diameter}px brand circle in the scene, found \
                     {circle_origins:?}"
                );
            };
            assert!(
                (circle_origin.x() - expected_inset).abs() < EPSILON
                    && (circle_origin.y() - expected_inset).abs() < EPSILON,
                "brand circle should be centered in the {TOTAL_SIZE}px footprint at \
                 ({expected_inset}, {expected_inset}), got {circle_origin:?}"
            );
        });
    });
}
