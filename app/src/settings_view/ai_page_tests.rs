use super::{
    AIInputWidget, AISettingsPageView, AgentAttributionToggleState, AgentAttributionWidget,
    CloudAgentComputerUseWidget, CloudHandoffWidget, GlobalAIWidget, OtherAIWidget,
    PluginDiscoveryWidget, derive_agent_attribution_toggle_state,
};
use crate::settings_view::settings_page::{SettingsWidget, search_terms_match};
use crate::workspaces::workspace::AdminEnablementSetting;

/// Every term the product spec requires to reach the `Agent Plugin discovery` switch.
///
/// Settings search filters per widget, so each of these must select the plugin widget and no
/// other Warp Agent widget — otherwise a user searching "plugin" gets a page of unrelated rows.
const REQUIRED_PLUGIN_SEARCH_TERMS: &[&str] = &[
    "agent",
    "plugin",
    "plugins",
    "discovery",
    "skills",
    "MCP",
    "disable",
    "stop",
];

/// The Warp Agent widgets the plugin switch has to be distinguishable from. Restricted to the
/// ones that need no `ViewContext` to build.
fn neighbouring_widget_terms() -> Vec<String> {
    let widgets: Vec<Box<dyn SettingsWidget<View = AISettingsPageView>>> = vec![
        Box::new(GlobalAIWidget::default()),
        Box::new(AIInputWidget::default()),
        Box::new(OtherAIWidget::default()),
        Box::new(AgentAttributionWidget::default()),
        Box::new(CloudHandoffWidget::default()),
        Box::new(CloudAgentComputerUseWidget::default()),
    ];
    widgets
        .iter()
        .map(|widget| widget.search_terms().to_owned())
        .collect()
}

#[test]
fn plugin_discovery_widget_declares_the_specified_search_terms() {
    let widget = PluginDiscoveryWidget::default();
    assert_eq!(
        widget.search_terms(),
        "agent plugin plugins discovery skills mcp disable stop"
    );
}

#[test]
fn every_required_term_matches_the_plugin_discovery_widget() {
    let widget = PluginDiscoveryWidget::default();
    for term in REQUIRED_PLUGIN_SEARCH_TERMS {
        assert!(
            search_terms_match(widget.search_terms(), term),
            "'{term}' must select the Agent Plugin discovery widget"
        );
    }
}

/// `plugin`, `discovery`, and `stop` are specific enough that they must isolate the switch.
///
/// `agent`, `skills`, `mcp`, and `disable` are deliberately excluded: they are shared page-level
/// vocabulary, and the spec asks for them so a page-level query still reaches the switch.
#[test]
fn specific_terms_isolate_the_plugin_discovery_widget() {
    let widget = PluginDiscoveryWidget::default();
    for term in ["plugin", "plugins", "discovery", "stop"] {
        assert!(
            search_terms_match(widget.search_terms(), term),
            "'{term}' must select the Agent Plugin discovery widget"
        );
        for neighbour in neighbouring_widget_terms() {
            assert!(
                !search_terms_match(&neighbour, term),
                "'{term}' must not also select the widget with terms '{neighbour}'"
            );
        }
    }
}

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
