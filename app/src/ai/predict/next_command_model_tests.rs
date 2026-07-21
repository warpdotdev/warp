use typed_path::TypedPathBuf;
use warp_completer::meta::SpannedItem;
use warp_completer::parsers::ParsedToken;
use warp_completer::signatures::CommandRegistry;
use warpui::App;

use super::*;
use crate::completer::SessionContext;
use crate::terminal::model::session::command_executor::testing::TestCommandExecutor;
use crate::terminal::model::session::{Session, SessionInfo};

#[test]
fn test_find_autosuggestion_from_history_same_directory() {
    let history_entries = [
        HistoryEntry::with_pwd_and_exit_code("cd Dotfiles", "/Users/tadej", 0),
        HistoryEntry::with_pwd_and_exit_code("cd Documents", "/Users/tadej", 0),
        HistoryEntry::command_only("cd Pictures"),
        HistoryEntry::with_pwd_and_exit_code("cd Downloads", "/Users/tadej/dev", 0),
    ];

    // We should return the most recent command from history that starts with the
    // buffer text and was executed in the current working directory.
    let autosuggestions = find_potential_autosuggestions_from_history(
        history_entries.iter(),
        "cd D",
        Some("/Users/tadej"),
    )
    .into_iter()
    .map(|history_entry| history_entry.command)
    .collect_vec();

    assert_eq!(
        autosuggestions,
        vec![
            "cd Documents".to_owned(),
            "cd Dotfiles".to_owned(),
            "cd Downloads".to_owned(),
        ]
    );
}

#[test]
fn test_find_autosuggestion_from_history_error_exit_code() {
    let history_entries = [
        HistoryEntry::with_pwd_and_exit_code("cd Dotfiles", "/Users/tadej", 0),
        HistoryEntry::with_pwd_and_exit_code("cd Documents", "/Users/tadej", 1),
        HistoryEntry::command_only("cd Pictures"),
        HistoryEntry::with_pwd_and_exit_code("cd Downloads", "/Users/tadej/dev", 0),
    ];

    // We want to return failed commands in case the user wants to run it again.
    let autosuggestions = find_potential_autosuggestions_from_history(
        history_entries.iter(),
        "cd D",
        Some("/Users/tadej"),
    )
    .into_iter()
    .map(|history_entry| history_entry.command)
    .collect_vec();

    assert_eq!(
        autosuggestions,
        vec![
            "cd Documents".to_owned(),
            "cd Dotfiles".to_owned(),
            "cd Downloads".to_owned(),
        ]
    );
}

#[test]
fn test_find_autosuggestion_from_history_no_working_dir() {
    let history_entries = [
        HistoryEntry::with_pwd_and_exit_code("cd Dotfiles", "/Users/tadej", 0),
        HistoryEntry::command_only("cd Pictures"),
        HistoryEntry::with_pwd_and_exit_code("cd Downloads", "/Users/tadej/dev", 0),
    ];

    // No working directory, so return the first successful command.
    let autosuggestions =
        find_potential_autosuggestions_from_history(history_entries.iter(), "cd D", None)
            .into_iter()
            .map(|history_entry| history_entry.command)
            .collect_vec();

    assert_eq!(
        autosuggestions,
        vec!["cd Downloads".to_owned(), "cd Dotfiles".to_owned(),]
    );
}

#[test]
fn test_find_autosuggestion_from_history_different_directory() {
    let history_entries = [
        HistoryEntry::with_pwd_and_exit_code("cd Dotfiles", "/Users/tadej", 0),
        HistoryEntry::with_pwd_and_exit_code("cd Documents", "/Users/tadej", 0),
        HistoryEntry::command_only("cd Pictures"),
        HistoryEntry::with_pwd_and_exit_code("cd Downloads", "/Users/tadej/dev", 0),
    ];

    // There are no commands in history that were executed in the current directory,
    // So we return the most recent command that starts with the buffer text.
    let autosuggestions = find_potential_autosuggestions_from_history(
        history_entries.iter(),
        "cd D",
        Some("/Users/jonas"),
    )
    .into_iter()
    .map(|history_entry| history_entry.command)
    .collect_vec();

    assert_eq!(
        autosuggestions,
        vec![
            "cd Downloads".to_owned(),
            "cd Documents".to_owned(),
            "cd Dotfiles".to_owned(),
        ]
    );

    // There isn't a current working directory, so return the most recent command that
    // starts with the buffer text.
    let autosuggestions =
        find_potential_autosuggestions_from_history(history_entries.iter(), "cd D", None)
            .into_iter()
            .map(|history_entry| history_entry.command)
            .collect_vec();

    assert_eq!(
        autosuggestions,
        vec![
            "cd Downloads".to_owned(),
            "cd Documents".to_owned(),
            "cd Dotfiles".to_owned(),
        ]
    );
}

#[test]
fn test_find_autosuggestion_from_history_no_matching_commands() {
    let history_entries = [
        HistoryEntry::with_pwd_and_exit_code("cd Dotfiles", "/Users/tadej", 0),
        HistoryEntry::with_pwd_and_exit_code("cd Documents", "/Users/tadej", 0),
        HistoryEntry::command_only("cd Pictures"),
        HistoryEntry::with_pwd_and_exit_code("cd Downloads", "/Users/tadej/dev", 0),
    ];

    let autosuggestions = find_potential_autosuggestions_from_history(
        history_entries.iter(),
        "cd Z",
        Some("/Users/jonas"),
    );

    assert_eq!(autosuggestions, vec![]);
}

#[test]
fn test_find_autosuggestion_from_history_matches_command_with_no_pwd() {
    let history_entries = [
        HistoryEntry::with_pwd_and_exit_code("cd Dotfiles", "/Users/tadej", 0),
        HistoryEntry::with_pwd_and_exit_code("cd Documents", "/Users/tadej", 0),
        HistoryEntry::command_only("cd Pictures"),
        HistoryEntry::with_pwd_and_exit_code("cd Downloads", "/Users/tadej/dev", 0),
    ];

    let autosuggestions = find_potential_autosuggestions_from_history(
        history_entries.iter(),
        "cd P",
        Some("/Users/tadej"),
    )
    .into_iter()
    .map(|history_entry| history_entry.command)
    .collect_vec();

    assert_eq!(autosuggestions, vec!["cd Pictures".to_owned()]);
}

#[test]
fn test_find_autosuggestion_from_history_with_no_pwd_and_no_working_directory() {
    let history_entries = [
        HistoryEntry::with_pwd_and_exit_code("cd Dotfiles", "/Users/tadej", 0),
        HistoryEntry::command_only("cd Documents"),
        HistoryEntry::with_pwd_and_exit_code("cd Downloads", "/Users/tadej", 0),
    ];

    // When no working directory is passed, it shouldn't consider a command with
    // no pwd to be executed in the "same" directory and prioritize it.
    let autosuggestions =
        find_potential_autosuggestions_from_history(history_entries.iter(), "cd D", None)
            .into_iter()
            .map(|history_entry| history_entry.command)
            .collect_vec();

    assert_eq!(
        autosuggestions,
        vec![
            "cd Downloads".to_owned(),
            "cd Documents".to_owned(),
            "cd Dotfiles".to_owned()
        ]
    );
}

fn test_session_context(cwd: TypedPathBuf, app: &App) -> SessionContext {
    let session = Session::new(
        SessionInfo::new_for_test(),
        Arc::new(TestCommandExecutor::default()),
    );
    app.read(|ctx| SessionContext::new(session, CommandRegistry::default().into(), cwd, ctx))
}

#[test]
fn test_feature_flag_arg_is_valid_with_no_whitespace_before_arg() {
    App::test((), |app| async move {
        let ctx = test_session_context(TypedPathBuf::from("/test/home/"), &app);

        let full_command = "cargo run --features=with_local_server,fast_dev";
        let with_local_server_arg = ParsedExpression::new(
            Expression::ValidatableArgument(vec![ArgType::Generator("feature_flags".into())]),
            ParsedToken::new("with_local_server".to_string()),
        )
        .spanned((21, 38));
        let fast_dev_arg = ParsedExpression::new(
            Expression::ValidatableArgument(vec![ArgType::Generator("feature_flags".into())]),
            ParsedToken::new("fast_dev".to_string()),
        )
        .spanned((39, 47));
        let is_valid = is_arg_valid(full_command, &with_local_server_arg, &ctx, None).await;
        assert!(is_valid);

        let is_valid = is_arg_valid(full_command, &fast_dev_arg, &ctx, None).await;
        assert!(is_valid);

        let is_valid = is_command_valid(full_command, Some(&ctx), None).await;
        assert!(is_valid);
    });
}

#[test]
fn test_normalize_ai_input_suggestion_response_restores_json_doubled_separators() {
    use warp_util::path::ShellFamily;

    // Build backslash runs of an exact length without multi-backslash literals, so
    // the expected counts are unambiguous.
    let run = |n| "\\".repeat(n);

    // AI echo of a UNC path: JSON serialization doubled every backslash. The
    // ingestion normalizer halves the uniform doubling, restoring the UNC prefix
    // and single separators in both `most_likely_action` and `commands`.
    let response = normalize_ai_input_suggestion_response(
        GenerateAIInputSuggestionsResponseV2 {
            commands: vec![format!("cat {}WSL${}Ubuntu{}file", run(8), run(4), run(4))],
            ai_queries: vec![],
            most_likely_action: format!("cat {}WSL${}Ubuntu{}file", run(8), run(4), run(4)),
        },
        ShellFamily::PowerShell,
    );
    assert_eq!(
        response.most_likely_action,
        format!("cat {}WSL${}Ubuntu{}file", run(4), run(2), run(2))
    );
    assert_eq!(
        response.commands,
        vec![format!("cat {}WSL${}Ubuntu{}file", run(4), run(2), run(2))]
    );

    // A regex literal the AI echoed as JSON-doubled (two backslashes -> four) is
    // restored to the intended two backslashes, not collapsed to one.
    let regex_response = normalize_ai_input_suggestion_response(
        GenerateAIInputSuggestionsResponseV2 {
            commands: vec![],
            ai_queries: vec![],
            most_likely_action: format!("Select-String '{}d+'", run(4)),
        },
        ShellFamily::PowerShell,
    );
    assert_eq!(
        regex_response.most_likely_action,
        format!("Select-String '{}d+'", run(2))
    );

    // POSIX-family sessions are a no-op: the response is returned unchanged.
    let posix_command = format!("cat {}server{}share", run(8), run(4));
    let posix_response = normalize_ai_input_suggestion_response(
        GenerateAIInputSuggestionsResponseV2 {
            commands: vec![posix_command.clone()],
            ai_queries: vec![],
            most_likely_action: posix_command.clone(),
        },
        ShellFamily::Posix,
    );
    assert_eq!(posix_response.most_likely_action, posix_command);
    assert_eq!(posix_response.commands, vec![posix_command]);

    // Already-correct single-separator text (history/completer shape, which is never
    // routed through this helper in production) is a no-op: no run of >= 4 means the
    // normalizer returns it borrowed and unchanged.
    let correct = format!("cat {}WSL${}Ubuntu{}file", run(2), run(1), run(1));
    let correct_response = normalize_ai_input_suggestion_response(
        GenerateAIInputSuggestionsResponseV2 {
            commands: vec![],
            ai_queries: vec![],
            most_likely_action: correct.clone(),
        },
        ShellFamily::PowerShell,
    );
    assert_eq!(correct_response.most_likely_action, correct);
}
