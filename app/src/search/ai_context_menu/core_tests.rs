#[cfg(not(target_family = "wasm"))]
use repo_metadata::repositories::DetectedRepositories;
use warpui::App;

use super::{
    AIContextMenuCategory, AtContextMenuCoreState, AtContextMenuGates,
    AtContextMenuQueryTransition, MAX_CONSECUTIVE_EMPTY_RESULTS_EVENTS, NavigationState,
    is_at_menu_trigger, shared_inserted_text, should_close_at_menu,
};
use crate::cloud_object::ObjectType;
use crate::code_review::diff_state::DiffMode;
use crate::search::ai_context_menu::mixer::AIContextMenuSearchableAction;
use crate::test_util::settings::initialize_settings_for_tests;

fn register_core_dependencies(app: &mut App) {
    initialize_settings_for_tests(app);
    #[cfg(not(target_family = "wasm"))]
    app.add_singleton_model(|_| DetectedRepositories::default());
}

/// Gates for an input locked to the shell, whose category list is the only one
/// that no feature flag varies.
fn terminal_mode_gates() -> AtContextMenuGates {
    AtContextMenuGates {
        is_ai_or_autodetect_mode: false,
        ..Default::default()
    }
}

#[test]
fn opens_only_on_an_at_symbol_at_a_word_boundary() {
    // (buffer, offset just past the typed character, opens the menu)
    let cases = [
        ("@", 1, true),
        ("look at @", 9, true),
        ("(@", 2, true),
        ("user@", 5, false),
        ("react18@", 8, false),
        ("", 0, false),
        ("abc", 3, false),
        ("@abc", 4, false),
    ];

    for (buffer, cursor, expected) in cases {
        assert_eq!(
            is_at_menu_trigger(buffer, cursor),
            expected,
            "buffer {buffer:?}, cursor {cursor}"
        );
    }
}

#[test]
fn closes_when_the_filter_breaks_or_the_cursor_leaves_the_at_symbol() {
    // (buffer, cursor, offset of the `@`, closes the menu)
    let cases = [
        ("@file", 5, 0, false),
        ("look at @file", 13, 8, false),
        ("@my file", 8, 0, false),
        ("look at @file", 3, 8, true),
        ("@my  file", 9, 0, true),
        ("@my\nfile", 8, 0, true),
        ("plain text", 10, 0, true),
    ];

    for (buffer, cursor, at_symbol_position, expected) in cases {
        assert_eq!(
            should_close_at_menu(buffer, cursor, at_symbol_position),
            expected,
            "buffer {buffer:?}, cursor {cursor}, at {at_symbol_position}"
        );
    }
}

#[test]
fn formats_the_text_each_action_inserts() {
    let cases = [
        (
            AIContextMenuSearchableAction::InsertText {
                text: "@web".to_owned(),
            },
            Some("@web"),
        ),
        (
            AIContextMenuSearchableAction::InsertDriveObject {
                object_type: ObjectType::Workflow,
                object_uid: "abc123".to_owned(),
            },
            Some("<workflow:abc123>"),
        ),
        (
            AIContextMenuSearchableAction::InsertPlan {
                ai_document_uid: "doc-1".to_owned(),
            },
            Some("<plan:doc-1>"),
        ),
        (
            AIContextMenuSearchableAction::InsertConversation {
                conversation_id: "convo-1".to_owned(),
            },
            Some("<convo:convo-1>"),
        ),
        (
            AIContextMenuSearchableAction::InsertSkill {
                name: "create-pr".to_owned(),
            },
            Some("/create-pr"),
        ),
        // The remaining two need surface-specific handling, so the core declines
        // them: file paths are rewritten against the session's working directory
        // in shell mode, and diff sets attach context instead of inserting text.
        (
            AIContextMenuSearchableAction::InsertFilePath {
                file_path: "src/main.rs".to_owned(),
            },
            None,
        ),
        (
            AIContextMenuSearchableAction::InsertDiffSet {
                diff_mode: DiffMode::Head,
            },
            None,
        ),
    ];

    for (action, expected) in cases {
        assert_eq!(
            shared_inserted_text(&action).as_deref(),
            expected,
            "action {action:?}"
        );
    }
}

#[test]
fn closes_after_repeated_empty_results_but_never_on_the_category_list() {
    let mut core = AtContextMenuCoreState::new(AtContextMenuGates::default());

    // The category list filters locally and installs no mixer sources, so empty
    // mixer results there say nothing about the user's progress.
    for _ in 0..MAX_CONSECUTIVE_EMPTY_RESULTS_EVENTS * 2 {
        assert!(!core.record_results_update(true, false, true));
    }

    core.enter_category(AIContextMenuCategory::Blocks);
    for _ in 0..MAX_CONSECUTIVE_EMPTY_RESULTS_EVENTS - 1 {
        assert!(!core.record_results_update(true, false, true));
    }

    assert!(core.record_results_update(true, false, true));
}

#[test]
fn only_counts_settled_empty_results_from_a_narrowing_query() {
    let mut core = AtContextMenuCoreState::new(AtContextMenuGates::default());
    core.enter_category(AIContextMenuCategory::Blocks);

    for _ in 0..MAX_CONSECUTIVE_EMPTY_RESULTS_EVENTS * 2 {
        // Still loading, so an empty result set means nothing yet.
        assert!(!core.record_results_update(true, true, true));
        // The user rewrote the query rather than narrowing it.
        assert!(!core.record_results_update(true, false, false));
        // Results came back.
        assert!(!core.record_results_update(false, false, true));
    }
}

#[test]
fn offers_only_current_folder_files_in_terminal_mode_without_a_repo() {
    App::test((), |mut app| async move {
        register_core_dependencies(&mut app);
        let mut core = AtContextMenuCoreState::new(terminal_mode_gates());

        let categories = app.read(|ctx| core.available_categories(ctx));
        app.read(|ctx| core.refresh_categories(ctx));

        assert_eq!(categories, vec![AIContextMenuCategory::CurrentFolderFiles]);
        // A single category has no list worth showing, so the menu opens into it.
        assert_eq!(
            core.navigation_state(),
            NavigationState::Category(AIContextMenuCategory::CurrentFolderFiles)
        );
    });
}

#[test]
fn falls_through_to_all_categories_only_when_no_category_name_matches() {
    App::test((), |mut app| async move {
        register_core_dependencies(&mut app);
        let mut core = AtContextMenuCoreState::new(terminal_mode_gates());

        let matched = app.read(|ctx| core.set_query("files", ctx));
        assert_eq!(matched, AtContextMenuQueryTransition::SourcesUnchanged);
        assert_eq!(core.navigation_state(), NavigationState::MainMenu);

        let unmatched = app.read(|ctx| core.set_query("zzzznomatch", ctx));
        assert_eq!(
            unmatched,
            AtContextMenuQueryTransition::EnteredAllCategories
        );
        assert_eq!(core.navigation_state(), NavigationState::AllCategories);
    });
}

#[test]
fn wraps_category_selection_at_both_ends() {
    App::test((), |mut app| async move {
        register_core_dependencies(&mut app);
        // AI mode always offers files, blocks, and skills, so the list is long
        // enough to wrap no matter how the optional categories are gated.
        let mut core = AtContextMenuCoreState::new(AtContextMenuGates::default());
        let count = app.read(|ctx| core.filtered_categories(ctx).len());
        assert!(count > 1, "expected AI mode to offer several categories");

        app.read(|ctx| core.select_previous_category(ctx));
        assert_eq!(core.selected_category_index(), count - 1);

        app.read(|ctx| core.select_next_category(ctx));
        assert_eq!(core.selected_category_index(), 0);
    });
}
