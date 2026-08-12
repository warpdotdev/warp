use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use pathfinder_geometry::rect::RectF;
use pathfinder_geometry::vector::{Vector2F, vec2f};
use warp_core::features::FeatureFlag;
use warp_core::settings::Setting;
use warp_core::ui::appearance::Appearance;
use warp_editor::render::element::VerticalExpansionBehavior;
use warp_util::user_input::UserInput;
use warpui::elements::ScrollbarWidth;
use warpui::elements::new_scrollable::ScrollableAppearance;
use warpui::event::ModifiersState;
use warpui::platform::WindowStyle;
use warpui::{
    App, Event, Presenter, SingletonEntity, TypedActionView, ViewHandle, WindowId,
    WindowInvalidation,
};

use super::{CodeEditorRenderOptions, CodeEditorView, CodeEditorViewAction};
use crate::AuthStateProvider;
use crate::cloud_object::model::persistence::CloudModel;
use crate::code::editor::find::view::FIND_INPUT_POSITION_ID;
use crate::editor::{EditorAction, InteractionState};
use crate::notebooks::editor::keys::NotebookKeybindings;
use crate::server::server_api::team::MockTeamClient;
use crate::server::server_api::workspace::MockWorkspaceClient;
use crate::settings::AppEditorSettings;
use crate::settings_view::keybindings::KeybindingChangedNotifier;
use crate::test_util::settings::initialize_settings_for_tests;
use crate::vim_registers::VimRegisters;
use crate::workspace::ActiveSession;
use crate::workspace::sync_inputs::SyncedInputState;
use crate::workspaces::user_workspaces::UserWorkspaces;

/// Sets up the singleton models `EditorView` depends on and creates a `CodeEditorView` window.
/// When `vim_enabled` is true, Vim mode is turned on *before* the view is constructed, matching
/// how the setting is enabled in real usage and ensuring the view's Vim FSA starts in Normal
/// mode (see `CodeEditorView::new`'s startup `escape` keypress).
fn initialize_editor(app: &mut App, vim_enabled: bool) -> (WindowId, ViewHandle<CodeEditorView>) {
    initialize_settings_for_tests(app);

    if vim_enabled {
        AppEditorSettings::handle(app).update(app, |settings, ctx| {
            let _ = settings.vim_mode.set_value(true, ctx);
        });
    }

    // Add all required singleton models for EditorView dependencies
    app.add_singleton_model(|_| Appearance::mock());
    app.add_singleton_model(|_| SyncedInputState::mock());
    app.add_singleton_model(|_| VimRegisters::new());
    app.add_singleton_model(|_| KeybindingChangedNotifier::mock());
    app.add_singleton_model(|_| AuthStateProvider::new_for_test());

    // Add mocks required by rich text editor (used in CommentEditor)
    app.add_singleton_model(CloudModel::mock);
    app.add_singleton_model(|_| ActiveSession::default());
    app.add_singleton_model(NotebookKeybindings::new);

    // Add UserWorkspaces mock (required by EditorView)
    let team_client_mock = Arc::new(MockTeamClient::new());
    let workspace_client_mock = Arc::new(MockWorkspaceClient::new());
    app.add_singleton_model(|ctx| {
        UserWorkspaces::mock(
            team_client_mock.clone(),
            workspace_client_mock.clone(),
            vec![],
            ctx,
        )
    });

    let (window, editor_view) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
        CodeEditorView::new(
            None,
            None,
            CodeEditorRenderOptions::new(VerticalExpansionBehavior::GrowToMaxHeight),
            ctx,
        )
        .with_horizontal_scrollbar_appearance(ScrollableAppearance::new(ScrollbarWidth::Auto, true))
    });

    (window, editor_view)
}

/// Renders every view currently registered for `window_id` into `presenter`. Used instead of
/// `AppContext::simulate_render_frame` (test-only within `warpui_core` itself, and not visible
/// to this crate) to build a real scene: the same layout/paint pass production code takes,
/// giving accurate painted bounds and letting simulated mouse/keyboard events hit-test and
/// dispatch exactly as they would for a real click or keypress.
fn render_all_views(app: &mut App, window_id: WindowId, presenter: &Rc<RefCell<Presenter>>) {
    let invalidation = WindowInvalidation {
        updated: app
            .read(|ctx| ctx.view_ids_for_window(window_id))
            .into_iter()
            .collect(),
        ..Default::default()
    };
    app.update(|ctx| {
        presenter.borrow_mut().invalidate(invalidation, ctx);
        presenter
            .borrow_mut()
            .build_scene(vec2f(800., 600.), 1., None, ctx);
    });
}

/// Renders the window into a fresh presenter and returns it along with the painted bounds of
/// the find input's `SavePosition` (`FIND_INPUT_POSITION_ID`), so a test can click at a real,
/// on-screen position.
fn render_and_find_input_bounds(
    app: &mut App,
    window_id: WindowId,
) -> (Rc<RefCell<Presenter>>, RectF) {
    let presenter = Rc::new(RefCell::new(Presenter::new(window_id)));
    render_all_views(app, window_id, &presenter);
    let bounds = presenter
        .borrow()
        .position_cache()
        .get_position(FIND_INPUT_POSITION_ID)
        .expect("find input should have a saved position after rendering");
    (presenter, bounds)
}

/// Simulates a real left click (mouse-down then mouse-up) at `position`, going through the same
/// hit-testing and event-dispatch path a user click takes.
fn click_at(
    app: &mut App,
    window_id: WindowId,
    presenter: &Rc<RefCell<Presenter>>,
    position: Vector2F,
) {
    app.update(|ctx| {
        ctx.simulate_window_event(
            Event::LeftMouseDown {
                position,
                modifiers: ModifiersState::default(),
                click_count: 1,
                is_first_mouse: false,
            },
            window_id,
            presenter.clone(),
        );
    });
    app.update(|ctx| {
        ctx.simulate_window_event(
            Event::LeftMouseUp {
                position,
                modifiers: ModifiersState::default(),
            },
            window_id,
            presenter.clone(),
        );
    });
}

#[test]
fn test_interaction_state_prevents_editing() {
    App::test((), |mut app| async move {
        let (_window, editor_view) = initialize_editor(&mut app, false);

        let text = editor_view.update(&mut app, |view, ctx| {
            view.handle_action(&CodeEditorViewAction::UserTyped(UserInput::new("abc")), ctx);
            view.text(ctx)
        });

        assert_eq!(text.as_str(), "abc");

        // Set to be only selectable
        editor_view.update(&mut app, |view, ctx| {
            view.set_interaction_state(InteractionState::Selectable, ctx);
        });

        let text = editor_view.update(&mut app, |view, ctx| {
            view.handle_action(&CodeEditorViewAction::UserTyped(UserInput::new("def")), ctx);
            view.text(ctx)
        });

        assert_eq!(text.as_str(), "abc");
    });
}

/// Regression test for the find bar's input becoming unclickable after Vim's Enter handling
/// moves focus back to the main editor and disables it. A real click (simulated mouse-down and
/// mouse-up at the input's painted bounds) must reclaim focus and re-enable editing, mirroring
/// what Cmd-F already does, and the field must actually receive subsequent typed input.
#[test]
fn test_click_on_disabled_find_input_reclaims_focus_after_vim_enter() {
    let _feature_flag_guard = FeatureFlag::VimCodeEditor.override_enabled(true);

    App::test((), |mut app| async move {
        let (window_id, editor_view) = initialize_editor(&mut app, true);

        editor_view.update(&mut app, |view, ctx| {
            view.handle_action(&CodeEditorViewAction::ShowFindBar, ctx);
        });

        let find_bar = editor_view
            .read(&app, |view, _ctx| view.find_bar.clone())
            .expect("find bar should be available");

        find_bar.update(&mut app, |find_bar, ctx| {
            find_bar.set_find_query(ctx, "abc");
        });

        let find_editor = find_bar.read(&app, |find_bar, _ctx| find_bar.find_editor_for_test());

        // Simulate pressing Enter in the find input. In Vim mode, this commits the query,
        // disables the input, and shifts focus back to the main editor.
        find_editor.update(&mut app, |editor, ctx| {
            editor.handle_action(&EditorAction::Enter, ctx);
        });

        assert!(!find_bar.read(&app, |find_bar, ctx| find_bar.is_find_input_editable(ctx)));
        assert!(find_bar.read(&app, |_, ctx| editor_view.is_focused(ctx)));

        // Click the find input for real: render a frame so its bounds are known, then dispatch
        // a LeftMouseDown/Up pair at its center, exactly the path a user click takes. If this
        // wiring were absent (or wired to the wrong element), the click would be swallowed by
        // the disabled editor and none of the following assertions would hold.
        let (presenter, bounds) = render_and_find_input_bounds(&mut app, window_id);
        click_at(&mut app, window_id, &presenter, bounds.center());

        assert!(
            find_bar.read(&app, |find_bar, ctx| find_bar.is_find_input_editable(ctx)),
            "clicking the find input should re-enable it"
        );
        assert!(
            find_bar.read(&app, |_, ctx| find_editor.is_focused(ctx)),
            "clicking the find input should focus it"
        );
        assert_eq!(
            find_editor.read(&app, |editor, ctx| editor.buffer_text(ctx)),
            "abc",
            "the query text should survive the click"
        );

        // Re-render so each editor's cached focus snapshot reflects the click's new focus state
        // (`typed_characters` dispatch routes by that snapshot), then verify the real
        // user-visible outcome: typed characters must now land in the find input rather than
        // being routed to Vim on the main editor. The input selects all its text on focus
        // (matching Cmd-F), so the typed character replaces "abc" rather than appending to it.
        render_all_views(&mut app, window_id, &presenter);
        app.update(|ctx| {
            ctx.simulate_window_event(
                Event::TypedCharacters {
                    chars: "!".to_string(),
                },
                window_id,
                presenter.clone(),
            );
        });
        assert_eq!(
            find_editor.read(&app, |editor, ctx| editor.buffer_text(ctx)),
            "!",
            "typing after the click should go to the find input"
        );
        assert_eq!(
            editor_view.read(&app, |view, ctx| view.text(ctx).into_string()),
            "",
            "typing after the click should not reach the main editor"
        );
    });
}

/// Regression test for the same disabled-and-unclickable find input left behind by Vim's
/// `search_word_at_cursor` (the `*`/`#` word-search commands), which disables the input without
/// ever having focused it in the first place.
#[test]
fn test_click_on_disabled_find_input_reclaims_focus_after_search_word_at_cursor() {
    let _feature_flag_guard = FeatureFlag::VimCodeEditor.override_enabled(true);

    App::test((), |mut app| async move {
        let (window_id, editor_view) = initialize_editor(&mut app, true);

        editor_view.update(&mut app, |view, ctx| {
            view.handle_action(
                &CodeEditorViewAction::UserTyped(UserInput::new("hello world")),
                ctx,
            );
            view.handle_action(&CodeEditorViewAction::CursorAtBufferStart, ctx);
            // Vim's "*" searches forward for the word under the cursor.
            view.handle_action(
                &CodeEditorViewAction::VimUserTyped(UserInput::new("*")),
                ctx,
            );
        });

        let find_bar = editor_view
            .read(&app, |view, _ctx| view.find_bar.clone())
            .expect("find bar should be available");
        let find_editor = find_bar.read(&app, |find_bar, _ctx| find_bar.find_editor_for_test());

        assert!(find_bar.read(&app, |find_bar, _ctx| find_bar.is_open()));
        assert!(!find_bar.read(&app, |find_bar, ctx| find_bar.is_find_input_editable(ctx)));
        assert_eq!(
            find_editor.read(&app, |editor, ctx| editor.buffer_text(ctx)),
            "hello"
        );

        let (presenter, bounds) = render_and_find_input_bounds(&mut app, window_id);
        click_at(&mut app, window_id, &presenter, bounds.center());

        assert!(
            find_bar.read(&app, |find_bar, ctx| find_bar.is_find_input_editable(ctx)),
            "clicking the find input should re-enable it"
        );
        assert!(
            find_bar.read(&app, |_, ctx| find_editor.is_focused(ctx)),
            "clicking the find input should focus it"
        );
        assert_eq!(
            find_editor.read(&app, |editor, ctx| editor.buffer_text(ctx)),
            "hello",
            "the searched word should survive the click"
        );
    });
}
