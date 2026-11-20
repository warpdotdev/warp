use warp_core::features::FeatureFlag;
use warpui::{App, EntityId, SingletonEntity as _, TypedActionView as _};

use crate::appearance::Appearance;
use crate::menu::MenuItem;
use crate::tab::SelectedTabColor;
use crate::ui_components::color_dot::TAB_COLOR_OPTIONS;
use crate::workspace::WorkspaceAction;
use crate::workspace::space::SpaceId;
use crate::workspace::tab_group::{TabGroup, TabGroupId};
use crate::workspace::view::Workspace;
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

/// Puts the live tabs at `indices` (contiguous) into a new tab group.
fn group_live_tabs(workspace: &mut Workspace, indices: &[usize]) -> TabGroupId {
    let group = TabGroup::new();
    let group_id = group.id;
    workspace.tab_groups.insert(group_id, group);
    for &index in indices {
        workspace.tabs[index].group_id = Some(group_id);
    }
    group_id
}

fn live_tab_ids(workspace: &Workspace) -> Vec<EntityId> {
    workspace
        .tabs
        .iter()
        .map(|tab| tab.pane_group.id())
        .collect()
}

fn tab_ids_in(workspace: &Workspace, space: SpaceId) -> Vec<EntityId> {
    workspace
        .all_tabs()
        .filter(|(tab_space, _)| *tab_space == space)
        .map(|(_, tab)| tab.pane_group.id())
        .collect()
}

#[test]
fn test_delete_active_space_moves_tabs_to_previous_space() {
    let _spaces_guard = FeatureFlag::Spaces.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let workspace = mock_workspace(&mut app);
        workspace.update(&mut app, |workspace, ctx| {
            let first_space = workspace.active_space;
            workspace.add_terminal_tab(false, ctx);
            workspace.activate_tab(0, ctx);
            let first_space_tabs = live_tab_ids(workspace);

            workspace.handle_action(&WorkspaceAction::NewSpace, ctx);
            let deleted_space = workspace.active_space;
            workspace.add_terminal_tab(false, ctx);
            let group_id = group_live_tabs(workspace, &[0, 1]);
            let deleted_space_tabs = live_tab_ids(workspace);
            let views = workspace
                .tabs
                .iter()
                .map(|tab| tab.pane_group.downgrade())
                .collect::<Vec<_>>();

            workspace.handle_action(&WorkspaceAction::DeleteActiveSpace, ctx);

            assert_eq!(workspace.spaces.len(), 1);
            assert_eq!(workspace.active_space, first_space);
            assert!(
                !workspace
                    .spaces
                    .iter()
                    .any(|space| space.id == deleted_space)
            );
            // The neighbor's own active tab is restored, and the moved tabs follow its tabs.
            assert_eq!(workspace.active_tab_index(), 0);
            assert_eq!(
                live_tab_ids(workspace),
                [first_space_tabs, deleted_space_tabs].concat()
            );
            assert!(workspace.tab_groups.contains_key(&group_id));
            assert_eq!(workspace.tabs[2].group_id, Some(group_id));
            assert_eq!(workspace.tabs[3].group_id, Some(group_id));
            // No session was closed.
            assert!(views.iter().all(|view| view.upgrade(ctx).is_some()));
        });
    });
}

#[test]
fn test_delete_first_space_moves_tabs_to_next_space() {
    let _spaces_guard = FeatureFlag::Spaces.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let workspace = mock_workspace(&mut app);
        workspace.update(&mut app, |workspace, ctx| {
            let first_space = workspace.active_space;
            let first_space_tabs = live_tab_ids(workspace);
            workspace.handle_action(&WorkspaceAction::NewSpace, ctx);
            let second_space = workspace.active_space;
            let second_space_tabs = live_tab_ids(workspace);

            workspace.handle_action(&WorkspaceAction::DeleteSpace(first_space), ctx);

            assert_eq!(workspace.spaces.len(), 1);
            assert_eq!(workspace.active_space, second_space);
            assert_eq!(
                live_tab_ids(workspace),
                [second_space_tabs, first_space_tabs].concat()
            );
            assert_eq!(
                workspace.active_tab_pane_group().id(),
                workspace.tabs[0].pane_group.id()
            );
        });
    });
}

#[test]
fn test_last_space_cannot_be_deleted() {
    let _spaces_guard = FeatureFlag::Spaces.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let workspace = mock_workspace(&mut app);
        workspace.update(&mut app, |workspace, ctx| {
            let space = workspace.active_space;
            let tabs = live_tab_ids(workspace);

            workspace.handle_action(&WorkspaceAction::DeleteActiveSpace, ctx);
            workspace.handle_action(&WorkspaceAction::DeleteSpace(space), ctx);

            assert_eq!(workspace.spaces.len(), 1);
            assert_eq!(workspace.active_space, space);
            assert_eq!(live_tab_ids(workspace), tabs);

            let terminal_colors = Appearance::as_ref(ctx).theme().terminal_colors().normal;
            let labels = workspace
                .space_menu_items(space, terminal_colors)
                .into_iter()
                .filter_map(|item| match item {
                    MenuItem::Item(fields) => Some(fields.label().to_string()),
                    _ => None,
                })
                .collect::<Vec<_>>();
            assert!(labels.contains(&"Rename space".to_string()));
            assert!(!labels.contains(&"Delete space".to_string()));
        });
    });
}

#[test]
fn test_move_tab_to_space_keeps_active_space() {
    let _spaces_guard = FeatureFlag::Spaces.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let workspace = mock_workspace(&mut app);
        workspace.update(&mut app, |workspace, ctx| {
            let first_space = workspace.active_space;
            workspace.add_terminal_tab(false, ctx);
            let [moved_tab, active_tab] = live_tab_ids(workspace)[..] else {
                panic!("expected two tabs");
            };
            workspace.handle_action(&WorkspaceAction::NewSpace, ctx);
            let second_space = workspace.active_space;
            let second_space_tabs = live_tab_ids(workspace);
            workspace.handle_action(&WorkspaceAction::SwitchToSpaceById(first_space), ctx);

            workspace.handle_action(
                &WorkspaceAction::MoveTabToSpace {
                    tab_index: 0,
                    space_id: Some(second_space),
                },
                ctx,
            );

            assert_eq!(workspace.active_space, first_space);
            assert_eq!(live_tab_ids(workspace), vec![active_tab]);
            assert_eq!(workspace.active_tab_pane_group().id(), active_tab);
            assert_eq!(
                tab_ids_in(workspace, second_space),
                [second_space_tabs, vec![moved_tab]].concat()
            );
        });
    });
}

#[test]
fn test_move_active_tab_to_new_space_activates_neighbor() {
    let _spaces_guard = FeatureFlag::Spaces.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let workspace = mock_workspace(&mut app);
        workspace.update(&mut app, |workspace, ctx| {
            let first_space = workspace.active_space;
            workspace.add_terminal_tab(false, ctx);
            workspace.add_terminal_tab(false, ctx);
            workspace.activate_tab(1, ctx);
            let [first, moved, next] = live_tab_ids(workspace)[..] else {
                panic!("expected three tabs");
            };

            workspace.handle_action(
                &WorkspaceAction::MoveTabToSpace {
                    tab_index: 1,
                    space_id: None,
                },
                ctx,
            );

            assert_eq!(workspace.spaces.len(), 2);
            assert_eq!(workspace.active_space, first_space);
            assert_eq!(live_tab_ids(workspace), vec![first, next]);
            assert_eq!(workspace.active_tab_pane_group().id(), next);
            assert_eq!(tab_ids_in(workspace, workspace.spaces[1].id), vec![moved]);
        });
    });
}

#[test]
fn test_move_tab_group_to_space_moves_members_and_group() {
    let _spaces_guard = FeatureFlag::Spaces.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let workspace = mock_workspace(&mut app);
        workspace.update(&mut app, |workspace, ctx| {
            let first_space = workspace.active_space;
            workspace.add_terminal_tab(false, ctx);
            workspace.add_terminal_tab(false, ctx);
            let group_id = group_live_tabs(workspace, &[0, 1]);
            let [member_a, member_b, kept] = live_tab_ids(workspace)[..] else {
                panic!("expected three tabs");
            };

            workspace.handle_action(
                &WorkspaceAction::MoveTabGroupToSpace {
                    group_id,
                    space_id: None,
                },
                ctx,
            );

            assert_eq!(workspace.active_space, first_space);
            assert_eq!(live_tab_ids(workspace), vec![kept]);
            assert!(!workspace.tab_groups.contains_key(&group_id));

            let new_space = workspace.spaces[1].id;
            workspace.handle_action(&WorkspaceAction::SwitchToSpaceById(new_space), ctx);
            assert_eq!(live_tab_ids(workspace), vec![member_a, member_b]);
            assert!(workspace.tab_groups.contains_key(&group_id));
            assert!(
                workspace
                    .tabs
                    .iter()
                    .all(|tab| tab.group_id == Some(group_id))
            );
        });
    });
}

#[test]
fn test_move_selection_to_space_keeps_whole_groups_together() {
    let _spaces_guard = FeatureFlag::Spaces.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let workspace = mock_workspace(&mut app);
        workspace.update(&mut app, |workspace, ctx| {
            let first_space = workspace.active_space;
            for _ in 0..4 {
                workspace.add_terminal_tab(false, ctx);
            }
            workspace.handle_action(&WorkspaceAction::NewSpace, ctx);
            let target_space = workspace.active_space;
            workspace.handle_action(&WorkspaceAction::SwitchToSpaceById(first_space), ctx);
            workspace.activate_tab(0, ctx);

            // [a, b] form a fully selected group; [c, d] a group of which only `c` is selected.
            let whole_group = group_live_tabs(workspace, &[0, 1]);
            let split_group = group_live_tabs(workspace, &[2, 3]);
            let [a, b, c, d, e] = live_tab_ids(workspace)[..] else {
                panic!("expected five tabs");
            };
            for index in [1, 2] {
                workspace.tabs[index].in_multi_selection = true;
            }
            assert_eq!(workspace.selected_tab_indices(), vec![0, 1, 2]);

            workspace.handle_action(
                &WorkspaceAction::MoveSelectedTabsToSpace {
                    space_id: Some(target_space),
                },
                ctx,
            );

            assert_eq!(workspace.active_space, first_space);
            assert_eq!(live_tab_ids(workspace), vec![d, e]);
            assert_eq!(workspace.active_tab_pane_group().id(), d);
            assert_eq!(workspace.tabs[0].group_id, Some(split_group));
            assert!(workspace.tab_groups.contains_key(&split_group));
            assert!(!workspace.tab_groups.contains_key(&whole_group));
            assert!(workspace.tabs.iter().all(|tab| !tab.in_multi_selection));

            workspace.handle_action(&WorkspaceAction::SwitchToSpaceById(target_space), ctx);
            assert_eq!(
                workspace.tabs[1..]
                    .iter()
                    .map(|tab| (tab.pane_group.id(), tab.group_id))
                    .collect::<Vec<_>>(),
                vec![
                    (a, Some(whole_group)),
                    (b, Some(whole_group)),
                    // A member moved without the rest of its group leaves the group.
                    (c, None),
                ]
            );
            assert!(workspace.tab_groups.contains_key(&whole_group));
            assert!(!workspace.tab_groups.contains_key(&split_group));
        });
    });
}

#[test]
fn test_moving_every_tab_away_switches_to_target_space() {
    let _spaces_guard = FeatureFlag::Spaces.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let workspace = mock_workspace(&mut app);
        workspace.update(&mut app, |workspace, ctx| {
            let first_space = workspace.active_space;
            let moved_tab = workspace.active_tab_pane_group().id();

            workspace.handle_action(
                &WorkspaceAction::MoveTabToSpace {
                    tab_index: 0,
                    space_id: None,
                },
                ctx,
            );

            // The emptied space stays in the switcher.
            assert_eq!(workspace.spaces.len(), 2);
            assert!(workspace.spaces.iter().any(|space| space.id == first_space));
            assert_eq!(workspace.active_space, workspace.spaces[1].id);
            assert_eq!(live_tab_ids(workspace), vec![moved_tab]);
        });
    });
}

#[test]
fn test_closing_last_tab_of_space_switches_to_previous_non_empty_space() {
    let _spaces_guard = FeatureFlag::Spaces.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let workspace = mock_workspace(&mut app);
        let window_id = workspace.update(&mut app, |workspace, ctx| {
            let first_space = workspace.active_space;
            let first_space_tab = workspace.active_tab_pane_group().id();
            workspace.handle_action(&WorkspaceAction::NewSpace, ctx);
            let closed_space = workspace.active_space;
            workspace.handle_action(&WorkspaceAction::NewSpace, ctx);
            workspace.handle_action(&WorkspaceAction::SwitchToSpaceById(closed_space), ctx);

            workspace.close_tab(0, true, false, ctx);

            // Both neighbors have tabs; the previous one wins.
            assert_eq!(workspace.active_space, first_space);
            assert_eq!(live_tab_ids(workspace), vec![first_space_tab]);
            assert_eq!(workspace.spaces.len(), 3);
            assert!(tab_ids_in(workspace, closed_space).is_empty());
            assert_eq!(workspace.all_tabs().count(), 2);
            ctx.window_id()
        });
        app.update(|_| ());
        assert!(app.is_window_open(window_id));
    });
}

#[test]
fn test_closing_last_tab_of_space_skips_empty_neighbors() {
    let _spaces_guard = FeatureFlag::Spaces.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let workspace = mock_workspace(&mut app);
        workspace.update(&mut app, |workspace, ctx| {
            let first_space = workspace.active_space;
            workspace.handle_action(&WorkspaceAction::NewSpace, ctx);
            let middle_space = workspace.active_space;
            workspace.handle_action(&WorkspaceAction::NewSpace, ctx);
            let last_space = workspace.active_space;
            let last_space_tab = workspace.active_tab_pane_group().id();
            workspace.handle_action(&WorkspaceAction::SwitchToSpaceById(first_space), ctx);
            // Empty the first space by moving its only tab into the middle one.
            workspace.handle_action(
                &WorkspaceAction::MoveTabToSpace {
                    tab_index: 0,
                    space_id: Some(middle_space),
                },
                ctx,
            );
            assert_eq!(workspace.active_space, middle_space);
            assert_eq!(workspace.tab_count(), 2);

            workspace.close_tab(1, true, false, ctx);
            workspace.close_tab(0, true, false, ctx);

            assert_eq!(workspace.active_space, last_space);
            assert_eq!(live_tab_ids(workspace), vec![last_space_tab]);
            assert_eq!(workspace.spaces.len(), 3);
        });
    });
}

#[test]
fn test_closing_last_tab_when_other_spaces_are_empty_closes_window() {
    let _spaces_guard = FeatureFlag::Spaces.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let workspace = mock_workspace(&mut app);
        workspace.update(&mut app, |workspace, ctx| {
            workspace.handle_action(
                &WorkspaceAction::MoveTabToSpace {
                    tab_index: 0,
                    space_id: None,
                },
                ctx,
            );
            assert_eq!(workspace.spaces.len(), 2);
            let active_space = workspace.active_space;
            let last_tab = live_tab_ids(workspace);

            workspace.close_tab(0, true, false, ctx);

            // As without spaces: the last tab isn't removed; the window close is requested
            // instead (asynchronously, so the test app keeps the window).
            assert_eq!(workspace.active_space, active_space);
            assert_eq!(live_tab_ids(workspace), last_tab);
        });
    });
}

#[test]
fn test_rename_and_recolor_space() {
    let _spaces_guard = FeatureFlag::Spaces.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let workspace = mock_workspace(&mut app);
        workspace.update(&mut app, |workspace, ctx| {
            let space = workspace.active_space;
            workspace
                .current_workspace_state
                .set_space_being_renamed(space);
            workspace
                .tab_group_rename_editor
                .update(ctx, |editor, ctx| {
                    editor.insert_selected_text("  Work  ", ctx);
                });
            workspace.finish_space_rename(ctx);
            assert_eq!(workspace.spaces[0].name.as_deref(), Some("Work"));
            assert_eq!(
                workspace.current_workspace_state.space_being_renamed(),
                None
            );

            let color = TAB_COLOR_OPTIONS[0];
            let toggle = WorkspaceAction::ToggleSpaceColor {
                color,
                space_id: space,
            };
            workspace.handle_action(&toggle, ctx);
            assert_eq!(workspace.spaces[0].color, SelectedTabColor::Color(color));
            workspace.handle_action(&toggle, ctx);
            assert_eq!(workspace.spaces[0].color, SelectedTabColor::Cleared);
        });
    });
}

#[test]
fn test_space_management_actions_are_noops_with_flag_disabled() {
    let _spaces_guard = FeatureFlag::Spaces.override_enabled(false);

    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let workspace = mock_workspace(&mut app);
        workspace.update(&mut app, |workspace, ctx| {
            workspace.add_terminal_tab(false, ctx);
            let tabs = live_tab_ids(workspace);
            workspace.handle_action(
                &WorkspaceAction::MoveTabToSpace {
                    tab_index: 0,
                    space_id: None,
                },
                ctx,
            );
            workspace.handle_action(&WorkspaceAction::DeleteActiveSpace, ctx);
            assert_eq!(workspace.spaces.len(), 1);
            assert_eq!(live_tab_ids(workspace), tabs);
        });
    });
}
