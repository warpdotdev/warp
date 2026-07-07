use std::collections::HashSet;
use std::path::PathBuf;

use ai::index::Symbol;

#[cfg(test)]
use super::*;

fn create_test_symbol(name: &str, type_prefix: Option<&str>) -> CodeSymbol {
    CodeSymbol {
        file_path: PathBuf::from("test.rs"),
        symbol: Symbol {
            name: name.to_string(),
            type_prefix: type_prefix.map(|s| s.to_string()),
            comment: None,
            line_number: 1,
        },
    }
}

fn create_test_symbol_in_file(
    name: &str,
    type_prefix: Option<&str>,
    file_path: &str,
) -> CodeSymbol {
    CodeSymbol {
        file_path: PathBuf::from(file_path),
        symbol: Symbol {
            name: name.to_string(),
            type_prefix: type_prefix.map(|s| s.to_string()),
            comment: None,
            line_number: 1,
        },
    }
}

fn search_code_symbols(symbols: &[CodeSymbol], query: &str) -> Vec<CodeSearchItem> {
    if query.is_empty() {
        return Vec::new();
    }

    symbols
        .iter()
        .map(|symbol| {
            let match_result = fuzzy_match_symbol_with_type(symbol, query);
            CodeSearchItem {
                code_symbol: symbol.clone(),
                match_result,
            }
        })
        .collect()
}

#[test]
fn test_fuzzy_match_symbol_with_type_basic_functionality() {
    let symbol = create_test_symbol("my_function", Some("fn"));

    let name_match = fuzzy_match_symbol_with_type(&symbol, "function");
    let type_match = fuzzy_match_symbol_with_type(&symbol, "fn");
    let combined_match = fuzzy_match_symbol_with_type(&symbol, "fn my_function");
    let no_match = fuzzy_match_symbol_with_type(&symbol, "xyz");

    assert!(name_match.score > 0);
    assert!(type_match.score > 0);
    assert!(combined_match.score > 0);
    assert_eq!(no_match.score, 0);
}

#[test]
fn test_fuzzy_match_symbol_with_type_no_type_handling() {
    let symbol = create_test_symbol("some_variable", None);

    let name_match = fuzzy_match_symbol_with_type(&symbol, "variable");
    let no_match = fuzzy_match_symbol_with_type(&symbol, "xyz");

    assert!(name_match.score > 0);
    assert_eq!(no_match.score, 0);
}

#[test]
fn test_symbol_cache_creation() {
    let symbols = vec![
        create_test_symbol("my_function", Some("fn")),
        create_test_symbol("MyStruct", Some("struct")),
        create_test_symbol("global_var", None),
        create_test_symbol("another_function", Some("fn")),
    ];

    let cache = SymbolCache::new(symbols);

    assert_eq!(cache.symbols.len(), 4);

    let symbol_names: Vec<&str> = cache
        .symbols
        .iter()
        .map(|s| s.symbol.name.as_str())
        .collect();
    assert!(symbol_names.contains(&"my_function"));
    assert!(symbol_names.contains(&"MyStruct"));
    assert!(symbol_names.contains(&"global_var"));
    assert!(symbol_names.contains(&"another_function"));
}

#[test]
fn test_search_code_symbols_basic_functionality() {
    let symbols = vec![
        create_test_symbol("my_function", Some("fn")),
        create_test_symbol("MyStruct", Some("struct")),
        create_test_symbol("global_var", None),
    ];

    let results = search_code_symbols(&symbols, "function");
    assert!(!results.is_empty());
    assert!(results
        .iter()
        .any(|r| r.code_symbol.symbol.name == "my_function"));

    let results = search_code_symbols(&symbols, "fn");
    assert!(!results.is_empty());
    assert!(results
        .iter()
        .any(|r| r.code_symbol.symbol.name == "my_function"));

    let results = search_code_symbols(&symbols, "fn function");
    assert!(!results.is_empty());
    assert!(results
        .iter()
        .any(|r| r.code_symbol.symbol.name == "my_function"));
}

#[test]
fn test_search_code_symbols_all_symbols_searched() {
    let symbols = vec![
        create_test_symbol("process_data", Some("fn")),
        create_test_symbol("DataProcessor", Some("struct")),
        create_test_symbol("my_variable", None),
    ];

    let results = search_code_symbols(&symbols, "data");

    assert!(results.len() >= 2);
    let found_names: Vec<&str> = results
        .iter()
        .map(|r| r.code_symbol.symbol.name.as_str())
        .collect();
    assert!(found_names.contains(&"process_data"));
    assert!(found_names.contains(&"DataProcessor"));
}

#[test]
fn test_search_code_symbols_empty_query() {
    let symbols = vec![
        create_test_symbol("my_function", Some("fn")),
        create_test_symbol("MyStruct", Some("struct")),
    ];

    let results = search_code_symbols(&symbols, "");
    assert!(results.is_empty());
}

#[test]
fn test_search_code_symbols_no_matches() {
    let symbols = vec![
        create_test_symbol("my_function", Some("fn")),
        create_test_symbol("MyStruct", Some("struct")),
    ];

    let results = search_code_symbols(&symbols, "nonexistent");
    assert_eq!(results.len(), 2);

    for result in results {
        assert_eq!(result.match_result.score, 0);
    }
}

#[test]
fn test_search_code_symbols_untyped_symbols() {
    let symbols = vec![
        create_test_symbol("my_function", Some("fn")),
        create_test_symbol("my_variable", None),
    ];

    let results = search_code_symbols(&symbols, "variable");
    assert!(!results.is_empty());
    assert!(results
        .iter()
        .any(|r| r.code_symbol.symbol.name == "my_variable"));
}

#[cfg(not(target_family = "wasm"))]
#[test]
fn test_finalize_zero_state_git_changed_first() {
    let items = vec![
        CodeSearchItem {
            code_symbol: create_test_symbol_in_file("unchanged_fn", Some("fn"), "src/lib.rs"),
            match_result: FuzzyMatchResult::no_match(),
        },
        CodeSearchItem {
            code_symbol: create_test_symbol_in_file("changed_fn", Some("fn"), "src/changed.rs"),
            match_result: FuzzyMatchResult::no_match(),
        },
        CodeSearchItem {
            code_symbol: create_test_symbol_in_file("another_fn", Some("fn"), "src/other.rs"),
            match_result: FuzzyMatchResult::no_match(),
        },
    ];
    let git_changed_files = HashSet::from(["src/changed.rs".to_string()]);

    let results = finalize_zero_state(items, &git_changed_files);

    assert_eq!(results.len(), 3);
    assert!(results[0].score() > results[1].score());
}

#[cfg(not(target_family = "wasm"))]
#[test]
fn test_finalize_query_returns_top_results() {
    let items: Vec<CodeSearchItem> = vec![
        CodeSearchItem {
            code_symbol: create_test_symbol("my_function", Some("fn")),
            match_result: fuzzy_match_symbol_with_type(
                &create_test_symbol("my_function", Some("fn")),
                "function",
            ),
        },
        CodeSearchItem {
            code_symbol: create_test_symbol("MyStruct", Some("struct")),
            match_result: fuzzy_match_symbol_with_type(
                &create_test_symbol("MyStruct", Some("struct")),
                "function",
            ),
        },
        CodeSearchItem {
            code_symbol: create_test_symbol("unrelated_var", None),
            match_result: fuzzy_match_symbol_with_type(
                &create_test_symbol("unrelated_var", None),
                "function",
            ),
        },
    ];

    let results = finalize_query(items);

    let best = results.iter().max_by_key(|r| r.score()).unwrap();
    assert_eq!(
        best.accept_result(),
        AIContextMenuSearchableAction::InsertText {
            text: "fn my_function in test.rs:1".to_string()
        }
    );
}

#[test]
fn test_fuzzy_match_code_symbols_3x_multiplier() {
    let symbol = create_test_symbol("my_function", Some("fn"));

    let match_result = fuzzy_match_symbol_with_type(&symbol, "function");

    // The score should be 3x the raw fuzzy match score.
    // We can verify the multiplier is applied by checking score > 0
    // and that it's divisible by 3 (since raw scores are integers).
    assert!(match_result.score > 0);
    assert_eq!(match_result.score % 3, 0);
}

#[cfg(not(target_family = "wasm"))]
#[test]
fn test_finalize_zero_state_respects_max_results() {
    let items: Vec<CodeSearchItem> = (0..300)
        .map(|i| CodeSearchItem {
            code_symbol: create_test_symbol_in_file(&format!("sym_{i}"), Some("fn"), "src/main.rs"),
            match_result: FuzzyMatchResult::no_match(),
        })
        .collect();

    let results = finalize_zero_state(items, &HashSet::new());

    assert_eq!(results.len(), 200);
}
