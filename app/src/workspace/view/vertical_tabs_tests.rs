use std::path::PathBuf;

use pathfinder_geometry::rect::RectF;
use pathfinder_geometry::vector::Vector2F;
use warpui::EntityId;
use warpui::elements::PositionedElementOffsetBounds;

use super::{
    AgentTabTextPreference, SummaryPaneKind, SummaryPaneKindIcons, TerminalAgentText,
    TerminalPrimaryLineData, TerminalPrimaryLineFont, VerticalTabsDetailTarget,
    VerticalTabsDetailTargetKind, VerticalTabsSummaryBranchEntry, VerticalTabsSummaryData,
    VerticalTabsSummaryPrimaryLabel, branch_label_display, coalesce_summary_branch_entries,
    code_detail_kind_label, compact_branch_subtitle_display, detail_sidecar_width_and_bounds,
    detail_target_for_hovered_row, non_terminal_search_text_fragments,
    pane_ids_for_display_granularity, pane_search_text_fragments, preferred_agent_tab_titles,
    push_normalized_unique_summary_label, search_fragments_contain_query,
    select_summary_pane_kind_icons, should_keep_detail_sidecar_visible_for_mouse_position,
    should_show_tab_group_header, sort_summary_primary_labels_status_first, summary_overflow_count,
    summary_search_text_fragments, terminal_kind_badge_label, terminal_primary_line_data,
    terminal_pull_request_badge_label, terminal_search_text_fragments,
    terminal_title_fallback_font, uses_outer_group_container, visible_pane_ids_for_detail_target,
    vtab_diff_stats_text,
};
use crate::ai::agent::conversation::ConversationStatus;
use crate::context_chips::display_chip::GitLineChanges;
use crate::pane_group::pane::IPaneType;
use crate::pane_group::{PaneId, TerminalPaneId};
use crate::safe_triangle::SafeTriangle;
use crate::terminal::CLIAgent;
use crate::workspace::tab_settings::VerticalTabsDisplayGranularity;

fn label(text: &str) -> VerticalTabsSummaryPrimaryLabel {
    VerticalTabsSummaryPrimaryLabel {
        text: text.to_string(),
        status: None,
    }
}

fn pane_id() -> PaneId {
    TerminalPaneId::dummy_terminal_pane_id().into()
}
fn code_summary_kind(title: &str) -> SummaryPaneKind {
    SummaryPaneKind::Code {
        title: title.to_string(),
    }
}

#[test]
fn summary_pane_kind_icons_render_single_icon_for_homogeneous_tabs() {
    assert_eq!(
        select_summary_pane_kind_icons([
            (EntityId::from_usize(10), SummaryPaneKind::Terminal),
            (EntityId::from_usize(20), SummaryPaneKind::Terminal),
        ]),
        Some(SummaryPaneKindIcons::Single(SummaryPaneKind::Terminal))
    );
}

#[test]
fn summary_pane_kind_icons_pick_two_oldest_distinct_pane_kinds() {
    assert_eq!(
        select_summary_pane_kind_icons([
            (EntityId::from_usize(30), SummaryPaneKind::Terminal),
            (EntityId::from_usize(20), code_summary_kind("main.rs")),
            (
                EntityId::from_usize(40),
                SummaryPaneKind::Notebook { is_plan: false },
            ),
            (EntityId::from_usize(10), SummaryPaneKind::Terminal),
        ]),
        Some(SummaryPaneKindIcons::Pair {
            primary: SummaryPaneKind::Terminal,
            secondary: code_summary_kind("main.rs"),
        })
    );
}

#[test]
fn summary_pane_kind_icons_recompute_when_oldest_kind_is_removed() {
    assert_eq!(
        select_summary_pane_kind_icons([
            (EntityId::from_usize(20), code_summary_kind("main.rs")),
            (EntityId::from_usize(30), SummaryPaneKind::Terminal),
        ]),
        Some(SummaryPaneKindIcons::Pair {
            primary: code_summary_kind("main.rs"),
            secondary: SummaryPaneKind::Terminal,
        })
    );
}

#[test]
fn summary_pane_kind_icons_distinguish_agent_terminals_from_plain_terminals() {
    assert_eq!(
        select_summary_pane_kind_icons([
            (EntityId::from_usize(10), SummaryPaneKind::Terminal),
            (
                EntityId::from_usize(20),
                SummaryPaneKind::CLIAgent {
                    agent: CLIAgent::Claude,
                    is_ambient: false,
                },
            ),
            (
                EntityId::from_usize(30),
                SummaryPaneKind::OzAgent { is_ambient: false },
            ),
        ]),
        Some(SummaryPaneKindIcons::Pair {
            primary: SummaryPaneKind::Terminal,
            secondary: SummaryPaneKind::CLIAgent {
                agent: CLIAgent::Claude,
                is_ambient: false,
            },
        })
    );
}

#[test]
fn summary_pane_kind_icons_distinguish_ambient_claude_from_local_claude() {
    // A local Claude session and a cloud-mode Claude session should count as distinct kinds
    // so they render with different icons (claude.svg vs claude_cloud.svg).
    assert_eq!(
        select_summary_pane_kind_icons([
            (
                EntityId::from_usize(10),
                SummaryPaneKind::CLIAgent {
                    agent: CLIAgent::Claude,
                    is_ambient: false,
                },
            ),
            (
                EntityId::from_usize(20),
                SummaryPaneKind::CLIAgent {
                    agent: CLIAgent::Claude,
                    is_ambient: true,
                },
            ),
        ]),
        Some(SummaryPaneKindIcons::Pair {
            primary: SummaryPaneKind::CLIAgent {
                agent: CLIAgent::Claude,
                is_ambient: false,
            },
            secondary: SummaryPaneKind::CLIAgent {
                agent: CLIAgent::Claude,
                is_ambient: true,
            },
        })
    );
}

#[test]
fn preferred_agent_tab_titles_default_to_title_like_text() {
    let agent_text = TerminalAgentText {
        conversation_display_title: Some("Generated Oz title".to_string()),
        conversation_latest_user_prompt: Some("Latest Oz prompt".to_string()),
        cli_agent_title: Some("CLI summary".to_string()),
        cli_agent_latest_user_prompt: Some("Latest CLI prompt".to_string()),
        is_oz_agent: true,
        cli_agent: Some(CLIAgent::Claude),
    };

    assert_eq!(
        preferred_agent_tab_titles(&agent_text, AgentTabTextPreference::ConversationTitle),
        (
            Some("Generated Oz title".to_string()),
            Some("CLI summary".to_string())
        )
    );
}

#[test]
fn preferred_agent_tab_titles_do_not_use_cli_prompt_when_disabled() {
    let agent_text = TerminalAgentText {
        conversation_display_title: None,
        conversation_latest_user_prompt: None,
        cli_agent_title: None,
        cli_agent_latest_user_prompt: Some("Latest CLI prompt".to_string()),
        is_oz_agent: false,
        cli_agent: Some(CLIAgent::Claude),
    };

    assert_eq!(
        preferred_agent_tab_titles(&agent_text, AgentTabTextPreference::ConversationTitle),
        (None, None)
    );
}

#[test]
fn terminal_primary_line_uses_terminal_title_when_disabled_cli_has_only_prompt() {
    let agent_text = TerminalAgentText {
        conversation_display_title: None,
        conversation_latest_user_prompt: None,
        cli_agent_title: None,
        cli_agent_latest_user_prompt: Some("Latest CLI prompt".to_string()),
        is_oz_agent: false,
        cli_agent: Some(CLIAgent::Claude),
    };
    let (conversation_title, cli_title) =
        preferred_agent_tab_titles(&agent_text, AgentTabTextPreference::ConversationTitle);

    let line = terminal_primary_line_data(
        false,
        conversation_title,
        cli_title,
        "Generated Claude Code title",
        "~/warp",
        terminal_title_fallback_font(&agent_text),
        Some("claude".to_string()),
    );

    assert_eq!(line.text(), "Generated Claude Code title");
    assert!(matches!(
        line,
        TerminalPrimaryLineData::Text {
            font: TerminalPrimaryLineFont::Ui,
            ..
        }
    ));
}

#[test]
fn preferred_agent_tab_titles_use_latest_prompt_when_enabled() {
    let agent_text = TerminalAgentText {
        conversation_display_title: Some("Generated Oz title".to_string()),
        conversation_latest_user_prompt: Some("Latest Oz prompt".to_string()),
        cli_agent_title: Some("CLI summary".to_string()),
        cli_agent_latest_user_prompt: Some("Latest CLI prompt".to_string()),
        is_oz_agent: true,
        cli_agent: Some(CLIAgent::Claude),
    };

    assert_eq!(
        preferred_agent_tab_titles(&agent_text, AgentTabTextPreference::LatestUserPrompt),
        (
            Some("Latest Oz prompt".to_string()),
            Some("Latest CLI prompt".to_string())
        )
    );
}

#[test]
fn terminal_primary_line_uses_cli_prompt_when_enabled_cli_has_prompt() {
    let agent_text = TerminalAgentText {
        conversation_display_title: None,
        conversation_latest_user_prompt: None,
        cli_agent_title: None,
        cli_agent_latest_user_prompt: Some("Latest CLI prompt".to_string()),
        is_oz_agent: false,
        cli_agent: Some(CLIAgent::Claude),
    };
    let (conversation_title, cli_title) =
        preferred_agent_tab_titles(&agent_text, AgentTabTextPreference::LatestUserPrompt);

    let line = terminal_primary_line_data(
        false,
        conversation_title,
        cli_title,
        "Generated Claude Code title",
        "~/warp",
        terminal_title_fallback_font(&agent_text),
        Some("claude".to_string()),
    );

    assert_eq!(line.text(), "Latest CLI prompt");
}

#[test]
fn terminal_primary_line_uses_cli_prompt_when_enabled_cli_is_long_running() {
    let agent_text = TerminalAgentText {
        conversation_display_title: None,
        conversation_latest_user_prompt: None,
        cli_agent_title: None,
        cli_agent_latest_user_prompt: Some("Latest CLI prompt".to_string()),
        is_oz_agent: false,
        cli_agent: Some(CLIAgent::Claude),
    };
    let (conversation_title, cli_title) =
        preferred_agent_tab_titles(&agent_text, AgentTabTextPreference::LatestUserPrompt);

    let line = terminal_primary_line_data(
        true,
        conversation_title,
        cli_title,
        "Generated Claude Code title",
        "~/warp",
        terminal_title_fallback_font(&agent_text),
        Some("claude".to_string()),
    );

    assert_eq!(line.text(), "Latest CLI prompt");
}

#[test]
fn preferred_agent_tab_titles_fall_back_when_preferred_text_is_missing() {
    let agent_text = TerminalAgentText {
        conversation_display_title: Some("Generated Oz title".to_string()),
        conversation_latest_user_prompt: None,
        cli_agent_title: None,
        cli_agent_latest_user_prompt: Some("Latest CLI prompt".to_string()),
        is_oz_agent: true,
        cli_agent: Some(CLIAgent::Claude),
    };

    assert_eq!(
        preferred_agent_tab_titles(&agent_text, AgentTabTextPreference::LatestUserPrompt),
        (
            Some("Generated Oz title".to_string()),
            Some("Latest CLI prompt".to_string())
        )
    );
}

fn pane_type_supports_vertical_tabs_detail_sidecar(pane_type: IPaneType) -> bool {
    matches!(
        pane_type,
        IPaneType::Terminal
            | IPaneType::Code
            | IPaneType::Notebook
            | IPaneType::Workflow
            | IPaneType::EnvVarCollection
            | IPaneType::AIFact
            | IPaneType::AIDocument
    )
}

fn collect_normalized_unique_summary_texts(
    texts: impl IntoIterator<Item = impl AsRef<str>>,
) -> Vec<String> {
    texts
        .into_iter()
        .filter_map(|text| {
            let normalized = text
                .as_ref()
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ");
            (!normalized.is_empty()).then_some(normalized)
        })
        .fold(Vec::new(), |mut values, normalized| {
            if !values.contains(&normalized) {
                values.push(normalized);
            }
            values
        })
}

#[test]
fn detail_sidecar_supports_terminal_code_and_warp_drive_object_panes() {
    assert!(pane_type_supports_vertical_tabs_detail_sidecar(
        IPaneType::Terminal
    ));
    assert!(pane_type_supports_vertical_tabs_detail_sidecar(
        IPaneType::Code
    ));
    assert!(pane_type_supports_vertical_tabs_detail_sidecar(
        IPaneType::Notebook
    ));
    assert!(pane_type_supports_vertical_tabs_detail_sidecar(
        IPaneType::Workflow
    ));
    assert!(pane_type_supports_vertical_tabs_detail_sidecar(
        IPaneType::EnvVarCollection
    ));
    assert!(pane_type_supports_vertical_tabs_detail_sidecar(
        IPaneType::AIFact
    ));
    assert!(pane_type_supports_vertical_tabs_detail_sidecar(
        IPaneType::AIDocument
    ));
    assert!(!pane_type_supports_vertical_tabs_detail_sidecar(
        IPaneType::Settings
    ));
}

#[test]
fn code_detail_kind_label_uses_programming_language_display_name() {
    assert_eq!(
        code_detail_kind_label("block_id.rs"),
        Some("Rust".to_string())
    );
    assert_eq!(
        code_detail_kind_label("Dockerfile"),
        Some("Dockerfile".to_string())
    );
}

#[test]
fn code_detail_kind_label_returns_none_when_language_is_unknown() {
    assert_eq!(code_detail_kind_label("notes.txt"), None);
}

#[test]
fn detail_target_matches_panes_granularity() {
    let pane_group_id = EntityId::new();
    let hovered_pane_id = pane_id();

    assert_eq!(
        detail_target_for_hovered_row(
            pane_group_id,
            hovered_pane_id,
            VerticalTabsDisplayGranularity::Panes,
        ),
        VerticalTabsDetailTarget::Pane {
            pane_group_id,
            pane_id: hovered_pane_id,
        }
    );
}

#[test]
fn detail_target_matches_tabs_granularity() {
    let pane_group_id = EntityId::new();
    let hovered_pane_id = pane_id();

    assert_eq!(
        detail_target_for_hovered_row(
            pane_group_id,
            hovered_pane_id,
            VerticalTabsDisplayGranularity::Tabs,
        ),
        VerticalTabsDetailTarget::Tab {
            pane_group_id,
            source_pane_id: hovered_pane_id,
        }
    );
}

#[test]
fn pane_detail_target_returns_hovered_pane_when_supported() {
    let hovered_pane_id = pane_id();

    assert_eq!(
        visible_pane_ids_for_detail_target(
            &[hovered_pane_id],
            hovered_pane_id,
            VerticalTabsDetailTargetKind::Pane,
            |pane_id| pane_id == hovered_pane_id,
        ),
        Some(vec![hovered_pane_id])
    );
}

#[test]
fn pane_detail_target_returns_none_when_hovered_pane_is_not_supported() {
    let hovered_pane_id = pane_id();

    assert_eq!(
        visible_pane_ids_for_detail_target(
            &[hovered_pane_id],
            hovered_pane_id,
            VerticalTabsDetailTargetKind::Pane,
            |_| false,
        ),
        None
    );
}

#[test]
fn tab_detail_target_returns_all_visible_panes_when_every_pane_is_supported() {
    let pane_1 = pane_id();
    let pane_2 = pane_id();
    let pane_3 = pane_id();
    let visible_pane_ids = vec![pane_1, pane_2, pane_3];

    assert_eq!(
        visible_pane_ids_for_detail_target(
            &visible_pane_ids,
            pane_2,
            VerticalTabsDetailTargetKind::Tab,
            |_| true,
        ),
        Some(visible_pane_ids)
    );
}

#[test]
fn tab_detail_target_returns_none_for_mixed_support_tabs() {
    let pane_1 = pane_id();
    let pane_2 = pane_id();
    let pane_3 = pane_id();

    assert_eq!(
        visible_pane_ids_for_detail_target(
            &[pane_1, pane_2, pane_3],
            pane_2,
            VerticalTabsDetailTargetKind::Tab,
            |pane_id| pane_id != pane_3,
        ),
        None
    );
}

#[test]
fn panes_granularity_returns_all_visible_panes_in_order() {
    let pane_1 = pane_id();
    let pane_2 = pane_id();
    let pane_3 = pane_id();
    let visible_pane_ids = vec![pane_1, pane_2, pane_3];

    assert_eq!(
        pane_ids_for_display_granularity(
            &visible_pane_ids,
            pane_2,
            VerticalTabsDisplayGranularity::Panes,
        ),
        visible_pane_ids
    );
}

#[test]
fn tabs_granularity_returns_focused_pane_when_present() {
    let pane_1 = pane_id();
    let pane_2 = pane_id();
    let pane_3 = pane_id();

    assert_eq!(
        pane_ids_for_display_granularity(
            &[pane_1, pane_2, pane_3],
            pane_2,
            VerticalTabsDisplayGranularity::Tabs,
        ),
        vec![pane_2]
    );
}

#[test]
fn tabs_granularity_falls_back_to_first_visible_pane_when_focused_pane_is_absent() {
    let pane_1 = pane_id();
    let pane_2 = pane_id();
    let pane_3 = pane_id();
    let focused_pane = pane_id();

    assert_eq!(
        pane_ids_for_display_granularity(
            &[pane_1, pane_2, pane_3],
            focused_pane,
            VerticalTabsDisplayGranularity::Tabs,
        ),
        vec![pane_1]
    );
}

#[test]
fn tabs_granularity_returns_empty_for_empty_visible_panes() {
    assert_eq!(
        pane_ids_for_display_granularity(&[], pane_id(), VerticalTabsDisplayGranularity::Tabs,),
        Vec::<PaneId>::new()
    );
}

#[test]
fn detail_sidecar_uses_default_width_when_space_allows() {
    let (width, bounds) = detail_sidecar_width_and_bounds(400.);
    assert_eq!(width, 320.);
    assert!(matches!(
        bounds,
        PositionedElementOffsetBounds::WindowBySize
    ));
}

#[test]
fn detail_sidecar_shrinks_to_fit_before_hitting_min_width() {
    let (width, bounds) = detail_sidecar_width_and_bounds(280.);
    assert_eq!(width, 280.);
    assert!(matches!(
        bounds,
        PositionedElementOffsetBounds::WindowBySize
    ));
}

#[test]
fn detail_sidecar_stops_shrinking_at_min_width_and_allows_clipping() {
    let (width, bounds) = detail_sidecar_width_and_bounds(180.);
    assert_eq!(width, 240.);
    assert!(matches!(bounds, PositionedElementOffsetBounds::Unbounded));
}

#[test]
fn detail_sidecar_visibility_helper_keeps_sidecar_visible_inside_sidecar_bounds() {
    let row_rect = RectF::new(Vector2F::new(0., 100.), Vector2F::new(100., 40.));
    let sidecar_rect = RectF::new(Vector2F::new(120., 50.), Vector2F::new(180., 220.));
    let mut safe_triangle = SafeTriangle::new();

    assert!(should_keep_detail_sidecar_visible_for_mouse_position(
        Vector2F::new(200., 120.),
        Some(row_rect),
        Some(sidecar_rect),
        &mut safe_triangle,
    ));
}

#[test]
fn detail_sidecar_visibility_helper_keeps_sidecar_visible_in_safe_triangle() {
    let row_rect = RectF::new(Vector2F::new(0., 100.), Vector2F::new(100., 40.));
    let sidecar_rect = RectF::new(Vector2F::new(120., 50.), Vector2F::new(180., 220.));
    let mut safe_triangle = SafeTriangle::new();
    safe_triangle.set_target_rect(Some(sidecar_rect));
    safe_triangle.update_position(Vector2F::new(90., 120.));

    assert!(should_keep_detail_sidecar_visible_for_mouse_position(
        Vector2F::new(110., 120.),
        Some(row_rect),
        Some(sidecar_rect),
        &mut safe_triangle,
    ));
}

#[test]
fn detail_sidecar_visibility_helper_clears_sidecar_outside_row_sidecar_and_safe_triangle() {
    let row_rect = RectF::new(Vector2F::new(0., 100.), Vector2F::new(100., 40.));
    let sidecar_rect = RectF::new(Vector2F::new(120., 50.), Vector2F::new(180., 220.));
    let mut safe_triangle = SafeTriangle::new();
    safe_triangle.update_position(Vector2F::new(200., 120.));

    assert!(!should_keep_detail_sidecar_visible_for_mouse_position(
        Vector2F::new(340., 120.),
        Some(row_rect),
        Some(sidecar_rect),
        &mut safe_triangle,
    ));
}

#[test]
fn panes_granularity_uses_outer_group_container() {
    assert!(uses_outer_group_container(
        VerticalTabsDisplayGranularity::Panes
    ));
}

#[test]
fn tabs_granularity_does_not_use_outer_group_container() {
    assert!(!uses_outer_group_container(
        VerticalTabsDisplayGranularity::Tabs
    ));
}

// Regression coverage for #9098 ("Tab names not rendered in tab bar, only
// first tab shows name"). The header gate previously read `has_custom_title
// || is_being_renamed`, which collapsed to `false` for every tab without a
// user-set rename — leaving multi-pane tabs with auto-generated names
// looking like they had no tab label at all. The new gate keeps the existing
// triggers and adds "any multi-pane tab", so every multi-pane group has a
// stable tab-level identifier in `Panes` granularity.
#[test]
fn tab_group_header_shows_for_custom_title() {
    assert!(should_show_tab_group_header(true, false, 1));
    assert!(should_show_tab_group_header(true, false, 3));
}

#[test]
fn tab_group_header_shows_while_renaming() {
    // The inline rename editor must always be reachable, even on
    // single-pane tabs with no prior custom title.
    assert!(should_show_tab_group_header(false, true, 1));
}

#[test]
fn tab_group_header_shows_for_multi_pane_tabs_without_custom_title() {
    // The #9098 case: an auto-named multi-pane tab. Each row only shows the
    // per-pane title (e.g. `travelplan` + `main`), so without a group header
    // there is no way to tell two such tabs apart in the sidebar.
    assert!(should_show_tab_group_header(false, false, 2));
    assert!(should_show_tab_group_header(false, false, 5));
}

#[test]
fn tab_group_header_hidden_for_single_pane_without_custom_title() {
    // Single-pane groups already surface the pane title in their only row.
    // Rendering the same string again as a header would duplicate it
    // immediately above itself, so the gate stays closed in this shape.
    assert!(!should_show_tab_group_header(false, false, 1));
    // Defensive: `0` should not crash or accidentally render a header for
    // an empty group (this shape shouldn't reach the renderer in practice,
    // but the helper is total and stays closed).
    assert!(!should_show_tab_group_header(false, false, 0));
}

#[test]
fn tab_group_header_distinguishes_two_auto_named_multi_pane_tabs() {
    // Models the screenshot in #9098: tab 1 has a custom title
    // ("Humanfigure"), tabs 2 and 3 are auto-named multi-pane groups
    // ("travelplan + main", "deponti + release/development"). Before the
    // fix only tab 1 showed a header; after the fix every multi-pane tab
    // gets one so the user can tell them apart at a glance.
    let renders_header: Vec<bool> = vec![
        should_show_tab_group_header(true, false, 2),  // tab 1
        should_show_tab_group_header(false, false, 2), // tab 2
        should_show_tab_group_header(false, false, 2), // tab 3
    ];
    assert_eq!(renders_header, vec![true, true, true]);
}

#[test]
fn terminal_primary_line_prefers_cli_agent_display_title() {
    let line = terminal_primary_line_data(
        false,
        None,
        Some("Review the failing tests".to_string()),
        "~/warp",
        "~/warp",
        TerminalPrimaryLineFont::Monospace,
        Some("cargo nextest run".to_string()),
    );

    assert_eq!(line.text(), "Review the failing tests");
}

#[test]
fn terminal_primary_line_prefers_cli_agent_display_title_over_conversation_title() {
    let line = terminal_primary_line_data(
        false,
        Some("Review the failing tests".to_string()),
        Some("Summarize the failures".to_string()),
        "~/warp",
        "~/warp",
        TerminalPrimaryLineFont::Monospace,
        Some("cargo nextest run".to_string()),
    );

    assert_eq!(line.text(), "Summarize the failures");
}

#[test]
fn terminal_primary_line_falls_through_to_terminal_title_when_cli_agent_has_no_plugin_data() {
    let line = terminal_primary_line_data(
        false,
        None,
        None,
        "codex - ~/warp",
        "~/warp",
        TerminalPrimaryLineFont::Monospace,
        Some("cargo nextest run".to_string()),
    );

    assert_eq!(line.text(), "codex - ~/warp");
}

#[test]
fn terminal_primary_line_uses_terminal_title_as_fallback() {
    let line = terminal_primary_line_data(
        false,
        None,
        None,
        "nvim src/workspace/view/vertical_tabs.rs",
        "~/warp",
        TerminalPrimaryLineFont::Monospace,
        Some("cargo nextest run".to_string()),
    );

    assert_eq!(line.text(), "nvim src/workspace/view/vertical_tabs.rs");
}

#[test]
fn terminal_primary_line_uses_last_completed_command_when_shell_title_matches_working_directory() {
    let line = terminal_primary_line_data(
        false,
        None,
        None,
        "~/warp",
        "~/warp",
        TerminalPrimaryLineFont::Monospace,
        Some("cargo nextest run".to_string()),
    );

    assert_eq!(line.text(), "cargo nextest run");
}

#[test]
fn terminal_primary_line_falls_back_to_new_session() {
    let line = terminal_primary_line_data(
        false,
        None,
        None,
        "~/warp",
        "~/warp",
        TerminalPrimaryLineFont::Monospace,
        None,
    );

    assert_eq!(line.text(), "New session");
    assert!(matches!(
        line,
        TerminalPrimaryLineData::Text {
            font: TerminalPrimaryLineFont::Ui,
            ..
        }
    ));
}

#[test]
fn terminal_primary_line_uses_monospace_for_last_completed_command() {
    let line = terminal_primary_line_data(
        false,
        None,
        None,
        "~/warp",
        "~/warp",
        TerminalPrimaryLineFont::Monospace,
        Some("cargo nextest run".to_string()),
    );

    assert!(matches!(
        line,
        TerminalPrimaryLineData::Text {
            font: TerminalPrimaryLineFont::Monospace,
            ..
        }
    ));
}

#[test]
fn terminal_search_fragments_include_rendered_terminal_badges() {
    let fragments = terminal_search_text_fragments(
        "Review the failing tests".to_string(),
        "~/warp".to_string(),
        Some("main".to_string()),
        terminal_kind_badge_label(false, Some(CLIAgent::Claude)),
        Some(terminal_pull_request_badge_label(
            "https://github.com/warpdotdev/warp-internal/pull/12345",
        )),
        Some(GitLineChanges {
            files_changed: 1,
            lines_added: 2,
            lines_removed: 3,
        }),
    );

    assert!(search_fragments_contain_query(&fragments, "claude"));
    assert!(search_fragments_contain_query(
        &fragments,
        "review the failing tests"
    ));
    assert!(search_fragments_contain_query(&fragments, "#12345"));
    assert!(search_fragments_contain_query(&fragments, "+2"));
    assert!(search_fragments_contain_query(&fragments, "-3"));
}

#[test]
fn pane_search_fragments_prepend_custom_title_and_keep_generated_metadata() {
    let fragments = pane_search_text_fragments(
        Some("Production API"),
        vec![
            "cargo nextest run".to_string(),
            "~/warp".to_string(),
            "Claude".to_string(),
        ],
    );

    assert_eq!(fragments[0], "Production API");
    assert!(search_fragments_contain_query(&fragments, "production api"));
    assert!(search_fragments_contain_query(&fragments, "cargo nextest"));
    assert!(search_fragments_contain_query(&fragments, "~/warp"));
    assert!(search_fragments_contain_query(&fragments, "claude"));
}

#[test]
fn pane_search_fragments_dedupe_custom_title_against_generated_text() {
    assert_eq!(
        pane_search_text_fragments(
            Some("  Production   API  "),
            vec![
                "Production API".to_string(),
                "~/warp".to_string(),
                "~/warp".to_string(),
            ],
        ),
        vec!["Production API".to_string(), "~/warp".to_string()]
    );
}

#[test]
fn non_terminal_search_fragments_only_include_rendered_text() {
    let fragments = non_terminal_search_text_fragments("Pane title", "and 2 more");

    assert!(search_fragments_contain_query(&fragments, "pane title"));
    assert!(search_fragments_contain_query(&fragments, "and 2 more"));
    assert!(!search_fragments_contain_query(&fragments, "notebook"));
    assert!(!search_fragments_contain_query(&fragments, "unsaved"));
}

#[test]
fn diff_stats_text_matches_rendered_badge_text() {
    assert_eq!(
        vtab_diff_stats_text(&GitLineChanges {
            files_changed: 1,
            lines_added: 2,
            lines_removed: 3,
        }),
        "+2 -3"
    );
    assert_eq!(
        vtab_diff_stats_text(&GitLineChanges {
            files_changed: 1,
            lines_added: 0,
            lines_removed: 0,
        }),
        "0"
    );
}

#[test]
fn branch_label_display_falls_back_without_branch_icon() {
    assert_eq!(
        branch_label_display(None, "~/warp"),
        ("~/warp".to_string(), false)
    );
    assert_eq!(
        branch_label_display(Some(""), "~/warp"),
        ("~/warp".to_string(), false)
    );
    assert_eq!(
        branch_label_display(Some("main"), "~/warp"),
        ("main".to_string(), true)
    );
}

#[test]
fn compact_branch_subtitle_falls_back_to_working_directory_without_branch_icon() {
    assert_eq!(
        compact_branch_subtitle_display(None, Some("~/warp")),
        Some(("~/warp".to_string(), false))
    );
    assert_eq!(
        compact_branch_subtitle_display(Some(""), Some("~/warp")),
        Some(("~/warp".to_string(), false))
    );
    assert_eq!(
        compact_branch_subtitle_display(Some("main"), Some("~/warp")),
        Some(("main".to_string(), true))
    );
}

#[test]
fn collect_normalized_unique_summary_texts_dedupes_after_whitespace_normalization() {
    assert_eq!(
        collect_normalized_unique_summary_texts([
            "  cargo   test  ",
            "cargo test",
            "",
            " git   status ",
        ]),
        vec!["cargo test".to_string(), "git status".to_string()]
    );
}

#[test]
fn collect_normalized_unique_summary_texts_preserves_first_seen_order() {
    assert_eq!(
        collect_normalized_unique_summary_texts([
            "~/warp-internal",
            "~/warp-server",
            "~/warp-internal",
            "~/warp-terraform",
        ]),
        vec![
            "~/warp-internal".to_string(),
            "~/warp-server".to_string(),
            "~/warp-terraform".to_string(),
        ]
    );
}

#[test]
fn coalesce_summary_branch_entries_groups_by_repo_and_branch() {
    let repo_a = PathBuf::from("/tmp/repo-a");
    let repo_b = PathBuf::from("/tmp/repo-b");
    let entries = vec![
        VerticalTabsSummaryBranchEntry {
            repo_path: repo_a.clone(),
            branch_name: "main".to_string(),
            diff_stats: None,
            pull_request_label: None,
            pull_request_url: None,
        },
        VerticalTabsSummaryBranchEntry {
            repo_path: repo_a.clone(),
            branch_name: "main".to_string(),
            diff_stats: Some(GitLineChanges {
                files_changed: 1,
                lines_added: 2,
                lines_removed: 3,
            }),
            pull_request_label: Some("#123".to_string()),
            pull_request_url: Some("https://github.com/acme/repo-a/pull/123".to_string()),
        },
        VerticalTabsSummaryBranchEntry {
            repo_path: repo_b.clone(),
            branch_name: "main".to_string(),
            diff_stats: Some(GitLineChanges {
                files_changed: 4,
                lines_added: 5,
                lines_removed: 6,
            }),
            pull_request_label: Some("#456".to_string()),
            pull_request_url: Some("https://github.com/acme/repo-b/pull/456".to_string()),
        },
    ];

    assert_eq!(
        coalesce_summary_branch_entries(entries),
        vec![
            VerticalTabsSummaryBranchEntry {
                repo_path: repo_a,
                branch_name: "main".to_string(),
                diff_stats: Some(GitLineChanges {
                    files_changed: 1,
                    lines_added: 2,
                    lines_removed: 3,
                }),
                pull_request_label: Some("#123".to_string()),
                pull_request_url: Some("https://github.com/acme/repo-a/pull/123".to_string()),
            },
            VerticalTabsSummaryBranchEntry {
                repo_path: repo_b,
                branch_name: "main".to_string(),
                diff_stats: Some(GitLineChanges {
                    files_changed: 4,
                    lines_added: 5,
                    lines_removed: 6,
                }),
                pull_request_label: Some("#456".to_string()),
                pull_request_url: Some("https://github.com/acme/repo-b/pull/456".to_string()),
            },
        ]
    );
}

#[test]
fn summary_overflow_count_caps_visible_region() {
    assert_eq!(summary_overflow_count(5, 3), 2);
    assert_eq!(summary_overflow_count(3, 3), 0);
    assert_eq!(summary_overflow_count(2, 3), 0);
}

#[test]
fn primary_labels_dedupe_preserves_first_seen_status() {
    let mut values = Vec::new();
    let mut seen = std::collections::HashMap::new();
    push_normalized_unique_summary_label(&mut values, &mut seen, "  cargo   test  ", None);
    push_normalized_unique_summary_label(
        &mut values,
        &mut seen,
        "cargo test",
        Some(ConversationStatus::InProgress),
    );

    assert_eq!(
        values,
        vec![VerticalTabsSummaryPrimaryLabel {
            text: "cargo test".to_string(),
            status: None,
        }]
    );
}

#[test]
fn primary_labels_preserve_status_through_aggregation() {
    let mut values = Vec::new();
    let mut seen = std::collections::HashMap::new();
    push_normalized_unique_summary_label(
        &mut values,
        &mut seen,
        "Plan a refactor",
        Some(ConversationStatus::InProgress),
    );
    push_normalized_unique_summary_label(
        &mut values,
        &mut seen,
        "Investigate failure",
        Some(ConversationStatus::Success),
    );
    push_normalized_unique_summary_label(&mut values, &mut seen, "cargo build", None);

    assert_eq!(
        values,
        vec![
            VerticalTabsSummaryPrimaryLabel {
                text: "Plan a refactor".to_string(),
                status: Some(ConversationStatus::InProgress),
            },
            VerticalTabsSummaryPrimaryLabel {
                text: "Investigate failure".to_string(),
                status: Some(ConversationStatus::Success),
            },
            VerticalTabsSummaryPrimaryLabel {
                text: "cargo build".to_string(),
                status: None,
            },
        ]
    );
}

#[test]
fn sort_summary_primary_labels_moves_status_first_and_preserves_order() {
    let mut values = vec![
        VerticalTabsSummaryPrimaryLabel {
            text: "plain terminal".to_string(),
            status: None,
        },
        VerticalTabsSummaryPrimaryLabel {
            text: "first conversation".to_string(),
            status: Some(ConversationStatus::InProgress),
        },
        VerticalTabsSummaryPrimaryLabel {
            text: "code pane".to_string(),
            status: None,
        },
        VerticalTabsSummaryPrimaryLabel {
            text: "second conversation".to_string(),
            status: Some(ConversationStatus::Success),
        },
        VerticalTabsSummaryPrimaryLabel {
            text: "last terminal".to_string(),
            status: None,
        },
    ];

    sort_summary_primary_labels_status_first(&mut values);

    assert_eq!(
        values,
        vec![
            VerticalTabsSummaryPrimaryLabel {
                text: "first conversation".to_string(),
                status: Some(ConversationStatus::InProgress),
            },
            VerticalTabsSummaryPrimaryLabel {
                text: "second conversation".to_string(),
                status: Some(ConversationStatus::Success),
            },
            VerticalTabsSummaryPrimaryLabel {
                text: "plain terminal".to_string(),
                status: None,
            },
            VerticalTabsSummaryPrimaryLabel {
                text: "code pane".to_string(),
                status: None,
            },
            VerticalTabsSummaryPrimaryLabel {
                text: "last terminal".to_string(),
                status: None,
            },
        ]
    );
}

#[test]
fn summary_search_fragments_include_hidden_overflow_values() {
    let summary = VerticalTabsSummaryData {
        primary_labels: vec![
            VerticalTabsSummaryPrimaryLabel {
                text: "Claude".to_string(),
                status: Some(ConversationStatus::InProgress),
            },
            label("Oz"),
            label("cargo"),
            label("code review"),
            label("hidden work"),
        ],
        working_directories: vec!["~/warp-internal".to_string(), "~/warp-server".to_string()],
        branch_entries: vec![
            VerticalTabsSummaryBranchEntry {
                repo_path: PathBuf::from("/tmp/repo-a"),
                branch_name: "main".to_string(),
                diff_stats: Some(GitLineChanges {
                    files_changed: 1,
                    lines_added: 2,
                    lines_removed: 3,
                }),
                pull_request_label: Some("#123".to_string()),
                pull_request_url: Some("https://github.com/acme/repo-a/pull/123".to_string()),
            },
            VerticalTabsSummaryBranchEntry {
                repo_path: PathBuf::from("/tmp/repo-b"),
                branch_name: "feature/hidden".to_string(),
                diff_stats: None,
                pull_request_label: None,
                pull_request_url: None,
            },
            VerticalTabsSummaryBranchEntry {
                repo_path: PathBuf::from("/tmp/repo-c"),
                branch_name: "cleanup".to_string(),
                diff_stats: None,
                pull_request_label: None,
                pull_request_url: None,
            },
            VerticalTabsSummaryBranchEntry {
                repo_path: PathBuf::from("/tmp/repo-d"),
                branch_name: "hidden-branch".to_string(),
                diff_stats: None,
                pull_request_label: Some("#789".to_string()),
                pull_request_url: Some("https://github.com/acme/repo-d/pull/789".to_string()),
            },
        ],
        has_unread_activity: false,
    };

    let fragments = summary_search_text_fragments(&summary, Some("Custom tab"));

    assert!(search_fragments_contain_query(&fragments, "custom tab"));
    assert!(search_fragments_contain_query(&fragments, "claude"));
    assert!(search_fragments_contain_query(&fragments, "hidden work"));
    assert!(search_fragments_contain_query(&fragments, "hidden-branch"));
    assert!(search_fragments_contain_query(&fragments, "#789"));
    assert!(search_fragments_contain_query(&fragments, "+2"));
    assert!(search_fragments_contain_query(&fragments, "-3"));
}
