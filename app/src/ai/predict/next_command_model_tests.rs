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
fn test_normalize_ai_input_suggestion_response_decodes_json_doubled_backslashes() {
    use warp_util::path::ShellFamily;

    // Build backslash runs of an exact length without multi-backslash literals, so
    // the expected counts are unambiguous. `run(n)` is n literal backslashes.
    let run = |n| "\\".repeat(n);

    // The AI prompt serializes command history as JSON (`serde_json::to_string`),
    // which doubles every backslash; the model can echo that JSON-escaped form back
    // as the "plain command." The ingestion helper reverses that encoding
    // (`\\` -> `\`) - the deterministic inverse of `serde_json::to_string` - so it
    // halves every doubled run, in both `most_likely_action` and `commands`.

    // Ordinary AI-doubled path (the core APP-4893 defect): each single separator
    // (one backslash) was doubled to two by JSON serialization. Halving restores
    // single separators.
    let response = normalize_ai_input_suggestion_response(
        GenerateAIInputSuggestionsResponseV2 {
            commands: vec![format!("cd C:{}Users{}alice{}repo", run(2), run(2), run(2))],
            ai_queries: vec![],
            most_likely_action: format!("cd C:{}Users{}alice{}repo", run(2), run(2), run(2)),
        },
        ShellFamily::PowerShell,
    );
    assert_eq!(
        response.most_likely_action,
        format!("cd C:{}Users{}alice{}repo", run(1), run(1), run(1))
    );
    assert_eq!(
        response.commands,
        vec![format!("cd C:{}Users{}alice{}repo", run(1), run(1), run(1))]
    );
    // JSON string decoding also restores quotes and control characters, not just
    // backslashes. This protects commands whose arguments contain JSON escapes.
    let escaped_response = normalize_ai_input_suggestion_response(
        GenerateAIInputSuggestionsResponseV2 {
            commands: vec![r#"Write-Output "hello\nworld\t!""#.to_owned()],
            ai_queries: vec![],
            most_likely_action: r#"echo "hello""#.to_owned(),
        },
        ShellFamily::PowerShell,
    );
    assert_eq!(escaped_response.most_likely_action, r#"echo "hello""#);
    assert_eq!(
        escaped_response.commands,
        vec!["Write-Output \"hello\nworld\t!\"".to_owned()]
    );

    // AI echo of a UNC path: the two leading UNC backslashes were doubled to four
    // and each separator to two. Halving restores the two-backslash UNC prefix and
    // single separators.
    let unc_response = normalize_ai_input_suggestion_response(
        GenerateAIInputSuggestionsResponseV2 {
            commands: vec![format!("cat {}WSL${}Ubuntu{}file", run(4), run(2), run(2))],
            ai_queries: vec![],
            most_likely_action: format!("cat {}WSL${}Ubuntu{}file", run(4), run(2), run(2)),
        },
        ShellFamily::PowerShell,
    );
    assert_eq!(
        unc_response.most_likely_action,
        format!("cat {}WSL${}Ubuntu{}file", run(2), run(1), run(1))
    );

    // An intentional double backslash the user typed (e.g. a regex literal inside a
    // single-quoted PowerShell string) is preserved, not collapsed: JSON
    // serialization doubled the user's two backslashes to four, the model echoed
    // four, and the JSON decode restores exactly two. This is the deterministic
    // inverse of `serde_json::to_string` - no heuristic, so quoted literals and
    // regex doubles are never corrupted.
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

    // POSIX-family sessions are a no-op: POSIX commands use backslashes as shell
    // escapes that must not be collapsed, so the response is returned unchanged.
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

    // An already-correct ordinary path whose separators are all single backslashes
    // (no doubled runs) is a no-op: there are no `\\` pairs to halve. (History and
    // completer values are never routed through this helper in production; this
    // guards that a correct ordinary path is not disturbed if it ever is.)
    let correct = format!("cd C:{}Users{}alice", run(1), run(1));
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
