use std::collections::{HashMap, HashSet};
use std::iter::FromIterator;

use string_offset::ByteOffset;
use typed_path::TypedPathBuf;

use crate::completer::EngineDirEntry;
use crate::completer::{context::CompletionContext, suggest::MatchRequirement};
use crate::completer::{
    describe::OptionCaseSensitivity,
    testing::{FakeCompletionContext, MockPathCompletionContext},
};
use crate::completer::{suggest::SuggestionType, TopLevelCommandCaseSensitivity};
use crate::meta::{Span, SpannedItem};
use crate::signatures::{
    testing::{add_content_signature, create_test_command_registry, git_signature, test_signature},
    CommandRegistry,
};

use super::{describe, Description};

#[cfg(windows)]
mod windows_constants {
    pub(super) const TEST_WORK_DIR: &str = r"C:\";
}

#[cfg(windows)]
use windows_constants::*;

#[cfg(unix)]
mod unix_constants {
    pub(super) const TEST_WORK_DIR: &str = "/home/";
}

#[cfg(unix)]
use unix_constants::*;

/// Given a line and position in the line, runs the completer at the position and returns
/// a Description struct for the word at pos
fn describe_at_cursor<T: CompletionContext>(
    line: &str,
    pos: ByteOffset,
    ctx: &T,
) -> Option<Description> {
    warpui::r#async::block_on(describe(line, pos, ctx))
}

#[test]
pub fn test_describe_top_level_commands_case_sensitive() {
    let ctx = FakeCompletionContext::new(CommandRegistry::default())
        .with_case_sensitivity()
        .with_top_level_commands(vec!["git", "networkQuality"]);

    assert_eq!(
        describe_at_cursor("git", ByteOffset::from(1), &ctx).map(Description::into_token_name),
        Some("git".into())
    );

    assert!(describe_at_cursor("GIT", ByteOffset::from(1), &ctx)
        .map(Description::into_token_name)
        .is_none());

    assert!(describe_at_cursor("GIt", ByteOffset::from(1), &ctx)
        .map(Description::into_token_name)
        .is_none());

    // The `TopLevelCommandCaseSensitivity` value does not matter since we check parts other than the top-level command.
    assert_eq!(
        describe_at_cursor("git status", ByteOffset::from(4), &ctx)
            .map(Description::into_token_name),
        Some("status".into())
    );

    // There should be no descriptions for `git Status` since `Status` is not a valid
    // subcommand.
    assert!(describe_at_cursor("git Status", ByteOffset::from(4), &ctx).is_none());

    assert_eq!(
        describe_at_cursor("git status --ahead-behind", ByteOffset::from(14), &ctx)
            .map(Description::into_token_name),
        Some("--ahead-behind".into())
    );

    assert!(describe_at_cursor("git status --AHEAD-behind", ByteOffset::from(14), &ctx).is_none());

    assert_eq!(
        describe_at_cursor("networkQuality", ByteOffset::from(1), &ctx)
            .map(Description::into_token_name),
        Some("networkQuality".into())
    )
}

#[test]
pub fn test_describe_top_level_commands_case_insensitive() {
    let ctx = FakeCompletionContext::new(CommandRegistry::default())
        .with_top_level_commands(vec!["git", "networkQuality"]);

    assert_eq!(
        describe_at_cursor("git", ByteOffset::from(1), &ctx).map(Description::into_token_name),
        Some("git".into())
    );

    assert_eq!(
        describe_at_cursor("GIT", ByteOffset::from(1), &ctx).map(Description::into_token_name),
        Some("git".into())
    );

    assert_eq!(
        describe_at_cursor("GIt", ByteOffset::from(1), &ctx).map(Description::into_token_name),
        Some("git".into())
    );

    assert_eq!(
        describe_at_cursor("git status && GIT checkout", ByteOffset::from(15), &ctx)
            .map(Description::into_token_name),
        Some("git".into())
    );

    // The `TopLevelCommandCaseSensitivity` value does not matter since we check parts other than the top-level command.
    assert_eq!(
        describe_at_cursor("git status", ByteOffset::from(4), &ctx)
            .map(Description::into_token_name),
        Some("status".into())
    );

    // There should be no descriptions for `git Status` since `Status` is not a valid
    // subcommand.
    assert!(describe_at_cursor("git Status", ByteOffset::from(4), &ctx).is_none());

    assert_eq!(
        describe_at_cursor("git status --ahead-behind", ByteOffset::from(14), &ctx)
            .map(Description::into_token_name),
        Some("--ahead-behind".into())
    );

    assert!(describe_at_cursor("git status --AHEAD-behind", ByteOffset::from(14), &ctx).is_none());

    assert_eq!(
        describe_at_cursor("networkQuality", ByteOffset::from(1), &ctx)
            .map(Description::into_token_name),
        Some("networkQuality".into())
    )
}

#[test]
pub fn test_xray_describe() {
    let ctx = FakeCompletionContext::new(CommandRegistry::default())
        .with_top_level_commands(["git"])
        .with_environment_variable_names(HashSet::from(["HOME".into()]));

    let line = r"git status $(git stash) && git checkout main && $HOME";

    assert_eq!(
        describe_at_cursor(line, ByteOffset::from(1), &ctx),
        Some(Description {
            token: "git".to_string().spanned(Span::new(0, 3)),
            description_text: Some("The stupid content tracker".to_string()),
            suggestion_type: SuggestionType::Command(
                TopLevelCommandCaseSensitivity::CaseInsensitive
            ),
        })
    );
    assert_eq!(
        describe_at_cursor(line, ByteOffset::from(5), &ctx),
        Some(Description {
            token: "status".to_string().spanned(Span::new(4, 10)),
            description_text: Some("Show the working tree status".to_string()),
            suggestion_type: SuggestionType::Subcommand
        })
    );
    assert_eq!(
        describe_at_cursor(line, ByteOffset::from(13), &ctx),
        Some(Description {
            token: "git".to_string().spanned(Span::new(13, 16)),
            description_text: Some("The stupid content tracker".to_string()),
            suggestion_type: SuggestionType::Command(
                TopLevelCommandCaseSensitivity::CaseInsensitive
            ),
        })
    );
    assert_eq!(
        describe_at_cursor(line, ByteOffset::from(18), &ctx),
        Some(Description {
            token: "stash".to_string().spanned(Span::new(17, 22)),
            description_text: Some("Temporarily stores all the modified tracked files".to_string()),
            suggestion_type: SuggestionType::Subcommand
        })
    );
    assert_eq!(
        describe_at_cursor(line, ByteOffset::from(28), &ctx),
        Some(Description {
            token: "git".to_string().spanned(Span::new(27, 30)),
            description_text: Some("The stupid content tracker".to_string()),
            suggestion_type: SuggestionType::Command(
                TopLevelCommandCaseSensitivity::CaseInsensitive
            ),
        })
    );
    assert_eq!(
        describe_at_cursor(line, ByteOffset::from(33), &ctx),
        Some(Description {
            token: "checkout".to_string().spanned(Span::new(31, 39)),
            description_text: Some("Switch branches or restore working tree files".to_string()),
            suggestion_type: SuggestionType::Subcommand
        },)
    );
    assert_eq!(
        describe_at_cursor(line, ByteOffset::from(50), &ctx),
        Some(Description {
            token: "$HOME".to_string().spanned(Span::new(48, 53)),
            description_text: None,
            suggestion_type: SuggestionType::Variable
        },)
    );
    assert!(describe_at_cursor(line, ByteOffset::from(25), &ctx).is_none());
}

#[test]
pub fn test_xray_describe_with_flags() {
    let ctx = FakeCompletionContext::new(CommandRegistry::default());

    let mut line = r"git commit -am";
    assert_eq!(
        describe_at_cursor(line, ByteOffset::from(11), &ctx),
        Some(Description {
            token: "-am".to_string().spanned(Span::new(11, 14)),
            description_text: Some("Use the given message as the commit message".to_string()),
            suggestion_type: SuggestionType::Option(
                MatchRequirement::EntireName,
                OptionCaseSensitivity::CaseSensitive
            )
        })
    );

    line = "git commit -a";
    assert_eq!(
        describe_at_cursor(line, ByteOffset::from(11), &ctx),
        Some(Description {
            token: "-a".to_string().spanned(Span::new(11, 13)),
            description_text: Some("Stage all modified and deleted paths".to_string()),
            suggestion_type: SuggestionType::Option(
                MatchRequirement::EntireName,
                OptionCaseSensitivity::CaseSensitive
            )
        })
    );

    line = "git commit --all";
    assert_eq!(
        describe_at_cursor(line, ByteOffset::from(11), &ctx),
        Some(Description {
            token: "--all".to_string().spanned(Span::new(11, 16)),
            description_text: Some("Stage all modified and deleted paths".to_string()),
            suggestion_type: SuggestionType::Option(
                MatchRequirement::EntireName,
                OptionCaseSensitivity::CaseSensitive
            )
        })
    );

    line = "git commit --All";
    assert_eq!(describe_at_cursor(line, ByteOffset::from(11), &ctx), None);
}

#[test]
pub fn test_xray_describe_with_directories() {
    let pwd = TypedPathBuf::from(TEST_WORK_DIR);
    let path_ctx = MockPathCompletionContext::new(pwd.clone())
        .with_entries_in_pwd([
            EngineDirEntry::test_dir("foo"),
            EngineDirEntry::test_file("foobar"),
        ])
        .with_entries(pwd.join("foo/"), [EngineDirEntry::test_dir("src")])
        .with_entries(pwd.join("foo/src/"), [EngineDirEntry::test_file("bar")]);

    let ctx = FakeCompletionContext::new(CommandRegistry::default())
        .with_path_completion_context(path_ctx);

    let mut line = r"ls foo/ && cd foo/src";

    assert_eq!(
        describe_at_cursor(line, ByteOffset::from(5), &ctx),
        Some(Description {
            token: "foo/".to_string().spanned(Span::new(3, 7)),
            description_text: Some("Directory".to_string()),
            suggestion_type: SuggestionType::Argument
        })
    );

    assert_eq!(
        describe_at_cursor(line, ByteOffset::from(17), &ctx),
        Some(Description {
            token: "foo/src".to_string().spanned(Span::new(14, 21)),
            description_text: Some("Directory".to_string()),
            suggestion_type: SuggestionType::Argument,
        })
    );

    line = r"cat foo/src/bar && cat foobar";

    assert_eq!(
        describe_at_cursor(line, ByteOffset::from(5), &ctx),
        Some(Description {
            token: "foo/src/bar".to_string().spanned(Span::new(4, 15)),
            description_text: Some("File".to_string()),
            suggestion_type: SuggestionType::Argument
        })
    );

    assert_eq!(
        describe_at_cursor(line, ByteOffset::from(25), &ctx),
        Some(Description {
            token: "foobar".to_string().spanned(Span::new(23, 29)),
            description_text: Some("File".to_string()),
            suggestion_type: SuggestionType::Argument,
        })
    );
}

/// Regression test for linear issues WAR-4244 and WAR-4245
#[test]
pub fn test_xray_describe_with_non_ascii_chars() {
    let registry = create_test_command_registry([git_signature()]);
    let ctx = FakeCompletionContext::new(registry);

    let line = r"漢字 && git checkout 漢字";

    assert_eq!(
        describe_at_cursor(line, ByteOffset::from(24), &ctx),
        Some(Description {
            token: "漢字".to_string().spanned(Span::new(23, 29)),
            description_text: None,
            suggestion_type: SuggestionType::Argument,
        })
    );
}

#[test]
pub fn test_xray_describe_single_char_line() {
    let aliases = HashMap::from_iter([("g".into(), "git".into())]);
    let ctx = FakeCompletionContext::new(CommandRegistry::default())
        .with_aliases(aliases.clone())
        .with_top_level_commands(aliases.into_keys());

    let line = "g";

    assert_eq!(
        describe_at_cursor(line, ByteOffset::from(0), &ctx),
        Some(Description {
            token: "g".to_string().spanned(Span::new(0, 1)),
            description_text: Some("Alias for \"git\"".to_string()),
            suggestion_type: SuggestionType::Command(TopLevelCommandCaseSensitivity::CaseSensitive),
        })
    );
}

#[test]
pub fn test_xray_describe_ndots() {
    let aliases = HashMap::from_iter([("...".into(), "cd ../../".into())]);
    let ctx = FakeCompletionContext::new(CommandRegistry::default())
        .with_aliases(aliases.clone())
        .with_top_level_commands(aliases.into_keys());

    let line = "...";

    assert_eq!(
        describe_at_cursor(line, ByteOffset::from(0), &ctx),
        Some(Description {
            token: "...".to_string().spanned(Span::new(0, 3)),
            description_text: Some("Alias for \"cd ../../\"".to_string()),
            suggestion_type: SuggestionType::Command(TopLevelCommandCaseSensitivity::CaseSensitive),
        })
    );
}

#[test]
pub fn test_xray_describe_functions() {
    let functions = HashSet::from_iter(["foo".into()]);
    let ctx = FakeCompletionContext::new(CommandRegistry::default())
        .with_functions(functions.clone())
        .with_top_level_commands(functions);

    let line = "foo";

    assert_eq!(
        describe_at_cursor(line, ByteOffset::from(0), &ctx),
        Some(Description {
            token: "foo".to_string().spanned(Span::new(0, 3)),
            description_text: Some("Shell function".to_string()),
            suggestion_type: SuggestionType::Command(TopLevelCommandCaseSensitivity::CaseSensitive),
        })
    );
}

#[test]
pub fn test_xray_describe_builtins() {
    let builtins = HashSet::from_iter(["exit".into()]);
    let ctx = FakeCompletionContext::new(CommandRegistry::default())
        .with_builtins(builtins.clone())
        .with_top_level_commands(builtins);

    let line = "exit";

    assert_eq!(
        describe_at_cursor(line, ByteOffset::from(0), &ctx),
        Some(Description {
            token: "exit".to_string().spanned(Span::new(0, 4)),
            description_text: Some("Shell builtin".to_string()),
            suggestion_type: SuggestionType::Command(TopLevelCommandCaseSensitivity::CaseSensitive),
        })
    );
}

#[test]
pub fn test_xray_describe_abbreviations() {
    let abbrs = HashMap::from_iter([("ga".into(), "git add".into())]);
    let ctx = FakeCompletionContext::new(CommandRegistry::default())
        .with_abbreviations(abbrs.clone())
        .with_top_level_commands(abbrs.into_keys());

    let line = "ga";

    assert_eq!(
        describe_at_cursor(line, ByteOffset::from(0), &ctx),
        Some(Description {
            token: "ga".to_string().spanned(Span::new(0, 2)),
            description_text: Some("Abbreviation for \"git add\"".to_string()),
            suggestion_type: SuggestionType::Command(TopLevelCommandCaseSensitivity::CaseSensitive),
        })
    );
}

#[test]
pub fn test_xray_describe_flag_with_equal_sign() {
    let registry = create_test_command_registry([test_signature()]);
    let ctx = FakeCompletionContext::new(registry);

    assert_eq!(
        describe_at_cursor("test --long=foo", ByteOffset::from(7), &ctx),
        Some(Description {
            token: "--long".to_string().spanned(Span::new(5, 11)),
            description_text: None,
            suggestion_type: SuggestionType::Option(
                MatchRequirement::EntireName,
                OptionCaseSensitivity::CaseSensitive,
            ),
        })
    );

    assert_eq!(
        describe_at_cursor("test --long=", ByteOffset::from(7), &ctx),
        Some(Description {
            token: "--long".to_string().spanned(Span::new(5, 11)),
            description_text: None,
            suggestion_type: SuggestionType::Option(
                MatchRequirement::EntireName,
                OptionCaseSensitivity::CaseSensitive,
            ),
        })
    );
}

#[test]
pub fn test_describe_file_paths_with_separator_in_middle() {
    let registry = create_test_command_registry([]);
    let pwd = TypedPathBuf::from(TEST_WORK_DIR);
    let path_ctx = MockPathCompletionContext::new(pwd.clone())
        .with_entries_in_pwd([EngineDirEntry::test_dir("foo")])
        .with_entries(pwd.join("foo/"), [EngineDirEntry::test_file("script")]);
    let ctx = FakeCompletionContext::new(registry).with_path_completion_context(path_ctx.clone());

    assert_eq!(
        describe_at_cursor("foo/script", ByteOffset::from(10), &ctx),
        Some(Description {
            token: "foo/script".to_string().spanned(Span::new(0, 10)),
            description_text: Some("File".to_string()),
            suggestion_type: SuggestionType::Argument
        })
    );
}

#[test]
fn test_describe_powershell_shortened_option() {
    let registry = create_test_command_registry([add_content_signature(), git_signature()]);
    let ctx = FakeCompletionContext::new(registry);

    // "-Enc" is specific enough to match "-Encoding" but not "-Exclude"
    assert_eq!(
        describe_at_cursor("Add-Content -Enc ASCII", ByteOffset::from(16), &ctx),
        Some(Description {
            token: "-Encoding".to_string().spanned(Span::new(12, 16)),
            description_text: None,
            suggestion_type: SuggestionType::Option(
                MatchRequirement::UniquePrefixOnly,
                OptionCaseSensitivity::CaseInsensitive
            ),
        })
    );

    // "-E" is not specific enough, so it shouldn't match.
    assert_eq!(
        describe_at_cursor("Add-Content -E ASCII", ByteOffset::from(14), &ctx),
        None
    );

    // Shouldn't apply to commands which aren't PowerShell cmdlets
    assert_eq!(
        describe_at_cursor("git branch --delet", ByteOffset::from(18), &ctx),
        None
    );
}

#[test]
fn test_describe_case_insensitive_option() {
    let registry = create_test_command_registry([add_content_signature(), git_signature()]);
    let ctx = FakeCompletionContext::new(registry);

    // "-enc" is specific enough to match "-Encoding" but not "-Exclude"
    assert_eq!(
        describe_at_cursor("Add-Content -enc UTF8", ByteOffset::from(16), &ctx),
        Some(Description {
            token: "-Encoding".to_string().spanned(Span::new(12, 16)),
            description_text: None,
            suggestion_type: SuggestionType::Option(
                MatchRequirement::UniquePrefixOnly,
                OptionCaseSensitivity::CaseInsensitive
            ),
        })
    );

    // "-e" is not specific enough, so it shouldn't match.
    assert_eq!(
        describe_at_cursor("Add-Content -e ASCII", ByteOffset::from(14), &ctx),
        None
    );
}
