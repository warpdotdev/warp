use warpui::integration::AssertionCallback;
use warpui::{async_assert, async_assert_eq};

use crate::integration_testing::view_getters::workspace_view;
use crate::tab::SelectedTabColor;
use crate::themes::theme::AnsiColorIdentifier;
use crate::workspace::tab_group::TabGroupId;

pub fn assert_focused_tab_index(tab_index: usize) -> AssertionCallback {
    Box::new(move |app, window_id| {
        let workspace = workspace_view(app, window_id);
        workspace.read(app, |view, _ctx| {
            async_assert_eq!(view.active_tab_index(), tab_index)
        })
    })
}

/// Assert how the workspace's tabs are grouped, along with the name and color
/// of each group.
///
/// `expected_memberships` holds one entry per tab, in tab order: `None` for an
/// ungrouped tab, or an index into `expected_groups`. Groups are numbered by
/// the order they first appear in the tab list, so this also pins down which
/// tabs share a single group -- two tabs only get the same index when they
/// carry the same `TabGroupId`.
///
/// Group colors are given the way a launch config writes them -- `None` for a
/// group that saved no color -- and are compared against the `SelectedTabColor`
/// restore is expected to produce.
pub fn assert_tab_groups(
    expected_memberships: Vec<Option<usize>>,
    expected_groups: Vec<(Option<&'static str>, Option<AnsiColorIdentifier>)>,
) -> AssertionCallback {
    Box::new(move |app, window_id| {
        let workspace = workspace_view(app, window_id);
        workspace.read(app, |view, _ctx| {
            let mut group_order: Vec<TabGroupId> = Vec::new();
            let memberships: Vec<Option<usize>> = view
                .tabs
                .iter()
                .map(|tab| {
                    tab.group_id.map(|id| {
                        group_order
                            .iter()
                            .position(|seen| *seen == id)
                            .unwrap_or_else(|| {
                                group_order.push(id);
                                group_order.len() - 1
                            })
                    })
                })
                .collect();

            if memberships != expected_memberships {
                return async_assert!(
                    false,
                    "Expected tab group memberships {expected_memberships:?}, but there were {memberships:?}"
                );
            }

            let groups: Vec<(Option<String>, SelectedTabColor)> = group_order
                .iter()
                .map(|id| {
                    let group = view
                        .tab_groups
                        .get(id)
                        .expect("A grouped tab's group must exist in the workspace");
                    (group.name.clone(), group.color)
                })
                .collect();
            let expected: Vec<(Option<String>, SelectedTabColor)> = expected_groups
                .iter()
                .map(|&(name, color)| {
                    (
                        name.map(str::to_owned),
                        color.map_or(SelectedTabColor::Unset, SelectedTabColor::Color),
                    )
                })
                .collect();

            async_assert_eq!(
                groups,
                expected,
                "Expected restored groups {expected:?}, but there were {groups:?}"
            )
        })
    })
}

/// Assert that there are a particular number of tabs in the workspace.
pub fn assert_tab_count(tab_count: usize) -> AssertionCallback {
    Box::new(move |app, window_id| {
        let workspace = workspace_view(app, window_id);
        workspace.read(app, |view, _ctx| {
            let actual_tab_count = view.tab_count();
            async_assert_eq!(
                actual_tab_count,
                tab_count,
                "Expected {} tabs, but there were {}",
                tab_count,
                actual_tab_count
            )
        })
    })
}
