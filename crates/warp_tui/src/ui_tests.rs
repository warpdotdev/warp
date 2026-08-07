use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use warp::appearance::Appearance;
use warp::tui_export::{dark_theme, light_theme};
use warpui::SingletonEntity;
use warpui_core::elements::MouseStateHandle;
use warpui_core::elements::animation::AnimationClock;
use warpui_core::elements::tui::{
    Color, TuiBuffer, TuiBufferExt, TuiConstraint, TuiElement, TuiEvent, TuiEventContext,
    TuiLayoutContext, TuiPaintContext, TuiPaintSurface, TuiRect, TuiScreenPosition, TuiSize,
    TuiText,
};
use warpui_core::event::ModifiersState;
use warpui_core::keymap::Keystroke;
use warpui_core::presenter::tui::TuiPresenter;
use warpui_core::{App, EntityId, EntityIdMap};

use super::{
    LoginBrowserOpenFailedParams, LoginFailedParams, LoginWaitingParams, compact_footer_path,
    conversation_restoring, horizontally_centered, login_browser_open_failed, login_failed,
    login_waiting, signed_out_welcome,
};
use crate::transient_hint::TransientHintTone;
use crate::zero_state_animation::ZeroStateAnimationConfig;

#[test]
fn compact_footer_path_preserves_short_paths() {
    assert_eq!(compact_footer_path("/erica/project"), "/erica/project");
}
#[test]
fn failed_login_renders_clickable_retry_and_handles_retry_keys() {
    App::test((), |app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        app.read(|app_ctx| {
            let retry_count = Rc::new(Cell::new(0));
            let retry_count_for_action = retry_count.clone();
            let mut element = login_failed(
                AnimationClock::starting_at(Duration::ZERO),
                Arc::new(ZeroStateAnimationConfig::default()),
                LoginFailedParams {
                    message: "Failed to generate device code",
                    retry_mouse: MouseStateHandle::default(),
                },
                app_ctx,
                move |_, _| retry_count_for_action.set(retry_count_for_action.get() + 1),
            );
            let area = TuiRect::new(0, 0, 80, 24);
            let mut rendered_views = EntityIdMap::default();
            let mut layout_ctx = TuiLayoutContext {
                rendered_views: &mut rendered_views,
            };
            element.layout(
                TuiConstraint::tight(TuiSize::new(area.width, area.height)),
                &mut layout_ctx,
                app_ctx,
            );
            element.after_layout(&mut layout_ctx, app_ctx);
            let mut buffer = TuiBuffer::empty(area);
            let mut paint_ctx = TuiPaintContext::new(&mut rendered_views);
            {
                let mut surface = TuiPaintSurface::new(&mut buffer);
                element.render(TuiScreenPosition::new(0, 0), &mut surface, &mut paint_ctx);
            }
            let lines = buffer.to_lines();
            let rendered = lines.join("\n");
            assert!(rendered.contains("Login failed: Failed to generate device code"));
            assert!(rendered.contains("Retry login (r)"));
            assert!(rendered.contains("Press enter or r to retry · Ctrl-C to exit"));
            assert!(!rendered.contains("Copy URL (c)"));

            let row = lines
                .iter()
                .position(|line| line.contains("Retry login (r)"))
                .expect("retry action renders") as u16;
            let col = lines[usize::from(row)]
                .find("Retry login (r)")
                .expect("retry action offset") as u16;
            let scene = Rc::new(paint_ctx.scene.clone());
            drop(paint_ctx);
            let mut event_ctx = TuiEventContext::new(scene, &mut rendered_views);
            event_ctx.set_origin_view(Some(EntityId::new()));
            for event in [
                TuiEvent::LeftMouseDown {
                    position: (col, row).into(),
                    modifiers: ModifiersState::default(),
                    click_count: 1,
                    is_first_mouse: false,
                },
                TuiEvent::LeftMouseUp {
                    position: (col, row).into(),
                    modifiers: ModifiersState::default(),
                },
            ] {
                assert!(element.dispatch_event(&event, &mut event_ctx, app_ctx));
            }
            assert_eq!(retry_count.get(), 1);

            let key_down = |key: &str| TuiEvent::KeyDown {
                keystroke: Keystroke {
                    key: key.to_owned(),
                    ..Default::default()
                },
                chars: String::new(),
                details: Default::default(),
                is_composing: false,
            };
            assert!(element.dispatch_event(&key_down("r"), &mut event_ctx, app_ctx));
            assert!(element.dispatch_event(&key_down("enter"), &mut event_ctx, app_ctx));
            assert!(!element.dispatch_event(&key_down("c"), &mut event_ctx, app_ctx));
            assert_eq!(retry_count.get(), 3);
        });
    });
}

#[test]
fn horizontally_centered_balances_available_space() {
    App::test((), |app| async move {
        app.read(|ctx| {
            let mut presenter = TuiPresenter::new();
            let frame = presenter.present_element(
                horizontally_centered(TuiText::new("center").finish()),
                TuiRect::new(0, 0, 20, 1),
                ctx,
            );
            let line = &frame.buffer.to_lines()[0];
            let left = line.find("center").expect("centered text renders");
            let right = 20 - left - "center".len();
            assert!(left.abs_diff(right) <= 1, "{line:?}");
        });
    });
}

#[test]
fn waiting_login_renders_copy_feedback() {
    App::test((), |app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        app.read(|app_ctx| {
            let browser_url =
                "https://app.warp.dev/device?user_code=ABCD-EFGH&source=warp-agent-cli";
            for (message, tone) in [
                ("Login URL copied to clipboard", TransientHintTone::Success),
                ("Unable to copy login URL", TransientHintTone::Error),
            ] {
                let mut presenter = TuiPresenter::new();
                let frame = presenter.present_element(
                    login_waiting(
                        AnimationClock::starting_at(Duration::ZERO),
                        Arc::new(ZeroStateAnimationConfig::default()),
                        LoginWaitingParams {
                            browser_url: Some(browser_url),
                            login_mouse: MouseStateHandle::default(),
                            copy_mouse: MouseStateHandle::default(),
                            copy_feedback: Some((message, tone)),
                        },
                        app_ctx,
                        |_, _| {},
                        |_, _| {},
                    ),
                    TuiRect::new(0, 0, 80, 24),
                    app_ctx,
                );
                let lines = frame.buffer.to_lines();
                assert!(
                    lines.iter().any(|line| line.contains(message)),
                    "waiting state should render copy feedback: {lines:?}"
                );
            }
        });
    });
}

#[test]
fn compact_footer_path_elides_middle_components() {
    assert_eq!(compact_footer_path("~/Documents/GitHub/warp"), "~/…/warp");
    assert_eq!(compact_footer_path("/long/path/to/project"), "/…/project");
    assert_eq!(
        compact_footer_path(r"C:\Users\erica\project"),
        r"C:\…\project"
    );
}

#[test]
fn conversation_loader_is_centered_and_animated() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            ctx.add_singleton_model(|_| Appearance::mock());
        });
        app.read(|app_ctx| {
            let element = conversation_restoring(app_ctx);
            let mut presenter = TuiPresenter::new();
            let frame = presenter.present_element(element, TuiRect::new(0, 0, 60, 7), app_ctx);
            let lines = frame.buffer.to_lines();
            let label = lines
                .iter()
                .find(|line| line.contains("Loading session..."))
                .expect("loading label should render");
            assert!(
                lines.iter().any(|line| {
                    line.contains("Esc or Ctrl-C to cancel and start a new session")
                })
            );

            assert!(
                label.find("Loading session...").is_some_and(|x| x > 0),
                "loading label should be horizontally centered: {label:?}"
            );
            assert!(
                frame.repaint_at.is_some(),
                "loading spinner should schedule a repaint"
            );
        });
    });
}

#[test]
fn signed_out_welcome_matches_designed_copy_and_layout() {
    App::test((), |app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        app.read(|app_ctx| {
            let mut presenter = TuiPresenter::new();
            let frame = presenter.present_element(
                signed_out_welcome(
                    AnimationClock::starting_at(Duration::ZERO),
                    Arc::new(ZeroStateAnimationConfig::default()),
                    MouseStateHandle::default(),
                    MouseStateHandle::default(),
                    app_ctx,
                    |_, _| {},
                    |_, _| {},
                ),
                TuiRect::new(0, 0, 80, 24),
                app_ctx,
            );
            let lines = frame.buffer.to_lines();

            for expected in [
                "Welcome to Warp",
                "> Press enter to get started",
                "Log in with Warp",
                "Copy login URL (c)",
                "What’s different about Warp",
                "✶ State of the art coding agents",
                "✶ Frontier and open-weight models",
                "✶ Fully customizable model routers",
                "✶ Orchestration for fleets of agents",
                "✶ Better shell command support",
            ] {
                assert!(
                    lines.iter().any(|line| line.contains(expected)),
                    "welcome should render {expected:?}: {lines:?}"
                );
            }
            let title_row = lines
                .iter()
                .position(|line| line.contains("Welcome to Warp"))
                .expect("welcome title renders");
            assert!(title_row > 0 && title_row < 12);
            assert_eq!(
                lines[title_row]
                    .find("Welcome to Warp")
                    .expect("welcome title offset"),
                3
            );
            assert!(frame.repaint_at.is_some());
            assert!(
                lines
                    .iter()
                    .all(|line| !line.contains("https://app.warp.dev/login")),
                "welcome must not render a URL before device authorization: {lines:?}"
            );
        });
    });
}

#[test]
fn signed_out_welcome_uses_figma_brand_colors_in_dark_and_light_themes() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| Appearance::mock());

        for (theme, brand_primary, brand_accent) in [
            (
                dark_theme(),
                Color::Rgb(210, 181, 255),
                Color::Rgb(226, 255, 212),
            ),
            (
                light_theme(),
                Color::Rgb(156, 88, 240),
                Color::Rgb(51, 119, 11),
            ),
        ] {
            Appearance::handle(&app).update(&mut app, |appearance, ctx| {
                appearance.set_theme(theme, ctx);
            });
            app.read(|app_ctx| {
                let mut presenter = TuiPresenter::new();
                let frame = presenter.present_element(
                    signed_out_welcome(
                        AnimationClock::starting_at(Duration::ZERO),
                        Arc::new(ZeroStateAnimationConfig::default()),
                        MouseStateHandle::default(),
                        MouseStateHandle::default(),
                        app_ctx,
                        |_, _| {},
                        |_, _| {},
                    ),
                    TuiRect::new(0, 0, 80, 24),
                    app_ctx,
                );
                let lines = frame.buffer.to_lines();
                let cell_color = |text: &str, offset: usize| {
                    let row = lines
                        .iter()
                        .position(|line| line.contains(text))
                        .expect("designed text renders");
                    let column = lines[row].find(text).expect("designed text offset") + offset;
                    frame.buffer[(u16::try_from(column).unwrap(), u16::try_from(row).unwrap())].fg
                };

                assert_eq!(cell_color("Welcome to Warp", 0), brand_primary);
                assert_eq!(cell_color("> Press enter to get started", 0), brand_accent);
                assert_eq!(cell_color("> Press enter to get started", 8), brand_accent);
                assert_eq!(
                    cell_color("✶ State of the art coding agents", 0),
                    brand_accent
                );
                assert_eq!(
                    cell_color("✶ Better shell command support", 0),
                    brand_accent
                );
            });
        }
    });
}

#[test]
fn waiting_login_generated_url_handles_click() {
    App::test((), |app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        app.read(|app_ctx| {
            let activated = Rc::new(Cell::new(false));
            let activated_for_click = activated.clone();
            let browser_url =
                "https://app.warp.dev/device?user_code=ABCD-EFGH&source=warp-agent-cli";
            let mut element = login_waiting(
                AnimationClock::starting_at(Duration::ZERO),
                Arc::new(ZeroStateAnimationConfig::default()),
                LoginWaitingParams {
                    browser_url: Some(browser_url),
                    login_mouse: MouseStateHandle::default(),
                    copy_mouse: MouseStateHandle::default(),
                    copy_feedback: None,
                },
                app_ctx,
                move |_, _| activated_for_click.set(true),
                |_, _| {},
            );
            let area = TuiRect::new(0, 0, 80, 24);
            let mut rendered_views = EntityIdMap::default();
            let mut layout_ctx = TuiLayoutContext {
                rendered_views: &mut rendered_views,
            };
            element.layout(
                TuiConstraint::tight(TuiSize::new(area.width, area.height)),
                &mut layout_ctx,
                app_ctx,
            );
            element.after_layout(&mut layout_ctx, app_ctx);
            let mut buffer = TuiBuffer::empty(area);
            let mut paint_ctx = TuiPaintContext::new(&mut rendered_views);
            {
                let mut surface = TuiPaintSurface::new(&mut buffer);
                element.render(TuiScreenPosition::new(0, 0), &mut surface, &mut paint_ctx);
            }
            let lines = buffer.to_lines();
            let row = lines
                .iter()
                .position(|line| line.contains("https://app.warp.dev/device?user_code="))
                .expect("generated URL renders") as u16;
            let col = lines[usize::from(row)]
                .find("https://app.warp.dev/device?user_code=")
                .expect("generated URL offset") as u16;
            let scene = Rc::new(paint_ctx.scene.clone());
            drop(paint_ctx);
            let mut event_ctx = TuiEventContext::new(scene, &mut rendered_views);
            event_ctx.set_origin_view(Some(EntityId::new()));
            for event in [
                TuiEvent::LeftMouseDown {
                    position: (col, row).into(),
                    modifiers: ModifiersState::default(),
                    click_count: 1,
                    is_first_mouse: false,
                },
                TuiEvent::LeftMouseUp {
                    position: (col, row).into(),
                    modifiers: ModifiersState::default(),
                },
            ] {
                assert!(element.dispatch_event(&event, &mut event_ctx, app_ctx));
            }
            assert!(activated.get());
        });
    });
}

#[test]
fn waiting_login_copy_control_handles_click_and_key() {
    App::test((), |app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        app.read(|app_ctx| {
            let copy_count = Rc::new(Cell::new(0));
            let copy_count_for_action = copy_count.clone();
            let browser_url =
                "https://app.warp.dev/device?user_code=ABCD-EFGH&source=warp-agent-cli";
            let mut element = login_waiting(
                AnimationClock::starting_at(Duration::ZERO),
                Arc::new(ZeroStateAnimationConfig::default()),
                LoginWaitingParams {
                    browser_url: Some(browser_url),
                    login_mouse: MouseStateHandle::default(),
                    copy_mouse: MouseStateHandle::default(),
                    copy_feedback: None,
                },
                app_ctx,
                |_, _| {},
                move |_, _| copy_count_for_action.set(copy_count_for_action.get() + 1),
            );
            let area = TuiRect::new(0, 0, 80, 24);
            let mut rendered_views = EntityIdMap::default();
            let mut layout_ctx = TuiLayoutContext {
                rendered_views: &mut rendered_views,
            };
            element.layout(
                TuiConstraint::tight(TuiSize::new(area.width, area.height)),
                &mut layout_ctx,
                app_ctx,
            );
            element.after_layout(&mut layout_ctx, app_ctx);
            let mut buffer = TuiBuffer::empty(area);
            let mut paint_ctx = TuiPaintContext::new(&mut rendered_views);
            {
                let mut surface = TuiPaintSurface::new(&mut buffer);
                element.render(TuiScreenPosition::new(0, 0), &mut surface, &mut paint_ctx);
            }
            let lines = buffer.to_lines();
            let row = lines
                .iter()
                .position(|line| line.contains("Copy URL (c)"))
                .expect("copy control renders") as u16;
            let col = lines[usize::from(row)]
                .find("Copy URL (c)")
                .expect("copy control offset") as u16;
            let scene = Rc::new(paint_ctx.scene.clone());
            drop(paint_ctx);
            let mut event_ctx = TuiEventContext::new(scene, &mut rendered_views);
            event_ctx.set_origin_view(Some(EntityId::new()));
            for event in [
                TuiEvent::LeftMouseDown {
                    position: (col, row).into(),
                    modifiers: ModifiersState::default(),
                    click_count: 1,
                    is_first_mouse: false,
                },
                TuiEvent::LeftMouseUp {
                    position: (col, row).into(),
                    modifiers: ModifiersState::default(),
                },
            ] {
                assert!(element.dispatch_event(&event, &mut event_ctx, app_ctx));
            }
            assert_eq!(copy_count.get(), 1);

            let copy_key = TuiEvent::KeyDown {
                keystroke: Keystroke {
                    key: "c".to_owned(),
                    ..Default::default()
                },
                chars: String::new(),
                details: Default::default(),
                is_composing: false,
            };
            assert!(element.dispatch_event(&copy_key, &mut event_ctx, app_ctx));
            assert_eq!(copy_count.get(), 2);
        });
    });
}
#[test]
fn signed_out_welcome_handles_enter_and_copy_shortcuts() {
    App::test((), |app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        app.read(|app_ctx| {
            let login_count = Rc::new(Cell::new(0));
            let copy_count = Rc::new(Cell::new(0));
            let login_count_for_enter = login_count.clone();
            let copy_count_for_key = copy_count.clone();
            let mut element = signed_out_welcome(
                AnimationClock::starting_at(Duration::ZERO),
                Arc::new(ZeroStateAnimationConfig::default()),
                MouseStateHandle::default(),
                MouseStateHandle::default(),
                app_ctx,
                move |_, _| login_count_for_enter.set(login_count_for_enter.get() + 1),
                move |_, _| copy_count_for_key.set(copy_count_for_key.get() + 1),
            );
            let mut rendered_views = EntityIdMap::default();
            let scene = Rc::new(Default::default());
            let mut event_ctx = TuiEventContext::new(scene, &mut rendered_views);
            event_ctx.set_origin_view(Some(EntityId::new()));
            let key_down = |key: &str| TuiEvent::KeyDown {
                keystroke: Keystroke {
                    key: key.to_owned(),
                    ..Default::default()
                },
                chars: String::new(),
                details: Default::default(),
                is_composing: false,
            };

            assert!(!element.dispatch_event(&key_down("a"), &mut event_ctx, app_ctx));
            assert_eq!(login_count.get(), 0);
            assert_eq!(copy_count.get(), 0);
            assert!(element.dispatch_event(&key_down("enter"), &mut event_ctx, app_ctx));
            assert_eq!(login_count.get(), 1);
            assert_eq!(copy_count.get(), 0);
            assert!(element.dispatch_event(&key_down("c"), &mut event_ctx, app_ctx));
            assert_eq!(login_count.get(), 1);
            assert_eq!(copy_count.get(), 1);
        });
    });
}

#[test]
fn signed_out_welcome_handles_login_link_click() {
    App::test((), |app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        app.read(|app_ctx| {
            let activated = Rc::new(Cell::new(false));
            let activated_for_click = activated.clone();
            let mut element = signed_out_welcome(
                AnimationClock::starting_at(Duration::ZERO),
                Arc::new(ZeroStateAnimationConfig::default()),
                MouseStateHandle::default(),
                MouseStateHandle::default(),
                app_ctx,
                move |_, _| activated_for_click.set(true),
                |_, _| {},
            );
            let area = TuiRect::new(0, 0, 80, 24);
            let mut rendered_views = EntityIdMap::default();
            let mut layout_ctx = TuiLayoutContext {
                rendered_views: &mut rendered_views,
            };
            element.layout(
                TuiConstraint::tight(TuiSize::new(area.width, area.height)),
                &mut layout_ctx,
                app_ctx,
            );
            element.after_layout(&mut layout_ctx, app_ctx);
            let mut buffer = TuiBuffer::empty(area);
            let mut paint_ctx = TuiPaintContext::new(&mut rendered_views);
            {
                let mut surface = TuiPaintSurface::new(&mut buffer);
                element.render(TuiScreenPosition::new(0, 0), &mut surface, &mut paint_ctx);
            }
            let lines = buffer.to_lines();
            let row = lines
                .iter()
                .position(|line| line.contains("Log in with Warp"))
                .expect("login action renders") as u16;
            let col = lines[usize::from(row)]
                .find("Log in with Warp")
                .expect("login action offset") as u16;
            let scene = Rc::new(paint_ctx.scene.clone());
            drop(paint_ctx);
            let mut event_ctx = TuiEventContext::new(scene, &mut rendered_views);
            event_ctx.set_origin_view(Some(EntityId::new()));
            for event in [
                TuiEvent::LeftMouseDown {
                    position: (col, row).into(),
                    modifiers: ModifiersState::default(),
                    click_count: 1,
                    is_first_mouse: false,
                },
                TuiEvent::LeftMouseUp {
                    position: (col, row).into(),
                    modifiers: ModifiersState::default(),
                },
            ] {
                assert!(element.dispatch_event(&event, &mut event_ctx, app_ctx));
            }
            assert!(activated.get());
        });
    });
}

#[test]
fn browser_open_failure_renders_exact_fallback_and_handles_recovery_keys() {
    App::test((), |app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        app.read(|app_ctx| {
            let browser_url =
                "https://app.warp.dev/device?user_code=ABCD-EFGH&source=warp-agent-cli";
            let mut presenter = TuiPresenter::new();
            let frame = presenter.present_element(
                login_browser_open_failed(
                    AnimationClock::starting_at(Duration::ZERO),
                    Arc::new(ZeroStateAnimationConfig::default()),
                    LoginBrowserOpenFailedParams {
                        browser_url,
                        login_mouse: MouseStateHandle::default(),
                        copy_mouse: MouseStateHandle::default(),
                        retry_mouse: MouseStateHandle::default(),
                        copy_feedback: None,
                    },
                    app_ctx,
                    |_, _| {},
                    |_, _| {},
                ),
                TuiRect::new(0, 0, 80, 24),
                app_ctx,
            );
            let rendered = frame.buffer.to_lines().join("\n");
            for expected in [
                "We couldn’t open your browser.",
                "Open this exact URL manually:",
                "https://app.warp.dev/device?user_code=ABCD-EF",
                "GH&source=warp-agent-cli",
                "Copy URL (c)",
                "Retry opening browser (r)",
            ] {
                assert!(
                    rendered.contains(expected),
                    "browser failure should render {expected:?}: {rendered:?}"
                );
            }
            assert!(!rendered.contains("Continue to terminal"));

            let retry_count = Rc::new(Cell::new(0));
            let copy_count = Rc::new(Cell::new(0));
            let retry_count_for_action = retry_count.clone();
            let copy_count_for_action = copy_count.clone();
            let mut element = login_browser_open_failed(
                AnimationClock::starting_at(Duration::ZERO),
                Arc::new(ZeroStateAnimationConfig::default()),
                LoginBrowserOpenFailedParams {
                    browser_url,
                    login_mouse: MouseStateHandle::default(),
                    copy_mouse: MouseStateHandle::default(),
                    retry_mouse: MouseStateHandle::default(),
                    copy_feedback: None,
                },
                app_ctx,
                move |_, _| retry_count_for_action.set(retry_count_for_action.get() + 1),
                move |_, _| copy_count_for_action.set(copy_count_for_action.get() + 1),
            );
            let mut rendered_views = EntityIdMap::default();
            let scene = Rc::new(Default::default());
            let mut event_ctx = TuiEventContext::new(scene, &mut rendered_views);
            event_ctx.set_origin_view(Some(EntityId::new()));
            for key in ["r", "c"] {
                let event = TuiEvent::KeyDown {
                    keystroke: Keystroke {
                        key: key.to_owned(),
                        ..Default::default()
                    },
                    chars: String::new(),
                    details: Default::default(),
                    is_composing: false,
                };
                assert!(element.dispatch_event(&event, &mut event_ctx, app_ctx));
            }
            let enter = TuiEvent::KeyDown {
                keystroke: Keystroke {
                    key: "enter".to_owned(),
                    ..Default::default()
                },
                chars: String::new(),
                details: Default::default(),
                is_composing: false,
            };
            assert!(!element.dispatch_event(&enter, &mut event_ctx, app_ctx));
            assert_eq!(retry_count.get(), 1);
            assert_eq!(copy_count.get(), 1);
        });
    });
}

#[test]
fn waiting_login_renders_actual_browser_url_and_requesting_fallback() {
    App::test((), |app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        app.read(|app_ctx| {
            let browser_url =
                "https://app.warp.dev/device?user_code=ABCD-EFGH&source=warp-agent-cli";
            for url in [Some(browser_url), None] {
                let mut presenter = TuiPresenter::new();
                let frame = presenter.present_element(
                    login_waiting(
                        AnimationClock::starting_at(Duration::ZERO),
                        Arc::new(ZeroStateAnimationConfig::default()),
                        LoginWaitingParams {
                            browser_url: url,
                            login_mouse: MouseStateHandle::default(),
                            copy_mouse: MouseStateHandle::default(),
                            copy_feedback: None,
                        },
                        app_ctx,
                        |_, _| {},
                        |_, _| {},
                    ),
                    TuiRect::new(0, 0, 80, 24),
                    app_ctx,
                );
                let lines = frame.buffer.to_lines();
                assert!(
                    lines
                        .iter()
                        .any(|line| line.contains("Waiting for login...")),
                    "{lines:?}"
                );
                if url.is_some() {
                    assert!(
                        lines.iter().any(|line| line.contains("Copy URL (c)")),
                        "waiting state should render the copy control: {lines:?}"
                    );
                    assert!(
                        lines.iter().any(|line| {
                            line.contains("https://app.warp.dev/device?user_code=ABCD-EF")
                        }),
                        "waiting state should render the start of the real URL: {lines:?}"
                    );
                    assert!(
                        lines
                            .iter()
                            .any(|line| line.contains("GH&source=warp-agent-cli")),
                        "waiting state should render the rest of the real URL: {lines:?}"
                    );
                } else {
                    assert!(
                        lines.iter().all(|line| !line.contains("Copy URL (c)")),
                        "requesting state must not render the copy control: {lines:?}"
                    );
                    assert!(
                        lines
                            .iter()
                            .any(|line| line.contains("Requesting a secure sign-in link...")),
                        "{lines:?}"
                    );
                }
            }
        });
    });
}
