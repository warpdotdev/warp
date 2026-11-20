use std::time::Duration;

use instant::Instant;
use settings::Setting as _;
use warp_core::features::FeatureFlag;
use warpui::geometry::rect::RectF;
use warpui::geometry::vector::{Vector2F, vec2f};
use warpui::{
    App, AppContext, EntityId, Event, SingletonEntity as _, TypedActionView as _, ViewContext,
    ViewHandle, WindowId,
};

use crate::appearance::Appearance;
use crate::menu::MenuItem;
use crate::report_if_error;
use crate::session_management::SessionNavigationData;
use crate::tab::{SelectedTabColor, TabData, tab_position_id};
use crate::ui_components::color_dot::TAB_COLOR_OPTIONS;
use crate::workspace::WorkspaceAction;
use crate::workspace::space::SpaceId;
use crate::workspace::swipe_detector::IDLE_GAP;
use crate::workspace::tab_group::{TabGroup, TabGroupId};
use crate::workspace::tab_settings::TabSettings;
use crate::workspace::util::PaneViewLocator;
use crate::workspace::view::Workspace;
use crate::workspace::view::spaces::{SpaceChipDrop, space_chip_drag_index};
use crate::workspace::view::tests::{initialize_app, mock_workspace, restored_workspace};
use crate::workspace::view::vertical_tabs::space_chip_position_id;
use crate::workspace::view::vertical_tabs::space_slide::{SlideFrom, SpaceSlide};

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
                    .filter(|(tab_space, ..)| *tab_space == space)
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
        .filter(|(tab_space, ..)| *tab_space == space)
        .map(|(.., tab)| tab.pane_group.id())
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

fn set_active_tab_title(workspace: &Workspace, title: &str, ctx: &mut ViewContext<Workspace>) {
    workspace
        .active_tab_pane_group()
        .update(ctx, |pane_group, ctx| pane_group.set_title(title, ctx));
}

fn add_titled_tab(workspace: &mut Workspace, title: &str, ctx: &mut ViewContext<Workspace>) {
    workspace.add_terminal_tab(false, ctx);
    set_active_tab_title(workspace, title, ctx);
}

fn tab_title(tab: &TabData, ctx: &AppContext) -> String {
    tab.pane_group
        .as_ref(ctx)
        .custom_title(ctx)
        .unwrap_or_default()
}

fn live_tab_titles(workspace: &Workspace, ctx: &AppContext) -> Vec<String> {
    workspace
        .tabs
        .iter()
        .map(|tab| tab_title(tab, ctx))
        .collect()
}

fn tab_titles_in(workspace: &Workspace, space: SpaceId, ctx: &AppContext) -> Vec<String> {
    workspace
        .all_tabs()
        .filter(|(tab_space, ..)| *tab_space == space)
        .map(|(.., tab)| tab_title(tab, ctx))
        .collect()
}

#[test]
fn test_restore_puts_tabs_of_unknown_spaces_in_first_space() {
    let _spaces_guard = FeatureFlag::Spaces.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let workspace = mock_workspace(&mut app);
        let mut snapshot = workspace.update(&mut app, |workspace, ctx| {
            set_active_tab_title(workspace, "first", ctx);
            workspace.handle_action(&WorkspaceAction::NewSpace, ctx);
            set_active_tab_title(workspace, "second", ctx);
            add_titled_tab(workspace, "orphan", ctx);
            add_titled_tab(workspace, "legacy", ctx);
            workspace.snapshot(ctx.window_id(), false, ctx)
        });
        for tab in &mut snapshot.tabs {
            match tab.custom_title.as_deref() {
                // Points at a space that wasn't saved.
                Some("orphan") => tab.space_id = Some(SpaceId::new()),
                // Saved before spaces existed.
                Some("legacy") => tab.space_id = None,
                _ => {}
            }
        }

        let restored = restored_workspace(&mut app, snapshot);
        restored.read(&app, |workspace, ctx| {
            assert_eq!(workspace.spaces.len(), 2);
            assert_eq!(workspace.active_space, workspace.spaces[1].id);
            assert_eq!(
                tab_titles_in(workspace, workspace.spaces[0].id, ctx),
                ["first", "orphan", "legacy"]
            );
            assert_eq!(live_tab_titles(workspace, ctx), ["second"]);
        });
    });
}

#[test]
fn test_restore_with_spaces_disabled_loads_every_tab_in_one_list() {
    let _pinned_tabs_guard = FeatureFlag::PinnedTabs.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let spaces_enabled = FeatureFlag::Spaces.override_enabled(true);
        let workspace = mock_workspace(&mut app);
        let snapshot = workspace.update(&mut app, |workspace, ctx| {
            set_active_tab_title(workspace, "first", ctx);
            workspace.handle_action(&WorkspaceAction::NewSpace, ctx);
            set_active_tab_title(workspace, "second", ctx);
            add_titled_tab(workspace, "pinned", ctx);
            workspace.handle_action(&WorkspaceAction::PinTab(1), ctx);
            assert_eq!(live_tab_titles(workspace, ctx), ["pinned", "second"]);
            workspace.activate_tab(1, ctx);
            workspace.snapshot(ctx.window_id(), false, ctx)
        });
        drop(spaces_enabled);
        let _spaces_disabled = FeatureFlag::Spaces.override_enabled(false);

        let restored = restored_workspace(&mut app, snapshot);
        restored.read(&app, |workspace, ctx| {
            assert_eq!(workspace.spaces.len(), 1);
            // The hidden space's pinned tab still lands in the pinned region at the front.
            assert_eq!(
                live_tab_titles(workspace, ctx),
                ["pinned", "first", "second"]
            );
            assert!(workspace.tabs[0].pinned);
            assert_eq!(
                tab_title(&workspace.tabs[workspace.active_tab_index()], ctx),
                "second"
            );
        });
    });
}

#[test]
fn test_restore_without_space_data_keeps_every_tab_in_one_space() {
    let _spaces_guard = FeatureFlag::Spaces.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let workspace = mock_workspace(&mut app);
        let mut snapshot = workspace.update(&mut app, |workspace, ctx| {
            set_active_tab_title(workspace, "first", ctx);
            workspace.handle_action(&WorkspaceAction::NewSpace, ctx);
            set_active_tab_title(workspace, "second", ctx);
            workspace.snapshot(ctx.window_id(), false, ctx)
        });
        // As saved before spaces existed.
        snapshot.spaces.clear();
        snapshot.active_space = None;
        for tab in &mut snapshot.tabs {
            tab.space_id = None;
        }

        let restored = restored_workspace(&mut app, snapshot);
        restored.read(&app, |workspace, ctx| {
            assert_eq!(workspace.spaces.len(), 1);
            assert_eq!(live_tab_titles(workspace, ctx), ["first", "second"]);
            assert_eq!(workspace.active_tab_index(), 1);
        });
    });
}

#[test]
fn test_restore_shows_nearest_space_with_tabs_when_active_space_has_none() {
    let _spaces_guard = FeatureFlag::Spaces.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let workspace = mock_workspace(&mut app);
        let mut snapshot = workspace.update(&mut app, |workspace, ctx| {
            set_active_tab_title(workspace, "first", ctx);
            workspace.handle_action(&WorkspaceAction::NewSpace, ctx);
            workspace.snapshot(ctx.window_id(), false, ctx)
        });
        // The active space's only tab isn't saved, as happens to read-only shared sessions.
        let active_space = snapshot.active_space;
        snapshot.tabs.retain(|tab| tab.space_id != active_space);
        snapshot.active_tab_index = 0;

        let restored = restored_workspace(&mut app, snapshot);
        restored.read(&app, |workspace, ctx| {
            assert_eq!(workspace.spaces.len(), 2);
            assert_eq!(workspace.active_space, workspace.spaces[0].id);
            assert_eq!(live_tab_titles(workspace, ctx), ["first"]);
        });
    });
}

#[test]
fn test_jumps_to_a_tab_in_a_hidden_space_switch_to_that_space() {
    let _spaces_guard = FeatureFlag::Spaces.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let workspace = mock_workspace(&mut app);
        workspace.update(&mut app, |workspace, ctx| {
            let first_space = workspace.active_space;
            workspace.add_terminal_tab(false, ctx);
            workspace.activate_tab(0, ctx);
            let hidden_tab = workspace.tabs[1].pane_group.clone();

            workspace.handle_action(&WorkspaceAction::NewSpace, ctx);
            let second_space = workspace.active_space;
            let second_space_tab = workspace.active_tab_pane_group().clone();

            // Ctrl+Tab and other palette jumps.
            workspace.activate_tab_by_pane_group_id(hidden_tab.id(), ctx);
            assert_eq!(workspace.active_space, first_space);
            assert_eq!(workspace.active_tab_index(), 1);
            assert_eq!(workspace.active_tab_pane_group().id(), hidden_tab.id());
            assert_eq!(workspace.tab_mru_order().first(), Some(&hidden_tab.id()));

            // Notification clicks and session/conversation navigation.
            let locator = PaneViewLocator {
                pane_group_id: second_space_tab.id(),
                pane_id: second_space_tab.as_ref(ctx).focused_pane_id(ctx),
            };
            workspace.focus_pane(locator, ctx);
            assert_eq!(workspace.active_space, second_space);
            assert_eq!(
                workspace.active_tab_pane_group().id(),
                second_space_tab.id()
            );

            // The first space remembers the tab the jump landed on.
            workspace.handle_action(&WorkspaceAction::SwitchToSpaceById(first_space), ctx);
            assert_eq!(workspace.active_tab_pane_group().id(), hidden_tab.id());
        });
    });
}

#[test]
fn test_ctrl_tab_lists_tabs_of_every_space_with_space_labels() {
    let _spaces_guard = FeatureFlag::Spaces.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let workspace = mock_workspace(&mut app);
        workspace.update(&mut app, |workspace, ctx| {
            let window_id = ctx.window_id();
            let single_space_tabs = workspace.tab_navigation_data(window_id, ctx);
            assert!(single_space_tabs.iter().all(|tab| tab.space_name.is_none()));

            let first_space = workspace.active_space;
            workspace.add_terminal_tab(false, ctx);
            let first_space_tabs = live_tab_ids(workspace);
            workspace.handle_action(&WorkspaceAction::NewSpace, ctx);
            workspace.spaces[1].name = Some("Infra".to_string());
            let second_space_tab = workspace.active_tab_pane_group().id();

            let tabs = workspace.tab_navigation_data(window_id, ctx);
            let entries = tabs
                .iter()
                .map(|tab| (tab.pane_group_id, tab.space_name.as_deref(), tab.tab_index))
                .collect::<Vec<_>>();
            // The active space comes first; the hidden space follows in its own MRU order
            // (most recent first) with its placeholder name.
            assert_eq!(
                entries,
                vec![
                    (second_space_tab, Some("Infra"), 1),
                    (first_space_tabs[1], Some("Space 1"), 2),
                    (first_space_tabs[0], Some("Space 1"), 1),
                ]
            );

            // Selecting a hidden space's entry switches to that space.
            workspace.activate_tab_by_pane_group_id(tabs[2].pane_group_id, ctx);
            assert_eq!(workspace.active_space, first_space);
            assert_eq!(workspace.active_tab_pane_group().id(), first_space_tabs[0]);
        });
    });
}

#[test]
fn test_vertical_tabs_search_stays_within_active_space() {
    let _spaces_guard = FeatureFlag::Spaces.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let workspace = mock_workspace(&mut app);
        workspace.update(&mut app, |workspace, ctx| {
            let name_group = |workspace: &mut Workspace, indices: &[usize]| {
                let group_id = group_live_tabs(workspace, indices);
                if let Some(group) = workspace.tab_groups.get_mut(&group_id) {
                    group.name = Some("alpha".to_string());
                }
            };
            let first_space = workspace.active_space;
            workspace.add_terminal_tab(false, ctx);
            workspace.add_terminal_tab(false, ctx);
            name_group(workspace, &[1, 2]);

            workspace.handle_action(&WorkspaceAction::NewSpace, ctx);
            workspace.add_terminal_tab(false, ctx);
            name_group(workspace, &[0, 1]);

            workspace.handle_action(&WorkspaceAction::SwitchToSpaceById(first_space), ctx);
            workspace.activate_tab(1, ctx);
            workspace.vertical_tabs_panel.search_query = "alpha".to_string();

            let matching = workspace.vertical_tabs_panel.matching_tab_indices(
                &workspace.tabs,
                &workspace.tab_groups,
                workspace.active_tab_index,
                ctx,
            );
            assert_eq!(matching, vec![1, 2]);

            // Cycling through search results never leaves the active space.
            workspace.activate_next_tab(ctx);
            assert_eq!(workspace.active_tab_index(), 2);
            workspace.activate_next_tab(ctx);
            assert_eq!(workspace.active_tab_index(), 1);
            assert_eq!(workspace.active_space, first_space);
        });
    });
}

#[test]
fn test_window_sessions_include_hidden_spaces() {
    let _spaces_guard = FeatureFlag::Spaces.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let workspace = mock_workspace(&mut app);
        let window_id = workspace.update(&mut app, |workspace, ctx| {
            workspace.add_terminal_tab(false, ctx);
            workspace.handle_action(&WorkspaceAction::NewSpace, ctx);
            assert_eq!(workspace.tab_count(), 1);
            assert_eq!(
                workspace.workspace_sessions(ctx.window_id(), ctx).count(),
                3
            );
            ctx.window_id()
        });

        // What the window-close and quit warnings count running commands in.
        let window_sessions = app.read(|ctx| {
            SessionNavigationData::all_sessions(ctx)
                .filter(|session| session.window_id() == window_id)
                .count()
        });
        assert_eq!(window_sessions, 3);
    });
}

#[test]
fn test_dragging_out_last_tab_of_space_keeps_window_and_switches_space() {
    let _spaces_guard = FeatureFlag::Spaces.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let workspace = mock_workspace(&mut app);
        let window_id = workspace.update(&mut app, |workspace, ctx| {
            let first_space = workspace.active_space;
            let first_space_tab = workspace.active_tab_pane_group().id();
            workspace.handle_action(&WorkspaceAction::NewSpace, ctx);

            // What a cross-window drop does to the source once the tab is in another window.
            workspace.remove_tab_without_undo(0, ctx);

            assert_eq!(workspace.active_space, first_space);
            assert_eq!(live_tab_ids(workspace), vec![first_space_tab]);
            assert_eq!(workspace.spaces.len(), 2);
            ctx.window_id()
        });
        app.update(|_| ());
        assert!(app.is_window_open(window_id));
    });
}

#[test]
fn test_tab_dropped_into_another_window_joins_its_active_space() {
    let _spaces_guard = FeatureFlag::Spaces.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let source = mock_workspace(&mut app);
        let (source_window, transferred_tab) = source.update(&mut app, |workspace, ctx| {
            workspace.add_terminal_tab(false, ctx);
            let transferred_tab = workspace
                .get_tab_transfer_info_for_attach(1, ctx)
                .expect("tab exists");
            workspace.prepare_for_transferred_tab_attach(&transferred_tab.pane_group, ctx);
            (ctx.window_id(), transferred_tab)
        });
        let moved_tab = transferred_tab.pane_group.id();

        let target = mock_workspace(&mut app);
        let (target_window, hidden_space, hidden_tabs) =
            target.update(&mut app, |workspace, ctx| {
                let hidden_space = workspace.active_space;
                let hidden_tabs = live_tab_ids(workspace);
                workspace.handle_action(&WorkspaceAction::NewSpace, ctx);
                (ctx.window_id(), hidden_space, hidden_tabs)
            });

        app.update(|ctx| {
            ctx.transfer_view_tree_to_window(moved_tab, source_window, target_window);
        });
        target.update(&mut app, |workspace, ctx| {
            let active_space = workspace.active_space;
            workspace.insert_transferred_tab_at_index(transferred_tab, 1, ctx);

            assert_eq!(workspace.active_space, active_space);
            assert_eq!(workspace.active_tab_pane_group().id(), moved_tab);
            assert!(tab_ids_in(workspace, active_space).contains(&moved_tab));
            assert_eq!(tab_ids_in(workspace, hidden_space), hidden_tabs);
        });
        source.update(&mut app, |workspace, ctx| {
            workspace.remove_tab_without_undo(1, ctx);
            assert_eq!(workspace.tab_count(), 1);
        });
    });
}

/// Feeds a horizontal two-finger swipe (`dx` per 16 ms frame) starting at `start`, and returns
/// the time of its last frame.
fn swipe_spaces(
    workspace: &mut Workspace,
    start: Instant,
    dx: f32,
    frames: u32,
    ctx: &mut ViewContext<Workspace>,
) -> Instant {
    let mut now = start;
    for frame in 0..frames {
        now = start + Duration::from_millis(16 * u64::from(frame));
        workspace.handle_space_swipe(vec2f(dx, 0.), true, now, ctx);
    }
    now
}

#[test]
fn test_swipe_switches_to_adjacent_space_once_per_gesture() {
    let _spaces_guard = FeatureFlag::Spaces.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let workspace = mock_workspace(&mut app);
        workspace.update(&mut app, |workspace, ctx| {
            workspace.handle_action(&WorkspaceAction::NewSpace, ctx);
            workspace.handle_action(&WorkspaceAction::NewSpace, ctx);
            workspace.handle_action(&WorkspaceAction::SwitchToSpace(1), ctx);
            let spaces = workspace
                .spaces
                .iter()
                .map(|space| space.id)
                .collect::<Vec<_>>();

            // Fingers moving left: next space. The long gesture, momentum tail included, switches
            // once, so it stops at the second space, not the third.
            let end = swipe_spaces(workspace, Instant::now(), -10., 40, ctx);
            assert_eq!(workspace.active_space, spaces[1]);

            // After a pause, fingers moving right: previous space.
            let end = swipe_spaces(workspace, end + IDLE_GAP, 10., 12, ctx);
            assert_eq!(workspace.active_space, spaces[0]);

            // No wrapping at the first space.
            swipe_spaces(workspace, end + IDLE_GAP, 10., 12, ctx);
            assert_eq!(workspace.active_space, spaces[0]);
        });
    });
}

#[test]
fn test_vertical_and_wheel_scrolls_do_not_switch_spaces() {
    let _spaces_guard = FeatureFlag::Spaces.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let workspace = mock_workspace(&mut app);
        workspace.update(&mut app, |workspace, ctx| {
            let first_space = workspace.active_space;
            workspace.handle_action(&WorkspaceAction::NewSpace, ctx);
            workspace.handle_action(&WorkspaceAction::SwitchToSpace(1), ctx);
            let start = Instant::now();
            for frame in 0..20 {
                let now = start + Duration::from_millis(16 * frame);
                workspace.handle_space_swipe(vec2f(-2., -20.), true, now, ctx);
                // Shift+wheel: horizontal, but not precise.
                workspace.handle_space_swipe(vec2f(-20., 0.), false, now, ctx);
            }
            assert_eq!(workspace.active_space, first_space);
        });
    });
}

#[test]
fn test_swipe_is_ignored_while_dragging_renaming_or_searching() {
    let _spaces_guard = FeatureFlag::Spaces.override_enabled(true);

    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let workspace = mock_workspace(&mut app);
        let (first_space, second_space, end) = workspace.update(&mut app, |workspace, ctx| {
            let first_space = workspace.active_space;
            workspace.handle_action(&WorkspaceAction::NewSpace, ctx);
            let second_space = workspace.active_space;
            workspace.handle_action(&WorkspaceAction::SwitchToSpace(1), ctx);

            workspace.current_workspace_state.is_tab_being_dragged = true;
            let end = swipe_spaces(workspace, Instant::now(), -10., 12, ctx);
            assert_eq!(workspace.active_space, first_space);
            // A swipe that was ignored stays ignored after the drag ends mid-gesture.
            workspace.current_workspace_state.is_tab_being_dragged = false;
            let end = swipe_spaces(workspace, end + Duration::from_millis(16), -10., 12, ctx);
            assert_eq!(workspace.active_space, first_space);

            workspace
                .current_workspace_state
                .set_space_being_renamed(first_space);
            let end = swipe_spaces(workspace, end + IDLE_GAP, -10., 12, ctx);
            assert_eq!(workspace.active_space, first_space);
            workspace
                .current_workspace_state
                .clear_space_being_renamed();
            (first_space, second_space, end)
        });

        workspace.update(&mut app, |workspace, ctx| {
            ctx.focus(&workspace.vertical_tabs_search_input);
        });
        let end = workspace.update(&mut app, |workspace, ctx| {
            assert!(workspace.vertical_tabs_search_input.is_focused(ctx));
            let end = swipe_spaces(workspace, end + IDLE_GAP, -10., 12, ctx);
            assert_eq!(workspace.active_space, first_space);
            workspace.focus_active_tab(ctx);
            end
        });

        workspace.update(&mut app, |workspace, ctx| {
            swipe_spaces(workspace, end + IDLE_GAP, -10., 12, ctx);
            assert_eq!(workspace.active_space, second_space);
        });
    });
}

/// Turns on vertical tabs and opens the panel. Unit tests build every frame eagerly, so the panel
/// is laid out and its element positions are cached after this.
fn open_vertical_tabs_panel(app: &mut App, workspace: &ViewHandle<Workspace>) {
    app.update(|ctx| {
        TabSettings::handle(ctx).update(ctx, |settings, ctx| {
            report_if_error!(settings.use_vertical_tabs.set_value(true, ctx));
        });
    });
    workspace.update(app, |workspace, ctx| {
        workspace.vertical_tabs_panel_open = true;
        ctx.notify();
    });
}

fn element_rect(app: &App, window_id: WindowId, position_id: &str) -> RectF {
    app.read(|ctx| ctx.element_position_by_id_at_last_frame(window_id, position_id))
        .unwrap_or_else(|| panic!("{position_id} was not rendered"))
}

fn send_mouse_event(app: &mut App, window_id: WindowId, event: Event) {
    let presenter = app.presenter(window_id).expect("window is open");
    app.update(move |ctx| {
        ctx.simulate_window_event(event, window_id, presenter);
    });
}

/// Presses the left button at `from`, drags to `to` in small steps, and releases there, the way
/// the platform delivers a real mouse drag.
fn drag_mouse(app: &mut App, window_id: WindowId, from: Vector2F, to: Vector2F) {
    const STEPS: u16 = 12;
    send_mouse_event(
        app,
        window_id,
        Event::LeftMouseDown {
            position: from,
            modifiers: Default::default(),
            click_count: 1,
            is_first_mouse: false,
        },
    );
    for step in 1..=STEPS {
        let position = from + (to - from) * (f32::from(step) / f32::from(STEPS));
        send_mouse_event(
            app,
            window_id,
            Event::LeftMouseDragged {
                position,
                modifiers: Default::default(),
            },
        );
    }
    send_mouse_event(
        app,
        window_id,
        Event::LeftMouseUp {
            position: to,
            modifiers: Default::default(),
        },
    );
}

/// Drags a row through the rendered panel with the flag off and on, and with cross-window drag off
/// (debug and OSS builds) and on (release builds).
#[test]
fn test_dragging_a_vertical_tab_reorders_tabs() {
    let _vertical_tabs_guard = FeatureFlag::VerticalTabs.override_enabled(true);
    let _grouped_guard = FeatureFlag::GroupedTabs.override_enabled(true);
    let _pinned_guard = FeatureFlag::PinnedTabs.override_enabled(true);
    for (spaces_enabled, drag_to_windows) in
        [(false, false), (true, false), (false, true), (true, true)]
    {
        let _spaces_guard = FeatureFlag::Spaces.override_enabled(spaces_enabled);
        let _drag_to_windows_guard =
            FeatureFlag::DragTabsToWindows.override_enabled(drag_to_windows);
        App::test((), |mut app| async move {
            initialize_app(&mut app);

            let workspace = mock_workspace(&mut app);
            let (window_id, tab_ids) = workspace.update(&mut app, |workspace, ctx| {
                workspace.add_terminal_tab(false, ctx);
                workspace.add_terminal_tab(false, ctx);
                (ctx.window_id(), live_tab_ids(workspace))
            });
            open_vertical_tabs_panel(&mut app, &workspace);

            let first = element_rect(&app, window_id, &tab_position_id(0));
            let last = element_rect(&app, window_id, &tab_position_id(2));
            drag_mouse(
                &mut app,
                window_id,
                first.center(),
                last.center() + vec2f(0., 4.),
            );

            workspace.read(&app, |workspace, _| {
                assert_eq!(
                    live_tab_ids(workspace),
                    vec![tab_ids[1], tab_ids[2], tab_ids[0]],
                    "spaces enabled: {spaces_enabled}, drag to windows: {drag_to_windows}"
                );
            });
        });
    }
}

#[test]
fn test_dragging_a_vertical_tab_reorders_tabs_after_switching_spaces() {
    let _vertical_tabs_guard = FeatureFlag::VerticalTabs.override_enabled(true);
    let _grouped_guard = FeatureFlag::GroupedTabs.override_enabled(true);
    let _pinned_guard = FeatureFlag::PinnedTabs.override_enabled(true);
    let _spaces_guard = FeatureFlag::Spaces.override_enabled(true);
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let workspace = mock_workspace(&mut app);
        let window_id = workspace.update(&mut app, |workspace, ctx| {
            workspace.add_terminal_tab(false, ctx);
            workspace.add_terminal_tab(false, ctx);
            workspace.add_terminal_tab(false, ctx);
            ctx.window_id()
        });
        open_vertical_tabs_panel(&mut app, &workspace);
        let tab_ids = workspace.update(&mut app, |workspace, ctx| {
            workspace.handle_action(&WorkspaceAction::NewSpace, ctx);
            workspace.add_terminal_tab(false, ctx);
            workspace.add_terminal_tab(false, ctx);
            live_tab_ids(workspace)
        });
        finish_space_slide(&mut app, &workspace);
        let first = element_rect(&app, window_id, &tab_position_id(0));
        let last = element_rect(&app, window_id, &tab_position_id(2));
        drag_mouse(
            &mut app,
            window_id,
            first.center(),
            last.center() + vec2f(0., 4.),
        );
        workspace.read(&app, |workspace, _| {
            assert_eq!(
                live_tab_ids(workspace),
                vec![tab_ids[1], tab_ids[2], tab_ids[0]]
            );
        });

        let tab_ids = workspace.update(&mut app, |workspace, ctx| {
            workspace.handle_action(&WorkspaceAction::SwitchToSpace(1), ctx);
            live_tab_ids(workspace)
        });
        finish_space_slide(&mut app, &workspace);
        let first = element_rect(&app, window_id, &tab_position_id(0));
        let last = element_rect(&app, window_id, &tab_position_id(3));
        drag_mouse(
            &mut app,
            window_id,
            last.center(),
            first.center() - vec2f(0., 4.),
        );
        workspace.read(&app, |workspace, _| {
            assert_eq!(
                live_tab_ids(workspace),
                vec![tab_ids[3], tab_ids[0], tab_ids[1], tab_ids[2]]
            );
        });
    });
}

/// Ends the tab list's space-switch slide, so the next frame paints rows where they rest.
fn finish_space_slide(app: &mut App, workspace: &ViewHandle<Workspace>) {
    workspace.update(app, |workspace, ctx| {
        workspace.vertical_tabs_panel.space_slide = None;
        ctx.notify();
    });
}

fn click_at(app: &mut App, window_id: WindowId, position: Vector2F, click_count: u32) {
    send_mouse_event(
        app,
        window_id,
        Event::LeftMouseDown {
            position,
            modifiers: Default::default(),
            click_count,
            is_first_mouse: false,
        },
    );
    send_mouse_event(
        app,
        window_id,
        Event::LeftMouseUp {
            position,
            modifiers: Default::default(),
        },
    );
}

fn chip_rect(app: &App, window_id: WindowId, space_id: SpaceId) -> RectF {
    element_rect(app, window_id, &space_chip_position_id(space_id))
}

#[test]
fn test_space_chip_drag_index_follows_neighbor_midpoints() {
    let chip = |x: f32, y: f32| Some(RectF::new(vec2f(x, y), vec2f(40., 20.)));
    let (before, after) = (chip(0., 0.), chip(100., 0.));
    // Dragged chip sits between them at x 50..90; its center starts at (70, 10).
    assert_eq!(space_chip_drag_index(1, vec2f(70., 10.), before, after), 1);
    assert_eq!(space_chip_drag_index(1, vec2f(19., 10.), before, after), 0);
    assert_eq!(space_chip_drag_index(1, vec2f(121., 10.), before, after), 2);
    assert_eq!(space_chip_drag_index(0, vec2f(-50., 10.), None, after), 0);
    assert_eq!(space_chip_drag_index(2, vec2f(500., 10.), before, None), 2);

    // Wrapped rows: the chip before is at the end of the row above, the chip after starts the
    // row below.
    let (before, after) = (chip(100., 0.), chip(0., 50.));
    assert_eq!(space_chip_drag_index(1, vec2f(150., 35.), before, after), 1);
    assert_eq!(space_chip_drag_index(1, vec2f(110., 10.), before, after), 0);
    assert_eq!(space_chip_drag_index(1, vec2f(30., 60.), before, after), 2);
}

#[test]
fn test_dragging_a_space_chip_reorders_spaces() {
    let _vertical_tabs_guard = FeatureFlag::VerticalTabs.override_enabled(true);
    let _spaces_guard = FeatureFlag::Spaces.override_enabled(true);
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let workspace = mock_workspace(&mut app);
        let (window_id, spaces) = workspace.update(&mut app, |workspace, ctx| {
            workspace.handle_action(&WorkspaceAction::NewSpace, ctx);
            workspace.handle_action(&WorkspaceAction::NewSpace, ctx);
            workspace.handle_action(&WorkspaceAction::SwitchToSpace(2), ctx);
            let spaces = workspace
                .spaces
                .iter()
                .map(|space| space.id)
                .collect::<Vec<_>>();
            (ctx.window_id(), spaces)
        });
        open_vertical_tabs_panel(&mut app, &workspace);

        let first = chip_rect(&app, window_id, spaces[0]);
        let last = chip_rect(&app, window_id, spaces[2]);
        drag_mouse(
            &mut app,
            window_id,
            first.center(),
            vec2f(last.max_x() + 2., last.center().y()),
        );

        workspace.read(&app, |workspace, _| {
            assert_eq!(
                workspace
                    .spaces
                    .iter()
                    .map(|space| space.id)
                    .collect::<Vec<_>>(),
                vec![spaces[1], spaces[2], spaces[0]]
            );
            // Dragging doesn't switch.
            assert_eq!(workspace.active_space, spaces[1]);
            // Moved unnamed spaces keep the name they were shown with.
            let names = workspace
                .spaces
                .iter()
                .map(|space| space.name.as_deref())
                .collect::<Vec<_>>();
            assert_eq!(
                names,
                vec![Some("Space 2"), Some("Space 3"), Some("Space 1")]
            );
        });
    });
}

#[test]
fn test_space_chip_click_double_click_and_right_click_still_work() {
    let _vertical_tabs_guard = FeatureFlag::VerticalTabs.override_enabled(true);
    let _spaces_guard = FeatureFlag::Spaces.override_enabled(true);
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let workspace = mock_workspace(&mut app);
        let (window_id, first_space, second_space) =
            workspace.update(&mut app, |workspace, ctx| {
                let first_space = workspace.active_space;
                workspace.handle_action(&WorkspaceAction::NewSpace, ctx);
                (ctx.window_id(), first_space, workspace.active_space)
            });
        open_vertical_tabs_panel(&mut app, &workspace);

        let first_chip = chip_rect(&app, window_id, first_space).center();
        click_at(&mut app, window_id, first_chip, 1);
        workspace.read(&app, |workspace, _| {
            assert_eq!(workspace.active_space, first_space);
            assert_eq!(
                workspace
                    .spaces
                    .iter()
                    .map(|space| space.id)
                    .collect::<Vec<_>>(),
                vec![first_space, second_space]
            );
        });

        let second_chip = chip_rect(&app, window_id, second_space).center();
        send_mouse_event(
            &mut app,
            window_id,
            Event::RightMouseDown {
                position: second_chip,
                cmd: false,
                shift: false,
                click_count: 1,
            },
        );
        workspace.update(&mut app, |workspace, ctx| {
            assert!(workspace.show_space_right_click_menu.is_some());
            workspace.toggle_space_right_click_menu(second_space, second_chip, ctx);
        });

        click_at(&mut app, window_id, second_chip, 1);
        click_at(&mut app, window_id, second_chip, 2);
        workspace.read(&app, |workspace, _| {
            assert_eq!(workspace.active_space, second_space);
            assert_eq!(
                workspace.current_workspace_state.space_being_renamed(),
                Some(second_space)
            );
        });
    });
}

/// Presses the left button at `from` and drags to `to` without releasing.
fn press_and_drag(app: &mut App, window_id: WindowId, from: Vector2F, to: Vector2F) {
    const STEPS: u16 = 12;
    send_mouse_event(
        app,
        window_id,
        Event::LeftMouseDown {
            position: from,
            modifiers: Default::default(),
            click_count: 1,
            is_first_mouse: false,
        },
    );
    for step in 1..=STEPS {
        let position = from + (to - from) * (f32::from(step) / f32::from(STEPS));
        send_mouse_event(
            app,
            window_id,
            Event::LeftMouseDragged {
                position,
                modifiers: Default::default(),
            },
        );
    }
}

fn release_mouse(app: &mut App, window_id: WindowId, position: Vector2F) {
    send_mouse_event(
        app,
        window_id,
        Event::LeftMouseUp {
            position,
            modifiers: Default::default(),
        },
    );
}

#[test]
fn test_dropping_a_tab_on_a_space_chip_moves_it_to_that_space() {
    let _vertical_tabs_guard = FeatureFlag::VerticalTabs.override_enabled(true);
    let _grouped_guard = FeatureFlag::GroupedTabs.override_enabled(true);
    let _spaces_guard = FeatureFlag::Spaces.override_enabled(true);
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let workspace = mock_workspace(&mut app);
        let (window_id, first_space, second_space, tab_ids) =
            workspace.update(&mut app, |workspace, ctx| {
                let first_space = workspace.active_space;
                workspace.handle_action(&WorkspaceAction::NewSpace, ctx);
                let second_space = workspace.active_space;
                workspace.handle_action(&WorkspaceAction::SwitchToSpace(1), ctx);
                workspace.add_terminal_tab(false, ctx);
                workspace.add_terminal_tab(false, ctx);
                workspace.activate_tab(0, ctx);
                (
                    ctx.window_id(),
                    first_space,
                    second_space,
                    live_tab_ids(workspace),
                )
            });
        let second_space_tabs =
            workspace.read(&app, |workspace, _| tab_ids_in(workspace, second_space));
        open_vertical_tabs_panel(&mut app, &workspace);

        let row = element_rect(&app, window_id, &tab_position_id(1)).center();
        let chip = chip_rect(&app, window_id, second_space).center();
        press_and_drag(&mut app, window_id, row, chip);
        workspace.read(&app, |workspace, _| {
            assert_eq!(
                workspace.space_chip_drop,
                Some((second_space, SpaceChipDrop::Tab(tab_ids[1])))
            );
        });
        release_mouse(&mut app, window_id, chip);

        workspace.read(&app, |workspace, _| {
            assert_eq!(workspace.space_chip_drop, None);
            assert_eq!(workspace.active_space, first_space);
            assert_eq!(live_tab_ids(workspace), vec![tab_ids[0], tab_ids[2]]);
            assert_eq!(workspace.active_tab_pane_group().id(), tab_ids[0]);
            let mut expected = second_space_tabs.clone();
            expected.push(tab_ids[1]);
            assert_eq!(tab_ids_in(workspace, second_space), expected);
        });

        // The active space's own chip isn't a drop target: the drag reorders as usual.
        let row = element_rect(&app, window_id, &tab_position_id(0)).center();
        let own_chip = chip_rect(&app, window_id, first_space).center();
        press_and_drag(&mut app, window_id, row, own_chip);
        workspace.read(&app, |workspace, _| {
            assert_eq!(workspace.space_chip_drop, None);
        });
        release_mouse(&mut app, window_id, own_chip);
        workspace.read(&app, |workspace, _| {
            assert_eq!(live_tab_ids(workspace), vec![tab_ids[2], tab_ids[0]]);
        });
    });
}

#[test]
fn test_dropping_a_tab_group_on_a_space_chip_moves_the_group() {
    let _vertical_tabs_guard = FeatureFlag::VerticalTabs.override_enabled(true);
    let _grouped_guard = FeatureFlag::GroupedTabs.override_enabled(true);
    let _spaces_guard = FeatureFlag::Spaces.override_enabled(true);
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let workspace = mock_workspace(&mut app);
        let (window_id, second_space, group_id, grouped_tab) =
            workspace.update(&mut app, |workspace, ctx| {
                workspace.handle_action(&WorkspaceAction::NewSpace, ctx);
                let second_space = workspace.active_space;
                workspace.handle_action(&WorkspaceAction::SwitchToSpace(1), ctx);
                workspace.add_terminal_tab(false, ctx);
                workspace.handle_action(&WorkspaceAction::NewTabGroupFromTab(1), ctx);
                let grouped_tab = workspace.tabs[1].pane_group.id();
                let group_id = workspace.tabs[1].group_id.expect("tab was grouped");
                (ctx.window_id(), second_space, group_id, grouped_tab)
            });
        open_vertical_tabs_panel(&mut app, &workspace);

        // A group's only member drags the whole group.
        let row = element_rect(&app, window_id, &tab_position_id(1)).center();
        let chip = chip_rect(&app, window_id, second_space).center();
        press_and_drag(&mut app, window_id, row, chip);
        workspace.read(&app, |workspace, _| {
            assert_eq!(
                workspace.space_chip_drop,
                Some((second_space, SpaceChipDrop::Group(group_id)))
            );
        });
        release_mouse(&mut app, window_id, chip);

        workspace.read(&app, |workspace, _| {
            assert!(!workspace.tab_groups.contains_key(&group_id));
            assert!(tab_ids_in(workspace, second_space).contains(&grouped_tab));
            assert!(workspace.all_tab_groups().any(|group| group.id == group_id));
        });
    });
}

fn slide_from(workspace: &Workspace) -> Option<SlideFrom> {
    workspace
        .vertical_tabs_panel
        .space_slide
        .map(|slide| slide.from)
}

/// Every way of switching spaces slides the tab list in from the side of the space switched to,
/// and a switch during a slide restarts it.
#[test]
fn test_switching_spaces_slides_the_tab_list_in_from_the_target_side() {
    let _vertical_tabs_guard = FeatureFlag::VerticalTabs.override_enabled(true);
    let _spaces_guard = FeatureFlag::Spaces.override_enabled(true);
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let workspace = mock_workspace(&mut app);
        open_vertical_tabs_panel(&mut app, &workspace);
        workspace.update(&mut app, |workspace, ctx| {
            assert_eq!(slide_from(workspace), None);
            let first_space = workspace.active_space;
            let first_space_tab = workspace.active_tab_pane_group().id();

            workspace.handle_action(&WorkspaceAction::NewSpace, ctx);
            assert_eq!(slide_from(workspace), Some(SlideFrom::Right));
            let first_slide = workspace.vertical_tabs_panel.space_slide;
            workspace.handle_action(&WorkspaceAction::NewSpace, ctx);

            workspace.handle_action(&WorkspaceAction::PrevSpace, ctx);
            assert_eq!(slide_from(workspace), Some(SlideFrom::Left));
            assert!(
                workspace
                    .vertical_tabs_panel
                    .space_slide
                    .map(|slide| slide.started_at)
                    >= first_slide.map(|slide| slide.started_at)
            );
            workspace.handle_action(&WorkspaceAction::NextSpace, ctx);
            assert_eq!(slide_from(workspace), Some(SlideFrom::Right));

            // Chip clicks and Switch to space N.
            workspace.handle_action(&WorkspaceAction::SwitchToSpaceById(first_space), ctx);
            assert_eq!(slide_from(workspace), Some(SlideFrom::Left));
            workspace.handle_action(&WorkspaceAction::SwitchToSpace(3), ctx);
            assert_eq!(slide_from(workspace), Some(SlideFrom::Right));

            // Fingers moving right: previous space.
            swipe_spaces(workspace, Instant::now(), 10., 12, ctx);
            assert_eq!(workspace.active_space_index(), 1);
            assert_eq!(slide_from(workspace), Some(SlideFrom::Left));

            // Jumps to a tab in a hidden space.
            workspace.activate_tab_by_pane_group_id(first_space_tab, ctx);
            assert_eq!(workspace.active_space, first_space);
            assert_eq!(slide_from(workspace), Some(SlideFrom::Left));
        });
    });
}

#[test]
fn test_switching_spaces_without_the_vertical_tabs_panel_does_not_slide() {
    let _vertical_tabs_guard = FeatureFlag::VerticalTabs.override_enabled(true);
    let _spaces_guard = FeatureFlag::Spaces.override_enabled(true);
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let workspace = mock_workspace(&mut app);
        workspace.update(&mut app, |workspace, ctx| {
            // Horizontal tabs.
            workspace.handle_action(&WorkspaceAction::NewSpace, ctx);
            assert_eq!(slide_from(workspace), None);
        });
        open_vertical_tabs_panel(&mut app, &workspace);
        workspace.update(&mut app, |workspace, ctx| {
            workspace.vertical_tabs_panel_open = false;
            workspace.handle_action(&WorkspaceAction::PrevSpace, ctx);
            assert_eq!(workspace.active_space_index(), 0);
            assert_eq!(slide_from(workspace), None);
        });
    });
}

#[test]
fn test_restoring_spaces_does_not_slide() {
    let _vertical_tabs_guard = FeatureFlag::VerticalTabs.override_enabled(true);
    let _spaces_guard = FeatureFlag::Spaces.override_enabled(true);
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let workspace = mock_workspace(&mut app);
        open_vertical_tabs_panel(&mut app, &workspace);
        let snapshot = workspace.update(&mut app, |workspace, ctx| {
            workspace.handle_action(&WorkspaceAction::NewSpace, ctx);
            workspace.snapshot(ctx.window_id(), false, ctx)
        });

        let restored = restored_workspace(&mut app, snapshot);
        restored.read(&app, |workspace, _| {
            assert!(workspace.vertical_tabs_panel_open);
            assert_eq!(workspace.active_space_index(), 1);
            assert_eq!(slide_from(workspace), None);
        });
    });
}

/// The rows are painted a full list width toward the entry side at the start of a slide, and back
/// in place once it ends.
#[test]
fn test_sliding_tab_list_is_painted_beside_its_resting_place() {
    let _vertical_tabs_guard = FeatureFlag::VerticalTabs.override_enabled(true);
    let _spaces_guard = FeatureFlag::Spaces.override_enabled(true);
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let workspace = mock_workspace(&mut app);
        let window_id = workspace.update(&mut app, |_, ctx| ctx.window_id());
        open_vertical_tabs_panel(&mut app, &workspace);
        let rest = element_rect(&app, window_id, &tab_position_id(0));

        for from in [SlideFrom::Right, SlideFrom::Left] {
            workspace.update(&mut app, |workspace, ctx| {
                // Starting in the future keeps the slide at its first frame for the whole test.
                workspace.vertical_tabs_panel.space_slide = Some(SpaceSlide {
                    from,
                    started_at: Instant::now() + Duration::from_secs(3600),
                });
                ctx.notify();
            });
            let sliding = element_rect(&app, window_id, &tab_position_id(0));
            assert_eq!(sliding.min_y(), rest.min_y());
            // The list is at least as wide as its rows.
            match from {
                SlideFrom::Right => assert!(sliding.min_x() >= rest.max_x() - 0.5),
                SlideFrom::Left => assert!(sliding.max_x() <= rest.min_x() + 0.5),
            }
        }

        finish_space_slide(&mut app, &workspace);
        assert_eq!(element_rect(&app, window_id, &tab_position_id(0)), rest);
    });
}
