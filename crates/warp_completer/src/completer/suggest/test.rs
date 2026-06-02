use std::collections::HashMap;
use std::iter::FromIterator;

use typed_path::TypedPathBuf;

use super::{
    suggestions, CompleterOptions, CompletionsFallbackStrategy, SuggestionResults, SuggestionType,
};
use crate::completer::context::CompletionContext;
use crate::completer::engine::EngineDirEntry;
use crate::completer::matchers::MatchStrategy;
use crate::completer::testing::{
    FakeCompletionContext, MockGeneratorContext, MockPathCompletionContext,
};
use crate::meta::Span;
use crate::signatures::testing::{
    cd_signature, create_test_command_registry, fuzzy_signature, git_signature, java_signature,
    ls_signature, npm_signature, signature_with_empty_positional, test_signature,
};
use crate::signatures::CommandRegistry;

cfg_if::cfg_if! {
    if #[cfg(not(feature = "v2"))] {
        use std::collections::HashSet;
        use crate::signatures::testing::add_content_signature;
    }
}

#[cfg(windows)]
mod windows_constants {
    pub(super) const TEST_WORK_DIR: &str = r"C:\Users\";
}

#[cfg(windows)]
use windows_constants::*;

#[cfg(unix)]
mod unix_constants {
    pub(super) const TEST_WORK_DIR: &str = "/home/";
    #[allow(dead_code)]
    pub(super) const TEST_ROOT_DIR: &str = "/";
}

#[cfg(unix)]
use unix_constants::*;

/// Same API as `suggestions` but not async because this is a test :)
fn suggestions_for_test<T: CompletionContext>(
    line: &str,
    pos: usize,
    options: CompleterOptions,
    ctx: &T,
) -> Option<SuggestionResults> {
    warpui_core::r#async::block_on(suggestions(line, pos, None, options, ctx))
}

/// Runs the completer at the end of the given line and returns the associated
/// suggestions, NOT including SuggestionType::Option (flags) by default. If
/// you want to include flags in the result, use complete_at_end_of_line_with_options
fn complete_at_end_of_line<T: CompletionContext>(line: &str, ctx: &T) -> Vec<String> {
    suggestions_for_test(
        line,
        line.len(),
        CompleterOptions {
            match_strategy: MatchStrategy::CaseInsensitive,
            fallback_strategy: CompletionsFallbackStrategy::FilePaths,
            suggest_file_path_completions_only: false,
            parse_quotes_as_literals: false,
        },
        ctx,
    )
    .into_iter()
    .flat_map(|res| res.suggestions)
    .filter_map(|s| match s.suggestion_type() {
        SuggestionType::Option(..) => None,
        _ => Some(s.suggestion.display.to_string()),
    })
    .collect()
}

/// Runs the completer at the end of the given line with suggest_file_path_completions_only true,
/// and returns the associated suggestions, NOT including SuggestionType::Option (flags) by default.
fn complete_at_end_of_line_file_path_only<T: CompletionContext>(
    line: &str,
    ctx: &T,
) -> Vec<String> {
    suggestions_for_test(
        line,
        line.len(),
        CompleterOptions {
            match_strategy: MatchStrategy::CaseInsensitive,
            fallback_strategy: CompletionsFallbackStrategy::FilePaths,
            suggest_file_path_completions_only: true,
            parse_quotes_as_literals: false,
        },
        ctx,
    )
    .into_iter()
    .flat_map(|res| res.suggestions)
    .filter_map(|s| match s.suggestion_type() {
        SuggestionType::Option(..) => None,
        _ => Some(s.suggestion.display.to_string()),
    })
    .collect()
}

// Runs the completer and then uses the query to filter down the results.
// Note that we need this as a helper test function to test some specific
// ordering/matching logic -- ideally this will eventually be baked into
// `suggestions` itself eventually.
fn complete_at_end_of_line_with_query<T: CompletionContext>(
    line: &str,
    query: &str,
    match_strategy: MatchStrategy,
    ctx: &T,
) -> Vec<String> {
    suggestions_for_test(
        line,
        line.len(),
        CompleterOptions {
            match_strategy,
            fallback_strategy: CompletionsFallbackStrategy::FilePaths,
            suggest_file_path_completions_only: false,
            parse_quotes_as_literals: false,
        },
        ctx,
    )
    .expect("suggestion results should be some")
    .filter_by_query(query, &['/'])
    .map(|s| s.suggestion.display.to_string())
    .collect()
}

/// Returns a vector of tuples each of which consists of the display string
/// for a matched suggestion and the matched char indices of that string
fn get_filtered_suggestions_with_query<'a, T: CompletionContext>(
    line: &'a str,
    query: &'a str,
    match_strategy: MatchStrategy,
    ctx: &'a T,
) -> Vec<(Vec<usize>, String)> {
    suggestions_for_test(
        line,
        line.len(),
        CompleterOptions {
            match_strategy,
            fallback_strategy: CompletionsFallbackStrategy::FilePaths,
            suggest_file_path_completions_only: false,
            parse_quotes_as_literals: false,
        },
        ctx,
    )
    .expect("suggestion results should be some")
    .filter_by_query(query, &['/'])
    .map(|s| (s.matching_indices, s.suggestion.display.to_string()))
    .collect()
}

fn complete_at_end_of_line_with_options<T: CompletionContext>(
    line: &str,
    match_strategy: MatchStrategy,
    ctx: &T,
) -> Vec<String> {
    suggestions_for_test(
        line,
        line.len(),
        CompleterOptions {
            match_strategy,
            fallback_strategy: CompletionsFallbackStrategy::FilePaths,
            suggest_file_path_completions_only: false,
            parse_quotes_as_literals: false,
        },
        ctx,
    )
    .into_iter()
    .flat_map(|res| res.suggestions)
    .map(|s| s.suggestion.display.to_string())
    .collect()
}

fn complete_at_cursor_position<T: CompletionContext>(
    line: &str,
    pos: usize,
    ctx: &T,
) -> Vec<String> {
    suggestions_for_test(
        line,
        pos,
        CompleterOptions {
            match_strategy: MatchStrategy::CaseInsensitive,
            fallback_strategy: CompletionsFallbackStrategy::FilePaths,
            suggest_file_path_completions_only: false,
            parse_quotes_as_literals: false,
        },
        ctx,
    )
    .into_iter()
    .flat_map(|res| res.suggestions)
    .map(|s| s.suggestion.display.to_string())
    .collect()
}

fn complete_replacement_span<T: CompletionContext>(line: &str, ctx: &T) -> Option<Span> {
    suggestions_for_test(
        line,
        line.len(),
        CompleterOptions {
            match_strategy: MatchStrategy::CaseInsensitive,
            fallback_strategy: CompletionsFallbackStrategy::FilePaths,
            suggest_file_path_completions_only: false,
            parse_quotes_as_literals: false,
        },
        ctx,
    )
    .map(|results| results.replacement_span)
}

fn complete_replacement_span_at_cursor_pos<T: CompletionContext>(
    line: &str,
    pos: usize,
    ctx: &T,
) -> Option<Span> {
    suggestions_for_test(
        line,
        pos,
        CompleterOptions {
            match_strategy: MatchStrategy::CaseInsensitive,
            fallback_strategy: CompletionsFallbackStrategy::FilePaths,
            suggest_file_path_completions_only: false,
            parse_quotes_as_literals: false,
        },
        ctx,
    )
    .map(|results| results.replacement_span)
}

#[test]
pub fn test_top_level_command_completions() {
    let registry = create_test_command_registry([]);
    let ctx = FakeCompletionContext::new(registry).with_top_level_commands(["git", "cd", "cargo"]);

    assert_eq!(complete_at_end_of_line("c", &ctx), vec!["cargo", "cd"]);
    assert_eq!(complete_at_end_of_line("g", &ctx), vec!["git"]);
}

#[test]
pub fn test_command_completions_requires_nonwhitespace() {
    let registry = create_test_command_registry([]);
    let ctx = FakeCompletionContext::new(registry).with_top_level_commands(["git", "cd", "cargo"]);

    // Empty strings produce no completions.
    assert!(complete_at_end_of_line("", &ctx).is_empty());
    // Whitespace (without any other characters) produces no completions.
    assert!(complete_at_end_of_line("\t\n\t", &ctx).is_empty());
}

#[test]
pub fn test_files_includes_directories() {
    let path_ctx = MockPathCompletionContext::default().with_entries_in_pwd([
        EngineDirEntry::test_dir("foo"),
        EngineDirEntry::test_file("foobar"),
    ]);
    let ctx = FakeCompletionContext::new(CommandRegistry::default())
        .with_path_completion_context(path_ctx);

    // Even though cat only accepts file, we also suggest directories since the user could
    // be trying to cat a nested file within the directory.
    assert_eq!(
        complete_at_end_of_line("cat ", &ctx),
        vec!["foo/", "foobar"],
    );
}

#[test]
pub fn test_completes_paths_with_space() {
    let registry = create_test_command_registry([test_signature(), cd_signature()]);

    let pwd = TypedPathBuf::from(TEST_WORK_DIR);

    let path_ctx = MockPathCompletionContext::new(pwd.clone())
        .with_entries_in_pwd([EngineDirEntry::test_dir("foo bar bazz")])
        .with_entries(
            pwd.join("foo bar bazz/"),
            [EngineDirEntry::test_dir("test")],
        );

    let ctx = FakeCompletionContext::new(registry)
        .with_supports_autocd(true)
        .with_top_level_commands(["cd", "test"])
        .with_path_completion_context(path_ctx);

    assert_eq!(
        complete_at_end_of_line(r"cd foo\ bar\ bazz/", &ctx),
        vec!["test/"],
    );

    // Path completions for top-level commands should also properly support directories with
    // spaces.
    assert_eq!(
        complete_at_end_of_line(r"foo\ bar\ bazz/", &ctx),
        vec!["test/"],
    );

    assert_eq!(
        complete_at_end_of_line(r"./foo\ bar\ bazz/", &ctx),
        vec!["test/"],
    );

    assert_eq!(
        complete_at_end_of_line("test --template-args-for-opt foo\\ bar\\ bazz/", &ctx),
        vec!["test/"],
    );
}

#[cfg_attr(
    windows,
    ignore = "CORE-3696: path sorting comparison function needs separators"
)]
#[test]
pub fn test_completes_dotfiles() {
    let pwd = TypedPathBuf::from(TEST_WORK_DIR);

    let path_ctx = MockPathCompletionContext::new(pwd.clone())
        .with_entries_in_pwd([
            EngineDirEntry::test_file(".foo"),
            EngineDirEntry::test_file("foobar"),
            EngineDirEntry::test_dir("foo"),
            EngineDirEntry::test_dir(".bar"),
        ])
        .with_entries(
            pwd.join("foo/"),
            [
                EngineDirEntry::test_file(".hidden"),
                EngineDirEntry::test_dir("src"),
            ],
        )
        .with_entries(pwd.join("foo/src/"), [EngineDirEntry::test_file(".hidden")]);

    let ctx = FakeCompletionContext::new(create_test_command_registry([
        cd_signature(),
        test_signature(),
    ]))
    .with_supports_autocd(true)
    .with_top_level_commands(["cd", "test"])
    .with_path_completion_context(path_ctx);

    assert_eq!(
        complete_at_end_of_line_with_query("cd ", "", MatchStrategy::CaseInsensitive, &ctx),
        vec!["foo/"],
    );

    assert_eq!(
        complete_at_end_of_line("cd .", &ctx),
        vec!["./", "../", ".bar/"],
    );

    // Dotfiles should not be included if the path does not start with a dot.
    assert_eq!(
        complete_at_end_of_line("cat ", &ctx),
        vec!["foo/", "foobar"],
    );

    assert_eq!(
        complete_at_end_of_line("cat .", &ctx),
        vec!["./", "../", ".bar/", ".foo"],
    );

    assert_eq!(
        complete_at_end_of_line("cat f", &ctx),
        vec!["foo/", "foobar"]
    );
    assert_eq!(complete_at_end_of_line("cat foo/", &ctx), vec!["src/"]);
    assert_eq!(
        complete_at_end_of_line("cat foo/.", &ctx),
        vec!["./", "../", ".hidden"]
    );

    assert_eq!(
        complete_at_end_of_line("cat foo/src/.", &ctx),
        vec!["./", "../", ".hidden"]
    );
}

#[test]
pub fn test_cursor_in_middle_of_line() {
    let path_ctx = MockPathCompletionContext::new(TypedPathBuf::from(TEST_WORK_DIR))
        .with_entries_in_pwd([EngineDirEntry::test_dir("foo")]);
    let ctx = FakeCompletionContext::new(CommandRegistry::default())
        .with_top_level_commands(["cargo", "cd"])
        .with_path_completion_context(path_ctx);

    let suggestions: Vec<String> = suggestions_for_test(
        "cd ~/",
        3,
        CompleterOptions {
            match_strategy: MatchStrategy::CaseInsensitive,
            fallback_strategy: CompletionsFallbackStrategy::FilePaths,
            suggest_file_path_completions_only: false,
            parse_quotes_as_literals: false,
        },
        &ctx,
    )
    .expect("suggestions should be some")
    .filter_by_query("", &['/'])
    .map(|s| s.suggestion.display.to_string())
    .collect();
    assert_eq!(suggestions, vec!["foo/"]);
    assert_eq!(
        complete_replacement_span_at_cursor_pos("cd ~/", 3, &ctx),
        Some(Span::new(3, 3))
    );

    assert_eq!(
        complete_at_cursor_position("cd ~/", 1, &ctx),
        vec!["cargo", "cd"]
    );

    assert_eq!(
        complete_replacement_span_at_cursor_pos("cd ~/", 1, &ctx),
        Some(Span::new(0, 1))
    );
}

#[test]
pub fn test_cd_from_home_dir() {
    let home_directory = TypedPathBuf::from(TEST_WORK_DIR);
    let pwd = home_directory.join("foo/");
    let path_ctx = MockPathCompletionContext::new(pwd)
        .with_home_directory(home_directory.to_string_lossy().to_string())
        .with_entries(
            home_directory,
            [
                EngineDirEntry::test_file("bar"),
                EngineDirEntry::test_dir("foo"),
                EngineDirEntry::test_dir("baz"),
            ],
        );
    let ctx = FakeCompletionContext::new(create_test_command_registry([cd_signature()]))
        .with_path_completion_context(path_ctx);

    assert_eq!(complete_at_end_of_line("cd ~/", &ctx), vec!["baz/", "foo/"]);
}

#[test]
pub fn test_completions_ordering_within_group() {
    let mut ctx = FakeCompletionContext::new(CommandRegistry::default());

    // TODO(completions-v2): Re-enable when embedded signatures are implemented. In the long-term we
    // should create a test signature for `chmod` instead of using real command signatures.
    cfg_if::cfg_if! {
        if #[cfg(not(feature = "v2"))] {
            let chmod_res = complete_at_end_of_line("chmod ", &ctx);
            assert_eq!(chmod_res, vec!["664", "744", "777", "a+rx", "u+x"],);
        }
    }

    let path_ctx = MockPathCompletionContext::new(TypedPathBuf::from(TEST_WORK_DIR))
        .with_entries_in_pwd([
            EngineDirEntry::test_file("foo"),
            EngineDirEntry::test_file("foobar"),
            EngineDirEntry::test_dir("foobaz"),
        ]);
    ctx = ctx.with_path_completion_context(path_ctx);

    let vim_res = complete_at_end_of_line("vim ", &ctx);
    assert_eq!(vim_res, vec!["foo", "foobar", "foobaz/"],);
}

#[test]
pub fn test_completions_ordering_across_groups() {
    let registry = create_test_command_registry([git_signature()]);

    let path_ctx = MockPathCompletionContext::new(TypedPathBuf::from(TEST_WORK_DIR))
        .with_entries_in_pwd([
            EngineDirEntry::test_dir("gids"),
            EngineDirEntry::test_dir("giants"),
        ]);
    let ctx = FakeCompletionContext::new(registry)
        .with_path_completion_context(path_ctx)
        .with_supports_autocd(true)
        .with_top_level_commands(vec!["gits", "git"]);

    let gi_prefix_results = complete_at_end_of_line("gi", &ctx);

    // commands before arguments/file paths (and each group should be sorted itself)
    assert_eq!(gi_prefix_results, vec!["git", "gits", "giants/", "gids/"],);
}

#[test]
pub fn test_file_path_completions_only() {
    let registry = create_test_command_registry([git_signature()]);

    let path_ctx = MockPathCompletionContext::new(TypedPathBuf::from(TEST_WORK_DIR))
        .with_entries_in_pwd([
            EngineDirEntry::test_dir("gids"),
            EngineDirEntry::test_dir("giants"),
        ]);
    let ctx = FakeCompletionContext::new(registry)
        .with_path_completion_context(path_ctx)
        .with_supports_autocd(false)
        .with_top_level_commands(vec!["gits", "git"]);

    let gi_prefix_results = complete_at_end_of_line_file_path_only("gi", &ctx);

    // Only file paths should exist, no commands
    assert_eq!(gi_prefix_results, vec!["giants/", "gids/"]);
}

#[test]
pub fn test_file_path_completions_only_trailing_whitespace() {
    let registry = create_test_command_registry([git_signature()]);

    let path_ctx = MockPathCompletionContext::new(TypedPathBuf::from(TEST_WORK_DIR))
        .with_entries_in_pwd([
            EngineDirEntry::test_dir("gids"),
            EngineDirEntry::test_dir("giants"),
        ]);
    let ctx = FakeCompletionContext::new(registry)
        .with_path_completion_context(path_ctx)
        .with_supports_autocd(false)
        .with_top_level_commands(vec!["gits", "git"]);

    let gi_prefix_results = complete_at_end_of_line_file_path_only("find me files in ", &ctx);

    // Only file paths should exist, no commands
    assert_eq!(gi_prefix_results, vec!["giants/", "gids/"]);
}

#[test]
pub fn test_named_flags() {
    let registry = create_test_command_registry([test_signature()]);
    let ctx = FakeCompletionContext::new(registry);

    // ensure all the named flags are appearing (ordering doesn't matter here)
    let mut results =
        complete_at_end_of_line_with_options("test --", MatchStrategy::CaseInsensitive, &ctx);
    results.sort();
    assert_eq!(
        results,
        vec![
            "--long",
            "--not-long",
            "--required-and-optional-args",
            "--required-args",
            "--required-args-with-var",
            "--template-args-for-opt"
        ]
    );

    assert!(complete_at_end_of_line("test --long", &ctx).is_empty());

    assert_eq!(
        complete_at_end_of_line("test --long ", &ctx),
        vec!["long-one", "long-two"]
    );

    assert_eq!(
        complete_at_end_of_line("test --long long", &ctx),
        vec!["long-one", "long-two"]
    );
}

#[test]
pub fn test_variadic_flags() {
    let registry = create_test_command_registry([test_signature()]);
    let ctx = FakeCompletionContext::new(registry);

    // ensure all the named flags are appearing (ordering doesn't matter here)
    let mut results =
        complete_at_end_of_line_with_options("test --", MatchStrategy::CaseInsensitive, &ctx);
    results.sort();
    assert_eq!(
        results,
        vec![
            "--long",
            "--not-long",
            "--required-and-optional-args",
            "--required-args",
            "--required-args-with-var",
            "--template-args-for-opt"
        ]
    );

    assert_eq!(
        complete_at_end_of_line("test --long ", &ctx),
        vec!["long-one", "long-two"]
    );

    assert_eq!(
        complete_at_end_of_line("test --long long-one --long ", &ctx),
        vec!["long-one", "long-two"]
    );

    assert_eq!(
        complete_at_end_of_line("test --long long-one --not-long ", &ctx),
        vec!["not-long-one", "not-long-two"]
    );

    assert_eq!(
        complete_at_end_of_line("test --long long-one ", &ctx),
        vec!["long-one", "long-two"]
    );

    assert_eq!(
        complete_at_end_of_line("test --not-long not-long-one --long ", &ctx),
        vec!["long-one", "long-two"]
    );

    assert_eq!(
        complete_at_end_of_line("test --not-long not-long-one --long long-one ", &ctx),
        vec!["long-one", "long-two"]
    );
}

// Tests that commands with multiple positional args complete with the correct results.
#[test]
pub fn test_positional_args() {
    let registry = create_test_command_registry([test_signature()]);
    let ctx = FakeCompletionContext::new(registry);

    assert_eq!(
        complete_at_end_of_line("test one ", &ctx),
        vec!["one-one", "one-two"]
    );

    assert_eq!(
        complete_at_end_of_line("test one one", &ctx),
        vec!["one-one", "one-two"]
    );

    assert_eq!(
        complete_at_end_of_line("test one one-one ", &ctx),
        vec!["two-one", "two-two"]
    );

    assert_eq!(
        complete_at_end_of_line("test one one-one two", &ctx),
        vec!["two-one", "two-two"]
    );

    // The "three" subcommand has the positionals. The first and second are required while
    // the third is optional.
    assert_eq!(
        complete_at_end_of_line("test three three-one three-two ", &ctx),
        vec!["three-three"]
    );

    // Completions should still show even if part of the argument is already present.
    assert_eq!(
        complete_at_end_of_line("test three three-one three-two three-", &ctx),
        vec!["three-three"]
    );

    assert!(
        complete_at_end_of_line("test three three-one three-two three-three ", &ctx).is_empty()
    );
}

#[cfg(not(feature = "v2"))]
#[test]
pub fn test_command_alias() {
    let registry = create_test_command_registry([test_signature()]);

    let generator_ctx = MockGeneratorContext::for_test_signature();
    let ctx = FakeCompletionContext::new(registry).with_generator_context(generator_ctx);

    // The test signature has an alias function, which expands subcommand "twelve" to "one".
    assert_eq!(
        complete_at_end_of_line("test twelve ", &ctx),
        vec!["one-one", "one-two"]
    );

    // The test signature has an alias function, which expands subcommand "nine" to "twelve".
    assert_eq!(
        complete_at_end_of_line("test nine ", &ctx),
        vec!["one-one", "one-two"]
    );

    // The test signature has an alias function, which expands subcommand "loop1" to "loop2", which
    // itself is an alias that expands back to "loop1".
    assert!(complete_at_end_of_line("test loop1 ", &ctx).is_empty());
}

#[test]
pub fn test_command_completions_includes_abbreviations() {
    let registry = create_test_command_registry([test_signature()]);
    let ctx = FakeCompletionContext::new(registry)
        .with_abbreviations(HashMap::from_iter([("gl".into(), "git log".into())]))
        .with_top_level_commands(vec!["git", "gl"]);

    let line = "g";
    let suggestion = suggestions_for_test(
        line,
        line.len(),
        CompleterOptions {
            match_strategy: MatchStrategy::CaseInsensitive,
            fallback_strategy: CompletionsFallbackStrategy::FilePaths,
            suggest_file_path_completions_only: false,
            parse_quotes_as_literals: false,
        },
        &ctx,
    )
    .into_iter()
    .flat_map(|res| res.suggestions)
    .find(|s| s.display() == "gl");

    let suggestion = suggestion.unwrap();
    // Assert that a suggestion was created for the abbreviation.
    assert_eq!(suggestion.display(), "gl");
    // The replacement span should be the expanded form of the abbreviation.
    assert_eq!(suggestion.replacement(), "git log");
}

#[test]
fn test_replacement_span() {
    let registry = create_test_command_registry([test_signature()]);
    let ctx = FakeCompletionContext::new(registry);

    assert_eq!(
        complete_replacement_span("test two ", &ctx),
        Some(Span::new(9, 9))
    );
    assert_eq!(
        complete_replacement_span("test two two-one ", &ctx),
        Some(Span::new(17, 17))
    );
    assert_eq!(
        complete_replacement_span("test four ", &ctx),
        Some(Span::new(10, 10))
    );
    assert_eq!(
        complete_replacement_span("test four four-one ", &ctx),
        Some(Span::new(19, 19))
    );
    assert_eq!(
        complete_replacement_span("test four four-one four-", &ctx),
        Some(Span::new(19, 24))
    );
    assert_eq!(
        complete_replacement_span("test four four-one four-two  ", &ctx),
        Some(Span::new(29, 29))
    );
    assert_eq!(
        complete_replacement_span("test four four-one four-two four-two four-", &ctx),
        Some(Span::new(37, 42))
    );
}

/// Verifies that we show completions for top level commands if we are completing on a command that
/// has an argument where `is_command` is true.
#[cfg(not(feature = "v2"))]
#[test]
pub fn test_completion_results_shows_top_level_commands_for_is_command_argument() {
    let registry = CommandRegistry::default();
    let ctx =
        FakeCompletionContext::new(registry).with_top_level_commands(["git", "sudo", "command"]);

    assert_eq!(
        complete_at_end_of_line("sudo ", &ctx),
        vec!["command", "git", "sudo"]
    );

    assert_eq!(
        complete_at_end_of_line("ENV=FOO sudo ", &ctx),
        vec!["command", "git", "sudo"]
    );

    // Multiple commands that have arguments that are top-level commands (in this case `sudo` and
    // `command`) should continue to surface top level commands.
    assert_eq!(
        complete_at_end_of_line("sudo command ", &ctx),
        vec!["command", "git", "sudo"]
    );
}

/// Verifies that we only surface a commands's static list of arguments (instead of all top level commands)
/// if a command's argument is marked as `is_command: true` but also has a list of static `ArgumentTypes`.
#[cfg(not(feature = "v2"))]
#[test]
pub fn test_completion_results_shows_arguments_for_is_command_argument() {
    let registry = CommandRegistry::default();
    registry.register_signature(test_signature());
    registry.register_signature(git_signature());
    let ctx =
        FakeCompletionContext::new(registry).with_top_level_commands(["git", "sudo", "command"]);

    // The `test nine` signature has a single argument that is marked as `is_command: true` but also has a single static
    // argument of "git". In this case we should only show "git", instead of all possible top level commands.
    assert_eq!(
        complete_at_end_of_line("sudo test nine git", &ctx),
        vec!["git"]
    );

    // We should still surface completions for git (because the `test nine` signature is marked with `is_command: true`)
    assert_eq!(
        complete_at_end_of_line("sudo test nine git ", &ctx),
        vec!["add", "branch", "checkout", "clone"]
    );
}

/// Verifies that we show subcommand / argument completions for an argument that is actually a top
/// level command.
/// e.g. `sudo git ` should surface completions for _git_.
#[cfg(not(feature = "v2"))]
#[test]
pub fn test_completion_results_for_is_command_argument() {
    // Create the normal command registry with our custom `git` signature.
    let registry = CommandRegistry::default();
    registry.register_signature(git_signature());

    let ctx = FakeCompletionContext::new(registry).with_top_level_commands(["git", "sudo"]);

    assert_eq!(
        complete_at_end_of_line("sudo git ", &ctx),
        vec!["add", "branch", "checkout", "clone"]
    );

    assert_eq!(
        complete_at_end_of_line("sudo git c", &ctx),
        vec!["checkout", "clone"]
    );

    // Completions for subcommands of the new top-level command should also work.
    assert_eq!(
        complete_at_end_of_line("sudo git checkout ", &ctx),
        vec!["漢字", "bob/卡b卡"]
    );
}

#[test]
pub fn test_variadic_args() {
    let registry = create_test_command_registry([test_signature()]);
    let ctx = FakeCompletionContext::new(registry);

    assert_eq!(complete_at_end_of_line("test two ", &ctx), vec!["two-one"]);

    // Even though the first positional is repeating, because there's another positional
    // after this one, we complete on that.
    assert_eq!(
        complete_at_end_of_line("test two two-one ", &ctx),
        vec!["two-two"]
    );

    assert_eq!(
        complete_at_end_of_line("test four ", &ctx),
        vec!["four-one"]
    );

    assert_eq!(
        complete_at_end_of_line("test four four-one ", &ctx),
        vec!["four-two"]
    );

    assert_eq!(
        complete_at_end_of_line("test four four-one four-", &ctx),
        vec!["four-two"]
    );

    assert_eq!(
        complete_at_end_of_line("test four four-one four-two  ", &ctx),
        vec!["four-two"]
    );

    assert_eq!(
        complete_at_end_of_line("test four four-one four-two four-two ", &ctx),
        vec!["four-two"]
    );
}

#[test]
pub fn test_variadic_args_for_option() {
    let registry = create_test_command_registry([git_signature()]);
    let ctx = FakeCompletionContext::new(registry);

    assert_eq!(
        complete_at_end_of_line("git branch --delete ", &ctx),
        vec!["branch-1", "second-branch"]
    );

    assert_eq!(
        complete_at_end_of_line("git branch --delete branch-1 ", &ctx),
        vec!["branch-1", "second-branch"]
    );

    assert_eq!(
        complete_at_end_of_line("git branch --delete branch-1 s", &ctx),
        vec!["second-branch"]
    );
}

#[cfg(not(feature = "v2"))]
#[test]
pub fn test_equal_sign_flag_does_not_bleed_variadic_suggestions() {
    let registry = create_test_command_registry([test_signature()]);
    let ctx = FakeCompletionContext::new(registry);

    // `--long` has variadic args ["long-one", "long-two"]. With space-delimited form,
    // additional values ARE expected after the first:
    assert_eq!(
        complete_at_end_of_line("test --long long-one ", &ctx),
        vec!["long-one", "long-two"]
    );

    // But with the '=' form, the flag is self-contained. The next token should get
    // subcommand completions, not more --long values.
    let mut eq_results = complete_at_end_of_line("test --long=long-one ", &ctx);
    eq_results.sort();
    assert_eq!(
        eq_results,
        vec!["eight", "five", "four", "nine", "one", "seven", "six", "three", "two"]
    );
}

#[cfg(not(feature = "v2"))]
#[test]
pub fn test_equal_sign_flag_completions() {
    let registry = create_test_command_registry([test_signature()]);
    let ctx = FakeCompletionContext::new(registry);

    assert_eq!(
        complete_at_end_of_line("test --long=long-o", &ctx),
        vec!["long-one"]
    );

    assert_eq!(
        complete_at_end_of_line("test --long=", &ctx),
        vec!["long-one", "long-two"]
    );

    assert_eq!(
        complete_at_end_of_line("test -r --long=", &ctx),
        vec!["long-one", "long-two"]
    );

    assert_eq!(
        complete_at_end_of_line("test -r -V --long=long-o", &ctx),
        vec!["long-one"]
    );
}

#[cfg(not(feature = "v2"))]
#[test]
pub fn test_equal_sign_flag_mixed_with_other_styles() {
    let registry = create_test_command_registry([test_signature()]);
    let ctx = FakeCompletionContext::new(registry);

    assert_eq!(
        complete_at_end_of_line("test --not-long=bar --long=", &ctx),
        vec!["long-one", "long-two"]
    );

    assert_eq!(
        complete_at_end_of_line("test --not-long bar --long=", &ctx),
        vec!["long-one", "long-two"]
    );

    assert_eq!(
        complete_at_end_of_line("test --long=foo --not-long ", &ctx),
        vec!["not-long-one", "not-long-two"]
    );

    assert_eq!(
        complete_at_end_of_line("test -r --not-long bar --long=long-o", &ctx),
        vec!["long-one"]
    );

    assert_eq!(
        complete_at_end_of_line("test -r --not-long=bar --long=", &ctx),
        vec!["long-one", "long-two"]
    );
}

#[test]
pub fn test_multiple_required_args_for_option() {
    let registry = create_test_command_registry([test_signature()]);
    let ctx = FakeCompletionContext::new(registry);

    assert_eq!(
        complete_at_end_of_line("test --required-args ", &ctx),
        vec!["arg-1-1", "arg-1-2"]
    );

    assert_eq!(
        complete_at_end_of_line("test --required-args arg-1-1 ", &ctx),
        vec!["arg-2-1", "arg-2-2"]
    );

    assert_eq!(
        complete_at_end_of_line("test --required-args arg-1-1 a", &ctx),
        vec!["arg-2-1", "arg-2-2"]
    );
}

#[test]
pub fn test_required_arg_with_variadic_arg_for_option() {
    let registry = create_test_command_registry([test_signature()]);
    let ctx = FakeCompletionContext::new(registry);

    assert_eq!(
        complete_at_end_of_line("test --required-args-with-var ", &ctx),
        vec!["arg-1", "arg-2"]
    );
}

#[test]
pub fn test_template_args_for_arg_for_option() {
    let registry = create_test_command_registry([test_signature()]);
    let pwd = TypedPathBuf::from(TEST_WORK_DIR);
    let path_ctx = MockPathCompletionContext::new(pwd.clone())
        .with_entries_in_pwd([
            EngineDirEntry::test_dir("foo"),
            EngineDirEntry::test_file("bar"),
        ])
        .with_entries(
            pwd.join("foo/"),
            [
                EngineDirEntry::test_dir("src"),
                EngineDirEntry::test_file("hidden"),
            ],
        );
    let ctx = FakeCompletionContext::new(registry).with_path_completion_context(path_ctx);

    assert_eq!(
        complete_at_end_of_line("test --template-args-for-opt ", &ctx),
        vec!["bar", "foo/"]
    );

    assert_eq!(
        complete_at_end_of_line("test --template-args-for-opt foo/", &ctx),
        vec!["hidden", "src/"]
    );
}

#[test]
pub fn test_complete_last_arg_after_non_variadic_option() {
    let registry = create_test_command_registry([ls_signature()]);
    let pwd = TypedPathBuf::from(TEST_WORK_DIR);
    let path_ctx = MockPathCompletionContext::new(pwd.clone())
        .with_entries_in_pwd([
            EngineDirEntry::test_dir("foo"),
            EngineDirEntry::test_file("bar"),
        ])
        .with_entries(
            pwd.join("foo/"),
            [
                EngineDirEntry::test_dir("src"),
                EngineDirEntry::test_file("hidden"),
            ],
        );
    let ctx = FakeCompletionContext::new(registry).with_path_completion_context(path_ctx);

    assert_eq!(
        complete_at_end_of_line("ls --color=auto ", &ctx),
        vec!["bar", "foo/"]
    );

    assert_eq!(
        complete_at_end_of_line("ls --color=auto f", &ctx),
        vec!["foo/"]
    );
}

/// TODO(CORE-646): The following two tests are ignored because they are failing. They
/// test the scenario where an option argument is optional or variadic, and so
/// it is ambiguous whether the user is trying to complete the option argument
/// or the next command argument. In this scenario, we should show suggestions
/// for both argument types.
#[ignore]
#[test]
pub fn test_complete_last_arg_after_optional_non_variadic_option() {
    let registry = create_test_command_registry([ls_signature()]);
    let pwd = TypedPathBuf::from(TEST_WORK_DIR);
    let path_ctx = MockPathCompletionContext::new(pwd.clone())
        .with_entries_in_pwd([
            EngineDirEntry::test_dir("foo"),
            EngineDirEntry::test_file("bar"),
        ])
        .with_entries(
            pwd.join("foo/"),
            [
                EngineDirEntry::test_dir("src"),
                EngineDirEntry::test_file("hidden"),
            ],
        );
    let ctx = FakeCompletionContext::new(registry).with_path_completion_context(path_ctx);

    // --color has an optional argument that must be supplied using `=`.
    // Since no `=` was supplied, we should complete on the next command
    // argument.
    assert_eq!(
        complete_at_end_of_line("ls --color ", &ctx),
        vec!["bar", "foo/"]
    );

    assert_eq!(complete_at_end_of_line("ls --color f", &ctx), vec!["foo/"]);
}

/// See above test.
#[ignore]
#[test]
pub fn test_complete_last_arg_after_variadic_option() {
    let registry = create_test_command_registry([ls_signature()]);
    let pwd = TypedPathBuf::from(TEST_WORK_DIR);
    let path_ctx = MockPathCompletionContext::new(pwd.clone())
        .with_entries_in_pwd([
            EngineDirEntry::test_dir("foo"),
            EngineDirEntry::test_file("bar"),
        ])
        .with_entries(
            pwd.join("foo/"),
            [
                EngineDirEntry::test_dir("src"),
                EngineDirEntry::test_file("hidden"),
            ],
        );
    let ctx = FakeCompletionContext::new(registry).with_path_completion_context(path_ctx);

    assert_eq!(
        complete_at_end_of_line("ls --test auto f", &ctx),
        vec!["force", "foo/"]
    );
}

/// TODO: we should fix these failing tests. These tests are currently failing
/// because we don't have a way of computing the positional index correctly
/// when nesting arguments under options.
/// See more here: https://linear.app/warpdotdev/issue/WAR-3660/fix-completions-for-arguments-under-options
#[ignore]
#[test]
pub fn test_completions_after_arguments_under_option() {
    let registry = create_test_command_registry([test_signature()]);

    let ctx = FakeCompletionContext::new(registry);

    // Required args only
    assert_eq!(
        complete_at_end_of_line("test --required-args arg-1-1 arg-2-2 ", &ctx),
        vec!["four", "one", "three", "two"]
    );

    // Required args followed by variadic arg
    assert_eq!(
        complete_at_end_of_line("test --required-args-with-var arg-1 ", &ctx),
        vec!["vararg-1", "vararg-2"]
    );

    assert_eq!(
        complete_at_end_of_line("test --required-args-with-var arg-1-1 vararg-1 ", &ctx),
        vec!["vararg-1", "vararg-2"]
    );
}

/// This suffers from the same problem above.
#[ignore]
#[test]
pub fn test_required_and_optional_args_for_option() {
    let registry = create_test_command_registry([test_signature()]);
    let ctx = FakeCompletionContext::new(registry);

    assert_eq!(
        complete_at_end_of_line("test --required-and-optional-args required-1 ", &ctx),
        vec!["four", "one", "optional-1", "optional-2", "three", "two"]
    );
}

#[test]
pub fn test_ls() {
    let pwd = TypedPathBuf::from(TEST_WORK_DIR);
    let path_ctx = MockPathCompletionContext::new(pwd.clone())
        .with_entries_in_pwd([
            EngineDirEntry::test_file("foo"),
            EngineDirEntry::test_file("foobar"),
            EngineDirEntry::test_dir("src"),
        ])
        .with_entries(pwd.join("src/"), [EngineDirEntry::test_file("buzz")]);
    let ctx = FakeCompletionContext::new(CommandRegistry::default())
        .with_path_completion_context(path_ctx);

    // "ls" with no flags should suggest files and folders
    assert_eq!(
        complete_at_end_of_line("ls ", &ctx),
        vec!["foo", "foobar", "src/"]
    );

    // "ls -l " should still suggest filenames
    assert_eq!(
        complete_at_end_of_line("ls -l ", &ctx),
        vec!["foo", "foobar", "src/"]
    );

    // Multiple flags should not affect completion results on args.
    assert_eq!(
        complete_at_end_of_line("ls -l -a ", &ctx),
        vec!["foo", "foobar", "src/"]
    );

    // Since the positional arg in ls is repeating, we should keep on suggesting filepaths
    // even if multiple args are already supplied.
    assert_eq!(
        complete_at_end_of_line("ls -l -a foo foo foo ", &ctx),
        vec!["foo", "foobar", "src/"]
    );
}

#[cfg(not(feature = "v2"))]
#[test]
pub fn test_mv() {
    let pwd = TypedPathBuf::from(TEST_WORK_DIR);
    let path_ctx = MockPathCompletionContext::new(pwd.clone())
        .with_entries_in_pwd([
            EngineDirEntry::test_file("foo"),
            EngineDirEntry::test_file("foobar"),
            EngineDirEntry::test_dir("src"),
        ])
        .with_entries(pwd.join("src/"), [EngineDirEntry::test_file("buzz")]);
    let ctx = FakeCompletionContext::new(CommandRegistry::default())
        .with_path_completion_context(path_ctx);

    assert_eq!(
        complete_at_end_of_line("mv ", &ctx),
        vec!["foo", "foobar", "src/"]
    );

    assert_eq!(
        complete_at_end_of_line("mv foo ", &ctx),
        vec!["foo", "foobar", "src/"]
    );

    assert_eq!(complete_at_end_of_line("mv foo sr", &ctx), vec!["src/"]);

    // Adding flags should still suggest filepaths correctly.
    assert_eq!(
        complete_at_end_of_line_with_options("mv -", MatchStrategy::CaseInsensitive, &ctx),
        vec!["-f", "-i", "-n", "-v"]
    );
    assert_eq!(
        complete_at_end_of_line("mv -R ", &ctx),
        vec!["foo", "foobar", "src/"]
    );
    assert_eq!(
        complete_at_end_of_line("mv -R f", &ctx),
        vec!["foo", "foobar"]
    );
    assert_eq!(
        complete_at_end_of_line("mv -R foo ", &ctx),
        vec!["foo", "foobar", "src/"]
    );
    assert_eq!(
        complete_at_end_of_line("mv -R foo src/", &ctx),
        vec!["buzz"]
    );
}

#[cfg(not(feature = "v2"))]
#[test]
pub fn test_env_var_completion() {
    let pwd = TypedPathBuf::from(TEST_WORK_DIR);
    let path_ctx = MockPathCompletionContext::new(pwd.clone())
        .with_home_directory(TEST_WORK_DIR.to_owned())
        .with_entries_in_pwd([
            EngineDirEntry::test_file("Cargo.toml"),
            EngineDirEntry::test_dir("target"),
            EngineDirEntry::test_dir("src"),
        ])
        .with_entries(pwd.join("src/"), [EngineDirEntry::test_dir("app")])
        .with_entries(pwd.join("src/app"), [EngineDirEntry::test_file("mod.rs")])
        .with_entries(
            pwd.join("target"),
            [
                EngineDirEntry::test_dir("debug"),
                EngineDirEntry::test_dir("release"),
            ],
        )
        .with_entries(
            pwd.join("target/debug"),
            [EngineDirEntry::test_file("warpui")],
        );

    let env_vars = HashSet::from_iter([("HOME".into()), ("BAR".into()), ("BAZZ".into())]);
    let ctx = FakeCompletionContext::new(CommandRegistry::default())
        .with_path_completion_context(path_ctx)
        .with_top_level_commands(["cargo", "cd", "git"])
        .with_environment_variable_names(env_vars);

    assert_eq!(
        complete_at_end_of_line("cd $", &ctx),
        vec!["$BAR", "$BAZZ", "$HOME"]
    );
    assert_eq!(
        complete_at_end_of_line("cd $B", &ctx),
        vec!["$BAR", "$BAZZ"]
    );

    assert_eq!(complete_at_end_of_line("cd $BAZ", &ctx), vec!["$BAZZ"]);

    assert!(complete_at_end_of_line("cd $BUZZ", &ctx).is_empty());

    // Completions for env vars should work for a command that is not an internal commamd.
    assert_eq!(
        complete_at_end_of_line("rustfmt $", &ctx),
        vec!["$BAR", "$BAZZ", "$HOME"]
    );
    assert_eq!(
        complete_at_end_of_line("rustfmt $B", &ctx),
        vec!["$BAR", "$BAZZ"]
    );

    assert_eq!(complete_at_end_of_line("rustfmt $BAZ", &ctx), vec!["$BAZZ"]);

    // Env vars should complete even if there is no command entered
    assert_eq!(
        complete_at_end_of_line("$", &ctx),
        vec!["$BAR", "$BAZZ", "$HOME"]
    );
    assert_eq!(complete_at_end_of_line("$B", &ctx), vec!["$BAR", "$BAZZ"]);

    assert_eq!(complete_at_end_of_line("$BAZ", &ctx), vec!["$BAZZ"]);

    assert_eq!(
        complete_at_end_of_line("cd $HOME/", &ctx),
        vec!["src/", "target/"]
    );

    assert_eq!(
        complete_at_end_of_line("$HOME/", &ctx),
        vec!["Cargo.toml", "src/", "target/"]
    );
}

#[cfg(not(feature = "v2"))]
#[test]
pub fn test_alias_completion() {
    let registry = CommandRegistry::default();
    registry.register_signature(test_signature());

    let pwd = TypedPathBuf::from(TEST_WORK_DIR);
    let path_ctx = MockPathCompletionContext::new(pwd.clone())
        .with_home_directory(TEST_WORK_DIR.to_owned())
        .with_entries_in_pwd([
            EngineDirEntry::test_file("Cargo.toml"),
            EngineDirEntry::test_dir("target"),
            EngineDirEntry::test_dir("src"),
        ])
        .with_entries(pwd.join("src/"), [EngineDirEntry::test_dir("app")])
        .with_entries(pwd.join("src/app"), [EngineDirEntry::test_file("mod.rs")])
        .with_entries(
            pwd.join("target"),
            [
                EngineDirEntry::test_dir("debug"),
                EngineDirEntry::test_dir("release"),
            ],
        )
        .with_entries(
            pwd.join("target/debug"),
            [EngineDirEntry::test_file("warpui")],
        );

    let aliases = HashMap::from_iter([
        ("first".into(), "test one".to_string()),
        ("second".into(), "first".to_string()),
        ("third".into(), "cd".to_string()),
        ("ls".into(), "ls -l".to_string()),
    ]);
    let ctx = FakeCompletionContext::new(registry)
        .with_path_completion_context(path_ctx)
        .with_aliases(aliases)
        .with_top_level_commands(["cargo", "cd", "git", "ls", "first", "second", "third"]);

    // An alias that expands to itself should not terminate and only expand once.
    assert_eq!(
        complete_at_end_of_line("ls ", &ctx),
        vec!["Cargo.toml", "src/", "target/"]
    );
}

#[test]
pub fn test_environment_variable_assignment() {
    let registry = create_test_command_registry([git_signature()]);
    let path_ctx = MockPathCompletionContext::new(TypedPathBuf::from(TEST_WORK_DIR))
        .with_home_directory(TEST_WORK_DIR.to_owned())
        .with_entries_in_pwd([EngineDirEntry::test_file("Cargo.toml")]);
    let aliases = HashMap::from_iter([("git".into(), "foo=Foo git".into())]);
    let ctx = FakeCompletionContext::new(registry)
        .with_path_completion_context(path_ctx)
        .with_aliases(aliases)
        .with_top_level_commands(["git"]);

    // Completion with environment variable assignment should work.
    assert_eq!(
        complete_at_end_of_line("foo=Foo git ", &ctx),
        vec!["add", "branch", "checkout", "clone"]
    );

    // Completion with multiple environment variable assignments should work.
    assert_eq!(
        complete_at_end_of_line("foo=Foo bar=Bar git ", &ctx),
        vec!["add", "branch", "checkout", "clone"]
    );

    // Completion with alias that expands to commands with environment variable assignment should work.
    assert_eq!(
        complete_at_end_of_line("git ", &ctx),
        vec!["add", "branch", "checkout", "clone"]
    );

    // Completion that includes environment variable assignment in the wrong position should not work.
    assert_eq!(
        complete_at_end_of_line("git foo=Foo ", &ctx),
        vec!["Cargo.toml"]
    );
}

#[test]
pub fn test_environment_variable_assignment_no_command() {
    let path_ctx = MockPathCompletionContext::new(TypedPathBuf::from(TEST_WORK_DIR))
        .with_home_directory(TEST_WORK_DIR.to_owned())
        .with_entries_in_pwd([EngineDirEntry::test_file("Cargo.toml")]);
    let ctx = FakeCompletionContext::new(CommandRegistry::default())
        .with_path_completion_context(path_ctx);

    // Completion with multiple environment variable assignments and no command
    // shouldn't result in an error.
    assert!(complete_at_end_of_line("foo=Foo bar=Bar", &ctx).is_empty());
}

#[test]
pub fn test_path_fallback() {
    let registry = create_test_command_registry([signature_with_empty_positional()]);

    let pwd = TypedPathBuf::from(TEST_WORK_DIR);
    let path_ctx = MockPathCompletionContext::new(pwd.clone())
        .with_home_directory(TEST_WORK_DIR.to_owned())
        .with_entries_in_pwd([
            EngineDirEntry::test_file("Cargo.toml"),
            EngineDirEntry::test_dir("target"),
            EngineDirEntry::test_dir("src"),
        ])
        .with_entries(pwd.join("src/"), [EngineDirEntry::test_dir("app")])
        .with_entries(pwd.join("src/app"), [EngineDirEntry::test_file("mod.rs")])
        .with_entries(
            pwd.join("target"),
            [
                EngineDirEntry::test_dir("debug"),
                EngineDirEntry::test_dir("release"),
            ],
        )
        .with_entries(
            pwd.join("target/debug"),
            [EngineDirEntry::test_file("warpui")],
        );

    let ctx = FakeCompletionContext::new(registry).with_path_completion_context(path_ctx);

    assert_eq!(
        complete_at_end_of_line("rustfmt ", &ctx),
        vec!["Cargo.toml", "src/", "target/"]
    );

    assert_eq!(
        complete_at_end_of_line("rustfmt --check ", &ctx),
        vec!["Cargo.toml", "src/", "target/"]
    );

    assert_eq!(
        complete_at_end_of_line("rustfmt --check C", &ctx),
        vec!["Cargo.toml"]
    );

    assert_eq!(
        complete_at_end_of_line("rustfmt --check C", &ctx),
        vec!["Cargo.toml"]
    );

    assert_eq!(
        complete_at_end_of_line("test-empty ", &ctx),
        vec!["Cargo.toml", "src/", "target/"]
    );

    // `test-empty` has a single positional but no completers so ensure we just fallback to paths.
    assert_eq!(complete_at_end_of_line("test-empty s", &ctx), vec!["src/"]);

    assert_eq!(
        complete_at_end_of_line("test-empty -m ", &ctx),
        vec!["Cargo.toml", "src/", "target/"]
    );

    assert_eq!(
        complete_at_end_of_line("test-empty -m s", &ctx),
        vec!["src/"]
    );
}

#[test]
pub fn test_autocd() {
    let pwd = TypedPathBuf::from(TEST_WORK_DIR);
    let path_ctx = MockPathCompletionContext::new(pwd.clone())
        .with_home_directory(TEST_WORK_DIR.to_owned())
        .with_entries_in_pwd([
            EngineDirEntry::test_file("Cargo.toml"),
            EngineDirEntry::test_dir("certs"),
            EngineDirEntry::test_dir("src"),
        ])
        .with_entries(pwd.join("src/"), [EngineDirEntry::test_dir("app")]);

    let registry = create_test_command_registry([]);
    let mut ctx = FakeCompletionContext::new(registry)
        .with_path_completion_context(path_ctx)
        .with_top_level_commands(["cargo", "cd", "git"])
        .with_supports_autocd(true);

    // Ensure that `certs` (a directory) is suggested because `autocd` is enabled, but
    // `Cargo.toml` is not because it is a file.
    assert_eq!(
        complete_at_end_of_line("c", &ctx),
        vec!["cargo", "cd", "certs/",]
    );

    assert_eq!(complete_at_end_of_line("ce", &ctx), vec!["certs/",]);

    ctx = ctx.with_supports_autocd(false);

    // `certs` should no longer be suggested because `auto_cd` is turned off.
    assert_eq!(complete_at_end_of_line("c", &ctx), vec!["cargo", "cd",]);
    assert!(complete_at_end_of_line("ce", &ctx).is_empty());
}

#[cfg(not(feature = "v2"))]
#[test]
pub fn test_completions() {
    let pwd = TypedPathBuf::from(TEST_WORK_DIR);
    let path_ctx = MockPathCompletionContext::new(pwd.clone())
        .with_home_directory(TEST_WORK_DIR.to_owned())
        .with_entries_in_pwd([
            EngineDirEntry::test_file("Cargo.toml"),
            EngineDirEntry::test_dir("target"),
            EngineDirEntry::test_dir("src"),
        ])
        .with_entries(pwd.join("src/"), [EngineDirEntry::test_dir("app")])
        .with_entries(pwd.join("src/app"), [EngineDirEntry::test_file("mod.rs")])
        .with_entries(
            pwd.join("target"),
            [
                EngineDirEntry::test_dir("debug"),
                EngineDirEntry::test_dir("release"),
            ],
        )
        .with_entries(
            pwd.join("target/debug"),
            [EngineDirEntry::test_file("warpui")],
        );

    let registry = create_test_command_registry([git_signature(), cd_signature()]);
    let ctx = FakeCompletionContext::new(registry)
        .with_path_completion_context(path_ctx)
        .with_top_level_commands(["cargo", "cd", "git"]);

    assert!(complete_at_end_of_line("cde", &ctx).is_empty());

    assert_eq!(complete_at_end_of_line("c", &ctx), vec!["cargo", "cd"]);

    assert_eq!(complete_at_end_of_line("git", &ctx), vec!["git"]);

    // The "-" here is included, even though it is hidden. It gets removed by a subsequent
    // `filter_by_query` call, which isn't tested here.
    assert_eq!(
        complete_at_end_of_line("cd ", &ctx),
        vec!["-", "src/", "target/"]
    );

    assert_eq!(complete_at_end_of_line("cd sr", &ctx), vec!["src/"]);

    assert_eq!(
        complete_at_end_of_line("cd target/de", &ctx),
        vec!["debug/"]
    );

    #[cfg(unix)]
    assert!(complete_at_end_of_line("cd /", &ctx).contains(&TEST_ROOT_DIR.to_owned()));

    // TODO(CORE-3696): test Windows root directory separately
    // #[cfg(windows)]
    // assert!(complete_at_end_of_line("cd C:", &ctx).contains(&TEST_ROOT_DIR.to_owned()));

    let git_subcommands = vec!["add", "branch", "checkout", "clone"];
    assert_eq!(complete_at_end_of_line("git ", &ctx), git_subcommands);

    assert_eq!(complete_at_end_of_line("git ad", &ctx), vec!["add"]);
    assert_eq!(
        complete_at_end_of_line("git c", &ctx),
        vec!["checkout", "clone"]
    );
    assert!(complete_at_end_of_line("git missing", &ctx).is_empty());

    assert_eq!(complete_at_end_of_line("cd ; gi", &ctx), vec!["git"]);

    assert_eq!(complete_at_end_of_line("cd ; git ", &ctx), git_subcommands);
    assert_eq!(
        complete_at_end_of_line("cd foo && git ", &ctx),
        git_subcommands
    );

    assert_eq!(
        complete_at_end_of_line("cd foo ; git check", &ctx),
        vec!["checkout"]
    );

    assert_eq!(
        complete_at_end_of_line(
            r#"ls -la && cat hello.txt
echo $(ls -la); cat password.secret | pbcopy
echo $(git check"#,
            &ctx
        ),
        vec!["checkout"]
    );
    /* end argument completer calls */
}

#[test]
fn completes_arguments() {
    let registry = create_test_command_registry([git_signature()]);
    let ctx = FakeCompletionContext::new(registry);

    assert_eq!(
        complete_at_end_of_line("ls -la && git c", &ctx),
        vec!["checkout", "clone"]
    );

    assert_eq!(
        complete_at_end_of_line("ls -la || echo $(git c", &ctx),
        vec!["checkout", "clone"]
    );
}

#[test]
fn test_completes_complicated_commands() {
    let registry = create_test_command_registry([]);
    let ctx = FakeCompletionContext::new(registry).with_top_level_commands(["cargo", "cd", "git"]);

    assert_eq!(
        complete_at_end_of_line("ls -la | c", &ctx),
        vec!["cargo", "cd"]
    );

    assert_eq!(
        complete_at_end_of_line("ls -la\necho $(c", &ctx),
        vec!["cargo", "cd"]
    );
}

#[test]
fn completes_many_args_under_option() {
    let registry = create_test_command_registry([git_signature()]);
    let ctx = FakeCompletionContext::new(registry);

    assert_eq!(
        complete_at_end_of_line("git branch -m ", &ctx),
        vec!["branch-1", "second-branch"]
    );

    assert_eq!(
        complete_at_end_of_line("git branch -m b", &ctx),
        vec!["branch-1"]
    );

    assert_eq!(
        complete_at_end_of_line("git branch -m branch-1 ", &ctx),
        vec!["branch-1", "second-branch"]
    );

    assert_eq!(
        complete_at_end_of_line("git branch -m branch-1 s", &ctx),
        vec!["second-branch"]
    );
}

#[cfg(not(feature = "v2"))]
#[test]
fn test_hidden_suggestion_only_appears_on_exact_match() {
    let registry = create_test_command_registry([cd_signature()]);
    let path_ctx = MockPathCompletionContext::new(TypedPathBuf::from(TEST_WORK_DIR))
        .with_entries_in_pwd([EngineDirEntry::test_dir("app")]);
    let ctx = FakeCompletionContext::new(registry).with_path_completion_context(path_ctx);

    // Need to use filter_by_query here to incorporate the exact match logic.
    let suggestion_results =
        complete_at_end_of_line_with_query("cd ", "", MatchStrategy::CaseInsensitive, &ctx);
    assert_eq!(suggestion_results, vec!["app/"]);

    let suggestion_results =
        complete_at_end_of_line_with_query("cd ", "-", MatchStrategy::CaseInsensitive, &ctx);
    assert_eq!(suggestion_results, vec!["-"]);
}

#[test]
fn test_subcommands_with_prefixes_of_one_another_are_completed_correctly() {
    let registry = create_test_command_registry([npm_signature()]);
    let ctx = FakeCompletionContext::new(registry);

    assert_eq!(complete_at_end_of_line("npm r", &ctx), vec!["r", "run"]);

    assert_eq!(complete_at_end_of_line("npm r ", &ctx), vec!["r-arg"]);

    assert_eq!(complete_at_end_of_line("npm ru", &ctx), vec!["run"]);
}

#[test]
fn test_completes_with_correct_ordering() {
    let registry = create_test_command_registry([test_signature()]);
    let ctx = FakeCompletionContext::new(registry);

    // Make sure the ordering is working as expected
    assert_eq!(
        complete_at_end_of_line_with_options("test ", MatchStrategy::CaseInsensitive, &ctx),
        vec![
            // High importance suggestions appear before default suggestions. Since they are tied in
            // priority, we sort them lexicographically.
            "--not-long",      // Priority 100, option
            "--required-args", // Priority 100, option
            "one",             // Priority 100, subcommand
            // Default priority suggestions retain the order produced by the completion engine,
            // sorted by `SuggestionType`.
            "eight",                        // Priority default, subcommand
            "five",                         // Priority default, subcommand
            "four",                         // Priority default, subcommand
            "nine",                         // Priority default, subcommand
            "seven",                        // Priority default, subcommand
            "six",                          // Priority default, subcommand
            "--required-and-optional-args", // Priority default, option
            "--template-args-for-opt",      // Priority default, option
            "-r",                           // Priority default, option
            "-V",                           // Priority default, option
            // Low importance suggestions appear after default suggestions.
            "two", // Priority 50, subcommand
            // Tied in order so we sort lexicographically.
            "--long",                   // Priority 1, option
            "--required-args-with-var", // Priority 1, option
            "three",                    // Priority 1, subcommand
        ]
    );
}

#[test]
fn test_completes_with_multiple_generators() {
    let registry = create_test_command_registry([test_signature()]);
    let generator_ctx = MockGeneratorContext::for_test_signature();
    let ctx = FakeCompletionContext::new(registry).with_generator_context(generator_ctx);

    // Make sure the new ordering is working as expected
    assert_eq!(
        complete_at_end_of_line_with_options("test five ", MatchStrategy::CaseInsensitive, &ctx),
        vec![
            // There are argument suggestions. They come from two generators,
            // so we should respect the order in which the generators were specified.
            "bar", "foo", // The first generator is not ordered, so we sort it.
            "def", "abc", // The second generator is already ordered.
        ]
    );
}

#[test]
fn test_completes_flags() {
    let registry = create_test_command_registry([git_signature()]);
    let ctx = FakeCompletionContext::new(registry);

    // Should complete all subcommands followed by all long hand flags followed by all short hand flags
    assert_eq!(
        complete_at_end_of_line_with_options("git ", MatchStrategy::CaseInsensitive, &ctx),
        vec![
            "add",
            "branch",
            "checkout",
            "clone",
            "--bare",
            "--help",
            "--version",
            "-c",
            "-p",
        ]
    );

    // But any non-dash prefix should prevent showing flags by default
    assert_eq!(
        complete_at_end_of_line_with_options("git c", MatchStrategy::CaseInsensitive, &ctx),
        vec!["checkout", "clone",]
    );

    // Should complete all short hand flags followed by all long hand flags
    assert_eq!(
        complete_at_end_of_line_with_options("ls -la; git -", MatchStrategy::CaseInsensitive, &ctx),
        vec!["-c", "-p", "--bare", "--help", "--version",]
    );

    // Should complete all not included short hand flags and the last flag.
    assert_eq!(
        complete_at_end_of_line_with_options(
            "ls -la; git -p",
            MatchStrategy::CaseInsensitive,
            &ctx
        ),
        vec!["-c", "-p"]
    );

    // Should not complete short hand flags that are already included except for the
    // last flag.
    assert_eq!(
        complete_at_end_of_line_with_options(
            "ls -la; git -pc",
            MatchStrategy::CaseInsensitive,
            &ctx
        ),
        vec!["-c"]
    );

    // Should complete all long hand flags only
    assert_eq!(
        complete_at_end_of_line_with_options(
            "ls -la; git --",
            MatchStrategy::CaseInsensitive,
            &ctx
        ),
        vec!["--bare", "--help", "--version",]
    );

    // Should complete long hand flags only (that begin with "v")
    assert_eq!(
        complete_at_end_of_line_with_options(
            "cat hello.txt | git --v",
            MatchStrategy::CaseInsensitive,
            &ctx
        ),
        vec!["--version"]
    );
}

#[test]
fn test_complete_long_hand_flags_single_dash() {
    let registry = create_test_command_registry([java_signature()]);
    let ctx = FakeCompletionContext::new(registry);

    // Should complete long hand flags that use a single dash (and not add
    // in the double-dashed versions).
    assert_eq!(
        complete_at_end_of_line_with_options("java -ve", MatchStrategy::CaseInsensitive, &ctx),
        vec!["-version"]
    );
    assert_eq!(
        complete_at_end_of_line_with_options("java -c", MatchStrategy::CaseInsensitive, &ctx),
        vec!["-classpath", "-cp"]
    );
}

#[test]
fn test_matching_indices() {
    let registry = create_test_command_registry([]);
    let path_ctx = MockPathCompletionContext::new(TypedPathBuf::from(TEST_WORK_DIR))
        .with_entries_in_pwd([
            EngineDirEntry::test_dir("a卡b"),
            EngineDirEntry::test_dir("卡b奇诺"),
            EngineDirEntry::test_dir("诺卡b奇"),
            EngineDirEntry::test_dir("奇诺卡b"),
        ]);
    let ctx = FakeCompletionContext::new(registry).with_path_completion_context(path_ctx);

    // We want to make sure that we are matching char indices
    assert_eq!(
        get_filtered_suggestions_with_query("cd 卡b", "卡b", MatchStrategy::CaseInsensitive, &ctx),
        vec![(vec![0, 1], String::from("卡b奇诺/"))]
    );

    assert_eq!(
        get_filtered_suggestions_with_query("cd 卡b", "卡b", MatchStrategy::CaseSensitive, &ctx),
        vec![(vec![0, 1], String::from("卡b奇诺/"))]
    );

    assert_eq!(
        get_filtered_suggestions_with_query("cd 卡b", "卡b", MatchStrategy::Fuzzy, &ctx),
        vec![
            (vec![0, 1], String::from("卡b奇诺/")),
            (vec![1, 2], String::from("诺卡b奇/")),
            (vec![2, 3], String::from("奇诺卡b/")),
            (vec![1, 2], String::from("a卡b/")),
        ]
    );
}

#[test]
fn test_matching_indices_nested_file_paths() {
    let registry = create_test_command_registry([]);
    let pwd = TypedPathBuf::from(TEST_WORK_DIR);
    let path_ctx = MockPathCompletionContext::new(pwd.clone())
        .with_entries_in_pwd([
            EngineDirEntry::test_dir("a卡b"),
            EngineDirEntry::test_dir("dir"),
        ])
        .with_entries(pwd.join("a卡b"), [EngineDirEntry::test_dir("c布d")])
        .with_entries(
            pwd.join("dir"),
            [
                EngineDirEntry::test_file("卡b奇诺"),
                EngineDirEntry::test_file("诺卡b奇"),
                EngineDirEntry::test_file("奇诺卡b"),
            ],
        )
        .with_entries(
            pwd.join("a卡b/c布d"),
            [
                EngineDirEntry::test_file("f咖啡gm牛nhdj奶"),
                EngineDirEntry::test_file("咖啡牛奶"),
            ],
        );
    let ctx = FakeCompletionContext::new(registry).with_path_completion_context(path_ctx);

    // We want to make sure that we are matching char indices
    assert_eq!(
        get_filtered_suggestions_with_query(
            "cd dir/",
            "dir/卡b",
            MatchStrategy::CaseInsensitive,
            &ctx
        ),
        vec![(vec![0, 1], String::from("卡b奇诺"))]
    );

    assert_eq!(
        get_filtered_suggestions_with_query("cd dir/", "dir/卡b", MatchStrategy::Fuzzy, &ctx),
        vec![
            (vec![0, 1], String::from("卡b奇诺")),
            (vec![1, 2], String::from("诺卡b奇")),
            (vec![2, 3], String::from("奇诺卡b")),
        ]
    );

    assert_eq!(
        get_filtered_suggestions_with_query("cd a卡b/", "a卡b/布", MatchStrategy::Fuzzy, &ctx),
        vec![(vec![1], String::from("c布d/"))]
    );

    assert_eq!(
        get_filtered_suggestions_with_query("cd a卡b/", "a卡b/cd", MatchStrategy::Fuzzy, &ctx),
        vec![(vec![0, 2], String::from("c布d/"))]
    );

    assert_eq!(
        get_filtered_suggestions_with_query(
            "cd a卡b/",
            "a卡b/cd",
            MatchStrategy::CaseInsensitive,
            &ctx
        ),
        Vec::<(Vec<usize>, String)>::new()
    );

    assert_eq!(
        get_filtered_suggestions_with_query(
            "cd a卡b/c布d",
            "a卡b/cd",
            MatchStrategy::CaseSensitive,
            &ctx
        ),
        Vec::<(Vec<usize>, String)>::new()
    );

    assert_eq!(
        get_filtered_suggestions_with_query(
            "cd a卡b/c布d/",
            "a卡b/c布d/咖啡牛奶",
            MatchStrategy::Fuzzy,
            &ctx
        ),
        vec![
            (vec![0, 1, 2, 3], String::from("咖啡牛奶")),
            (vec![1, 2, 5, 10], String::from("f咖啡gm牛nhdj奶")),
        ]
    );

    assert_eq!(
        get_filtered_suggestions_with_query(
            "cd a卡b/c布d/",
            "a卡b/c布d/咖啡牛奶",
            MatchStrategy::CaseSensitive,
            &ctx
        ),
        vec![(vec![0, 1, 2, 3], String::from("咖啡牛奶"))]
    );

    assert_eq!(
        get_filtered_suggestions_with_query(
            "cd a卡b/c布d/",
            "a卡b/c布d/咖啡牛奶",
            MatchStrategy::CaseInsensitive,
            &ctx
        ),
        vec![(vec![0, 1, 2, 3], String::from("咖啡牛奶"))]
    );

    assert_eq!(
        get_filtered_suggestions_with_query(
            "cd a卡b/c布d/",
            "a卡b/c布d/g牛h奶",
            MatchStrategy::Fuzzy,
            &ctx
        ),
        vec![(vec![3, 5, 7, 10], String::from("f咖啡gm牛nhdj奶"))]
    );

    assert_eq!(
        get_filtered_suggestions_with_query("cd a卡b/c布d/", "牛奶", MatchStrategy::Fuzzy, &ctx),
        vec![
            (vec![2, 3], String::from("咖啡牛奶")),
            (vec![5, 10], String::from("f咖啡gm牛nhdj奶")),
        ]
    );

    assert_eq!(
        get_filtered_suggestions_with_query(
            "cd a卡b/c布d/",
            "牛奶",
            MatchStrategy::CaseInsensitive,
            &ctx
        ),
        Vec::<(Vec<usize>, String)>::new()
    );

    assert_eq!(
        get_filtered_suggestions_with_query(
            "cd a卡b/c布d/",
            "a卡b/c布d/gdj",
            MatchStrategy::Fuzzy,
            &ctx
        ),
        vec![(vec![3, 8, 9], String::from("f咖啡gm牛nhdj奶"))]
    );
}

#[test]
fn test_matching_indices_for_git_branches() {
    let registry = create_test_command_registry([git_signature()]);
    let ctx = FakeCompletionContext::new(registry);

    // We want to make sure that we are matching char indices
    assert_eq!(
        get_filtered_suggestions_with_query(
            "git checkout bob/卡",
            "bob/卡",
            MatchStrategy::Fuzzy,
            &ctx
        ),
        vec![(vec![0, 1, 2, 3, 4], String::from("bob/卡b卡"))]
    );

    assert_eq!(
        get_filtered_suggestions_with_query(
            "git checkout bob/b",
            "bob/b",
            MatchStrategy::Fuzzy,
            &ctx
        ),
        vec![(vec![0, 1, 2, 3, 5], String::from("bob/卡b卡"))]
    );
}

#[test]
fn test_fuzzy_completions() {
    let registry = create_test_command_registry([fuzzy_signature()]);
    let ctx = FakeCompletionContext::new(registry);

    // Prefix matches should still respect ordering logic
    assert_eq!(
        complete_at_end_of_line_with_options("fuzzy prefix", MatchStrategy::Fuzzy, &ctx),
        vec!["prefix2", "prefix1", "suffix-pre-fix"]
    );

    assert_eq!(
        complete_at_end_of_line_with_options("fuzzy prefix", MatchStrategy::CaseInsensitive, &ctx),
        vec!["prefix2", "prefix1"]
    );
}

#[test]
fn test_exact_match_completions() {
    let registry = create_test_command_registry([test_signature()]);
    let ctx = FakeCompletionContext::new(registry);

    // Neither suggestion is an exact match so just use regular prefix ordering
    assert_eq!(
        complete_at_end_of_line_with_options("test six si", MatchStrategy::CaseInsensitive, &ctx),
        vec!["six-arg-2", "six-arg"]
    );

    // Exact match suggestion should be the first suggestion even if others have more priority.
    // Need to use filter_by_query here to incorporate the exact match logic.
    let suggestions = complete_at_end_of_line_with_query(
        "test six six-arg",
        "six-arg",
        MatchStrategy::CaseInsensitive,
        &ctx,
    );
    assert_eq!(suggestions, vec!["six-arg", "six-arg-2"]);
}

#[test]
fn test_deduplication() {
    let registry = create_test_command_registry([test_signature()]);
    let generator_ctx = MockGeneratorContext::for_test_signature();
    let ctx = FakeCompletionContext::new(registry).with_generator_context(generator_ctx);

    // The arg for this subcommand has the same, ordered generator repeated twice.
    // So if we weren't dedup'ing properly, then we would see ["def", "abc", "def", "abc"].
    assert_eq!(
        complete_at_end_of_line_with_options("test seven ", MatchStrategy::CaseInsensitive, &ctx),
        vec!["def", "abc",]
    );
}

/// Test to ensure that we also suggest subcommands if the user is in a positional where there
/// is an optional, variadic, argument.
#[test]
fn test_variadic_optional_arguments_also_completes_subcommands() {
    let registry = create_test_command_registry([test_signature()]);
    let ctx = FakeCompletionContext::new(registry);

    assert_eq!(
        complete_at_end_of_line("test eight ", &ctx),
        vec!["eight-arg"]
    );

    assert_eq!(
        complete_at_end_of_line("test eight eight-arg ", &ctx),
        vec!["eight-arg-2", "eight-subcommand"]
    );
}

#[test]
pub fn test_file_paths_with_separator_in_middle() {
    let registry = create_test_command_registry([]);
    let pwd = TypedPathBuf::from(TEST_WORK_DIR);
    let path_ctx = MockPathCompletionContext::new(pwd.clone())
        .with_entries_in_pwd([EngineDirEntry::test_dir("foo")])
        .with_entries(
            pwd.join("foo/"),
            [
                EngineDirEntry::test_file("script"),
                EngineDirEntry::test_file("script1"),
            ],
        );
    let ctx = FakeCompletionContext::new(registry).with_path_completion_context(path_ctx.clone());

    assert_eq!(
        complete_at_end_of_line("foo/scri", &ctx),
        vec!["script", "script1"]
    );
}

#[test]
pub fn test_case_sensitivity_ordering() {
    let registry = create_test_command_registry([]);
    let ctx = FakeCompletionContext::new(registry)
        .with_top_level_commands(["git", "GIT", "gitter", "GITTER"]);

    // The exact match appears first, followed by the case-insensitive exact match,
    // followed by any prefix matches.
    assert_eq!(
        complete_at_end_of_line_with_query("git", "git", MatchStrategy::Fuzzy, &ctx),
        vec!["git", "GIT", "GITTER", "gitter"]
    );
}

/// Regression test for Linear issue CORE-1885.
#[test]
pub fn test_autocd_completions_with_tilde() {
    let pwd = TypedPathBuf::from(TEST_WORK_DIR);
    let path_ctx = MockPathCompletionContext::new(pwd.clone())
        .with_home_directory(TEST_WORK_DIR.to_owned())
        .with_entries_in_pwd([
            EngineDirEntry::test_file("Cargo.toml"),
            EngineDirEntry::test_dir("certs"),
            EngineDirEntry::test_dir("src"),
        ])
        .with_entries(pwd.join("src/"), [EngineDirEntry::test_dir("app")]);

    let registry = create_test_command_registry([]);
    let ctx = FakeCompletionContext::new(registry)
        .with_path_completion_context(path_ctx)
        .with_top_level_commands(["cargo", "cd", "git"])
        .with_supports_autocd(true);

    assert_eq!(complete_at_end_of_line("~/src/", &ctx), vec!["app/"]);
}

#[cfg(not(feature = "v2"))]
#[test]
fn test_option_name_with_missing_required_value() {
    let registry = create_test_command_registry([git_signature()]);

    let path_ctx = MockPathCompletionContext::new(TypedPathBuf::from(TEST_WORK_DIR));
    let ctx = FakeCompletionContext::new(registry).with_path_completion_context(path_ctx);

    assert_eq!(
        complete_at_end_of_line_with_options(
            "git branch --delete",
            MatchStrategy::CaseInsensitive,
            &ctx
        ),
        vec!["--delete"]
    );
}

/// TODO(CORE-2795)
#[cfg(not(feature = "v2"))]
#[test]
fn test_powershell_parser_directives_for_flags() {
    let registry = create_test_command_registry([add_content_signature()]);
    let pwd = TypedPathBuf::from(TEST_WORK_DIR);
    let path_ctx = MockPathCompletionContext::new(pwd).with_entries_in_pwd([
        EngineDirEntry::test_dir("foo"),
        EngineDirEntry::test_file("bar"),
    ]);
    let ctx = FakeCompletionContext::new(registry).with_path_completion_context(path_ctx);

    assert_eq!(
        complete_at_end_of_line_with_options(
            "Add-Content -F",
            MatchStrategy::CaseInsensitive,
            &ctx
        ),
        vec!["-Force"]
    );
    assert_eq!(
        complete_at_end_of_line_with_options(
            "Add-Content -Force -E",
            MatchStrategy::CaseInsensitive,
            &ctx
        ),
        vec!["-Encoding", "-Exclude"]
    );
    assert_eq!(
        complete_at_end_of_line_with_options(
            "Add-Content -Force -En",
            MatchStrategy::CaseInsensitive,
            &ctx
        ),
        vec!["-Encoding"]
    );
    assert_eq!(
        complete_at_end_of_line_with_options(
            "Add-Content -Force -Encoding",
            MatchStrategy::CaseInsensitive,
            &ctx
        ),
        vec!["-Encoding"]
    );
    assert_eq!(
        complete_at_end_of_line_with_options(
            "Add-Content -Force -enc",
            MatchStrategy::CaseInsensitive,
            &ctx
        ),
        vec!["-Encoding"]
    );
    assert_eq!(
        complete_at_end_of_line("Add-Content -Force -Encoding ", &ctx),
        vec!["ASCII", "UTF8"]
    );
    assert_eq!(
        complete_at_end_of_line("Add-Content -Force -Enc ", &ctx),
        vec!["ASCII", "UTF8"]
    );
    assert_eq!(
        complete_at_end_of_line_with_options(
            "Add-Content -F -Enc ASCII b",
            MatchStrategy::CaseInsensitive,
            &ctx
        ),
        vec!["bar"]
    );
}

/// TODO(CORE-2795)
#[cfg(not(feature = "v2"))]
#[test]
fn test_powershell_parser_directives_for_case_insensitivity() {
    let registry = create_test_command_registry([add_content_signature()]);
    let pwd = TypedPathBuf::from(TEST_WORK_DIR);
    let path_ctx = MockPathCompletionContext::new(pwd).with_entries_in_pwd([
        EngineDirEntry::test_dir("foo"),
        EngineDirEntry::test_file("bar"),
    ]);
    let ctx = FakeCompletionContext::new(registry).with_path_completion_context(path_ctx);

    assert_eq!(
        complete_at_end_of_line_with_options(
            "add-content -F",
            MatchStrategy::CaseInsensitive,
            &ctx
        ),
        vec!["-Force"]
    );
}
