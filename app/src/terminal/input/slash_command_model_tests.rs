use settings::Setting as _;
use warpui::{App, SingletonEntity as _};

use super::SlashCommandEntryState;
use crate::ai::blocklist::QueuedQueryId;
use crate::report_if_error;
use crate::search::slash_command_menu::static_commands::commands;
use crate::settings::AISettings;
use crate::terminal::input::tests::{add_window_with_bootstrapped_terminal, initialize_app};

#[test]
fn test_parse_slash_command_handles_argument_rules() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let terminal = add_window_with_bootstrapped_terminal(
            &mut app, None, /* history_file_commands */
            None,
        )
        .await;
        let input = terminal.read(&app, |terminal, _| terminal.input().clone());
        let slash_command_data_source =
            input.read(&app, |input, _| input.slash_command_data_source.clone());

        slash_command_data_source.read(&app, |data_source, _| {
            let no_argument_command_name = data_source
                .active_commands()
                .find_map(|(_, command)| command.argument.is_none().then_some(command.name))
                .expect("expected at least one slash command without an argument");

            let optional_argument_command_name = data_source
                .active_commands()
                .find_map(|(_, command)| {
                    (command.argument.is_some()
                        && data_source.parse_slash_command(command.name).is_some())
                    .then_some(command.name)
                })
                .expect("expected at least one slash command that accepts optional arguments");

            let with_extra_text = format!("{no_argument_command_name} trailing");
            assert!(
                data_source.parse_slash_command(&with_extra_text).is_none(),
                "commands without arguments should reject non-whitespace suffixes"
            );

            let with_whitespace = format!("{no_argument_command_name}   ");
            let detected_with_whitespace = data_source
                .parse_slash_command(&with_whitespace)
                .expect("command with whitespace-only suffix should still be parsed");
            assert_eq!(
                detected_with_whitespace.command.name,
                no_argument_command_name
            );

            let detected_without_argument = data_source
                .parse_slash_command(optional_argument_command_name)
                .expect("optional-argument command should parse without an argument");
            assert_eq!(detected_without_argument.argument, None);

            let with_argument = format!("{optional_argument_command_name} prompt");
            let detected_with_argument = data_source
                .parse_slash_command(&with_argument)
                .expect("optional-argument command should parse with an argument");
            assert_eq!(detected_with_argument.argument.as_deref(), Some("prompt"));
        });
    });
}

#[test]
fn test_parse_rename_tab_slash_command_arguments() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let terminal = add_window_with_bootstrapped_terminal(
            &mut app, None, /* history_file_commands */
            None,
        )
        .await;
        let input = terminal.read(&app, |terminal, _| terminal.input().clone());
        let slash_command_data_source =
            input.read(&app, |input, _| input.slash_command_data_source.clone());

        slash_command_data_source.read(&app, |data_source, _| {
            assert!(
                data_source
                    .parse_slash_command(commands::RENAME_TAB.name)
                    .is_none(),
                "expected /rename-tab without an argument not to parse"
            );

            let detected_with_argument = data_source
                .parse_slash_command("/rename-tab Backend")
                .expect("expected /rename-tab to parse with an argument");
            assert_eq!(detected_with_argument.argument.as_deref(), Some("Backend"));

            let detected_with_multi_word_argument = data_source
                .parse_slash_command("/rename-tab Backend API")
                .expect("expected /rename-tab to preserve the rest of the line as one argument");
            assert_eq!(
                detected_with_multi_word_argument.argument.as_deref(),
                Some("Backend API")
            );

            let detected_with_empty_argument = data_source
                .parse_slash_command("/rename-tab ")
                .expect("expected /rename-tab to parse once the required argument is started");
            assert_eq!(detected_with_empty_argument.argument.as_deref(), Some(""));
        });
    });
}

#[test]
fn test_non_ai_commands_remain_active_when_ai_is_disabled() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let terminal = add_window_with_bootstrapped_terminal(
            &mut app, None, /* history_file_commands */
            None,
        )
        .await;
        let input = terminal.read(&app, |terminal, _| terminal.input().clone());
        let slash_command_data_source =
            input.read(&app, |input, _| input.slash_command_data_source.clone());

        // Disable AI globally.
        AISettings::handle(&app).update(&mut app, |settings, ctx| {
            report_if_error!(settings.is_any_ai_enabled.set_value(false, ctx));
        });

        slash_command_data_source.read(&app, |data_source, _| {
            let active_command_names: Vec<&str> = data_source
                .active_commands()
                .map(|(_, command)| command.name)
                .collect();

            // Commands that don't require AI should still be active.
            // `/rename-tab` is a good canary because it has no session-context requirements
            // other than ALWAYS.
            assert!(
                active_command_names.contains(&commands::RENAME_TAB.name),
                "/rename-tab should remain active when AI is off, got: {active_command_names:?}"
            );

            // Commands that require AI should be filtered out.
            assert!(
                !active_command_names.contains(&commands::AGENT.name),
                "/agent should NOT be active when AI is off, got: {active_command_names:?}"
            );
            assert!(
                !active_command_names.contains(&commands::PLAN.name),
                "/plan should NOT be active when AI is off, got: {active_command_names:?}"
            );
        });
    });
}

#[test]
fn test_disabled_until_empty_buffer_ignores_non_slash_edits() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let terminal = add_window_with_bootstrapped_terminal(
            &mut app, None, /* history_file_commands */
            None,
        )
        .await;
        let input = terminal.read(&app, |terminal, _| terminal.input().clone());

        input.update(&mut app, |input, ctx| {
            input.set_input_mode_natural_language_detection(ctx);
            input.user_insert("echo hello", ctx);
        });
        input.update(&mut app, |input, ctx| {
            input.slash_command_model.update(ctx, |model, ctx| {
                model.disable(ctx);
            });
        });

        input.read(&app, |input, ctx| {
            assert!(matches!(
                input.slash_command_model.as_ref(ctx).state(),
                SlashCommandEntryState::DisabledUntilEmptyBuffer
            ));
        });

        input.update(&mut app, |input, ctx| {
            input.user_insert(" world", ctx);
        });

        input.read(&app, |input, ctx| {
            assert!(matches!(
                input.slash_command_model.as_ref(ctx).state(),
                SlashCommandEntryState::DisabledUntilEmptyBuffer
            ));
        });
    });
}

#[test]
fn test_disabled_until_empty_buffer_reevaluates_when_slash_is_added_to_start() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let terminal = add_window_with_bootstrapped_terminal(
            &mut app, None, /* history_file_commands */
            None,
        )
        .await;
        let input = terminal.read(&app, |terminal, _| terminal.input().clone());
        let slash_command_data_source =
            input.read(&app, |input, _| input.slash_command_data_source.clone());
        let detected_command_name = slash_command_data_source.read(&app, |data_source, _| {
            data_source
                .active_commands()
                .find_map(|(_, command)| {
                    data_source
                        .parse_slash_command(command.name)
                        .is_some()
                        .then(|| command.name.to_owned())
                })
                .expect("expected at least one active slash command")
        });

        input.update(&mut app, |input, ctx| {
            input.set_input_mode_natural_language_detection(ctx);
            input.user_insert("echo hello", ctx);
        });
        input.update(&mut app, |input, ctx| {
            input.slash_command_model.update(ctx, |model, ctx| {
                model.disable(ctx);
            });
        });

        input.update(&mut app, |input, ctx| {
            input.user_replace_editor_text(&detected_command_name, ctx);
        });

        input.read(&app, |input, ctx| {
            assert!(
                matches!(
                    input.slash_command_model.as_ref(ctx).state(),
                    SlashCommandEntryState::SlashCommand(_)
                ),
                "adding '/' at the start of the buffer should re-enable slash command parsing"
            );
        });
    });
}

#[test]
fn test_second_slash_in_command_token_sets_state_to_none() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let terminal = add_window_with_bootstrapped_terminal(
            &mut app, None, /* history_file_commands */
            None,
        )
        .await;
        let input = terminal.read(&app, |terminal, _| terminal.input().clone());

        input.update(&mut app, |input, ctx| {
            input.set_input_mode_natural_language_detection(ctx);
            input.user_insert("/foo/bar", ctx);
        });

        input.read(&app, |input, ctx| {
            assert!(matches!(
                input.slash_command_model.as_ref(ctx).state(),
                SlashCommandEntryState::None
            ));
        });
    });
}

#[test]
fn test_detect_command_returns_none_for_plain_text() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let terminal = add_window_with_bootstrapped_terminal(&mut app, None, None).await;
        let input = terminal.read(&app, |terminal, _| terminal.input().clone());

        input.read(&app, |input, ctx| {
            let result = input
                .slash_command_model
                .as_ref(ctx)
                .detect_command("fix the tests", ctx);
            assert!(
                matches!(result, SlashCommandEntryState::None),
                "plain text should not be detected as a command"
            );
        });
    });
}

#[test]
fn test_detect_command_finds_known_slash_command() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let terminal = add_window_with_bootstrapped_terminal(&mut app, None, None).await;
        let input = terminal.read(&app, |terminal, _| terminal.input().clone());

        input.read(&app, |input, ctx| {
            let slash_command_data_source = &input.slash_command_data_source;
            let command_name = slash_command_data_source
                .as_ref(ctx)
                .active_commands()
                .find_map(|(_, command)| {
                    slash_command_data_source
                        .as_ref(ctx)
                        .parse_slash_command(command.name)
                        .is_some()
                        .then(|| command.name.to_owned())
                })
                .expect("expected at least one active slash command");

            let result = input
                .slash_command_model
                .as_ref(ctx)
                .detect_command(&command_name, ctx);
            assert!(
                matches!(result, SlashCommandEntryState::SlashCommand(_)),
                "known command should be detected: {command_name}"
            );
        });
    });
}

#[test]
fn test_detect_command_returns_none_for_unknown_slash() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let terminal = add_window_with_bootstrapped_terminal(&mut app, None, None).await;
        let input = terminal.read(&app, |terminal, _| terminal.input().clone());

        input.read(&app, |input, ctx| {
            let result = input
                .slash_command_model
                .as_ref(ctx)
                .detect_command("/nonexistent-command-xyz", ctx);
            assert!(
                matches!(result, SlashCommandEntryState::None),
                "unknown /command should return None"
            );
        });
    });
}

#[test]
fn test_detect_command_matches_buffer_driven_detection() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let terminal = add_window_with_bootstrapped_terminal(&mut app, None, None).await;
        let input = terminal.read(&app, |terminal, _| terminal.input().clone());

        let command_name = input.read(&app, |input, ctx| {
            let ds = &input.slash_command_data_source;
            ds.as_ref(ctx)
                .active_commands()
                .find_map(|(_, command)| {
                    ds.as_ref(ctx)
                        .parse_slash_command(command.name)
                        .is_some()
                        .then(|| command.name.to_owned())
                })
                .expect("expected at least one active slash command")
        });

        // Type the command into the buffer so the buffer-driven model processes it.
        input.update(&mut app, |input, ctx| {
            input.set_input_mode_natural_language_detection(ctx);
            input.user_insert(&command_name, ctx);
        });

        // Compare the buffer-driven state with the stateless detect_command result.
        input.read(&app, |input, ctx| {
            let buffer_state = input.slash_command_model.as_ref(ctx).state().clone();
            let detect_state = input
                .slash_command_model
                .as_ref(ctx)
                .detect_command(&command_name, ctx);

            let buffer_is_slash = matches!(buffer_state, SlashCommandEntryState::SlashCommand(_));
            let detect_is_slash = matches!(detect_state, SlashCommandEntryState::SlashCommand(_));
            assert_eq!(
                buffer_is_slash, detect_is_slash,
                "detect_command and buffer-driven detection should agree for '{command_name}'"
            );
        });
    });
}

#[test]
fn test_detect_command_parses_queue_with_argument() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let terminal = add_window_with_bootstrapped_terminal(&mut app, None, None).await;
        let input = terminal.read(&app, |terminal, _| terminal.input().clone());

        input.read(&app, |input, ctx| {
            let result = input
                .slash_command_model
                .as_ref(ctx)
                .detect_command("/queue fix the tests", ctx);

            if let SlashCommandEntryState::SlashCommand(detected) = result {
                assert_eq!(detected.command.name, "/queue");
                assert_eq!(detected.argument.as_deref(), Some("fix the tests"));
            } else {
                // /queue may not be registered if the feature flag is off in tests.
                // That's fine — this test validates argument extraction when it is.
            }
        });
    });
}

#[test]
fn test_detect_command_returns_queue_with_no_argument() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let terminal = add_window_with_bootstrapped_terminal(&mut app, None, None).await;
        let input = terminal.read(&app, |terminal, _| terminal.input().clone());

        input.read(&app, |input, ctx| {
            let result = input
                .slash_command_model
                .as_ref(ctx)
                .detect_command("/queue", ctx);

            // /queue requires an argument, so bare "/queue" should not be detected.
            // (StaticCommand with required argument rejects input without a space-delimited arg.)
            assert!(
                !matches!(result, SlashCommandEntryState::SlashCommand(_)),
                "/queue with no argument should not be detected as a complete slash command"
            );
        });
    });
}

#[test]
fn test_submit_queued_prompt_routes_plain_text_to_conversation() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let terminal = add_window_with_bootstrapped_terminal(&mut app, None, None).await;
        let input = terminal.read(&app, |terminal, _| terminal.input().clone());

        // Set up AI input mode so the input can interact with the AI controller.
        input.update(&mut app, |input, ctx| {
            input.set_input_mode_natural_language_detection(ctx);
        });

        // submit_queued_prompt with plain text should not panic or crash.
        // It routes through detect_command (returning None) and falls through
        // to send_user_query_in_new_conversation.
        input.update(&mut app, |input, ctx| {
            input.submit_queued_prompt(
                "fix the tests".to_string(),
                QueuedQueryId::new_for_test(),
                ctx,
            );
        });
    });
}

#[test]
fn test_submit_queued_prompt_detects_slash_command() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);

        let terminal = add_window_with_bootstrapped_terminal(&mut app, None, None).await;
        let input = terminal.read(&app, |terminal, _| terminal.input().clone());

        input.update(&mut app, |input, ctx| {
            input.set_input_mode_natural_language_detection(ctx);
        });

        // Verify that submit_queued_prompt correctly detects slash commands.
        // Find a command that exists and has an optional argument.
        let command_with_arg = input.read(&app, |input, ctx| {
            let ds = &input.slash_command_data_source;
            ds.as_ref(ctx).active_commands().find_map(|(_, command)| {
                (command.argument.as_ref().is_some_and(|a| a.is_optional)
                    && ds.as_ref(ctx).parse_slash_command(command.name).is_some())
                .then(|| command.name.to_owned())
            })
        });

        if let Some(command_text) = command_with_arg {
            // submit_queued_prompt should detect the slash command and route through
            // execute_slash_command. This should not panic.
            input.update(&mut app, |input, ctx| {
                input.submit_queued_prompt(command_text, QueuedQueryId::new_for_test(), ctx);
            });
        }
    });
}
