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
fn command_registry_contains_commands_for_both_surfaces() {
    let registry = Registry::new();

    assert_eq!(
        registry
            .get_command_with_name(UPGRADE.name)
            .map(|command| command.supported_surfaces),
        Some(SlashCommandSurfaces::TuiOnly)
    );
    assert!(matches!(
        registry
            .get_command_with_name(ADD_MCP.name)
            .map(|command| command.supported_surfaces),
        Some(SlashCommandSurfaces::GuiOnly { .. })
    ));
}

#[test]
fn voice_command_is_registered_only_for_tui_mode() {
    assert!(
        all_commands(settings::SettingsMode::Tui)
            .iter()
            .any(|command| command == &VOICE)
    );
    assert!(
        !all_commands(settings::SettingsMode::Gui)
            .iter()
            .any(|command| command == &VOICE)
    );
    assert!(VOICE.argument.is_none());
    assert_eq!(VOICE.description, "Start voice input (Ctrl-S)");
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
fn api_keys_command_is_tui_only_and_has_no_arguments() {
    let command = all_commands(settings::SettingsMode::Tui)
        .into_iter()
        .find(|command| command.kind == SlashCommandKind::ApiKeys)
        .expect("expected /api-keys to be registered in TUI mode");
    assert_eq!(command, API_KEYS);
    assert!(!command.auto_enter_ai_mode);
    assert_eq!(command.availability, Availability::AI_ENABLED);
    assert!(command.argument.is_none());
    assert_eq!(command.description, "View and manage API keys");
    assert!(
        all_commands(settings::SettingsMode::Gui)
            .iter()
            .all(|command| command.kind != SlashCommandKind::ApiKeys)
    );
    assert!(
        all_commands(settings::SettingsMode::Tui)
            .iter()
            .all(|command| !matches!(command.name, "/add-api-key" | "/clear-provider-api-key"))
    );
}

#[test]
fn connect_grok_command_is_tui_only_and_has_no_arguments() {
    let command = all_commands(settings::SettingsMode::Tui)
        .into_iter()
        .find(|command| command.kind == SlashCommandKind::ConnectGrok)
        .expect("expected /connect-grok to be registered in TUI mode");
    assert_eq!(command, CONNECT_GROK);
    assert_eq!(command.name, "/connect-grok");
    assert_eq!(command.supported_surfaces, SlashCommandSurfaces::TuiOnly);
    assert_eq!(command.availability, Availability::AI_ENABLED);
    assert!(!command.auto_enter_ai_mode);
    assert!(command.argument.is_none());
    assert_eq!(
        command.description,
        "Connect your Grok (X Premium / SuperGrok) account"
    );
    assert!(
        all_commands(settings::SettingsMode::Gui)
            .iter()
            .all(|command| command.kind != SlashCommandKind::ConnectGrok)
    );
}

#[test]
fn manage_billing_command_is_always_available_only_in_tui_mode() {
    let command = all_commands(settings::SettingsMode::Tui)
        .into_iter()
        .find(|command| command.kind == SlashCommandKind::ManageBilling)
        .expect("expected /manage-billing to be registered in TUI mode");

    assert_eq!(command, MANAGE_BILLING);
    assert_eq!(command.availability, Availability::ALWAYS);
    assert_eq!(command.supported_surfaces, SlashCommandSurfaces::TuiOnly);
    assert!(!command.auto_enter_ai_mode);
    assert!(command.argument.is_none());
    assert!(
        all_commands(settings::SettingsMode::Gui)
            .iter()
            .all(|command| command.kind != SlashCommandKind::ManageBilling)
    );
}

#[test]
fn upgrade_command_is_always_available_only_in_tui_mode() {
    let command = all_commands(settings::SettingsMode::Tui)
        .into_iter()
        .find(|command| command.kind == SlashCommandKind::Upgrade)
        .expect("expected /upgrade to be registered in TUI mode");

    assert_eq!(command, UPGRADE);
    assert_eq!(command.availability, Availability::ALWAYS);
    assert_eq!(command.supported_surfaces, SlashCommandSurfaces::TuiOnly);
    assert!(!command.auto_enter_ai_mode);
    assert!(command.argument.is_none());
    assert!(
        all_commands(settings::SettingsMode::Gui)
            .iter()
            .all(|command| command.kind != SlashCommandKind::Upgrade)
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
fn statusline_command_is_always_available_only_in_tui_mode() {
    let command = all_commands(settings::SettingsMode::Tui)
        .into_iter()
        .find(|command| command.kind == SlashCommandKind::Statusline)
        .expect("expected /statusline to be registered in TUI mode");
    assert_eq!(command, STATUSLINE);
    assert_eq!(command.availability, Availability::ALWAYS);
    assert_eq!(command.supported_surfaces, SlashCommandSurfaces::TuiOnly);
    assert!(!command.auto_enter_ai_mode);
    assert!(command.argument.is_none());
    assert!(
        all_commands(settings::SettingsMode::Gui)
            .iter()
            .all(|command| command.kind != SlashCommandKind::Statusline)
    );
}

#[test]
fn reset_statusline_command_is_always_available_only_in_tui_mode() {
    let command = all_commands(settings::SettingsMode::Tui)
        .into_iter()
        .find(|command| command.kind == SlashCommandKind::ResetStatusline)
        .expect("expected /reset-statusline to be registered in TUI mode");
    assert_eq!(command, RESET_STATUSLINE);
    assert_eq!(command.availability, Availability::ALWAYS);
    assert_eq!(command.supported_surfaces, SlashCommandSurfaces::TuiOnly);
    assert!(!command.auto_enter_ai_mode);
    assert!(command.argument.is_none());
    assert!(
        all_commands(settings::SettingsMode::Gui)
            .iter()
            .all(|command| command.kind != SlashCommandKind::ResetStatusline)
    );
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
fn version_command_is_not_registered() {
    for settings_mode in [settings::SettingsMode::Gui, settings::SettingsMode::Tui] {
        assert!(
            all_commands(settings_mode)
                .iter()
                .all(|command| command.name != "/version")
        );
    }
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
fn copy_debugging_id_command_is_registered_for_gui_and_tui() {
    for settings_mode in [settings::SettingsMode::Gui, settings::SettingsMode::Tui] {
        assert!(
            all_commands(settings_mode)
                .iter()
                .any(|command| command.kind == SlashCommandKind::CopyDebuggingId),
            "/copy-debugging-id should be registered in {settings_mode:?} mode"
        );
    }
}

#[test]
fn copy_debugging_id_command_has_correct_registry_metadata() {
    let command = all_commands(settings::SettingsMode::Tui)
        .into_iter()
        .find(|command| command.kind == SlashCommandKind::CopyDebuggingId)
        .expect("expected /copy-debugging-id to be registered");

    assert_eq!(command.name, "/copy-debugging-id");
    assert_eq!(command.kind, SlashCommandKind::CopyDebuggingId);
    assert_eq!(
        command.supported_surfaces,
        SlashCommandSurfaces::GuiAndTui {
            icon_path: "bundled/svg/copy.svg"
        }
    );
    assert!(!command.auto_enter_ai_mode);
    assert_eq!(command.availability, Availability::ACTIVE_CONVERSATION);
    assert!(command.argument.is_none());
    // Available when there is an active conversation.
    assert!(command.is_active(Availability::ACTIVE_CONVERSATION));
    // Hidden when there is no active conversation.
    assert!(!command.is_active(Availability::ALWAYS));
}

#[test]
fn clear_command_is_registered_only_for_tui_mode() {
    assert!(
        all_commands(settings::SettingsMode::Tui)
            .iter()
            .any(|command| command.kind == SlashCommandKind::Clear),
        "/clear should be registered in TUI mode"
    );
    assert!(
        all_commands(settings::SettingsMode::Gui)
            .iter()
            .all(|command| command.kind != SlashCommandKind::Clear),
        "/clear should not be registered in GUI mode"
    );
}

#[test]
fn clear_command_has_correct_registry_metadata() {
    let command = all_commands(settings::SettingsMode::Tui)
        .into_iter()
        .find(|command| command.kind == SlashCommandKind::Clear)
        .expect("expected /clear to be registered in TUI mode");

    assert_eq!(command.name, "/clear");
    assert_eq!(command.kind, SlashCommandKind::Clear);
    assert_eq!(command.supported_surfaces, SlashCommandSurfaces::TuiOnly);
    assert_eq!(command.supported_surfaces.gui_icon_path(), None);
    assert!(!command.auto_enter_ai_mode);
    assert_eq!(
        command.availability,
        Availability::NO_LRC_CONTROL | Availability::AI_ENABLED | Availability::NOT_CLOUD_AGENT
    );

    let argument = command
        .argument
        .as_ref()
        .expect("expected /clear to declare an argument");
    assert!(argument.is_optional);
    assert!(argument.should_execute_on_selection);
    assert_eq!(argument.hint_text, None);
}

#[test]
fn clear_command_is_active_only_outside_cloud_mode() {
    let local_context =
        Availability::NO_LRC_CONTROL | Availability::AI_ENABLED | Availability::NOT_CLOUD_AGENT;
    assert!(CLEAR.is_active(local_context));

    // NOT_CLOUD_AGENT is absent → cloud context.
    let cloud_context = Availability::NO_LRC_CONTROL | Availability::AI_ENABLED;
    assert!(!CLEAR.is_active(cloud_context));
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

#[test]
fn theme_command_is_registered_only_for_tui_mode() {
    let tui_commands = all_commands(settings::SettingsMode::Tui);
    let command = tui_commands
        .iter()
        .find(|command| command.kind == SlashCommandKind::Theme)
        .expect("expected /theme to be registered in TUI mode");

    assert_eq!(command, &THEME);
    assert_eq!(command.availability, Availability::ALWAYS);
    let argument = command
        .argument
        .as_ref()
        .expect("expected /theme to require an argument");
    assert!(!argument.is_optional);
    assert!(!argument.should_execute_on_selection);
    assert_eq!(argument.hint_text, Some("<auto|light|dark>"));
    assert!(
        all_commands(settings::SettingsMode::Gui)
            .iter()
            .all(|command| command.kind != SlashCommandKind::Theme)
    );
}
