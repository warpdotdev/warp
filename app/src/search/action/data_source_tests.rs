use std::sync::Arc;

use warpui::keymap::BindingDescription;

use super::{ActionSearcher, FuzzyActionSearcher};
use crate::search::command_palette::mixer::CommandPaletteItemAction;
use crate::util::bindings::CommandBinding;

fn go_back_binding() -> CommandBinding {
    let mut binding = CommandBinding::new(
        "workspace:navigate_back".to_string(),
        "Go Back".to_string(),
        None,
    );
    binding.description =
        BindingDescription::new("Go Back").with_search_keywords(["navigate", "history"]);
    binding
}

fn searcher_with_binding(binding: CommandBinding) -> FuzzyActionSearcher {
    let mut searcher = FuzzyActionSearcher {
        all_bindings: Default::default(),
    };
    let binding = Arc::new(binding);
    searcher.all_bindings.insert(binding.id, binding);
    searcher
}

#[test]
fn test_fuzzy_search_matches_search_keywords() {
    let searcher = searcher_with_binding(go_back_binding());

    let results = searcher.search("history").expect("search should succeed");
    assert_eq!(results.len(), 1);
    let CommandPaletteItemAction::AcceptBinding { binding } = results[0].accept_result() else {
        panic!("expected an AcceptBinding action");
    };
    assert_eq!(binding.name, "workspace:navigate_back");

    let results = searcher
        .search("unrelated query")
        .expect("search should succeed");
    assert!(results.is_empty());
}

#[cfg(not(target_family = "wasm"))]
#[test]
fn test_full_text_search_matches_search_keywords() {
    use super::full_text_searcher::FullTextActionSearcher;

    let mut searcher = FullTextActionSearcher::new();
    let binding = Arc::new(go_back_binding());
    searcher.bindings_mut().insert(binding.id, binding);
    searcher.build_index();

    let results = searcher.search("history").expect("search should succeed");
    assert_eq!(results.len(), 1);
    let CommandPaletteItemAction::AcceptBinding { binding } = results[0].accept_result() else {
        panic!("expected an AcceptBinding action");
    };
    assert_eq!(binding.name, "workspace:navigate_back");

    let results = searcher.search("unrelated").expect("search should succeed");
    assert!(results.is_empty());
}
