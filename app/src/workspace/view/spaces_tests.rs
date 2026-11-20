use warp_core::features::FeatureFlag;
use warpui::{App, TypedActionView as _};

use crate::workspace::WorkspaceAction;
use crate::workspace::view::tests::{initialize_app, mock_workspace};

#[test]
fn test_switch_space_restores_each_spaces_active_tab() {
    let _spaces_guard = FeatureFlag::Spaces.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let workspace = mock_workspace(&mut app);
        workspace.update(&mut app, |workspace, ctx| {
            workspace.add_terminal_tab(false, ctx);
            workspace.add_terminal_tab(false, ctx);
            workspace.activate_tab(1, ctx);
            let first_space_active_tab = workspace.active_tab_pane_group().id();

            workspace.handle_action(&WorkspaceAction::NewSpace, ctx);
            assert_eq!(workspace.spaces.len(), 2);
            assert_eq!(workspace.active_space, workspace.spaces[1].id);
            // Switching into an empty space opens a tab there.
            assert_eq!(workspace.tab_count(), 1);

            workspace.add_terminal_tab(false, ctx);
            workspace.activate_tab(0, ctx);
            let second_space_active_tab = workspace.active_tab_pane_group().id();

            workspace.handle_action(&WorkspaceAction::SwitchToSpace(1), ctx);
            assert_eq!(workspace.active_space, workspace.spaces[0].id);
            assert_eq!(workspace.tab_count(), 3);
            assert_eq!(workspace.active_tab_index(), 1);
            assert_eq!(
                workspace.active_tab_pane_group().id(),
                first_space_active_tab
            );

            workspace.handle_action(&WorkspaceAction::SwitchToSpace(2), ctx);
            assert_eq!(workspace.tab_count(), 2);
            assert_eq!(workspace.active_tab_index(), 0);
            assert_eq!(
                workspace.active_tab_pane_group().id(),
                second_space_active_tab
            );

            // Hidden tabs are kept, not closed.
            assert_eq!(workspace.all_tabs().count(), 5);
        });
    });
}

#[test]
fn test_new_tab_lands_in_active_space() {
    let _spaces_guard = FeatureFlag::Spaces.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let workspace = mock_workspace(&mut app);
        workspace.update(&mut app, |workspace, ctx| {
            let first_space = workspace.active_space;
            workspace.handle_action(&WorkspaceAction::NewSpace, ctx);
            let second_space = workspace.active_space;

            workspace.handle_action(&WorkspaceAction::AddDefaultTab, ctx);
            workspace.handle_action(
                &WorkspaceAction::AddTerminalTab {
                    hide_homepage: false,
                },
                ctx,
            );

            let tabs_in = |space| {
                workspace
                    .all_tabs()
                    .filter(|(tab_space, _)| *tab_space == space)
                    .count()
            };
            assert_eq!(tabs_in(first_space), 1);
            assert_eq!(tabs_in(second_space), 3);
        });
    });
}

#[test]
fn test_next_and_prev_space_do_not_wrap() {
    let _spaces_guard = FeatureFlag::Spaces.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let workspace = mock_workspace(&mut app);
        workspace.update(&mut app, |workspace, ctx| {
            workspace.handle_action(&WorkspaceAction::NewSpace, ctx);
            workspace.handle_action(&WorkspaceAction::NewSpace, ctx);
            assert_eq!(workspace.active_space_index(), 2);

            workspace.handle_action(&WorkspaceAction::NextSpace, ctx);
            assert_eq!(workspace.active_space_index(), 2);

            workspace.handle_action(&WorkspaceAction::PrevSpace, ctx);
            assert_eq!(workspace.active_space_index(), 1);
            workspace.handle_action(&WorkspaceAction::PrevSpace, ctx);
            assert_eq!(workspace.active_space_index(), 0);
            workspace.handle_action(&WorkspaceAction::PrevSpace, ctx);
            assert_eq!(workspace.active_space_index(), 0);

            workspace.handle_action(&WorkspaceAction::NextSpace, ctx);
            assert_eq!(workspace.active_space_index(), 1);
        });
    });
}

#[test]
fn test_switch_to_out_of_range_space_is_noop() {
    let _spaces_guard = FeatureFlag::Spaces.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let workspace = mock_workspace(&mut app);
        workspace.update(&mut app, |workspace, ctx| {
            workspace.handle_action(&WorkspaceAction::NewSpace, ctx);
            let active_space = workspace.active_space;
            let active_tab = workspace.active_tab_pane_group().id();

            for number in [0, 3, 9] {
                workspace.handle_action(&WorkspaceAction::SwitchToSpace(number), ctx);
                assert_eq!(workspace.active_space, active_space);
                assert_eq!(workspace.active_tab_pane_group().id(), active_tab);
            }
            assert_eq!(workspace.spaces.len(), 2);
        });
    });
}

#[test]
fn test_space_actions_are_noops_with_flag_disabled() {
    let _spaces_guard = FeatureFlag::Spaces.override_enabled(false);

    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let workspace = mock_workspace(&mut app);
        workspace.update(&mut app, |workspace, ctx| {
            let active_space = workspace.active_space;
            workspace.handle_action(&WorkspaceAction::NewSpace, ctx);
            assert_eq!(workspace.spaces.len(), 1);
            assert_eq!(workspace.active_space, active_space);
            assert_eq!(workspace.tab_count(), 1);
        });
    });
}

#[test]
fn test_remove_stashed_tab_keeps_hidden_space_consistent() {
    let _spaces_guard = FeatureFlag::Spaces.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let workspace = mock_workspace(&mut app);
        workspace.update(&mut app, |workspace, ctx| {
            workspace.add_terminal_tab(false, ctx);
            assert_eq!(workspace.active_tab_index(), 1);
            let exited_tab = workspace.active_tab_pane_group().id();
            let remaining_tab = workspace.tabs[0].pane_group.id();

            workspace.handle_action(&WorkspaceAction::NewSpace, ctx);
            workspace.remove_stashed_tab(exited_tab, ctx);

            workspace.handle_action(&WorkspaceAction::SwitchToSpace(1), ctx);
            assert_eq!(workspace.tab_count(), 1);
            assert_eq!(workspace.active_tab_pane_group().id(), remaining_tab);
        });
    });
}
