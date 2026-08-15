use settings_page::{FilteredPageType, MatchData, PageType, SettingsWidget, search_terms_match};
use warpui::elements::Empty;
use warpui::{App, AppContext, Element, Entity, View};

use super::*;
use crate::appearance::Appearance;
use crate::workspaces::workspace::{BillingMetadata, CustomerType};

fn billing_metadata(customer_type: CustomerType) -> BillingMetadata {
    BillingMetadata {
        customer_type,
        ..Default::default()
    }
}

#[test]
fn paid_workspace_without_team_shows_only_workspace_badge() {
    let billing_metadata = billing_metadata(CustomerType::Enterprise);

    let presentation = plan_header_presentation(Some(&billing_metadata), false, false);

    assert_eq!(presentation.badge_label.as_deref(), Some("Enterprise"));
    assert!(!presentation.show_personal_upgrade);
}

#[test]
fn free_workspace_without_team_shows_free_badge_once() {
    let billing_metadata = billing_metadata(CustomerType::Free);

    let presentation = plan_header_presentation(Some(&billing_metadata), false, false);

    assert_eq!(presentation.badge_label.as_deref(), Some("Free"));
    assert!(presentation.show_personal_upgrade);
}

#[test]
fn paid_workspace_with_team_shows_only_workspace_badge() {
    let billing_metadata = billing_metadata(CustomerType::Enterprise);

    let presentation = plan_header_presentation(Some(&billing_metadata), true, false);

    assert_eq!(presentation.badge_label.as_deref(), Some("Enterprise"));
    assert!(!presentation.show_personal_upgrade);
}

#[test]
fn anonymous_account_shows_free_badge_once() {
    let presentation = plan_header_presentation(None, false, true);

    assert_eq!(presentation.badge_label.as_deref(), Some("Free"));
    assert!(presentation.show_personal_upgrade);
}

#[test]
fn signed_in_account_without_workspace_shows_free_badge_once() {
    let presentation = plan_header_presentation(None, false, false);

    assert_eq!(presentation.badge_label.as_deref(), Some("Free"));
    assert!(presentation.show_personal_upgrade);
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

// ── Display labels ─────────────────────────────────────────────────

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

// ── slug / from_slug ───────────────────────────────────────────────

/// Every `SettingsSection` variant.
///
/// `all_sections_list_is_exhaustive` keeps this honest: adding a variant
/// breaks the exhaustive match there, which is the prompt to add it here.
const ALL_SECTIONS: &[SettingsSection] = &[
    SettingsSection::About,
    SettingsSection::Account,
    SettingsSection::BillingAndUsage,
    SettingsSection::Appearance,
    SettingsSection::Features,
    SettingsSection::Keybindings,
    SettingsSection::Privacy,
    SettingsSection::Referrals,
    SettingsSection::Scripting,
    SettingsSection::SharedBlocks,
    SettingsSection::Teams,
    SettingsSection::WarpDrive,
    SettingsSection::Warpify,
    SettingsSection::WarpAgent,
    SettingsSection::AgentProfiles,
    SettingsSection::AgentMCPServers,
    SettingsSection::Knowledge,
    SettingsSection::ThirdPartyCLIAgents,
    SettingsSection::CodeIndexing,
    SettingsSection::EditorAndCodeReview,
    SettingsSection::CloudEnvironments,
    SettingsSection::OzCloudAPIKeys,
];

#[test]
fn all_sections_list_is_exhaustive() {
    fn is_listed(section: SettingsSection) -> bool {
        let known = match section {
            SettingsSection::About
            | SettingsSection::Account
            | SettingsSection::BillingAndUsage
            | SettingsSection::Appearance
            | SettingsSection::Features
            | SettingsSection::Keybindings
            | SettingsSection::Privacy
            | SettingsSection::Referrals
            | SettingsSection::Scripting
            | SettingsSection::SharedBlocks
            | SettingsSection::Teams
            | SettingsSection::WarpDrive
            | SettingsSection::Warpify
            | SettingsSection::WarpAgent
            | SettingsSection::AgentProfiles
            | SettingsSection::AgentMCPServers
            | SettingsSection::Knowledge
            | SettingsSection::ThirdPartyCLIAgents
            | SettingsSection::CodeIndexing
            | SettingsSection::EditorAndCodeReview
            | SettingsSection::CloudEnvironments
            | SettingsSection::OzCloudAPIKeys => section,
        };
        ALL_SECTIONS.contains(&known)
    }

    for section in ALL_SECTIONS {
        assert!(is_listed(*section), "{section:?} is missing from the list");
    }
}

#[test]
fn every_section_round_trips_through_its_slug() {
    for section in ALL_SECTIONS {
        assert_eq!(
            SettingsSection::from_slug(section.slug()),
            Some(*section),
            "{section:?} should round-trip through its slug"
        );
    }
}

#[test]
fn slugs_are_unique_across_sections() {
    let mut slugs: Vec<&str> = ALL_SECTIONS.iter().map(|section| section.slug()).collect();
    let total = slugs.len();
    slugs.sort_unstable();
    slugs.dedup();
    assert_eq!(slugs.len(), total, "two sections share a slug");
}

#[test]
fn slugs_were_seeded_from_the_display_labels_they_replaced() {
    // Slugs were seeded from the Display strings that used to double as the
    // persistence key, so no data migration was needed. Display is now free to
    // diverge; if it does, update this test rather than the slugs, which are a
    // stored contract.
    for section in ALL_SECTIONS {
        assert_eq!(
            section.slug(),
            section.to_string(),
            "{section:?} slug diverged from the Display label it was seeded from"
        );
    }
}

#[test]
fn from_slug_accepts_legacy_spellings() {
    // Both the legacy "Oz" name and the current "Warp Agent" slug must resolve
    // to SettingsSection::WarpAgent so existing deep links, persisted sessions
    // and external callers keep working after the user-facing rename (see
    // specs/GH1063/product.md, Behavior #8).
    assert_eq!(
        SettingsSection::from_slug("Oz"),
        Some(SettingsSection::WarpAgent)
    );
    assert_eq!(
        SettingsSection::from_slug("WarpDrive"),
        Some(SettingsSection::WarpDrive)
    );
    assert_eq!(
        SettingsSection::from_slug("AgentProfiles"),
        Some(SettingsSection::AgentProfiles)
    );
    assert_eq!(
        SettingsSection::from_slug("AgentMCPServers"),
        Some(SettingsSection::AgentMCPServers)
    );
    assert_eq!(
        SettingsSection::from_slug("ThirdPartyCLIAgents"),
        Some(SettingsSection::ThirdPartyCLIAgents)
    );
    assert_eq!(
        SettingsSection::from_slug("CodeIndexing"),
        Some(SettingsSection::CodeIndexing)
    );
    assert_eq!(
        SettingsSection::from_slug("EditorAndCodeReview"),
        Some(SettingsSection::EditorAndCodeReview)
    );
    assert_eq!(
        SettingsSection::from_slug("CloudEnvironments"),
        Some(SettingsSection::CloudEnvironments)
    );
    assert_eq!(
        SettingsSection::from_slug("OzCloudAPIKeys"),
        Some(SettingsSection::OzCloudAPIKeys)
    );
}

#[test]
fn from_slug_maps_superseded_page_names_to_the_page_that_replaced_them() {
    // `AI`, `Code` and `MCP Servers` named pages that have since been split or
    // moved. Persisted sessions and warpctrl callers still use them, so they
    // resolve here, at the boundary, rather than existing as sections of their
    // own that every caller would have to remember to normalize.
    assert_eq!(
        SettingsSection::from_slug("AI"),
        Some(SettingsSection::WarpAgent)
    );
    assert_eq!(
        SettingsSection::from_slug("Code"),
        Some(SettingsSection::CodeIndexing)
    );
    assert_eq!(
        SettingsSection::from_slug("MCP Servers"),
        Some(SettingsSection::AgentMCPServers)
    );
}

#[test]
fn from_slug_rejects_unknown_input() {
    assert_eq!(SettingsSection::from_slug("Not a page"), None);
    assert_eq!(SettingsSection::from_slug(""), None);
}

// ── Collapsed umbrella nav-stop behavior ────────────────────────────────────
// Verify that arrow-key navigation lands on a collapsed umbrella as a single
// stop (and activates it by jumping to the first subpage, which auto-expands
// the umbrella) instead of silently skipping over it.

use nav::{SettingsNavItem, SettingsUmbrella};

/// The Agents umbrella's subpages, mirroring the list `SettingsView::new`
/// declares. Duplicated here rather than shared so these tests can assert
/// fixed nav-stop indices against a deliberately trimmed sidebar.
const AGENT_SUBPAGES: &[SettingsSection] = &[
    SettingsSection::WarpAgent,
    SettingsSection::AgentProfiles,
    SettingsSection::AgentMCPServers,
    SettingsSection::Knowledge,
    SettingsSection::ThirdPartyCLIAgents,
];

/// Builds the nav-items layout used by `SettingsView::new`, matching the real
/// sidebar ordering so tests exercise realistic nav orders.
fn realistic_nav_items() -> Vec<SettingsNavItem> {
    vec![
        SettingsNavItem::Page(SettingsSection::Account),
        SettingsNavItem::Umbrella(SettingsUmbrella::new("Agents", AGENT_SUBPAGES.to_vec())),
        SettingsNavItem::Page(SettingsSection::BillingAndUsage),
        SettingsNavItem::Umbrella(SettingsUmbrella::new(
            "Code",
            vec![
                SettingsSection::CodeIndexing,
                SettingsSection::EditorAndCodeReview,
            ],
        )),
        SettingsNavItem::Umbrella(SettingsUmbrella::new(
            "Cloud platform",
            vec![
                SettingsSection::CloudEnvironments,
                SettingsSection::OzCloudAPIKeys,
            ],
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

    let stops = build_nav_stops(&nav_items, |section| !AGENT_SUBPAGES.contains(&section));

    // The Agents umbrella's subpages are all hidden, so the entire umbrella
    // should be absent from the nav order.
    assert!(
        stops
            .iter()
            .all(|s| !matches!(s, NavStop::CollapsedUmbrella { nav_index: 1, .. })),
        "Agents umbrella should not appear when none of its subpages are visible"
    );
    // The still-visible Code / Cloud platform umbrellas remain as stops.
    assert!(
        stops
            .iter()
            .any(|s| matches!(s, NavStop::CollapsedUmbrella { nav_index: 3, .. }))
    );
    assert!(
        stops
            .iter()
            .any(|s| matches!(s, NavStop::CollapsedUmbrella { nav_index: 4, .. }))
    );
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
    assert!(
        stops
            .iter()
            .any(|s| matches!(s, NavStop::Section(SettingsSection::Account)))
    );
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
    // Filter out all Agents subpages (and therefore the umbrella) entirely.
    let stops = build_nav_stops(&nav_items, |section| !AGENT_SUBPAGES.contains(&section));

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

// ── PageType filter lifecycle across a rebuild (APP-4922) ────────────────────
// Rebuilding a page's PageType resets its widget filter to every widget, so an
// active query has to be reapplied for only matching widgets to render. No page
// rebuilds itself on navigation any more (each subpage owns its own view), but
// these tests still pin the underlying PageType::Uncategorized filter lifecycle
// and the real search_terms_match predicate that the invariant rests on.

/// Minimal View so PageType<V> can be instantiated in a unit test without the
/// full SettingsView/ViewContext a real settings page requires.
struct TestSettingsView;

impl Entity for TestSettingsView {
    type Event = ();
}

impl View for TestSettingsView {
    fn ui_name() -> &'static str {
        "TestSettingsView"
    }

    fn render(&self, _: &AppContext) -> Box<dyn Element> {
        Empty::new().finish()
    }
}

/// A SettingsWidget whose only test-relevant state is its search terms; render
/// is never invoked by the filter lifecycle under test.
struct StubWidget {
    terms: &'static str,
}

impl SettingsWidget for StubWidget {
    type View = TestSettingsView;

    fn search_terms(&self) -> &str {
        self.terms
    }

    fn render(&self, _: &Self::View, _: &Appearance, _: &AppContext) -> Box<dyn Element> {
        Empty::new().finish()
    }
}

/// A fresh Uncategorized page mirroring build_page -> new_uncategorized: every
/// widget index visible by default.
fn stub_widgets_page() -> PageType<TestSettingsView> {
    let widgets: Vec<Box<dyn SettingsWidget<View = TestSettingsView>>> = vec![
        Box::new(StubWidget {
            terms: "warp agent global ai toggle",
        }),
        Box::new(StubWidget {
            terms: "active ai autosuggestions prompt",
        }),
        Box::new(StubWidget {
            terms: "ai input model api key",
        }),
        Box::new(StubWidget {
            terms: "file search fuzzy opener",
        }),
        Box::new(StubWidget {
            terms: "voice input",
        }),
    ];
    PageType::new_uncategorized(widgets, None)
}

/// Number of widgets the page would render under its current filter.
fn visible_widget_count<V: View>(page: &PageType<V>) -> usize {
    let FilteredPageType::Uncategorized { widgets, .. } = page.get_filtered() else {
        panic!("expected Uncategorized page");
    };
    widgets.len()
}

#[test]
fn search_terms_match_direct_unit_checks() {
    // Empty query matches everything (mirrors PageType::update_filter's guard).
    assert!(search_terms_match("warp agent global ai toggle", ""));
    // All-words, case-insensitive, non-contiguous.
    assert!(search_terms_match(
        "active ai autosuggestions prompt",
        "autosuggestions"
    ));
    assert!(search_terms_match(
        "active ai autosuggestions prompt",
        "ACTIVE AI"
    ));
    assert!(search_terms_match(
        "file search fuzzy opener",
        "file search"
    ));
    // Every word must appear.
    assert!(!search_terms_match(
        "warp agent global ai toggle",
        "file search"
    ));
    assert!(!search_terms_match(
        "active ai autosuggestions prompt",
        "autosuggestions key"
    ));
}

#[test]
fn rebuild_resets_filter_to_all_widgets() {
    // Searching "file search" matches exactly one widget. A freshly built page
    // (mirroring build_page -> new_uncategorized) resets the filter to every
    // widget, so without reapplying update_filter the page would show all
    // widgets.
    App::test((), |mut app| async move {
        app.update(|ctx| {
            let mut page = stub_widgets_page();
            let md = page.update_filter("file search", ctx);
            assert!(md.is_truthy());
            assert_eq!(visible_widget_count(&page), 1);

            let rebuilt = stub_widgets_page();
            assert_eq!(
                visible_widget_count(&rebuilt),
                5,
                "rebuild resets the filter to all widgets when update_filter isn't reapplied"
            );
        });
    });
}

#[test]
fn rebuild_with_reapply_keeps_only_matching_widgets() {
    // The fix: after a rebuild, reapply update_filter with the active query so
    // only matching widgets render on the restored subpage.
    App::test((), |mut app| async move {
        app.update(|ctx| {
            let mut page = stub_widgets_page();
            page.update_filter("file search", ctx);
            assert_eq!(visible_widget_count(&page), 1);

            let mut rebuilt = stub_widgets_page();
            rebuilt.update_filter("file search", ctx);
            assert_eq!(
                visible_widget_count(&rebuilt),
                1,
                "reapplying the filter after a rebuild keeps only matching widgets visible"
            );
        });
    });
}

#[test]
fn reapply_handles_multi_word_and_case() {
    // A multi-word, case-insensitive query survives the rebuild + reapply cycle.
    App::test((), |mut app| async move {
        app.update(|ctx| {
            let mut page = stub_widgets_page();
            page.update_filter("AI INPUT", ctx);
            assert_eq!(visible_widget_count(&page), 1);

            let mut rebuilt = stub_widgets_page();
            rebuilt.update_filter("AI INPUT", ctx);
            assert_eq!(visible_widget_count(&rebuilt), 1);
        });
    });
}

#[test]
fn empty_query_after_reapply_shows_all_widgets() {
    // When the search is cleared, the subpage shows all widgets again.
    App::test((), |mut app| async move {
        app.update(|ctx| {
            let mut page = stub_widgets_page();
            page.update_filter("agent", ctx);
            assert_eq!(visible_widget_count(&page), 1);

            let mut rebuilt = stub_widgets_page();
            rebuilt.update_filter("", ctx);
            assert_eq!(
                visible_widget_count(&rebuilt),
                5,
                "an empty query restores every widget on the subpage"
            );
        });
    });
}
