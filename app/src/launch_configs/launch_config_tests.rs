use std::path::PathBuf;

use super::{CommandTemplate, LaunchConfig, PaneMode, PaneTemplateType, TabTemplate};
use crate::app_state::{
    AppState, BranchSnapshot, LeafContents, LeafSnapshot, NotebookPaneSnapshot, PaneFlex,
    PaneNodeSnapshot, SplitDirection, TabGroupSnapshot, TabSnapshot, TerminalPaneSnapshot,
    WindowSnapshot,
};
use crate::drive::OpenWarpDriveObjectSettings;
use crate::tab::SelectedTabColor;
use crate::themes::theme::AnsiColorIdentifier;
use crate::workspace::tab_group::TabGroupId;

fn single_tab_snapshot(root: PaneNodeSnapshot) -> AppState {
    AppState {
        windows: vec![WindowSnapshot {
            tabs: vec![TabSnapshot {
                custom_title: None,
                default_directory_color: None,
                selected_color: SelectedTabColor::default(),
                root,
                left_panel: None,
                right_panel: None,
                group_id: None,
                pinned: false,
            }],
            active_tab_index: 0,
            team_uid: None,
            bounds: None,
            quake_mode: false,
            universal_search_width: None,
            warp_ai_width: None,
            voltron_width: None,
            warp_drive_index_width: None,
            left_panel_open: false,
            vertical_tabs_panel_open: false,
            fullscreen_state: Default::default(),
            left_panel_width: None,
            right_panel_width: None,
            agent_management_filters: None,
            tab_groups: vec![],
        }],
        active_window_index: Some(0),
        block_lists: Default::default(),
        running_mcp_servers: Default::default(),
    }
}

fn multi_tab_snapshot(active_tab_index: usize, tabs: Vec<TabSnapshot>) -> AppState {
    AppState {
        windows: vec![WindowSnapshot {
            tabs,
            active_tab_index,
            team_uid: None,
            bounds: None,
            quake_mode: false,
            universal_search_width: None,
            warp_ai_width: None,
            voltron_width: None,
            warp_drive_index_width: None,
            left_panel_open: false,
            vertical_tabs_panel_open: false,
            fullscreen_state: Default::default(),
            left_panel_width: None,
            right_panel_width: None,
            agent_management_filters: None,
            tab_groups: vec![],
        }],
        active_window_index: Some(0),
        block_lists: Default::default(),
        running_mcp_servers: Default::default(),
    }
}

#[test]
fn test_config_from_snapshot_flattens_single_pane() {
    // If only one pane of the branch can be saved into a launch configuration, it should
    // be flattened to a single leaf.

    let state = single_tab_snapshot(PaneNodeSnapshot::Branch(BranchSnapshot {
        direction: SplitDirection::Vertical,
        children: vec![
            (
                PaneFlex(1.),
                PaneNodeSnapshot::Leaf(LeafSnapshot {
                    is_focused: true,
                    custom_vertical_tabs_title: None,
                    contents: LeafContents::Notebook(NotebookPaneSnapshot::CloudNotebook {
                        notebook_id: None,
                        settings: OpenWarpDriveObjectSettings::default(),
                    }),
                }),
            ),
            (
                PaneFlex(1.),
                PaneNodeSnapshot::Leaf(LeafSnapshot {
                    is_focused: true,
                    custom_vertical_tabs_title: None,
                    contents: LeafContents::Terminal(TerminalPaneSnapshot {
                        uuid: vec![],
                        cwd: Some("/some/dir".into()),
                        is_active: true,
                        is_read_only: false,
                        shell_launch_data: None,
                        input_config: None,
                        llm_model_override: None,
                        active_profile_id: None,
                        conversation_ids_to_restore: vec![],
                        active_conversation_id: None,
                    }),
                }),
            ),
        ],
    }));

    let template = LaunchConfig::from_snapshot("Test".into(), &state);
    assert_eq!(
        template.windows[0].tabs[0].layout,
        PaneTemplateType::PaneTemplate {
            is_focused: Some(true),
            cwd: PathBuf::from("/some/dir"),
            commands: vec![],
            pane_mode: PaneMode::Terminal,
            shell: None,
        },
    )
}

#[test]
fn test_config_from_snapshot_filters_panes() {
    let state = single_tab_snapshot(PaneNodeSnapshot::Branch(BranchSnapshot {
        direction: SplitDirection::Vertical,
        children: vec![
            (
                PaneFlex(1.),
                PaneNodeSnapshot::Leaf(LeafSnapshot {
                    is_focused: true,
                    custom_vertical_tabs_title: None,
                    contents: LeafContents::Terminal(TerminalPaneSnapshot {
                        uuid: vec![],
                        cwd: Some("/path/to/dir".into()),
                        is_active: true,
                        is_read_only: false,
                        shell_launch_data: None,
                        input_config: None,
                        llm_model_override: None,
                        active_profile_id: None,
                        conversation_ids_to_restore: vec![],
                        active_conversation_id: None,
                    }),
                }),
            ),
            (
                PaneFlex(1.),
                PaneNodeSnapshot::Leaf(LeafSnapshot {
                    is_focused: false,
                    custom_vertical_tabs_title: None,
                    contents: LeafContents::Notebook(NotebookPaneSnapshot::CloudNotebook {
                        notebook_id: None,
                        settings: OpenWarpDriveObjectSettings::default(),
                    }),
                }),
            ),
            (
                PaneFlex(1.),
                PaneNodeSnapshot::Leaf(LeafSnapshot {
                    is_focused: false,
                    custom_vertical_tabs_title: None,
                    contents: LeafContents::Terminal(TerminalPaneSnapshot {
                        uuid: vec![],
                        cwd: Some("/some/dir".into()),
                        is_active: true,
                        is_read_only: false,
                        shell_launch_data: None,
                        input_config: None,
                        llm_model_override: None,
                        active_profile_id: None,
                        conversation_ids_to_restore: vec![],
                        active_conversation_id: None,
                    }),
                }),
            ),
        ],
    }));

    let template = LaunchConfig::from_snapshot("Test".into(), &state);
    assert_eq!(
        template.windows[0].tabs[0].layout,
        PaneTemplateType::PaneBranchTemplate {
            split_direction: SplitDirection::Vertical.into(),
            panes: vec![
                PaneTemplateType::PaneTemplate {
                    is_focused: Some(true),
                    cwd: PathBuf::from("/path/to/dir"),
                    commands: vec![],
                    pane_mode: PaneMode::Terminal,
                    shell: None,
                },
                PaneTemplateType::PaneTemplate {
                    is_focused: Some(false),
                    cwd: PathBuf::from("/some/dir"),
                    commands: vec![],
                    pane_mode: PaneMode::Terminal,
                    shell: None,
                },
            ]
        }
    )
}

#[test]
fn test_config_from_snapshot_filters_tabs() {
    // If no panes of a tab are valid, it's filtered out entirely.

    let state = single_tab_snapshot(PaneNodeSnapshot::Branch(BranchSnapshot {
        direction: SplitDirection::Vertical,
        children: vec![(
            PaneFlex(1.),
            PaneNodeSnapshot::Leaf(LeafSnapshot {
                is_focused: true,
                custom_vertical_tabs_title: None,
                contents: LeafContents::Notebook(NotebookPaneSnapshot::CloudNotebook {
                    notebook_id: None,
                    settings: OpenWarpDriveObjectSettings::default(),
                }),
            }),
        )],
    }));

    let template = LaunchConfig::from_snapshot("Test".into(), &state);
    assert!(template.windows[0].tabs.is_empty())
}

#[test]
fn test_tab_level_commands_are_applied_to_leaf_layout() {
    let config: LaunchConfig = serde_yaml::from_str(
        r#"
name: Legacy Commands
windows:
  - tabs:
      - layout:
          cwd: /tmp
        commands:
          - exec: echo hello
"#,
    )
    .expect("launch config should parse");

    let layout = config.windows[0].tabs[0].layout_with_tab_commands();

    assert_eq!(
        layout,
        PaneTemplateType::PaneTemplate {
            cwd: PathBuf::from("/tmp"),
            commands: vec![CommandTemplate {
                exec: "echo hello".to_string()
            }],
            is_focused: None,
            pane_mode: PaneMode::Terminal,
            shell: None,
        }
    );
}

#[test]
fn test_tab_level_commands_are_applied_to_focused_pane_in_branch_layout() {
    let config: LaunchConfig = serde_yaml::from_str(
        r#"
name: Legacy Commands
windows:
  - tabs:
      - layout:
          split_direction: horizontal
          panes:
            - cwd: /tmp/left
              is_focused: false
            - cwd: /tmp/right
              is_focused: true
        commands:
          - exec: echo focused
"#,
    )
    .expect("launch config should parse");

    let layout = config.windows[0].tabs[0].layout_with_tab_commands();

    assert_eq!(
        layout,
        PaneTemplateType::PaneBranchTemplate {
            split_direction: SplitDirection::Horizontal.into(),
            panes: vec![
                PaneTemplateType::PaneTemplate {
                    cwd: PathBuf::from("/tmp/left"),
                    commands: vec![],
                    is_focused: Some(false),
                    pane_mode: PaneMode::Terminal,
                    shell: None,
                },
                PaneTemplateType::PaneTemplate {
                    cwd: PathBuf::from("/tmp/right"),
                    commands: vec![CommandTemplate {
                        exec: "echo focused".to_string()
                    }],
                    is_focused: Some(true),
                    pane_mode: PaneMode::Terminal,
                    shell: None,
                },
            ],
        }
    );
}

#[test]
fn test_tab_level_commands_are_applied_to_first_pane_without_focused_pane() {
    let config: LaunchConfig = serde_yaml::from_str(
        r#"
name: Legacy Commands
windows:
  - tabs:
      - layout:
          split_direction: horizontal
          panes:
            - cwd: /tmp/left
            - cwd: /tmp/right
        commands:
          - exec: echo first
"#,
    )
    .expect("launch config should parse");

    let layout = config.windows[0].tabs[0].layout_with_tab_commands();

    assert_eq!(
        layout,
        PaneTemplateType::PaneBranchTemplate {
            split_direction: SplitDirection::Horizontal.into(),
            panes: vec![
                PaneTemplateType::PaneTemplate {
                    cwd: PathBuf::from("/tmp/left"),
                    commands: vec![CommandTemplate {
                        exec: "echo first".to_string()
                    }],
                    is_focused: None,
                    pane_mode: PaneMode::Terminal,
                    shell: None,
                },
                PaneTemplateType::PaneTemplate {
                    cwd: PathBuf::from("/tmp/right"),
                    commands: vec![],
                    is_focused: None,
                    pane_mode: PaneMode::Terminal,
                    shell: None,
                },
            ],
        }
    );
}

#[test]
fn test_config_with_active_tab_index() {
    let state = multi_tab_snapshot(
        1,
        vec![
            TabSnapshot {
                custom_title: None,
                default_directory_color: None,
                selected_color: SelectedTabColor::default(),
                root: PaneNodeSnapshot::Branch(BranchSnapshot {
                    direction: SplitDirection::Vertical,
                    children: vec![(
                        PaneFlex(1.),
                        PaneNodeSnapshot::Leaf(LeafSnapshot {
                            is_focused: true,
                            custom_vertical_tabs_title: None,
                            contents: LeafContents::Terminal(TerminalPaneSnapshot {
                                uuid: vec![],
                                cwd: Some("/path/to/dir".into()),
                                is_active: true,
                                is_read_only: false,
                                shell_launch_data: None,
                                input_config: None,
                                llm_model_override: None,
                                active_profile_id: None,
                                conversation_ids_to_restore: vec![],
                                active_conversation_id: None,
                            }),
                        }),
                    )],
                }),
                left_panel: None,
                right_panel: None,
                group_id: None,
                pinned: false,
            };
            3
        ],
    );

    let template = LaunchConfig::from_snapshot("Test".into(), &state);
    assert_eq!(template.windows[0].active_tab_index, Some(1))
}

#[test]
fn test_config_with_active_tab_index_and_filtered_tabs() {
    let state = multi_tab_snapshot(
        1,
        vec![
            TabSnapshot {
                custom_title: None,
                default_directory_color: None,
                selected_color: SelectedTabColor::default(),
                root: PaneNodeSnapshot::Branch(BranchSnapshot {
                    direction: SplitDirection::Vertical,
                    children: vec![(
                        PaneFlex(1.),
                        PaneNodeSnapshot::Leaf(LeafSnapshot {
                            is_focused: true,
                            custom_vertical_tabs_title: None,
                            contents: LeafContents::Notebook(NotebookPaneSnapshot::CloudNotebook {
                                notebook_id: None,
                                settings: OpenWarpDriveObjectSettings::default(),
                            }),
                        }),
                    )],
                }),
                left_panel: None,
                right_panel: None,
                group_id: None,
                pinned: false,
            },
            TabSnapshot {
                custom_title: None,
                default_directory_color: None,
                selected_color: SelectedTabColor::default(),
                root: PaneNodeSnapshot::Branch(BranchSnapshot {
                    direction: SplitDirection::Vertical,
                    children: vec![(
                        PaneFlex(1.),
                        PaneNodeSnapshot::Leaf(LeafSnapshot {
                            is_focused: true,
                            custom_vertical_tabs_title: None,
                            contents: LeafContents::Terminal(TerminalPaneSnapshot {
                                uuid: vec![],
                                cwd: Some("/path/to/dir".into()),
                                is_active: true,
                                is_read_only: false,
                                shell_launch_data: None,
                                input_config: None,
                                llm_model_override: None,
                                active_profile_id: None,
                                conversation_ids_to_restore: vec![],
                                active_conversation_id: None,
                            }),
                        }),
                    )],
                }),
                left_panel: None,
                right_panel: None,
                group_id: None,
                pinned: false,
            },
        ],
    );

    let template = LaunchConfig::from_snapshot("Test".into(), &state);
    assert_eq!(template.windows[0].active_tab_index, Some(0))
}

#[test]
fn test_config_with_active_tab_being_filtered() {
    let state = multi_tab_snapshot(
        1,
        vec![
            TabSnapshot {
                custom_title: None,
                default_directory_color: None,
                selected_color: SelectedTabColor::default(),
                root: PaneNodeSnapshot::Branch(BranchSnapshot {
                    direction: SplitDirection::Vertical,
                    children: vec![(
                        PaneFlex(1.),
                        PaneNodeSnapshot::Leaf(LeafSnapshot {
                            is_focused: true,
                            custom_vertical_tabs_title: None,
                            contents: LeafContents::Terminal(TerminalPaneSnapshot {
                                uuid: vec![],
                                cwd: Some("/path/to/dir".into()),
                                is_active: true,
                                is_read_only: false,
                                shell_launch_data: None,
                                input_config: None,
                                llm_model_override: None,
                                active_profile_id: None,
                                conversation_ids_to_restore: vec![],
                                active_conversation_id: None,
                            }),
                        }),
                    )],
                }),
                left_panel: None,
                right_panel: None,
                group_id: None,
                pinned: false,
            },
            TabSnapshot {
                custom_title: None,
                default_directory_color: None,
                selected_color: SelectedTabColor::default(),
                root: PaneNodeSnapshot::Branch(BranchSnapshot {
                    direction: SplitDirection::Vertical,
                    children: vec![(
                        PaneFlex(1.),
                        PaneNodeSnapshot::Leaf(LeafSnapshot {
                            is_focused: true,
                            custom_vertical_tabs_title: None,
                            contents: LeafContents::Notebook(NotebookPaneSnapshot::CloudNotebook {
                                notebook_id: None,
                                settings: OpenWarpDriveObjectSettings::default(),
                            }),
                        }),
                    )],
                }),
                left_panel: None,
                right_panel: None,
                group_id: None,
                pinned: false,
            },
        ],
    );

    let template = LaunchConfig::from_snapshot("Test".into(), &state);
    assert_eq!(template.windows[0].active_tab_index, None)
}

// ---------------------------------------------------------------------------
// Tab groups (#13898)
// ---------------------------------------------------------------------------

fn terminal_tab(cwd: &str, group_id: Option<TabGroupId>) -> TabSnapshot {
    TabSnapshot {
        custom_title: None,
        default_directory_color: None,
        selected_color: SelectedTabColor::default(),
        root: PaneNodeSnapshot::Leaf(LeafSnapshot {
            is_focused: true,
            custom_vertical_tabs_title: None,
            contents: LeafContents::Terminal(TerminalPaneSnapshot {
                uuid: vec![],
                cwd: Some(cwd.into()),
                is_active: true,
                is_read_only: false,
                shell_launch_data: None,
                input_config: None,
                llm_model_override: None,
                active_profile_id: None,
                conversation_ids_to_restore: vec![],
                active_conversation_id: None,
            }),
        }),
        left_panel: None,
        right_panel: None,
        group_id,
        pinned: false,
    }
}

/// A tab that cannot be saved into a launch config, so it drops out of the
/// template and shifts every later tab's index.
fn unsaveable_tab(group_id: Option<TabGroupId>) -> TabSnapshot {
    TabSnapshot {
        custom_title: None,
        default_directory_color: None,
        selected_color: SelectedTabColor::default(),
        root: PaneNodeSnapshot::Leaf(LeafSnapshot {
            is_focused: true,
            custom_vertical_tabs_title: None,
            contents: LeafContents::Notebook(NotebookPaneSnapshot::CloudNotebook {
                notebook_id: None,
                settings: OpenWarpDriveObjectSettings::default(),
            }),
        }),
        left_panel: None,
        right_panel: None,
        group_id,
        pinned: false,
    }
}

fn grouped_snapshot(tabs: Vec<TabSnapshot>, tab_groups: Vec<TabGroupSnapshot>) -> AppState {
    let mut state = multi_tab_snapshot(0, tabs);
    state.windows[0].tab_groups = tab_groups;
    state
}

fn group(name: &str, id: TabGroupId) -> TabGroupSnapshot {
    TabGroupSnapshot {
        id,
        name: Some(name.to_string()),
        color: SelectedTabColor::Color(AnsiColorIdentifier::Blue),
        collapsed: false,
        pinned: false,
    }
}

#[test]
fn test_config_from_snapshot_preserves_tab_groups() {
    let group_id = TabGroupId::new();
    let state = grouped_snapshot(
        vec![
            terminal_tab("/a", Some(group_id)),
            terminal_tab("/b", None),
            terminal_tab("/c", Some(group_id)),
        ],
        vec![group("backend", group_id)],
    );

    let config = LaunchConfig::from_snapshot("test".to_string(), &state);
    let window = &config.windows[0];

    assert_eq!(window.tab_groups.len(), 1);
    assert_eq!(window.tab_groups[0].name.as_deref(), Some("backend"));
    assert_eq!(window.tab_groups[0].color, Some(AnsiColorIdentifier::Blue));

    // Membership survives, and an ungrouped tab stays ungrouped.
    assert_eq!(window.tabs[0].group, Some(0));
    assert_eq!(window.tabs[1].group, None);
    assert_eq!(window.tabs[2].group, Some(0));
}

#[test]
fn test_config_from_snapshot_remaps_groups_around_unsaveable_tabs() {
    // The first group's only tab cannot be saved, so that group must not
    // survive -- and the second group's index must shift down with it.
    // Membership is carried on each tab's `group_id`, never on its position,
    // which is what makes this hold once the tab list is renumbered.
    let dropped_group = TabGroupId::new();
    let kept_group = TabGroupId::new();

    let state = grouped_snapshot(
        vec![
            unsaveable_tab(Some(dropped_group)),
            terminal_tab("/a", None),
            terminal_tab("/b", Some(kept_group)),
        ],
        vec![group("cloud", dropped_group), group("local", kept_group)],
    );

    let config = LaunchConfig::from_snapshot("test".to_string(), &state);
    let window = &config.windows[0];

    assert_eq!(window.tabs.len(), 2);
    assert_eq!(window.tab_groups.len(), 1, "empty group must be dropped");
    assert_eq!(window.tab_groups[0].name.as_deref(), Some("local"));

    assert_eq!(window.tabs[0].group, None);
    assert_eq!(
        window.tabs[1].group,
        Some(0),
        "the surviving group moved from index 1 to 0"
    );
}

#[test]
fn test_config_from_snapshot_omits_tab_groups_when_there_are_none() {
    // Configs saved from ungrouped windows must serialize exactly as before,
    // so existing launch configs keep round-tripping unchanged.
    let state = grouped_snapshot(vec![terminal_tab("/a", None)], vec![]);

    let config = LaunchConfig::from_snapshot("test".to_string(), &state);

    assert!(config.windows[0].tab_groups.is_empty());
    assert_eq!(config.windows[0].tabs[0].group, None);

    let yaml = serde_yaml::to_string(&config).expect("serializes");
    assert!(!yaml.contains("tab_groups"), "got:\n{yaml}");
    assert!(!yaml.contains("group:"), "got:\n{yaml}");
}

fn tab_in_group(group: Option<usize>) -> TabTemplate {
    TabTemplate {
        title: None,
        layout: PaneTemplateType::PaneTemplate {
            cwd: PathBuf::from("/tmp"),
            commands: vec![],
            is_focused: None,
            pane_mode: PaneMode::Terminal,
            shell: None,
        },
        commands: vec![],
        color: None,
        group,
    }
}

#[test]
fn test_resolve_group_memberships_keeps_contiguous_runs_intact() {
    let tabs = vec![
        tab_in_group(Some(0)),
        tab_in_group(Some(0)),
        tab_in_group(None),
        tab_in_group(Some(1)),
    ];

    assert_eq!(
        super::resolve_group_memberships(&tabs, 2),
        vec![Some(0), Some(0), None, Some(1)]
    );
}

#[test]
fn test_resolve_group_memberships_ungroups_a_split_run() {
    // The tab bar renders each contiguous run as its own container, so
    // honoring the second run would draw two containers with one group id.
    let tabs = vec![
        tab_in_group(Some(0)),
        tab_in_group(None),
        tab_in_group(Some(0)),
    ];

    assert_eq!(
        super::resolve_group_memberships(&tabs, 1),
        vec![Some(0), None, None],
        "the group's second run must not reopen it"
    );
}

#[test]
fn test_resolve_group_memberships_ungroups_a_run_split_by_another_group() {
    let tabs = vec![
        tab_in_group(Some(0)),
        tab_in_group(Some(1)),
        tab_in_group(Some(0)),
    ];

    assert_eq!(
        super::resolve_group_memberships(&tabs, 2),
        vec![Some(0), Some(1), None]
    );
}

#[test]
fn test_resolve_group_memberships_drops_out_of_range_indices() {
    // Hand-edited YAML pointing past the end of `tab_groups`.
    let tabs = vec![tab_in_group(Some(7)), tab_in_group(Some(0))];

    assert_eq!(
        super::resolve_group_memberships(&tabs, 1),
        vec![None, Some(0)]
    );
}
