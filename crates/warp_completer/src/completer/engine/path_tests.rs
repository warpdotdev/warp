use warp_command_signatures::IconType;

use super::*;
use crate::completer::testing::MockPathCompletionContext;

#[cfg(windows)]
mod windows_constants {
    pub(super) const TEST_HOME_DIR: &str = r"C:\Users\test";
}

#[cfg(windows)]
use windows_constants::*;

#[cfg(unix)]
mod unix_constants {
    pub(super) const TEST_HOME_DIR: &str = "/users/test";
}

#[cfg(unix)]
use unix_constants::*;

#[test]
fn test_split_path() {
    let path = TypedPathBuf::from_unix("/Users/warpuser");
    let ctx = MockPathCompletionContext::new(path.clone())
        .with_home_directory("/Users/warpuser".to_owned());

    let split_path = SplitPath::new("~/Warp.app", &ctx);

    assert_eq!(
        split_path,
        SplitPath {
            directory_absolute_path: path.clone(),
            directory_relative_path_name: "~/".to_owned(),
            file_name: "Warp.app".to_owned(),
            unresolved_environment_variable: false,
        }
    );

    let split_path = SplitPath::new("Warp.app/Contents", &ctx);
    assert_eq!(
        split_path,
        SplitPath {
            directory_absolute_path: TypedPathBuf::from("/Users/warpuser/Warp.app/"),
            directory_relative_path_name: "Warp.app/".to_owned(),
            file_name: "Contents".to_owned(),
            unresolved_environment_variable: false,
        }
    );

    let split_path = SplitPath::new("Warp.app/macOS/bin/warp.o", &ctx);
    assert_eq!(
        split_path,
        SplitPath {
            directory_absolute_path: TypedPathBuf::from("/Users/warpuser/Warp.app/macOS/bin/"),
            directory_relative_path_name: "Warp.app/macOS/bin/".to_owned(),
            file_name: "warp.o".to_owned(),
            unresolved_environment_variable: false,
        }
    );
}

fn file_entry(file_name: &str) -> EngineDirEntry {
    EngineDirEntry {
        file_name: file_name.to_owned(),
        file_type: EngineFileType::File,
    }
}

fn dir_entry(file_name: &str) -> EngineDirEntry {
    EngineDirEntry {
        file_name: file_name.to_owned(),
        file_type: EngineFileType::Directory,
    }
}

#[cfg_attr(
    windows,
    ignore = "CORE-3696: path sorting comparison function needs separators"
)]
#[test]
pub fn test_sorted_paths_relative_to() {
    let ctx = MockPathCompletionContext::default().with_entries_in_pwd([
        file_entry("Cargo.toml"),
        dir_entry("src"),
        dir_entry("target"),
        dir_entry(".hidden"),
    ]);

    assert_eq!(
        warpui_core::r#async::block_on(sorted_paths_relative_to(
            &ParsedToken::empty(),
            MatchStrategy::CaseInsensitive,
            &ctx
        ))
        .into_iter()
        .map(|matched_suggestion| matched_suggestion.suggestion)
        .collect_vec(),
        vec![
            Suggestion::with_same_display_and_replacement(
                "Cargo.toml",
                Some("File".into()),
                SuggestionType::Argument,
                Priority::default(),
            )
            .with_icon_override(IconType::File)
            .with_file_type(EngineFileType::File),
            Suggestion::with_same_display_and_replacement(
                "src/",
                Some("Directory".into()),
                SuggestionType::Argument,
                Priority::default(),
            )
            .with_icon_override(IconType::Folder)
            .with_file_type(EngineFileType::Directory),
            Suggestion::with_same_display_and_replacement(
                "target/",
                Some("Directory".into()),
                SuggestionType::Argument,
                Priority::default(),
            )
            .with_icon_override(IconType::Folder)
            .with_file_type(EngineFileType::Directory),
        ]
    );

    assert_eq!(
        warpui_core::r#async::block_on(sorted_paths_relative_to(
            &ParsedToken::new("sr"),
            MatchStrategy::CaseInsensitive,
            &ctx
        ))
        .into_iter()
        .map(|matched_suggestion| matched_suggestion.suggestion)
        .collect_vec(),
        vec![
            Suggestion::with_same_display_and_replacement(
                "src/",
                Some("Directory".into()),
                SuggestionType::Argument,
                Priority::default(),
            )
            .with_icon_override(IconType::Folder)
            .with_file_type(EngineFileType::Directory)
        ]
    );

    assert_eq!(
        warpui_core::r#async::block_on(sorted_paths_relative_to(
            &ParsedToken::new("."),
            MatchStrategy::CaseInsensitive,
            &ctx
        ))
        .into_iter()
        .map(|matched_suggestion| matched_suggestion.suggestion)
        .collect_vec(),
        vec![
            Suggestion::with_same_display_and_replacement(
                "./",
                Some("Directory".into()),
                SuggestionType::Argument,
                Priority::default(),
            )
            .with_icon_override(IconType::Folder)
            .with_file_type(EngineFileType::Directory),
            Suggestion::with_same_display_and_replacement(
                "../",
                Some("Directory".into()),
                SuggestionType::Argument,
                Priority::default(),
            )
            .with_icon_override(IconType::Folder)
            .with_file_type(EngineFileType::Directory),
            Suggestion::with_same_display_and_replacement(
                ".hidden/",
                Some("Directory".into()),
                SuggestionType::Argument,
                Priority::default(),
            )
            .with_icon_override(IconType::Folder)
            .with_file_type(EngineFileType::Directory),
        ]
    );
}

#[test]
pub fn test_sorted_directories_relative_to() {
    let ctx = MockPathCompletionContext::default().with_entries_in_pwd([
        file_entry("Cargo.toml"),
        dir_entry("src"),
        dir_entry("target"),
        dir_entry(".hidden"),
    ]);

    assert_eq!(
        warpui_core::r#async::block_on(sorted_directories_relative_to(
            &ParsedToken::empty(),
            MatchStrategy::CaseInsensitive,
            &ctx
        ))
        .into_iter()
        .map(|matched_suggestion| matched_suggestion.suggestion)
        .collect_vec(),
        vec![
            Suggestion::with_same_display_and_replacement(
                "src/",
                Some("Directory".into()),
                SuggestionType::Argument,
                Priority::default(),
            )
            .with_icon_override(IconType::Folder)
            .with_file_type(EngineFileType::Directory),
            Suggestion::with_same_display_and_replacement(
                "target/",
                Some("Directory".into()),
                SuggestionType::Argument,
                Priority::default(),
            )
            .with_icon_override(IconType::Folder)
            .with_file_type(EngineFileType::Directory),
        ]
    );

    assert_eq!(
        warpui_core::r#async::block_on(sorted_directories_relative_to(
            &ParsedToken::new("s"),
            MatchStrategy::CaseInsensitive,
            &ctx
        ))
        .into_iter()
        .map(|matched_suggestion| matched_suggestion.suggestion)
        .collect_vec(),
        vec![
            Suggestion::with_same_display_and_replacement(
                "src/",
                Some("Directory".into()),
                SuggestionType::Argument,
                Priority::default(),
            )
            .with_icon_override(IconType::Folder)
            .with_file_type(EngineFileType::Directory)
        ]
    );
}

/// Verify that path suggestions are sorted case-insensitively so that uppercase entries
/// don't always appear before lowercase ones.
#[cfg_attr(
    windows,
    ignore = "CORE-3696: path sorting comparison function needs separators"
)]
#[test]
pub fn test_sorted_paths_case_insensitive_ordering() {
    let ctx = MockPathCompletionContext::default().with_entries_in_pwd([
        file_entry("Zebra.txt"),
        file_entry("apple.txt"),
        dir_entry("Banana"),
        file_entry("cherry.txt"),
    ]);

    let suggestions: Vec<String> = warpui_core::r#async::block_on(sorted_paths_relative_to(
        &ParsedToken::empty(),
        MatchStrategy::CaseInsensitive,
        &ctx,
    ))
    .into_iter()
    .map(|matched_suggestion| matched_suggestion.suggestion.display.to_string())
    .collect();

    // Expected case-insensitive order: apple, Banana, cherry, Zebra
    assert_eq!(
        suggestions,
        vec!["apple.txt", "Banana/", "cherry.txt", "Zebra.txt"]
    );
}

fn mock_path_completion_ctx_special_characters() -> MockPathCompletionContext {
    MockPathCompletionContext::default()
        .with_home_directory(TEST_HOME_DIR.to_owned())
        .with_entries_in_pwd([dir_entry("!nice ~"), dir_entry("~"), dir_entry("~foo")])
}

/// Check that special characters are properly escaped in the Suggestion.
#[test]
pub fn test_path_completions_with_special_characters_relative_to_cwd() {
    let ctx = mock_path_completion_ctx_special_characters();

    assert_eq!(
        warpui_core::r#async::block_on(sorted_directories_relative_to(
            &ParsedToken::empty(),
            MatchStrategy::CaseInsensitive,
            &ctx
        ))
        .into_iter()
        .map(|matched_suggestion| matched_suggestion.suggestion)
        .collect_vec(),
        vec![
            Suggestion::new(
                "!nice ~/",
                r"\!nice\ \~/",
                Some("Directory".into()),
                SuggestionType::Argument,
                Priority::default(),
            )
            .with_icon_override(IconType::Folder)
            .with_file_type(EngineFileType::Directory),
            Suggestion::new(
                "~/",
                r"\~/",
                Some("Directory".into()),
                SuggestionType::Argument,
                Priority::default(),
            )
            .with_icon_override(IconType::Folder)
            .with_file_type(EngineFileType::Directory),
            Suggestion::new(
                "~foo/",
                r"\~foo/",
                Some("Directory".into()),
                SuggestionType::Argument,
                Priority::default(),
            )
            .with_icon_override(IconType::Folder)
            .with_file_type(EngineFileType::Directory),
        ]
    );
}

/// Check that we can match on special characters at the beginning of the file name.
#[test]
pub fn test_path_completions_with_special_character_case_insensitive() {
    let ctx = mock_path_completion_ctx_special_characters();
    assert_eq!(
        warpui_core::r#async::block_on(sorted_directories_relative_to(
            &ParsedToken::new("~"),
            MatchStrategy::CaseInsensitive,
            &ctx
        ))
        .into_iter()
        .map(|matched_suggestion| matched_suggestion.suggestion)
        .collect_vec(),
        vec![
            Suggestion::new(
                "~/",
                r"\~/",
                Some("Directory".into()),
                SuggestionType::Argument,
                Priority::default(),
            )
            .with_icon_override(IconType::Folder)
            .with_file_type(EngineFileType::Directory),
            Suggestion::new(
                "~foo/",
                r"\~foo/",
                Some("Directory".into()),
                SuggestionType::Argument,
                Priority::default(),
            )
            .with_icon_override(IconType::Folder)
            .with_file_type(EngineFileType::Directory),
        ]
    );
}

/// Check that we can match on special characters regardless of their position in the file name.
#[test]
pub fn test_path_completions_with_special_characters_fuzzy() {
    let ctx = mock_path_completion_ctx_special_characters();

    assert_eq!(
        warpui_core::r#async::block_on(sorted_directories_relative_to(
            &ParsedToken::new("~"),
            MatchStrategy::Fuzzy,
            &ctx
        ))
        .into_iter()
        .map(|matched_suggestion| matched_suggestion.suggestion)
        .collect_vec(),
        vec![
            Suggestion::new(
                "!nice ~/",
                r"\!nice\ \~/",
                Some("Directory".into()),
                SuggestionType::Argument,
                Priority::default(),
            )
            .with_icon_override(IconType::Folder)
            .with_file_type(EngineFileType::Directory),
            Suggestion::new(
                "~/",
                r"\~/",
                Some("Directory".into()),
                SuggestionType::Argument,
                Priority::default(),
            )
            .with_icon_override(IconType::Folder)
            .with_file_type(EngineFileType::Directory),
            Suggestion::new(
                "~foo/",
                r"\~foo/",
                Some("Directory".into()),
                SuggestionType::Argument,
                Priority::default(),
            )
            .with_icon_override(IconType::Folder)
            .with_file_type(EngineFileType::Directory),
        ]
    );
}

fn mock_path_completion_ctx_special_characters_home_dir() -> MockPathCompletionContext {
    MockPathCompletionContext::default()
        .with_home_directory(TEST_HOME_DIR.to_owned())
        .with_entries_in_pwd([dir_entry("~")])
        .with_entries(TEST_HOME_DIR.into(), [dir_entry(r"~ testdir")])
}

/// Check that tilde expansion works with path completion and special characters in Suggestions.
#[test]
pub fn test_path_completions_tilde_expansion() {
    let ctx = mock_path_completion_ctx_special_characters_home_dir();

    assert_eq!(
        warpui_core::r#async::block_on(sorted_directories_relative_to(
            &ParsedToken::new("~/"),
            MatchStrategy::Fuzzy,
            &ctx
        ))
        .into_iter()
        .map(|matched_suggestion| matched_suggestion.suggestion)
        .collect_vec(),
        vec![
            Suggestion::new(
                "~ testdir/",
                r"~/\~\ testdir/",
                Some("Directory".into()),
                SuggestionType::Argument,
                Priority::default(),
            )
            .with_icon_override(IconType::Folder)
            .with_file_type(EngineFileType::Directory),
        ]
    );
}

/// Check that $HOME home directory expansion works with special characters in the suggestions.
#[test]
pub fn test_path_completions_home_env_var_special_characters() {
    let ctx = mock_path_completion_ctx_special_characters_home_dir();

    assert_eq!(
        warpui_core::r#async::block_on(sorted_directories_relative_to(
            &ParsedToken::new("$HOME/"),
            MatchStrategy::Fuzzy,
            &ctx
        ))
        .into_iter()
        .map(|matched_suggestion| matched_suggestion.suggestion)
        .collect_vec(),
        vec![
            Suggestion::new(
                "~ testdir/",
                r"$HOME/\~\ testdir/",
                Some("Directory".into()),
                SuggestionType::Argument,
                Priority::default(),
            )
            .with_icon_override(IconType::Folder)
            .with_file_type(EngineFileType::Directory),
        ]
    );
}

#[cfg(unix)]
#[test]
pub fn test_sorted_cd_directories_no_cdpath_matches_existing_behavior() {
    let ctx = MockPathCompletionContext::default()
        .with_entries_in_pwd([dir_entry("local-only"), dir_entry("shared")]);

    let from_cd = warpui_core::r#async::block_on(sorted_cd_directories(
        &ParsedToken::empty(),
        MatchStrategy::CaseInsensitive,
        &ctx,
    ));
    let from_default = warpui_core::r#async::block_on(sorted_directories_relative_to(
        &ParsedToken::empty(),
        MatchStrategy::CaseInsensitive,
        &ctx,
    ));
    assert_eq!(from_cd, from_default);
}

#[cfg(unix)]
#[test]
pub fn test_sorted_cd_directories_includes_cdpath_entries() {
    let ctx = MockPathCompletionContext::default()
        .with_entries_in_pwd([dir_entry("local-only"), dir_entry("shared")])
        .with_entries(
            TypedPathBuf::from("/srv/projects"),
            [
                dir_entry("shared"),
                dir_entry("extra-dir"),
                file_entry("a-file"),
            ],
        )
        .with_cdpath("/srv/projects".to_owned());

    let displays: Vec<String> = warpui_core::r#async::block_on(sorted_cd_directories(
        &ParsedToken::empty(),
        MatchStrategy::CaseInsensitive,
        &ctx,
    ))
    .into_iter()
    .map(|m| m.suggestion.display.to_string())
    .collect();

    // CDPATH entries first (in order), pwd appended as fallback. `shared`
    // appears in both, so the first occurrence (the CDPATH one) wins. Within
    // each directory, listings are sorted alphabetically.
    assert_eq!(displays, vec!["extra-dir/", "shared/", "local-only/"]);
}

#[cfg(unix)]
#[test]
pub fn test_sorted_cd_directories_ignores_cdpath_for_absolute_token() {
    let ctx = MockPathCompletionContext::default()
        .with_entries_in_pwd([dir_entry("local-only")])
        .with_entries(
            TypedPathBuf::from("/srv/projects"),
            [dir_entry("extra-dir")],
        )
        .with_entries(TypedPathBuf::from("/abs"), [dir_entry("absdir")])
        .with_cdpath("/srv/projects".to_owned());

    let displays: Vec<String> = warpui_core::r#async::block_on(sorted_cd_directories(
        &ParsedToken::new("/abs/"),
        MatchStrategy::CaseInsensitive,
        &ctx,
    ))
    .into_iter()
    .map(|m| m.suggestion.display.to_string())
    .collect();

    assert!(displays.iter().all(|d| d != "extra-dir/"));
    assert!(displays.contains(&"absdir/".to_owned()));
}

#[cfg(unix)]
#[test]
pub fn test_sorted_cd_directories_skips_dot_entry_in_cdpath() {
    // `.` in CDPATH is handled by the pwd-relative pass; skip it on overlay
    // to avoid double-listing pwd contents.
    let ctx = MockPathCompletionContext::default()
        .with_entries_in_pwd([dir_entry("local-only")])
        .with_cdpath(".".to_owned());

    let displays: Vec<String> = warpui_core::r#async::block_on(sorted_cd_directories(
        &ParsedToken::empty(),
        MatchStrategy::CaseInsensitive,
        &ctx,
    ))
    .into_iter()
    .map(|m| m.suggestion.display.to_string())
    .collect();
    assert_eq!(displays, vec!["local-only/"]);
}

#[cfg(unix)]
#[test]
pub fn test_sorted_cd_directories_resolves_relative_cdpath_against_pwd() {
    // CDPATH=src must resolve to <pwd>/src, not be passed raw.
    let ctx = MockPathCompletionContext::new(TypedPathBuf::from("/work/proj"))
        .with_entries_in_pwd([dir_entry("local-only")])
        .with_entries(
            TypedPathBuf::from("/work/proj/src"),
            [dir_entry("inner-mod")],
        )
        .with_cdpath("src".to_owned());

    let displays: Vec<String> = warpui_core::r#async::block_on(sorted_cd_directories(
        &ParsedToken::empty(),
        MatchStrategy::CaseInsensitive,
        &ctx,
    ))
    .into_iter()
    .map(|m| m.suggestion.display.to_string())
    .collect();
    assert_eq!(displays, vec!["inner-mod/", "local-only/"]);
}

#[cfg(unix)]
#[test]
pub fn test_sorted_cd_directories_resolves_parent_relative_cdpath() {
    // CDPATH=.. must resolve to <pwd>/.. so siblings of pwd are reachable.
    let ctx = MockPathCompletionContext::new(TypedPathBuf::from("/work/proj"))
        .with_entries_in_pwd([dir_entry("local-only")])
        .with_entries(
            TypedPathBuf::from("/work/proj/.."),
            [dir_entry("sibling-dir")],
        )
        .with_cdpath("..".to_owned());

    let displays: Vec<String> = warpui_core::r#async::block_on(sorted_cd_directories(
        &ParsedToken::empty(),
        MatchStrategy::CaseInsensitive,
        &ctx,
    ))
    .into_iter()
    .map(|m| m.suggestion.display.to_string())
    .collect();
    assert_eq!(displays, vec!["sibling-dir/", "local-only/"]);
}

#[cfg(unix)]
#[test]
pub fn test_sorted_cd_directories_expands_tilde_in_cdpath() {
    // Tilde-prefixed CDPATH=~/code must expand to the shell's home dir.
    let ctx = MockPathCompletionContext::new(TypedPathBuf::from("/work/proj"))
        .with_home_directory("/home/me".to_owned())
        .with_entries_in_pwd([dir_entry("local-only")])
        .with_entries(
            TypedPathBuf::from("/home/me/code"),
            [dir_entry("from-home")],
        )
        .with_cdpath("~/code".to_owned());

    let displays: Vec<String> = warpui_core::r#async::block_on(sorted_cd_directories(
        &ParsedToken::empty(),
        MatchStrategy::CaseInsensitive,
        &ctx,
    ))
    .into_iter()
    .map(|m| m.suggestion.display.to_string())
    .collect();
    assert_eq!(displays, vec!["from-home/", "local-only/"]);
}

#[cfg(unix)]
#[test]
pub fn test_sorted_cd_directories_pwd_at_dot_position_is_first() {
    // CDPATH=":/srv/projects": the empty leading entry means pwd is searched
    // before /srv/projects, matching shell semantics.
    let ctx = MockPathCompletionContext::new(TypedPathBuf::from("/work/proj"))
        .with_entries_in_pwd([dir_entry("local-only"), dir_entry("shared")])
        .with_entries(
            TypedPathBuf::from("/srv/projects"),
            [dir_entry("shared"), dir_entry("extra-dir")],
        )
        .with_cdpath(":/srv/projects".to_owned());

    let displays: Vec<String> = warpui_core::r#async::block_on(sorted_cd_directories(
        &ParsedToken::empty(),
        MatchStrategy::CaseInsensitive,
        &ctx,
    ))
    .into_iter()
    .map(|m| m.suggestion.display.to_string())
    .collect();
    assert_eq!(displays, vec!["local-only/", "shared/", "extra-dir/"]);
}

#[cfg(unix)]
#[test]
pub fn test_sorted_cd_directories_pwd_at_dot_in_middle() {
    // CDPATH="/srv/a:.:/srv/b": pwd appears between the two CDPATH entries.
    let ctx = MockPathCompletionContext::new(TypedPathBuf::from("/work/proj"))
        .with_entries_in_pwd([dir_entry("from-pwd")])
        .with_entries(TypedPathBuf::from("/srv/a"), [dir_entry("from-a")])
        .with_entries(TypedPathBuf::from("/srv/b"), [dir_entry("from-b")])
        .with_cdpath("/srv/a:.:/srv/b".to_owned());

    let displays: Vec<String> = warpui_core::r#async::block_on(sorted_cd_directories(
        &ParsedToken::empty(),
        MatchStrategy::CaseInsensitive,
        &ctx,
    ))
    .into_iter()
    .map(|m| m.suggestion.display.to_string())
    .collect();
    assert_eq!(displays, vec!["from-a/", "from-pwd/", "from-b/"]);
}

#[cfg(unix)]
fn mock_path_completion_ctx_env_var() -> MockPathCompletionContext {
    MockPathCompletionContext::new(TypedPathBuf::from("/work/proj"))
        .with_entries_in_pwd([dir_entry("local-only")])
        .with_entries(
            TypedPathBuf::from("/tmp/proj"),
            [dir_entry("src"), file_entry("README.md")],
        )
        .with_entries(TypedPathBuf::from("/tmp/proj/src"), [dir_entry("app")])
        .with_environment_variable("PROJ", "/tmp/proj")
}

/// A set variable resolves, and the replacement keeps the original `$VAR/` prefix, exactly as
/// `$HOME/` does today (see `test_path_completions_home_env_var_special_characters`).
#[cfg(unix)]
#[test]
pub fn test_path_completions_env_var_resolves() {
    let ctx = mock_path_completion_ctx_env_var();

    assert_eq!(
        warpui_core::r#async::block_on(sorted_directories_relative_to(
            &ParsedToken::new("$PROJ/"),
            MatchStrategy::Fuzzy,
            &ctx
        ))
        .into_iter()
        .map(|matched_suggestion| matched_suggestion.suggestion)
        .collect_vec(),
        vec![
            Suggestion::new(
                "src/",
                "$PROJ/src/",
                Some("Directory".into()),
                SuggestionType::Argument,
                Priority::default(),
            )
            .with_icon_override(IconType::Folder)
            .with_file_type(EngineFileType::Directory),
        ]
    );
}

/// The `${VAR}/` brace form resolves the same way as the bare `$VAR/` form.
#[cfg(unix)]
#[test]
pub fn test_path_completions_env_var_brace_form_resolves() {
    let ctx = mock_path_completion_ctx_env_var();

    assert_eq!(
        warpui_core::r#async::block_on(sorted_directories_relative_to(
            &ParsedToken::new("${PROJ}/"),
            MatchStrategy::Fuzzy,
            &ctx
        ))
        .into_iter()
        .map(|matched_suggestion| matched_suggestion.suggestion)
        .collect_vec(),
        vec![
            Suggestion::new(
                "src/",
                "${PROJ}/src/",
                Some("Directory".into()),
                SuggestionType::Argument,
                Priority::default(),
            )
            .with_icon_override(IconType::Folder)
            .with_file_type(EngineFileType::Directory),
        ]
    );
}

/// A `$VAR` reference nested deeper in the token (e.g. `$PROJ/src/`) resolves against the
/// variable's value joined with the rest of the token.
#[cfg(unix)]
#[test]
pub fn test_path_completions_env_var_nested_path_resolves() {
    let ctx = mock_path_completion_ctx_env_var();

    assert_eq!(
        warpui_core::r#async::block_on(sorted_directories_relative_to(
            &ParsedToken::new("$PROJ/src/"),
            MatchStrategy::Fuzzy,
            &ctx
        ))
        .into_iter()
        .map(|matched_suggestion| matched_suggestion.suggestion)
        .collect_vec(),
        vec![
            Suggestion::new(
                "app/",
                "$PROJ/src/app/",
                Some("Directory".into()),
                SuggestionType::Argument,
                Priority::default(),
            )
            .with_icon_override(IconType::Folder)
            .with_file_type(EngineFileType::Directory),
        ]
    );
}

/// An unset variable yields no suggestions at all -- not even `.`/`..` -- rather than falling
/// back to treating the token as a literal relative path.
#[cfg(unix)]
#[test]
pub fn test_path_completions_unset_env_var_yields_no_suggestions() {
    let ctx = mock_path_completion_ctx_env_var();

    assert!(
        warpui_core::r#async::block_on(sorted_directories_relative_to(
            &ParsedToken::new("$MISSING/"),
            MatchStrategy::Fuzzy,
            &ctx
        ))
        .is_empty()
    );

    assert!(
        warpui_core::r#async::block_on(sorted_paths_relative_to(
            &ParsedToken::new("$MISSING/"),
            MatchStrategy::Fuzzy,
            &ctx
        ))
        .is_empty()
    );
}

/// Once a `$VAR/` token expands to an absolute path, `$CDPATH` must be skipped for it, just as
/// it is for any other absolute token (e.g. `/abs/`).
#[cfg(unix)]
#[test]
pub fn test_sorted_cd_directories_ignores_cdpath_for_env_var_token() {
    let ctx = mock_path_completion_ctx_env_var()
        .with_entries(
            TypedPathBuf::from("/srv/projects"),
            [dir_entry("extra-dir")],
        )
        .with_cdpath("/srv/projects".to_owned());

    let displays: Vec<String> = warpui_core::r#async::block_on(sorted_cd_directories(
        &ParsedToken::new("$PROJ/"),
        MatchStrategy::CaseInsensitive,
        &ctx,
    ))
    .into_iter()
    .map(|m| m.suggestion.display.to_string())
    .collect();

    assert_eq!(displays, vec!["src/"]);
}

/// A repeated separator immediately after the variable reference (e.g. `$VAR//App`) must not be
/// treated as an absolute suffix that discards the resolved variable base: `TypedPathBuf::push`
/// replaces its receiver outright when pushed an absolute path, so an unstripped remainder like
/// `/src` would otherwise search `/` instead of the variable's value. Covers both the bare and
/// brace forms.
#[cfg(unix)]
#[test]
pub fn test_path_completions_env_var_repeated_separator_resolves() {
    let ctx = mock_path_completion_ctx_env_var();

    assert_eq!(
        warpui_core::r#async::block_on(sorted_directories_relative_to(
            &ParsedToken::new("$PROJ//src"),
            MatchStrategy::Fuzzy,
            &ctx
        ))
        .into_iter()
        .map(|matched_suggestion| matched_suggestion.suggestion)
        .collect_vec(),
        vec![
            Suggestion::new(
                "src/",
                "$PROJ//src/",
                Some("Directory".into()),
                SuggestionType::Argument,
                Priority::default(),
            )
            .with_icon_override(IconType::Folder)
            .with_file_type(EngineFileType::Directory),
        ]
    );

    assert_eq!(
        warpui_core::r#async::block_on(sorted_directories_relative_to(
            &ParsedToken::new("${PROJ}//src"),
            MatchStrategy::Fuzzy,
            &ctx
        ))
        .into_iter()
        .map(|matched_suggestion| matched_suggestion.suggestion)
        .collect_vec(),
        vec![
            Suggestion::new(
                "src/",
                "${PROJ}//src/",
                Some("Directory".into()),
                SuggestionType::Argument,
                Priority::default(),
            )
            .with_icon_override(IconType::Folder)
            .with_file_type(EngineFileType::Directory),
        ]
    );
}

/// A variable whose value is relative (rather than absolute) resolves against the shell's pwd,
/// not the Warp process's own cwd.
#[cfg(unix)]
#[test]
pub fn test_path_completions_env_var_relative_value_resolves_against_pwd() {
    let ctx = MockPathCompletionContext::new(TypedPathBuf::from("/work/proj"))
        .with_entries_in_pwd([dir_entry("local-only")])
        .with_entries(
            TypedPathBuf::from("/work/proj/projects"),
            [dir_entry("app")],
        )
        .with_environment_variable("ROOT", "projects");

    assert_eq!(
        warpui_core::r#async::block_on(sorted_directories_relative_to(
            &ParsedToken::new("$ROOT/"),
            MatchStrategy::Fuzzy,
            &ctx
        ))
        .into_iter()
        .map(|matched_suggestion| matched_suggestion.suggestion)
        .collect_vec(),
        vec![
            Suggestion::new(
                "app/",
                "$ROOT/app/",
                Some("Directory".into()),
                SuggestionType::Argument,
                Priority::default(),
            )
            .with_icon_override(IconType::Folder)
            .with_file_type(EngineFileType::Directory),
        ]
    );
}

/// When a `$VAR/` token resolves to a *relative* path, `$CDPATH` eligibility must follow that
/// resolved-path semantics (not assume every `$VAR/` token is absolute): the relative value gets
/// resolved against each `$CDPATH` entry in turn, exactly like a relative `$CDPATH` entry itself
/// is resolved against pwd.
#[cfg(unix)]
#[test]
pub fn test_sorted_cd_directories_applies_cdpath_for_relative_env_var_value() {
    let ctx = MockPathCompletionContext::new(TypedPathBuf::from("/work/proj"))
        .with_entries_in_pwd([dir_entry("local-only")])
        .with_entries(
            TypedPathBuf::from("/srv/projects/relvar"),
            [dir_entry("from-cdpath")],
        )
        .with_environment_variable("RELVAR", "relvar")
        .with_cdpath("/srv/projects".to_owned());

    let displays: Vec<String> = warpui_core::r#async::block_on(sorted_cd_directories(
        &ParsedToken::new("$RELVAR/"),
        MatchStrategy::CaseInsensitive,
        &ctx,
    ))
    .into_iter()
    .map(|m| m.suggestion.display.to_string())
    .collect();

    assert_eq!(displays, vec!["from-cdpath/"]);
}
