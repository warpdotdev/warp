use super::*;
use settings_page::MatchData;

// ── SettingsSection classification ──────────────────────────────────────────

#[test]
fn ai_subpages_are_identified() {
    assert!(SettingsSection::WarpAgent.is_ai_subpage());
    assert!(SettingsSection::AgentProfiles.is_ai_subpage());
    assert!(SettingsSection::AgentMCPServers.is_ai_subpage());
    assert!(SettingsSection::Knowledge.is_ai_subpage());
    assert!(SettingsSection::ThirdPartyCLIAgents.is_ai_subpage());

    assert!(!SettingsSection::AI.is_ai_subpage());
    assert!(!SettingsSection::Account.is_ai_subpage());
    assert!(!SettingsSection::CodeIndexing.is_ai_subpage());
}

#[test]
fn code_subpages_are_identified() {
    assert!(SettingsSection::CodeIndexing.is_code_subpage());
    assert!(SettingsSection::EditorAndCodeReview.is_code_subpage());

    assert!(!SettingsSection::Code.is_code_subpage());
    assert!(!SettingsSection::WarpAgent.is_code_subpage());
}

#[test]
fn cloud_platform_subpages_are_identified() {
    assert!(SettingsSection::CloudEnvironments.is_cloud_platform_subpage());
    assert!(SettingsSection::OzCloudAPIKeys.is_cloud_platform_subpage());

    assert!(!SettingsSection::Account.is_cloud_platform_subpage());
    assert!(!SettingsSection::WarpAgent.is_cloud_platform_subpage());
}

#[test]
fn is_subpage_covers_all_umbrella_types() {
    // All subpages under any umbrella should return true.
    for section in SettingsSection::ai_subpages() {
        assert!(section.is_subpage(), "{section:?} should be a subpage");
    }
    assert!(SettingsSection::CodeIndexing.is_subpage());
    assert!(SettingsSection::EditorAndCodeReview.is_subpage());
    assert!(SettingsSection::CloudEnvironments.is_subpage());
    assert!(SettingsSection::OzCloudAPIKeys.is_subpage());

    // Top-level pages should not be subpages.
    assert!(!SettingsSection::Account.is_subpage());
    assert!(!SettingsSection::AI.is_subpage());
    assert!(!SettingsSection::Code.is_subpage());
    assert!(!SettingsSection::Privacy.is_subpage());
}

// ── parent_page_section mapping ─────────────────────────────────────────────

#[test]
fn ai_subpages_map_to_ai_backing_page() {
    assert_eq!(
        SettingsSection::WarpAgent.parent_page_section(),
        SettingsSection::AI
    );
    assert_eq!(
        SettingsSection::AgentProfiles.parent_page_section(),
        SettingsSection::AI
    );
    assert_eq!(
        SettingsSection::Knowledge.parent_page_section(),
        SettingsSection::AI
    );
    assert_eq!(
        SettingsSection::ThirdPartyCLIAgents.parent_page_section(),
        SettingsSection::AI
    );
}

#[test]
fn agent_mcp_servers_maps_to_mcp_servers_page() {
    // AgentMCPServers renders the standalone MCPServers page, not the AI page.
    assert_eq!(
        SettingsSection::AgentMCPServers.parent_page_section(),
        SettingsSection::MCPServers
    );
}

#[test]
fn code_subpages_map_to_code_backing_page() {
    assert_eq!(
        SettingsSection::CodeIndexing.parent_page_section(),
        SettingsSection::Code
    );
    assert_eq!(
        SettingsSection::EditorAndCodeReview.parent_page_section(),
        SettingsSection::Code
    );
}

#[test]
fn cloud_platform_subpages_map_to_their_backing_pages() {
    assert_eq!(
        SettingsSection::CloudEnvironments.parent_page_section(),
        SettingsSection::CloudEnvironments
    );
    assert_eq!(
        SettingsSection::OzCloudAPIKeys.parent_page_section(),
        SettingsSection::OzCloudAPIKeys
    );
}

#[test]
fn non_subpage_sections_map_to_themselves() {
    assert_eq!(
        SettingsSection::Account.parent_page_section(),
        SettingsSection::Account
    );
    assert_eq!(
        SettingsSection::AI.parent_page_section(),
        SettingsSection::AI
    );
    assert_eq!(
        SettingsSection::Privacy.parent_page_section(),
        SettingsSection::Privacy
    );
}

// ── ai_subpages list ────────────────────────────────────────────────────────

#[test]
fn ai_subpages_list_contains_all_ai_subpage_variants() {
    let subpages = SettingsSection::ai_subpages();
    assert!(subpages.contains(&SettingsSection::WarpAgent));
    assert!(subpages.contains(&SettingsSection::AgentProfiles));
    assert!(subpages.contains(&SettingsSection::AgentMCPServers));
    assert!(subpages.contains(&SettingsSection::Knowledge));
    assert!(subpages.contains(&SettingsSection::ThirdPartyCLIAgents));
}

#[test]
fn ai_subpages_list_does_not_contain_non_subpages() {
    let subpages = SettingsSection::ai_subpages();
    assert!(!subpages.contains(&SettingsSection::AI));
    assert!(!subpages.contains(&SettingsSection::Account));
    assert!(!subpages.contains(&SettingsSection::Code));
}

// ── MatchData behavior ──────────────────────────────────────────────────────

#[test]
fn match_data_uncounted_true_is_truthy() {
    assert!(MatchData::Uncounted(true).is_truthy());
}

#[test]
fn match_data_uncounted_false_is_not_truthy() {
    assert!(!MatchData::Uncounted(false).is_truthy());
}

#[test]
fn match_data_countable_nonzero_is_truthy() {
    assert!(MatchData::Countable(3).is_truthy());
    assert!(MatchData::Countable(1).is_truthy());
}

#[test]
fn match_data_countable_zero_is_not_truthy() {
    assert!(!MatchData::Countable(0).is_truthy());
}

// ── Display / FromStr round-trip ────────────────────────────────────────────

#[test]
fn subpage_display_names_are_correct() {
    assert_eq!(SettingsSection::WarpAgent.to_string(), "Warp Agent");
    assert_eq!(SettingsSection::AgentProfiles.to_string(), "Profiles");
    assert_eq!(SettingsSection::AgentMCPServers.to_string(), "MCP servers");
    assert_eq!(SettingsSection::Knowledge.to_string(), "Knowledge");
    assert_eq!(
        SettingsSection::ThirdPartyCLIAgents.to_string(),
        "Third party CLI agents"
    );
    assert_eq!(
        SettingsSection::CodeIndexing.to_string(),
        "Indexing and projects"
    );
    assert_eq!(
        SettingsSection::EditorAndCodeReview.to_string(),
        "Editor and Code Review"
    );
    assert_eq!(
        SettingsSection::CloudEnvironments.to_string(),
        "Environments"
    );
    assert_eq!(
        SettingsSection::OzCloudAPIKeys.to_string(),
        "Oz Cloud API Keys"
    );
}

#[test]
fn subpage_from_str_parses_display_names() {
    // Both the legacy "Oz" name and the new "Warp Agent" display name must
    // resolve to SettingsSection::WarpAgent so existing deep links, persisted
    // telemetry strings, and external callers continue to work after the
    // user-facing rename (see specs/GH1063/product.md, Behavior #8).
    assert_eq!(
        SettingsSection::from_str("Oz"),
        Ok(SettingsSection::WarpAgent)
    );
    assert_eq!(
        SettingsSection::from_str("Warp Agent"),
        Ok(SettingsSection::WarpAgent)
    );
    assert_eq!(
        SettingsSection::from_str("Profiles"),
        Ok(SettingsSection::AgentProfiles)
    );
    assert_eq!(
        SettingsSection::from_str("Knowledge"),
        Ok(SettingsSection::Knowledge)
    );
    assert_eq!(
        SettingsSection::from_str("Indexing and projects"),
        Ok(SettingsSection::CodeIndexing)
    );
    assert_eq!(
        SettingsSection::from_str("Editor and Code Review"),
        Ok(SettingsSection::EditorAndCodeReview)
    );
    assert_eq!(
        SettingsSection::from_str("Oz Cloud API Keys"),
        Ok(SettingsSection::OzCloudAPIKeys)
    );
}

// ── Subpage search filter simulation ────────────────────────────────────────
// These tests simulate the per-subpage search filtering logic used in
// handle_search_editor_event: each subpage should only be visible if its
// own widgets' search terms match, not if a sibling subpage's terms match.

/// Helper: given a map of subpage→MatchData, returns which subpages are visible.
fn visible_subpages(
    subpage_filter: &HashMap<SettingsSection, MatchData>,
    subpages: &[SettingsSection],
) -> Vec<SettingsSection> {
    subpages
        .iter()
        .filter(|s| {
            subpage_filter
                .get(s)
                .map(|md| md.is_truthy())
                .unwrap_or(false)
        })
        .copied()
        .collect()
}

#[test]
fn search_knowledge_shows_only_knowledge_subpage() {
    // Simulate: searching "knowledge" matched the Knowledge subpage but not others.
    let mut filter = HashMap::new();
    filter.insert(SettingsSection::WarpAgent, MatchData::Countable(0));
    filter.insert(SettingsSection::AgentProfiles, MatchData::Countable(0));
    filter.insert(SettingsSection::Knowledge, MatchData::Countable(1));
    filter.insert(
        SettingsSection::ThirdPartyCLIAgents,
        MatchData::Countable(0),
    );

    let visible = visible_subpages(&filter, SettingsSection::ai_subpages());

    assert_eq!(visible, vec![SettingsSection::Knowledge]);
}

#[test]
fn search_agent_shows_profiles_and_cli_agents() {
    // "agent" appears in both AgentProfiles and ThirdPartyCLIAgents search terms.
    let mut filter = HashMap::new();
    filter.insert(SettingsSection::WarpAgent, MatchData::Countable(0));
    filter.insert(SettingsSection::AgentProfiles, MatchData::Countable(2));
    filter.insert(SettingsSection::Knowledge, MatchData::Countable(0));
    filter.insert(
        SettingsSection::ThirdPartyCLIAgents,
        MatchData::Countable(1),
    );

    let visible = visible_subpages(&filter, SettingsSection::ai_subpages());

    assert!(visible.contains(&SettingsSection::AgentProfiles));
    assert!(visible.contains(&SettingsSection::ThirdPartyCLIAgents));
    assert!(!visible.contains(&SettingsSection::WarpAgent));
    assert!(!visible.contains(&SettingsSection::Knowledge));
}

#[test]
fn empty_search_shows_no_subpages_in_filter() {
    // When search is cleared, subpage_filter is empty — all subpages fall back
    // to their backing page visibility (Uncounted(true) by default).
    let filter: HashMap<SettingsSection, MatchData> = HashMap::new();

    let visible = visible_subpages(&filter, SettingsSection::ai_subpages());

    // No entries in filter means no subpage-specific filtering; all return false
    // from the filter map. The actual rendering code falls back to the backing
    // page's pages_filter which defaults to Uncounted(true).
    assert!(visible.is_empty());
}

#[test]
fn search_with_no_matches_hides_all_subpages() {
    let mut filter = HashMap::new();
    filter.insert(SettingsSection::WarpAgent, MatchData::Countable(0));
    filter.insert(SettingsSection::AgentProfiles, MatchData::Countable(0));
    filter.insert(SettingsSection::Knowledge, MatchData::Countable(0));
    filter.insert(
        SettingsSection::ThirdPartyCLIAgents,
        MatchData::Countable(0),
    );

    let visible = visible_subpages(&filter, SettingsSection::ai_subpages());

    assert!(visible.is_empty());
}

/// Helper: check if an umbrella should be visible given a subpage filter.
fn umbrella_visible(
    subpage_filter: &HashMap<SettingsSection, MatchData>,
    umbrella_subpages: &[SettingsSection],
) -> bool {
    umbrella_subpages.iter().any(|s| {
        subpage_filter
            .get(s)
            .map(|md| md.is_truthy())
            .unwrap_or(false)
    })
}

#[test]
fn umbrella_hidden_when_no_subpages_match() {
    let mut filter = HashMap::new();
    filter.insert(SettingsSection::WarpAgent, MatchData::Countable(0));
    filter.insert(SettingsSection::AgentProfiles, MatchData::Countable(0));
    filter.insert(SettingsSection::Knowledge, MatchData::Countable(0));
    filter.insert(
        SettingsSection::ThirdPartyCLIAgents,
        MatchData::Countable(0),
    );

    assert!(!umbrella_visible(&filter, SettingsSection::ai_subpages()));
}

// ── cycle_pages search filter ────────────────────────────────────────────────
// These tests validate the logic added to cycle_pages() to ensure arrow key
// navigation respects the active search filter.

/// Mirrors the filter predicate used in cycle_pages() when search is active.
fn section_passes_nav_filter(
    section: SettingsSection,
    subpage_filter: &HashMap<SettingsSection, MatchData>,
    pages_filter: &[(SettingsSection, MatchData)],
) -> bool {
    if let Some(md) = subpage_filter.get(&section) {
        md.is_truthy()
    } else {
        let backing = section.parent_page_section();
        pages_filter
            .iter()
            .any(|(s, md)| *s == backing && md.is_truthy())
    }
}

#[test]
fn nav_filter_includes_matching_subpage_and_excludes_others() {
    let mut subpage_filter = HashMap::new();
    subpage_filter.insert(SettingsSection::WarpAgent, MatchData::Countable(0));
    subpage_filter.insert(SettingsSection::AgentProfiles, MatchData::Countable(0));
    subpage_filter.insert(SettingsSection::Knowledge, MatchData::Countable(1));
    subpage_filter.insert(
        SettingsSection::ThirdPartyCLIAgents,
        MatchData::Countable(0),
    );

    // No page-level filter entries needed since all AI subpages have subpage_filter entries.
    let pages_filter: Vec<(SettingsSection, MatchData)> = vec![];

    assert!(!section_passes_nav_filter(
        SettingsSection::WarpAgent,
        &subpage_filter,
        &pages_filter
    ));
    assert!(!section_passes_nav_filter(
        SettingsSection::AgentProfiles,
        &subpage_filter,
        &pages_filter
    ));
    assert!(section_passes_nav_filter(
        SettingsSection::Knowledge,
        &subpage_filter,
        &pages_filter
    ));
    assert!(!section_passes_nav_filter(
        SettingsSection::ThirdPartyCLIAgents,
        &subpage_filter,
        &pages_filter
    ));
}

#[test]
fn nav_filter_falls_back_to_pages_filter_for_top_level_pages() {
    // Top-level pages (Account, Appearance, etc.) have no subpage_filter entry.
    // They fall back to pages_filter using parent_page_section() == themselves.
    let subpage_filter: HashMap<SettingsSection, MatchData> = HashMap::new();
    let pages_filter = vec![
        (SettingsSection::Account, MatchData::Uncounted(true)),
        (SettingsSection::Appearance, MatchData::Countable(0)),
        (SettingsSection::Features, MatchData::Uncounted(true)),
    ];

    assert!(section_passes_nav_filter(
        SettingsSection::Account,
        &subpage_filter,
        &pages_filter
    ));
    assert!(!section_passes_nav_filter(
        SettingsSection::Appearance,
        &subpage_filter,
        &pages_filter
    ));
    assert!(section_passes_nav_filter(
        SettingsSection::Features,
        &subpage_filter,
        &pages_filter
    ));
}

#[test]
fn umbrella_visible_when_any_subpage_matches() {
    let mut filter = HashMap::new();
    filter.insert(SettingsSection::WarpAgent, MatchData::Countable(0));
    filter.insert(SettingsSection::AgentProfiles, MatchData::Countable(0));
    filter.insert(SettingsSection::Knowledge, MatchData::Countable(1));
    filter.insert(
        SettingsSection::ThirdPartyCLIAgents,
        MatchData::Countable(0),
    );

    assert!(umbrella_visible(&filter, SettingsSection::ai_subpages()));
}

// ── Search auto-select simulation ───────────────────────────────────────────
// These tests simulate the auto-select logic in handle_search_editor_event:
// when the current subpage is filtered out by search, the view should jump
// to the first visible subpage or page.

/// Simulates the "is current still visible" check from the search handler.
/// Returns true if `current` is still visible given the subpage_filter and
/// a list of (backing_section, is_truthy) pairs for pages_filter.
fn is_current_visible(
    current: SettingsSection,
    subpage_filter: &HashMap<SettingsSection, MatchData>,
    pages_visible: &[(SettingsSection, bool)],
) -> bool {
    if let Some(md) = subpage_filter.get(&current) {
        return md.is_truthy();
    }
    let backing = current.parent_page_section();
    pages_visible
        .iter()
        .any(|(section, visible)| *section == backing && *visible)
}

/// Simulates finding the first visible section from the nav_items order.
fn first_visible_section(
    nav_order: &[SettingsSection],
    subpage_filter: &HashMap<SettingsSection, MatchData>,
    pages_visible: &[(SettingsSection, bool)],
) -> Option<SettingsSection> {
    nav_order.iter().copied().find(|section| {
        if let Some(md) = subpage_filter.get(section) {
            md.is_truthy()
        } else {
            let backing = section.parent_page_section();
            pages_visible
                .iter()
                .any(|(s, visible)| *s == backing && *visible)
        }
    })
}

#[test]
fn auto_select_jumps_away_from_filtered_out_subpage() {
    // User is on Knowledge, searches "agent" which matches Profiles but not Knowledge.
    let mut filter = HashMap::new();
    filter.insert(SettingsSection::WarpAgent, MatchData::Countable(0));
    filter.insert(SettingsSection::AgentProfiles, MatchData::Countable(2));
    filter.insert(SettingsSection::Knowledge, MatchData::Countable(0));
    filter.insert(
        SettingsSection::ThirdPartyCLIAgents,
        MatchData::Countable(1),
    );

    let current = SettingsSection::Knowledge;
    assert!(
        !is_current_visible(current, &filter, &[]),
        "Knowledge should not be visible when it has 0 matches"
    );

    // The nav order: Oz, Profiles, ..., Knowledge, ThirdPartyCLI
    let nav_order = SettingsSection::ai_subpages();
    let first = first_visible_section(nav_order, &filter, &[]);
    assert_eq!(
        first,
        Some(SettingsSection::AgentProfiles),
        "Should auto-select Profiles as the first visible subpage"
    );
}

#[test]
fn auto_select_stays_on_current_when_it_matches() {
    // User is on Knowledge, searches "knowledge" which matches Knowledge.
    let mut filter = HashMap::new();
    filter.insert(SettingsSection::WarpAgent, MatchData::Countable(0));
    filter.insert(SettingsSection::AgentProfiles, MatchData::Countable(0));
    filter.insert(SettingsSection::Knowledge, MatchData::Countable(1));
    filter.insert(
        SettingsSection::ThirdPartyCLIAgents,
        MatchData::Countable(0),
    );

    let current = SettingsSection::Knowledge;
    assert!(
        is_current_visible(current, &filter, &[]),
        "Knowledge should remain visible when it has matches"
    );
}

#[test]
fn auto_select_falls_back_to_top_level_page_when_no_subpages_match() {
    // All AI subpages filtered out, but Account (top-level) is still visible.
    let mut filter = HashMap::new();
    filter.insert(SettingsSection::WarpAgent, MatchData::Countable(0));
    filter.insert(SettingsSection::AgentProfiles, MatchData::Countable(0));
    filter.insert(SettingsSection::Knowledge, MatchData::Countable(0));
    filter.insert(
        SettingsSection::ThirdPartyCLIAgents,
        MatchData::Countable(0),
    );

    let pages_visible = vec![
        (SettingsSection::Account, true),
        (SettingsSection::AI, false),
    ];

    // Nav order includes top-level Account before the AI subpages.
    let nav_order = vec![
        SettingsSection::Account,
        SettingsSection::WarpAgent,
        SettingsSection::AgentProfiles,
        SettingsSection::Knowledge,
        SettingsSection::ThirdPartyCLIAgents,
    ];

    let first = first_visible_section(&nav_order, &filter, &pages_visible);
    assert_eq!(
        first,
        Some(SettingsSection::Account),
        "Should fall back to Account when no subpages match"
    );
}

#[test]
fn auto_select_handles_standalone_subpage_via_backing_page() {
    // AgentMCPServers has its own backing page (MCPServers), not in subpage_filter.
    // It should be visible if its backing page is visible.
    let filter = HashMap::new(); // no per-subpage entries for AgentMCPServers

    let pages_visible = vec![
        (SettingsSection::MCPServers, true),
        (SettingsSection::AI, false),
    ];

    let current = SettingsSection::AgentMCPServers;
    assert!(
        is_current_visible(current, &filter, &pages_visible),
        "AgentMCPServers should be visible via its MCPServers backing page"
    );
}

#[test]
fn auto_select_with_no_matches_anywhere() {
    let mut filter = HashMap::new();
    filter.insert(SettingsSection::WarpAgent, MatchData::Countable(0));
    filter.insert(SettingsSection::AgentProfiles, MatchData::Countable(0));

    let pages_visible = vec![
        (SettingsSection::Account, false),
        (SettingsSection::AI, false),
    ];

    let nav_order = vec![
        SettingsSection::Account,
        SettingsSection::WarpAgent,
        SettingsSection::AgentProfiles,
    ];

    let first = first_visible_section(&nav_order, &filter, &pages_visible);
    assert_eq!(
        first, None,
        "No section should be selected when nothing matches"
    );
}

// ── Backward compatibility ──────────────────────────────────────────────────

#[test]
fn legacy_ai_section_maps_to_oz_default() {
    // SettingsSection::AI should be treated as backward-compat and map to Oz
    // via the code in set_and_refresh_current_page_internal.
    // Here we just verify the parent_page_section is still AI (for page lookup).
    assert_eq!(
        SettingsSection::AI.parent_page_section(),
        SettingsSection::AI
    );
    // And that AI is NOT itself a subpage.
    assert!(!SettingsSection::AI.is_subpage());
}

// ── Collapsed umbrella nav-stop behavior ────────────────────────────────────
// Verify that arrow-key navigation lands on a collapsed umbrella as a single
// stop (and activates it by jumping to the first subpage, which auto-expands
// the umbrella) instead of silently skipping over it.

use nav::{SettingsNavItem, SettingsUmbrella};

/// Builds the nav-items layout used by `SettingsView::new`, matching the real
/// sidebar ordering so tests exercise realistic nav orders.
fn realistic_nav_items() -> Vec<SettingsNavItem> {
    vec![
        SettingsNavItem::Page(SettingsSection::Account),
        SettingsNavItem::Umbrella(SettingsUmbrella::new(
            "Agents",
            SettingsSection::ai_subpages().to_vec(),
        )),
        SettingsNavItem::Page(SettingsSection::BillingAndUsage),
        SettingsNavItem::Umbrella(SettingsUmbrella::new(
            "Code",
            SettingsSection::code_subpages().to_vec(),
        )),
        SettingsNavItem::Umbrella(SettingsUmbrella::new(
            "Cloud platform",
            SettingsSection::cloud_platform_subpages().to_vec(),
        )),
        SettingsNavItem::Page(SettingsSection::Teams),
    ]
}

/// Mutably flips an umbrella's `expanded` flag at `nav_index`.
fn set_expanded(nav_items: &mut [SettingsNavItem], nav_index: usize, expanded: bool) {
    if let Some(SettingsNavItem::Umbrella(u)) = nav_items.get_mut(nav_index) {
        u.expanded = expanded;
    } else {
        panic!("nav_items[{nav_index}] is not an Umbrella");
    }
}

#[test]
fn collapsed_umbrella_is_a_single_nav_stop() {
    let nav_items = realistic_nav_items();
    // All umbrellas default to collapsed.
    let stops = build_nav_stops(&nav_items, |_| true);

    // Expect: Account, <Agents umbrella>, BillingAndUsage, <Code umbrella>,
    // <Cloud platform umbrella>, Teams.
    assert_eq!(stops.len(), 6);
    assert!(matches!(
        stops[0],
        NavStop::Section(SettingsSection::Account)
    ));
    assert!(matches!(
        stops[1],
        NavStop::CollapsedUmbrella {
            nav_index: 1,
            first_subpage: SettingsSection::WarpAgent,
            last_subpage: SettingsSection::ThirdPartyCLIAgents,
        }
    ));
    assert!(matches!(
        stops[2],
        NavStop::Section(SettingsSection::BillingAndUsage)
    ));
    assert!(matches!(
        stops[3],
        NavStop::CollapsedUmbrella {
            nav_index: 3,
            first_subpage: SettingsSection::CodeIndexing,
            last_subpage: SettingsSection::EditorAndCodeReview,
        }
    ));
    assert!(matches!(
        stops[4],
        NavStop::CollapsedUmbrella {
            nav_index: 4,
            first_subpage: SettingsSection::CloudEnvironments,
            last_subpage: SettingsSection::OzCloudAPIKeys,
        }
    ));
    assert!(matches!(stops[5], NavStop::Section(SettingsSection::Teams)));
}

#[test]
fn expanded_umbrella_produces_section_stop_per_subpage() {
    let mut nav_items = realistic_nav_items();
    // Expand the Agents umbrella so each of its subpages becomes a nav stop.
    set_expanded(&mut nav_items, 1, true);

    let stops = build_nav_stops(&nav_items, |_| true);

    // Expect: Account, WarpAgent, AgentProfiles, AgentMCPServers, Knowledge,
    // ThirdPartyCLIAgents, BillingAndUsage, <Code umbrella>,
    // <Cloud platform umbrella>, Teams.
    let sections: Vec<_> = stops
        .iter()
        .map(|s| match s {
            NavStop::Section(section) => format!("{section:?}"),
            NavStop::CollapsedUmbrella { nav_index, .. } => format!("Umbrella@{nav_index}"),
        })
        .collect();
    assert_eq!(
        sections,
        vec![
            "Account",
            "WarpAgent",
            "AgentProfiles",
            "AgentMCPServers",
            "Knowledge",
            "ThirdPartyCLIAgents",
            "BillingAndUsage",
            "Umbrella@3",
            "Umbrella@4",
            "Teams",
        ]
    );
}

#[test]
fn collapsed_umbrella_with_filtered_subpages_uses_first_visible_subpage() {
    // When a search filter hides the first subpage, activating the collapsed
    // umbrella should land on the *next* visible subpage (still auto-expanding).
    let nav_items = realistic_nav_items();

    let stops = build_nav_stops(&nav_items, |section| {
        // Hide WarpAgent (first AI subpage); keep the rest.
        section != SettingsSection::WarpAgent
    });

    let agents_stop = stops
        .iter()
        .find(|s| matches!(s, NavStop::CollapsedUmbrella { nav_index: 1, .. }))
        .expect("Agents umbrella should still be a collapsed stop");

    match agents_stop {
        NavStop::CollapsedUmbrella {
            first_subpage,
            last_subpage,
            ..
        } => {
            assert_eq!(
                *first_subpage,
                SettingsSection::AgentProfiles,
                "WarpAgent is hidden by the filter, so the first visible subpage is AgentProfiles"
            );
            assert_eq!(
                *last_subpage,
                SettingsSection::ThirdPartyCLIAgents,
                "last_subpage is unaffected by hiding WarpAgent and should remain the last visible subpage"
            );
        }
        _ => unreachable!(),
    }
}

#[test]
fn umbrella_with_no_visible_subpages_is_skipped_entirely() {
    let nav_items = realistic_nav_items();

    let stops = build_nav_stops(&nav_items, |section| !section.is_ai_subpage());

    // The Agents umbrella's subpages are all AI subpages, so the entire
    // umbrella should be absent from the nav order.
    assert!(
        stops
            .iter()
            .all(|s| !matches!(s, NavStop::CollapsedUmbrella { nav_index: 1, .. })),
        "Agents umbrella should not appear when none of its subpages are visible"
    );
    // The still-visible Code / Cloud platform umbrellas remain as stops.
    assert!(stops
        .iter()
        .any(|s| matches!(s, NavStop::CollapsedUmbrella { nav_index: 3, .. })));
    assert!(stops
        .iter()
        .any(|s| matches!(s, NavStop::CollapsedUmbrella { nav_index: 4, .. })));
}

#[test]
fn filtered_out_top_level_page_is_skipped() {
    let nav_items = realistic_nav_items();

    let stops = build_nav_stops(&nav_items, |section| section != SettingsSection::Teams);

    assert!(
        !stops
            .iter()
            .any(|s| matches!(s, NavStop::Section(SettingsSection::Teams))),
        "Teams should be filtered out entirely"
    );
    // But other pages remain.
    assert!(stops
        .iter()
        .any(|s| matches!(s, NavStop::Section(SettingsSection::Account))));
}

// ── current_stop_index ──────────────────────────────────────────────────────

#[test]
fn current_stop_index_matches_section_stop() {
    let nav_items = realistic_nav_items();
    let stops = build_nav_stops(&nav_items, |_| true);

    let idx = current_stop_index(&stops, &nav_items, SettingsSection::BillingAndUsage);
    assert_eq!(idx, Some(2));
}

#[test]
fn current_stop_index_maps_subpage_to_collapsed_umbrella() {
    // Edge case: the user manually collapsed the Agents umbrella while still
    // on one of its subpages. The collapsed umbrella should match as the
    // current stop so arrow-key cycling continues from the umbrella's position.
    let nav_items = realistic_nav_items();
    let stops = build_nav_stops(&nav_items, |_| true);

    let idx = current_stop_index(&stops, &nav_items, SettingsSection::Knowledge);
    assert_eq!(
        idx,
        Some(1),
        "Knowledge is under the collapsed Agents umbrella at nav_index 1"
    );
}

#[test]
fn current_stop_index_returns_none_when_section_is_not_present() {
    let nav_items = realistic_nav_items();
    // Filter out all AI subpages (and therefore the Agents umbrella) entirely.
    let stops = build_nav_stops(&nav_items, |section| !section.is_ai_subpage());

    // Knowledge isn't directly in stops, and no remaining collapsed umbrella
    // contains it, so current_stop_index should return None.
    assert_eq!(
        current_stop_index(&stops, &nav_items, SettingsSection::Knowledge),
        None
    );
}

// ── next_stop_index wrapping ────────────────────────────────────────────────

#[test]
fn next_stop_index_wraps_at_ends() {
    assert_eq!(next_stop_index(0, 3, CycleDirection::Up), 2);
    assert_eq!(next_stop_index(2, 3, CycleDirection::Down), 0);
    assert_eq!(next_stop_index(1, 3, CycleDirection::Up), 0);
    assert_eq!(next_stop_index(1, 3, CycleDirection::Down), 2);
}

#[test]
fn next_stop_index_handles_single_stop() {
    assert_eq!(next_stop_index(0, 1, CycleDirection::Up), 0);
    assert_eq!(next_stop_index(0, 1, CycleDirection::Down), 0);
}

// ── End-to-end cycling (no search) ──────────────────────────────────────────
// These tests simulate the sequence of nav-stop activations that would result
// from repeatedly pressing Down/Up, ensuring a collapsed umbrella is never
// skipped over.

/// Computes the section that would become active after applying the direction
/// once, starting from `current`. Mirrors the final target-resolution step in
/// `cycle_pages`.
fn simulate_cycle(
    nav_items: &[SettingsNavItem],
    stops: &[NavStop],
    current: SettingsSection,
    direction: CycleDirection,
) -> SettingsSection {
    let active = current_stop_index(stops, nav_items, current)
        .expect("current should exist in stops in these tests");
    let next = next_stop_index(active, stops.len(), direction);
    match stops[next] {
        NavStop::Section(section) => section,
        NavStop::CollapsedUmbrella {
            first_subpage,
            last_subpage,
            ..
        } => match direction {
            CycleDirection::Up => last_subpage,
            CycleDirection::Down => first_subpage,
        },
    }
}

#[test]
fn arrow_down_from_account_with_collapsed_agents_lands_on_first_subpage() {
    let nav_items = realistic_nav_items();
    let stops = build_nav_stops(&nav_items, |_| true);

    // Pressing Down from Account should auto-expand Agents and select WarpAgent,
    // not skip over to BillingAndUsage.
    let next = simulate_cycle(
        &nav_items,
        &stops,
        SettingsSection::Account,
        CycleDirection::Down,
    );
    assert_eq!(next, SettingsSection::WarpAgent);
}

#[test]
fn arrow_up_from_billing_and_usage_with_collapsed_agents_lands_on_last_subpage() {
    let nav_items = realistic_nav_items();
    let stops = build_nav_stops(&nav_items, |_| true);

    // Pressing Up from BillingAndUsage should land on the collapsed Agents
    // umbrella, which resolves to ThirdPartyCLIAgents (last visible subpage)
    // so the user continues moving in natural reading order rather than being
    // jumped back to the top of the umbrella.
    let next = simulate_cycle(
        &nav_items,
        &stops,
        SettingsSection::BillingAndUsage,
        CycleDirection::Up,
    );
    assert_eq!(next, SettingsSection::ThirdPartyCLIAgents);
}

#[test]
fn arrow_up_into_collapsed_umbrella_respects_search_filter_for_last_subpage() {
    let nav_items = realistic_nav_items();
    // Hide the last two AI subpages; the last *visible* subpage of the
    // still-collapsed Agents umbrella should be AgentMCPServers.
    let is_visible = |section: SettingsSection| {
        !matches!(
            section,
            SettingsSection::Knowledge | SettingsSection::ThirdPartyCLIAgents
        )
    };
    let stops = build_nav_stops(&nav_items, is_visible);

    // From BillingAndUsage, Up should land on the last *visible* AI subpage
    // (AgentMCPServers), not on the filtered-out Knowledge/ThirdPartyCLIAgents
    // or on the first subpage WarpAgent.
    let next = simulate_cycle(
        &nav_items,
        &stops,
        SettingsSection::BillingAndUsage,
        CycleDirection::Up,
    );
    assert_eq!(next, SettingsSection::AgentMCPServers);
}

#[test]
fn arrow_down_from_expanded_last_subpage_leaves_umbrella() {
    let mut nav_items = realistic_nav_items();
    set_expanded(&mut nav_items, 1, true); // expand Agents
    let stops = build_nav_stops(&nav_items, |_| true);

    // ThirdPartyCLIAgents is the last Agents subpage; Down should move to
    // BillingAndUsage (the next top-level page in the nav order).
    let next = simulate_cycle(
        &nav_items,
        &stops,
        SettingsSection::ThirdPartyCLIAgents,
        CycleDirection::Down,
    );
    assert_eq!(next, SettingsSection::BillingAndUsage);
}

#[test]
fn arrow_down_across_adjacent_collapsed_umbrellas() {
    let nav_items = realistic_nav_items();
    // Both Code and Cloud platform umbrellas are collapsed.
    let stops = build_nav_stops(&nav_items, |_| true);

    // From BillingAndUsage, Down should land on the first Code subpage
    // (Code umbrella auto-expands).
    let next_after_billing = simulate_cycle(
        &nav_items,
        &stops,
        SettingsSection::BillingAndUsage,
        CycleDirection::Down,
    );
    assert_eq!(next_after_billing, SettingsSection::CodeIndexing);

    // From the Code umbrella stop (i.e. the user is "on" CodeIndexing which
    // maps back to the collapsed umbrella), pressing Down again should land
    // on the Cloud platform umbrella's first subpage.
    let next_after_code = simulate_cycle(
        &nav_items,
        &stops,
        SettingsSection::CodeIndexing,
        CycleDirection::Down,
    );
    assert_eq!(next_after_code, SettingsSection::CloudEnvironments);
}

#[test]
fn arrow_down_collapsed_umbrella_respects_search_filter() {
    let nav_items = realistic_nav_items();
    // Search filter hides WarpAgent and AgentProfiles so the first visible AI
    // subpage is AgentMCPServers.
    let is_visible = |section: SettingsSection| {
        !matches!(
            section,
            SettingsSection::WarpAgent | SettingsSection::AgentProfiles
        )
    };
    let stops = build_nav_stops(&nav_items, is_visible);

    // From Account, Down should land on AgentMCPServers (first visible
    // subpage of the still-collapsed Agents umbrella), not on WarpAgent /
    // AgentProfiles.
    let next = simulate_cycle(
        &nav_items,
        &stops,
        SettingsSection::Account,
        CycleDirection::Down,
    );
    assert_eq!(next, SettingsSection::AgentMCPServers);
}
