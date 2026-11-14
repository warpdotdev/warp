use warpui::App;

use super::tests::{initialize_app, mock_workspace};
use super::*;
use crate::editor::EditorView;
use crate::pane_group::SettingsPane;

fn visible_settings(
    workspace: &ViewHandle<Workspace>,
    app: &App,
) -> (PaneViewLocator, ViewHandle<SettingsView>) {
    workspace.read(app, |workspace, ctx| {
        let locator = SettingsPaneManager::as_ref(ctx)
            .find_pane(workspace.window_id)
            .expect("a live Settings pane should be registered");
        let group = workspace
            .get_pane_group_view_with_id(locator.pane_group_id)
            .expect("Settings should belong to a tab in this window")
            .as_ref(ctx);
        assert!(!group.is_pane_hidden_for_close(locator.pane_id));
        let settings = group
            .downcast_pane_by_id::<SettingsPane>(locator.pane_id)
            .unwrap()
            .settings_view(ctx);
        (locator, settings)
    })
}

fn detach_tab(
    app: &mut App,
    source: &ViewHandle<Workspace>,
    tab_index: usize,
) -> ViewHandle<Workspace> {
    let _skip_login = FeatureFlag::SkipFirebaseAnonymousUser.override_enabled(true);
    let _force_login = FeatureFlag::ForceLogin.override_enabled(false);
    let _onboarding = FeatureFlag::AgentOnboarding.override_enabled(false);
    source.update(app, |workspace, ctx| {
        let transferred = workspace.get_tab_transfer_info(tab_index, ctx).unwrap();
        workspace.prepare_for_transferred_tab_attach(&transferred.pane_group, ctx);
        let window_id = crate::root_view::create_transferred_window(
            transferred,
            ctx.window_id(),
            vec2f(1000., 600.),
            Vector2F::zero(),
            false,
            ctx,
        );
        workspace.remove_tab_without_undo(tab_index, ctx);
        workspace.set_suppress_detach_panes_on_window_close(false);
        WorkspaceRegistry::as_ref(ctx).get(window_id, ctx).unwrap()
    })
}

fn handoff_tab(
    app: &mut App,
    source: &ViewHandle<Workspace>,
    target: &ViewHandle<Workspace>,
    tab_index: usize,
) -> ViewHandle<PaneGroup> {
    let target_window = app.read(|ctx| target.window_id(ctx));
    let insertion_index = target.read(app, |workspace, _| workspace.tab_count());
    source.update(app, |workspace, ctx| {
        let transferred = workspace.get_tab_transfer_info(tab_index, ctx).unwrap();
        let pane_group = transferred.pane_group.clone();
        let source_window = ctx.window_id();
        workspace.prepare_for_transferred_tab_attach(&pane_group, ctx);
        CrossWindowTabDrag::handle(ctx).update(ctx, |drag, ctx| {
            drag.begin_single_tab_drag(
                source_window,
                Vector2F::zero(),
                vec2f(1000., 600.),
                Vector2F::zero(),
                false,
                vec2f(120., 34.),
            );
            drag.execute_handoff_single_tab_to_other(
                AttachTarget {
                    window_id: target_window,
                    insertion_index,
                },
                transferred,
                source_window,
                ctx,
            );
        });
        workspace.remove_tab_without_undo(tab_index, ctx);
        workspace.set_suppress_detach_panes_on_window_close(false);
        pane_group
    })
}

fn undo_close(app: &mut App) {
    app.update(|ctx| {
        UndoCloseStack::handle(ctx).update(ctx, |stack, ctx| stack.undo_close(ctx));
    });
}

#[test]
fn settings_transfer_source_reopens_after_destination_window_close() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let source = mock_workspace(&mut app);
        source.update(&mut app, |workspace, ctx| {
            workspace.show_settings_with_section(Some(SettingsSection::BillingAndUsage), ctx);
        });
        let (_, moved_settings) = visible_settings(&source, &app);

        let destination = detach_tab(&mut app, &source, 1);
        let destination_window = app.read(|ctx| destination.window_id(ctx));
        assert_eq!(
            app.read(|ctx| moved_settings.window_id(ctx)),
            destination_window
        );
        app.update(|ctx| ctx.simulate_window_closed(destination_window));
        source.update(&mut app, |workspace, ctx| {
            workspace.handle_action(&WorkspaceAction::ShowSettings, ctx);
            workspace.handle_action(
                &WorkspaceAction::ShowSettingsPage(SettingsSection::Appearance),
                ctx,
            );
        });

        let (_, reopened_settings) = visible_settings(&source, &app);
        assert_ne!(reopened_settings, moved_settings);
        app.read(|ctx| assert_eq!(reopened_settings.window_id(ctx), source.window_id(ctx)));
        assert_eq!(
            reopened_settings.read(&app, |settings, _| settings.current_settings_section()),
            SettingsSection::Appearance
        );
        assert_eq!(source.read(&app, |workspace, _| workspace.tab_count()), 2);
        source.read(&app, |workspace, ctx| {
            assert_eq!(workspace.settings_pane, reopened_settings);
            assert_eq!(
                SettingsPaneManager::as_ref(ctx).settings_view(workspace.window_id),
                reopened_settings
            );
        });
    });
}

#[test]
fn settings_transfer_navigation_and_events_follow_repeated_handoffs() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let source = mock_workspace(&mut app);
        let destination = mock_workspace(&mut app);
        source.update(&mut app, |workspace, ctx| workspace.show_settings(ctx));
        let (locator, settings) = visible_settings(&source, &app);

        handoff_tab(&mut app, &source, &destination, 1);
        assert_eq!(
            visible_settings(&destination, &app),
            (locator, settings.clone())
        );
        destination.update(&mut app, |workspace, ctx| {
            workspace.handle_action(
                &WorkspaceAction::ShowSettingsPageWithSearch {
                    search_query: "cursor".to_owned(),
                    section: Some(SettingsSection::Appearance),
                },
                ctx,
            );
        });
        assert_eq!(
            settings.read(&app, |settings, _| settings.current_settings_section()),
            SettingsSection::Appearance
        );
        app.read(|ctx| {
            assert!(ctx.view_descendants(destination.window_id(ctx), settings.id())
                .iter()
                .filter_map(|id| ctx.view_with_id::<EditorView>(destination.window_id(ctx), *id))
                .any(|editor| editor.as_ref(ctx).buffer_text(ctx) == "cursor"));
            assert_eq!(
                SettingsPaneManager::as_ref(ctx).find_pane(source.window_id(ctx)),
                None
            );
        });
        settings.update(&mut app, |_, ctx| {
            ctx.emit(SettingsViewEvent::OpenMCPServerCollection)
        });
        assert_eq!(source.read(&app, |workspace, _| workspace.tab_count()), 1);
        assert_eq!(
            settings.read(&app, |settings, _| settings.current_settings_section()),
            SettingsSection::AgentMCPServers
        );

        handoff_tab(&mut app, &destination, &source, 1);
        assert_eq!(visible_settings(&source, &app), (locator, settings.clone()));
        settings.update(&mut app, |_, ctx| {
            ctx.emit(SettingsViewEvent::OpenMCPServerCollection)
        });
        assert_eq!(
            destination.read(&app, |workspace, _| workspace.tab_count()),
            1
        );
        destination.update(&mut app, |workspace, ctx| {
            workspace.show_settings_with_section(Some(SettingsSection::BillingAndUsage), ctx);
        });
        assert_ne!(visible_settings(&destination, &app).1, settings);
    });
}

#[test]
fn settings_transfer_collision_discards_the_incoming_settings_only_tab() {
    let _undo = FeatureFlag::UndoClosedPanes.override_enabled(true);
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let source = mock_workspace(&mut app);
        let destination = mock_workspace(&mut app);
        source.update(&mut app, |workspace, ctx| workspace.show_settings(ctx));
        destination.update(&mut app, |workspace, ctx| {
            workspace.show_settings_with_section(Some(SettingsSection::Appearance), ctx);
        });
        let existing = visible_settings(&destination, &app);

        handoff_tab(&mut app, &source, &destination, 1);

        assert_eq!(
            destination.read(&app, |workspace, _| workspace.tab_count()),
            2
        );
        assert_eq!(visible_settings(&destination, &app), existing);
        assert!(app.read(|ctx| UndoCloseStack::as_ref(ctx).is_empty()));
        destination.read(&app, |workspace, ctx| {
            assert_eq!(
                workspace.active_tab_pane_group().id(),
                existing.0.pane_group_id
            );
            assert_eq!(workspace.settings_pane, existing.1);
            assert_eq!(
                existing.1.as_ref(ctx).current_settings_section(),
                SettingsSection::Appearance
            );
        });
    });
}

#[test]
fn settings_transfer_collision_preserves_other_panes_without_undoing_the_duplicate() {
    let _undo = FeatureFlag::UndoClosedPanes.override_enabled(true);
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let source = mock_workspace(&mut app);
        let destination = mock_workspace(&mut app);
        source.update(&mut app, |workspace, ctx| {
            workspace.show_settings(ctx);
            workspace.active_tab_pane_group().update(ctx, |group, ctx| {
                group.add_terminal_pane(Direction::Right, None, ctx);
            });
        });
        let incoming = visible_settings(&source, &app).0;
        source.update(&mut app, |workspace, ctx| {
            workspace.focus_pane(incoming, ctx)
        });
        destination.update(&mut app, |workspace, ctx| workspace.show_settings(ctx));
        let existing = visible_settings(&destination, &app);

        let transferred = handoff_tab(&mut app, &source, &destination, 1);

        transferred.read(&app, |group, ctx| {
            assert_eq!(group.pane_count(), 1);
            assert!(!group.has_pane_id(incoming.pane_id));
            assert!(group.has_pane_id(group.focused_pane_id(ctx)));
        });
        assert_eq!(
            destination.read(&app, |workspace, _| workspace.tab_count()),
            3
        );
        assert_eq!(visible_settings(&destination, &app), existing);
        assert!(app.read(|ctx| UndoCloseStack::as_ref(ctx).is_empty()));
    });
}

#[test]
fn settings_transfer_hidden_pane_is_adopted_only_when_undo_restores_it() {
    let _undo = FeatureFlag::UndoClosedPanes.override_enabled(true);
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let source = mock_workspace(&mut app);
        let destination = mock_workspace(&mut app);
        source.update(&mut app, |workspace, ctx| workspace.show_settings(ctx));
        let (locator, settings) = visible_settings(&source, &app);
        source.update(&mut app, |workspace, ctx| {
            workspace.active_tab_pane_group().update(ctx, |group, ctx| {
                group.add_terminal_pane(Direction::Right, None, ctx);
                group.close_pane(locator.pane_id, ctx);
            });
        });

        let transferred = handoff_tab(&mut app, &source, &destination, 1);
        app.read(|ctx| {
            assert_eq!(
                SettingsPaneManager::as_ref(ctx).find_pane(source.window_id(ctx)),
                None
            );
            assert_eq!(
                SettingsPaneManager::as_ref(ctx).find_pane(destination.window_id(ctx)),
                None
            );
        });
        assert!(transferred.read(&app, |group, _| {
            group.is_pane_hidden_for_close(locator.pane_id)
        }));
        app.read(|ctx| assert_eq!(settings.window_id(ctx), destination.window_id(ctx)));

        undo_close(&mut app);
        assert_eq!(
            visible_settings(&destination, &app),
            (locator, settings.clone())
        );
        destination.update(&mut app, |workspace, ctx| {
            workspace.show_settings_with_section(Some(SettingsSection::Appearance), ctx);
        });
        assert_eq!(
            settings.read(&app, |view, _| view.current_settings_section()),
            SettingsSection::Appearance
        );

        transferred.update(&mut app, |group, ctx| {
            group.close_pane(locator.pane_id, ctx)
        });
        destination.update(&mut app, |workspace, ctx| workspace.show_settings(ctx));
        let existing = visible_settings(&destination, &app);
        undo_close(&mut app);
        assert_eq!(visible_settings(&destination, &app), existing);
        assert!(!transferred.read(&app, |group, _| group.has_pane_id(locator.pane_id)));
        destination.read(&app, |workspace, _| {
            assert_eq!(
                workspace.active_tab_pane_group().id(),
                existing.0.pane_group_id
            );
        });
    });
}

#[test]
fn settings_transfer_discards_only_hidden_panes_aliasing_live_source_settings() {
    let _undo = FeatureFlag::UndoClosedPanes.override_enabled(true);
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let source = mock_workspace(&mut app);
        let destination = mock_workspace(&mut app);
        source.update(&mut app, |workspace, ctx| workspace.show_settings(ctx));
        let (hidden, settings) = visible_settings(&source, &app);
        source.update(&mut app, |workspace, ctx| {
            workspace.active_tab_pane_group().update(ctx, |group, ctx| {
                group.add_terminal_pane(Direction::Right, None, ctx);
                group.close_pane(hidden.pane_id, ctx);
            });
            workspace.show_settings_with_section(Some(SettingsSection::Appearance), ctx);
        });
        let live = visible_settings(&source, &app);
        assert_eq!(live.1, settings);

        let transferred = handoff_tab(&mut app, &source, &destination, 1);

        assert!(!transferred.read(&app, |group, _| group.has_pane_id(hidden.pane_id)));
        assert_eq!(visible_settings(&source, &app), live);
        app.read(|ctx| assert_eq!(settings.window_id(ctx), source.window_id(ctx)));
        assert!(app.read(|ctx| UndoCloseStack::as_ref(ctx).is_empty()));
        undo_close(&mut app);
        assert!(!transferred.read(&app, |group, _| group.has_pane_id(hidden.pane_id)));
        app.update(|ctx| ctx.simulate_window_closed(destination.window_id(ctx)));
        source.update(&mut app, |workspace, ctx| {
            workspace.show_settings_with_section(Some(SettingsSection::BillingAndUsage), ctx);
        });
        assert_eq!(visible_settings(&source, &app), live);
        assert_eq!(
            settings.read(&app, |view, _| view.current_settings_section()),
            SettingsSection::BillingAndUsage
        );
    });
}

#[test]
fn settings_transfer_preserves_same_window_close_reopen_and_tab_undo() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let workspace = mock_workspace(&mut app);
        workspace.update(&mut app, |workspace, ctx| workspace.show_settings(ctx));
        let original = visible_settings(&workspace, &app);

        workspace.update(&mut app, |workspace, ctx| {
            workspace.remove_tab(1, true, true, ctx)
        });
        undo_close(&mut app);
        assert_eq!(visible_settings(&workspace, &app), original);
        workspace.update(&mut app, |workspace, ctx| {
            workspace.remove_tab(1, false, true, ctx);
            workspace.show_settings_with_section(Some(SettingsSection::Appearance), ctx);
        });
        let reopened = visible_settings(&workspace, &app);
        assert_ne!(reopened.0, original.0);
        assert_eq!(reopened.1, original.1);
        assert_eq!(
            reopened
                .1
                .read(&app, |view, _| view.current_settings_section()),
            SettingsSection::Appearance
        );
    });
}

#[test]
fn settings_transfer_tab_undo_collision_keeps_destination_singleton() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let source = mock_workspace(&mut app);
        let destination = mock_workspace(&mut app);
        source.update(&mut app, |workspace, ctx| workspace.show_settings(ctx));
        handoff_tab(&mut app, &source, &destination, 1);
        destination.update(&mut app, |workspace, ctx| {
            workspace.remove_tab(1, true, true, ctx);
            workspace.show_settings_with_section(Some(SettingsSection::Appearance), ctx);
        });
        let existing = visible_settings(&destination, &app);

        undo_close(&mut app);

        assert_eq!(visible_settings(&destination, &app), existing);
        destination.read(&app, |workspace, ctx| {
            assert_eq!(workspace.tab_count(), 2);
            assert_eq!(
                workspace.active_tab_pane_group().id(),
                existing.0.pane_group_id
            );
            assert_eq!(workspace.settings_pane, existing.1);
            assert_eq!(
                existing.1.as_ref(ctx).current_settings_section(),
                SettingsSection::Appearance
            );
        });
        assert!(app.read(|ctx| UndoCloseStack::as_ref(ctx).is_empty()));
    });
}
