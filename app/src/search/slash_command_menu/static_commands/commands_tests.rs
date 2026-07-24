use std::collections::HashSet;

use super::*;

#[test]
fn command_names_and_kinds_are_unique_per_surface() {
    for settings_mode in [settings::SettingsMode::Gui, settings::SettingsMode::Tui] {
        let mut names = HashSet::new();
        let mut kinds = HashSet::new();
        for command in all_commands(settings_mode) {
            assert!(
                names.insert(command.name),
                "duplicate slash command name on {settings_mode:?}: {}",
                command.name
            );
            assert!(
                kinds.insert(command.kind),
                "duplicate slash command kind on {settings_mode:?}: {:?}",
                command.kind
            );
        }
    }
}

#[test]
fn gui_icon_metadata_matches_surface_support() {
    let mut checked_kinds = HashSet::new();
    for settings_mode in [settings::SettingsMode::Gui, settings::SettingsMode::Tui] {
        for command in all_commands(settings_mode) {
            if checked_kinds.insert(command.kind) {
                assert_eq!(
                    command.supported_surfaces.gui_icon_path().is_some(),
                    command.supports_gui(),
                    "{} has inconsistent GUI icon metadata",
                    command.name
                );
            }
        }
    }
}
#[test]
fn command_registry_filters_explicit_surface_metadata() {
    for settings_mode in [settings::SettingsMode::Gui, settings::SettingsMode::Tui] {
        for command in all_commands(settings_mode) {
            assert!(
                command.supports_surface(settings_mode),
                "{} should support {settings_mode:?}",
                command.name
            );
        }
    }
    assert_eq!(COST.kind, SlashCommandKind::Cost);
    assert!(matches!(
        COST.supported_surfaces,
        SlashCommandSurfaces::GuiAndTui {
            icon_path: "bundled/svg/bar-chart-04.svg"
        }
    ));
    assert_eq!(EXIT.kind, SlashCommandKind::Exit);
    assert_eq!(EXIT.supported_surfaces, SlashCommandSurfaces::TuiOnly);
    assert_eq!(ADD_MCP.kind, SlashCommandKind::AddMcp);
    assert!(matches!(
        ADD_MCP.supported_surfaces,
        SlashCommandSurfaces::GuiOnly {
            icon_path: "bundled/svg/dataflow.svg"
        }
    ));
}
#[test]
fn view_logs_command_is_registered_only_for_tui_mode() {
    assert!(
        all_commands(settings::SettingsMode::Tui)
            .iter()
            .any(|command| command == &VIEW_LOGS)
    );
    assert!(
        !all_commands(settings::SettingsMode::Gui)
            .iter()
            .any(|command| command == &VIEW_LOGS)
    );
}

#[test]
fn auto_approve_command_is_local_agent_action_without_arguments() {
    let tui_commands = all_commands(settings::SettingsMode::Tui);
    let command = tui_commands
        .iter()
        .find(|command| command.name == AUTO_APPROVE.name)
        .expect("expected /auto-approve to be registered in TUI mode");
    assert!(
        all_commands(settings::SettingsMode::Gui)
            .iter()
            .all(|command| command.name != AUTO_APPROVE.name)
    );

    assert_eq!(command.description, "Toggle auto approve");
    assert_eq!(command.supported_surfaces.gui_icon_path(), None);
    assert!(!command.auto_enter_ai_mode);
    assert_eq!(
        command.availability,
        Availability::AGENT_VIEW
            | Availability::ACTIVE_CONVERSATION
            | Availability::AI_ENABLED
            | Availability::NOT_CLOUD_AGENT
    );
    assert!(command.argument.is_none());
    assert!(command.is_active(
        Availability::AGENT_VIEW
            | Availability::ACTIVE_CONVERSATION
            | Availability::AI_ENABLED
            | Availability::NOT_CLOUD_AGENT
    ));
    assert!(!command.is_active(
        Availability::AGENT_VIEW
            | Availability::ACTIVE_CONVERSATION
            | Availability::AI_ENABLED
            | Availability::CLOUD_AGENT
    ));
}

#[test]
fn logout_command_is_registered_only_for_tui_mode() {
    assert!(
        all_commands(settings::SettingsMode::Tui)
            .iter()
            .any(|command| command == &LOGOUT)
    );
    assert!(
        !all_commands(settings::SettingsMode::Gui)
            .iter()
            .any(|command| command == &LOGOUT)
    );
}

#[test]
fn version_command_is_registered_only_for_tui_mode() {
    assert!(
        all_commands(settings::SettingsMode::Tui)
            .iter()
            .any(|command| command == &VERSION)
    );
    assert!(
        !all_commands(settings::SettingsMode::Gui)
            .iter()
            .any(|command| command == &VERSION)
    );
}

#[test]
fn rename_tab_command_requires_argument() {
    let command = COMMAND_REGISTRY
        .get_command_with_name(RENAME_TAB.name)
        .expect("expected /rename-tab to be registered");
    let argument = command
        .argument
        .as_ref()
        .expect("expected /rename-tab to require an argument");

    assert!(!argument.is_optional);
    assert!(!argument.should_execute_on_selection);
    assert_eq!(argument.hint_text, Some("<tab name>"));
}

#[test]
fn rename_conversation_command_is_active_conversation_scoped_and_requires_argument() {
    let command = COMMAND_REGISTRY
        .get_command_with_name(RENAME_CONVERSATION.name)
        .expect("expected /rename-conversation to be registered");
    let argument = command
        .argument
        .as_ref()
        .expect("expected /rename-conversation to require an argument");

    assert_eq!(command.name, "/rename-conversation");
    assert_eq!(
        command.supported_surfaces.gui_icon_path(),
        Some("bundled/svg/pencil-line.svg")
    );
    assert!(!command.auto_enter_ai_mode);
    assert_eq!(
        command.availability,
        Availability::AGENT_VIEW | Availability::ACTIVE_CONVERSATION | Availability::AI_ENABLED,
    );
    assert!(!argument.is_optional);
    assert!(!argument.should_execute_on_selection);
    assert_eq!(argument.hint_text, Some("<new title>"));
}

#[cfg(not(target_family = "wasm"))]
#[test]
fn continue_locally_command_is_registered() {
    let command = COMMAND_REGISTRY
        .get_command_with_name(CONTINUE_LOCALLY.name)
        .expect("expected /continue-locally to be registered");

    assert_eq!(command.name, "/continue-locally");
    assert_eq!(
        command.supported_surfaces.gui_icon_path(),
        Some("bundled/svg/arrow-split.svg")
    );
    assert!(command.auto_enter_ai_mode);
    assert_eq!(
        command.availability,
        Availability::AGENT_VIEW
            | Availability::ACTIVE_CONVERSATION
            | Availability::AI_ENABLED
            | Availability::CLOUD_AGENT
    );

    let argument = command
        .argument
        .as_ref()
        .expect("expected /continue-locally to declare an argument");
    assert!(argument.is_optional);
    assert!(!argument.should_execute_on_selection);
    assert_eq!(
        argument.hint_text,
        Some("<optional prompt to send in local conversation>")
    );
}

#[test]
fn set_tab_color_command_requires_argument() {
    let command = COMMAND_REGISTRY
        .get_command_with_name(SET_TAB_COLOR.name)
        .expect("expected /set-tab-color to be registered");
    let argument = command
        .argument
        .as_ref()
        .expect("expected /set-tab-color to require an argument");

    assert!(!argument.is_optional);
    assert!(!argument.should_execute_on_selection);

    let hint = argument
        .hint_text
        .expect("/set-tab-color hint text is set dynamically");
    for color in color_dot::TAB_COLOR_OPTIONS {
        let lower = color.to_string().to_ascii_lowercase();
        assert!(hint.contains(&lower), "hint should mention `{lower}`");
    }
    assert!(hint.contains("none"), "hint should mention `none`");
}

#[test]
fn handoff_command_inserts_into_buffer_on_selection() {
    // `/handoff` must NOT execute on selection: selecting it from the slash menu should
    // insert `/handoff ` into the input buffer so the user can append an optional follow-up
    // prompt before executing (matching how other argument-taking commands like `/fork`
    // behave). See REMOTE-2029.
    let argument = MOVE_TO_CLOUD
        .argument
        .as_ref()
        .expect("expected /handoff to declare an argument");

    assert_eq!(MOVE_TO_CLOUD.name, "/handoff");
    assert!(argument.is_optional);
    assert!(!argument.should_execute_on_selection);
    assert_eq!(argument.hint_text, Some("<optional follow-up prompt>"));
}

#[test]
fn strip_command_prefix_matches_orchestrate() {
    let result = strip_command_prefix("/orchestrate deploy services", "/orchestrate");
    assert_eq!(result, Some("deploy services".to_string()));
}

#[test]
fn strip_command_prefix_no_match() {
    let result = strip_command_prefix("just a normal query", "/plan");
    assert_eq!(result, None);
}

#[test]
fn strip_command_prefix_empty() {
    let result = strip_command_prefix("", "/plan");
    assert_eq!(result, None);
}

#[test]
fn strip_command_prefix_no_trailing_space() {
    // "/plan" alone (no trailing space) should NOT be stripped
    let result = strip_command_prefix("/plan", "/plan");
    assert_eq!(result, None);
}

#[test]
fn strip_command_prefix_trailing_space_only() {
    // "/plan " with nothing after should strip to empty string
    let result = strip_command_prefix("/plan ", "/plan");
    assert_eq!(result, Some(String::new()));
}

#[test]
fn strip_command_prefix_substring_not_matched() {
    // "/planning" should not match "/plan"
    let result = strip_command_prefix("/planning something", "/plan");
    assert_eq!(result, None);
}

#[test]
fn natural_language_detection_command_is_registered_only_for_tui_mode() {
    let tui_commands = all_commands(settings::SettingsMode::Tui);
    assert!(
        tui_commands
            .iter()
            .any(|command| command == &NATURAL_LANGUAGE_DETECTION)
    );

    let gui_commands = all_commands(settings::SettingsMode::Gui);
    assert!(
        !gui_commands
            .iter()
            .any(|command| command == &NATURAL_LANGUAGE_DETECTION)
    );
}

#[test]
fn natural_language_detection_command_is_ai_enabled_and_executes_immediately() {
    let command = all_commands(settings::SettingsMode::Tui)
        .into_iter()
        .find(|command| command.kind == SlashCommandKind::NaturalLanguageDetection)
        .expect("expected /natural-language-detection to be registered in TUI mode");
    assert_eq!(command.availability, Availability::AI_ENABLED);
    assert!(!command.auto_enter_ai_mode);
    assert!(command.argument.is_none());
}
