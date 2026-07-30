use super::{
    AgentAttributionToggleState, cli_agent_widgets, derive_agent_attribution_toggle_state,
};
#[cfg(not(target_family = "wasm"))]
use super::{CLIAgentWidget, cli_agent_settings_widget_id};
#[cfg(not(target_family = "wasm"))]
use crate::settings_view::settings_page::SettingsWidget;
use crate::settings_view::settings_page::search_terms_match;
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
fn cli_agent_widgets_filter_to_individual_settings() {
    let widgets = cli_agent_widgets();

    let matching_terms = |query| {
        widgets
            .iter()
            .filter(|widget| search_terms_match(widget.search_terms(), query))
            .map(|widget| widget.search_terms())
            .collect::<Vec<_>>()
    };

    assert_eq!(matching_terms("ctrl enter").len(), 1);
    assert_eq!(matching_terms("toolbar layout").len(), 1);
    assert_eq!(matching_terms("auto dismiss").len(), 1);
    assert_eq!(matching_terms("third party cli agent").len(), widgets.len());
    assert_eq!(matching_terms("").len(), widgets.len());
}

#[cfg(not(target_family = "wasm"))]
#[test]
fn cli_agent_deeplink_targets_primary_toolbar_widget() {
    assert_eq!(
        cli_agent_settings_widget_id(),
        CLIAgentWidget::static_widget_id()
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
