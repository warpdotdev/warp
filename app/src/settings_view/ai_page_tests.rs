use warpui::App;

use super::super::settings_page::{FilteredPageType, MatchData, PageType, SettingsWidget};
use super::{
    AISettingsPageView, AgentAttributionToggleState, AgentAttributionWidget,
    CloudAgentComputerUseWidget, derive_agent_attribution_toggle_state,
};
use crate::workspaces::workspace::AdminEnablementSetting;

#[test]
fn respect_user_setting_returns_user_pref_unlocked() {
    let state = derive_agent_attribution_toggle_state(
        &AdminEnablementSetting::RespectUserSetting,
        true,
        true,
    );
    assert_eq!(
        state,
        AgentAttributionToggleState {
            is_enabled: true,
            is_forced_by_org: false,
            is_disabled: false,
        }
    );
}

#[test]
fn respect_user_setting_with_user_off_returns_unchecked_unlocked() {
    let state = derive_agent_attribution_toggle_state(
        &AdminEnablementSetting::RespectUserSetting,
        false,
        true,
    );
    assert_eq!(
        state,
        AgentAttributionToggleState {
            is_enabled: false,
            is_forced_by_org: false,
            is_disabled: false,
        }
    );
}

#[test]
fn team_enable_locks_toggle_on_regardless_of_user_pref() {
    let state = derive_agent_attribution_toggle_state(&AdminEnablementSetting::Enable, false, true);
    assert_eq!(
        state,
        AgentAttributionToggleState {
            is_enabled: true,
            is_forced_by_org: true,
            is_disabled: true,
        }
    );
}

#[test]
fn team_disable_locks_toggle_off_regardless_of_user_pref() {
    let state = derive_agent_attribution_toggle_state(&AdminEnablementSetting::Disable, true, true);
    assert_eq!(
        state,
        AgentAttributionToggleState {
            is_enabled: false,
            is_forced_by_org: true,
            is_disabled: true,
        }
    );
}

#[test]
fn ai_globally_disabled_marks_toggle_disabled_but_not_forced() {
    let state = derive_agent_attribution_toggle_state(
        &AdminEnablementSetting::RespectUserSetting,
        true,
        false,
    );
    assert_eq!(
        state,
        AgentAttributionToggleState {
            is_enabled: true,
            is_forced_by_org: false,
            is_disabled: true,
        }
    );
}

#[test]
fn team_force_takes_precedence_over_global_ai_disabled() {
    let state =
        derive_agent_attribution_toggle_state(&AdminEnablementSetting::Enable, false, false);
    assert_eq!(
        state,
        AgentAttributionToggleState {
            is_enabled: true,
            is_forced_by_org: true,
            is_disabled: true,
        }
    );
}

// ── AI subpage search rebuild/restore cycle (APP-4910) ───────────────────────
// AI subpages share SettingsView::restore_active_subpage_filter with the Code
// subpages, so the same rebuild/restore invariant must hold: after a rebuild via
// `PageType::new_uncategorized` (what `AISettingsPageView::set_active_subpage`
// does) every widget is visible, and reapplying the active query via
// `update_filter` narrows `get_filtered()` back to the matching widget(s).

fn filtered_ai_search_terms(page: &PageType<AISettingsPageView>) -> Vec<&str> {
    match page.get_filtered() {
        FilteredPageType::Uncategorized { widgets, .. } => {
            widgets.iter().map(|widget| widget.search_terms()).collect()
        }
        _ => panic!("expected an Uncategorized page after a subpage rebuild"),
    }
}

fn match_count(match_data: MatchData) -> usize {
    match match_data {
        MatchData::Countable(n) => n,
        MatchData::Uncounted(true) => 1,
        MatchData::Uncounted(false) => 0,
    }
}

#[test]
fn ai_subpage_search_reapplies_filter_after_restore() {
    App::test((), |mut app| async move {
        app.update(|ctx| {
            // Build an AI subpage the way AISettingsPageView::set_active_subpage does:
            // a fresh PageType::new_uncategorized whose filter starts with every
            // widget visible.
            let widgets: Vec<Box<dyn SettingsWidget<View = AISettingsPageView>>> = vec![
                Box::new(AgentAttributionWidget::default())
                    as Box<dyn SettingsWidget<View = AISettingsPageView>>,
                Box::new(CloudAgentComputerUseWidget::default()),
            ];
            let total = widgets.len();
            let mut page = PageType::new_uncategorized(widgets, None);

            // Regression signature: the rebuilt AI subpage starts all-visible.
            assert_eq!(
                filtered_ai_search_terms(&page).len(),
                total,
                "a freshly rebuilt AI subpage must start with every widget visible"
            );

            // The fix reapplies the active query after the rebuild via the shared
            // restore_active_subpage_filter helper used by both AI and Code pages.
            let matches = page.update_filter("agent attribution", ctx);
            assert_eq!(match_count(matches), 1);

            let restored = filtered_ai_search_terms(&page);
            assert_eq!(restored.len(), 1);
            assert_eq!(
                restored[0],
                AgentAttributionWidget::default().search_terms(),
                "only the Agent Attribution widget should match \"agent attribution\""
            );
        });
    });
}
