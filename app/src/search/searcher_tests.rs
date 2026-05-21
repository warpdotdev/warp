use crate::define_search_schema;
use crate::search::searcher::{CustomTokenizer, MIN_MEMORY_BUDGET};
use itertools::Itertools;
use std::sync::Arc;
use tantivy::tokenizer::{TextAnalyzer, Token};
use warpui::r#async::executor::Background;

fn token_stream_helper(text: &str) -> Vec<Token> {
    let mut a = TextAnalyzer::from(CustomTokenizer::default());
    let mut token_stream = a.token_stream(text);
    let mut tokens: Vec<Token> = vec![];
    let mut add_token = |token: &Token| {
        tokens.push(token.clone());
    };
    token_stream.process(&mut add_token);
    tokens
}

fn assert_token(token: &Token, position: usize, text: &str, from: usize, to: usize) {
    assert_eq!(
        token.position, position,
        "expected position {position} but {token:?}"
    );
    assert_eq!(token.text, text, "expected text {text} but {token:?}");
    assert_eq!(
        token.offset_from, from,
        "expected offset_from {from} but {token:?}"
    );
    assert_eq!(token.offset_to, to, "expected offset_to {to} but {token:?}");
}

#[test]
fn test_tokenizer_simple() {
    let tokens = token_stream_helper("Hello, happy tax payer!");
    assert_eq!(tokens.len(), 4);
    assert_token(&tokens[0], 0, "Hello", 0, 5);
    assert_token(&tokens[1], 1, "happy", 7, 12);
    assert_token(&tokens[2], 2, "tax", 13, 16);
    assert_token(&tokens[3], 3, "payer", 17, 22);
}

#[test]
fn test_tokenizer_warp_special_chars() {
    // Test string includes warp-related terms with hyphen, underscore, forward slash, backslash, and colon
    let test_string = "warp-cli/launch_command:run C:\\\\Program_Files\\\\Warp\\\\core-engine.dll check_status:/dev/warp_drive-0";
    let tokens = token_stream_helper(test_string);

    assert_eq!(tokens.len(), 25);
    assert_token(&tokens[0], 0, "warp-cli/launch_command:run", 0, 27);
    assert_token(&tokens[1], 1, "warp", 0, 4);
    assert_token(&tokens[2], 2, "cli", 5, 8);
    assert_token(&tokens[3], 3, "launch_command", 9, 23);
    assert_token(&tokens[4], 4, "launch", 9, 15);
    assert_token(&tokens[5], 5, "command", 16, 23);
    assert_token(&tokens[6], 6, "run", 24, 27);
    assert_token(
        &tokens[7],
        7,
        "C:\\\\Program_Files\\\\Warp\\\\core-engine",
        28,
        64,
    );
    assert_token(&tokens[15], 15, "dll", 65, 68);
    assert_token(&tokens[16], 16, "check_status:/dev/warp_drive-0", 69, 99);
}

#[test]
fn test_searcher() {
    define_search_schema!(
        schema_name: TEST_SCHEMA,
        config_name: SchemaConfig,
        search_doc: SearchDoc,
        identifying_doc: IdentifyingDoc,
        search_fields: [name: 1.0],
        id_fields: [id: u64]
    );
    let search_strings = ["run warp on web server", "run warp-on-web server"];

    let searcher = TEST_SCHEMA.create_searcher(MIN_MEMORY_BUDGET);
    searcher
        .build_index(
            search_strings
                .iter()
                .enumerate()
                .map(|(id, name)| SearchDoc {
                    name: (*name).to_owned(),
                    id: id as u64,
                }),
        )
        .unwrap();

    let result = searcher.search_full_doc("warp on web").unwrap();
    assert_eq!(
        result.len(),
        2,
        "both search strings should match with the custom tokenizer"
    );
    assert_eq!(
        result[0].highlights.name,
        vec![4, 5, 6, 7, 9, 10, 12, 13, 14],
        "should highlight the correct positions"
    );
    assert_eq!(
        result[1].highlights.name,
        vec![4, 5, 6, 7, 9, 10, 12, 13, 14],
        "should highlight the correct positions"
    );

    let result = searcher.search_full_doc("warp-on-web").unwrap();
    assert_eq!(
        result.len(),
        1,
        "should only match the second search string"
    );
    assert_eq!(
        result[0].values.name, "run warp-on-web server",
        "should match the second search string"
    );
    assert_eq!(
        result[0].highlights.name,
        (4..15).collect_vec(),
        "should highlight the correct positions"
    );
}

#[test]
fn test_searcher_scores() {
    define_search_schema!(
        schema_name: TEST_SCHEMA,
        config_name: SchemaConfig,
        search_doc: SearchDoc,
        identifying_doc: IdentifyingDoc,
        search_fields: [name: 1.0],
        id_fields: [id: u64]
    );

    let search_strings = ["run warp on web server", "run warp_on_web:server"];

    let searcher = TEST_SCHEMA.create_searcher(MIN_MEMORY_BUDGET);
    searcher
        .build_index(
            search_strings
                .iter()
                .enumerate()
                .map(|(id, name)| SearchDoc {
                    name: (*name).to_owned(),
                    id: id as u64,
                }),
        )
        .unwrap();

    let result = searcher.search_full_doc("warp").unwrap();
    assert_eq!(
        result.len(),
        2,
        "both search strings should match with the custom tokenizer"
    );
    let score_delta = result[0].score - result[1].score;
    assert!(
        score_delta > 0.0,
        "the first search string should have a higher score than the second"
    );
    assert!(
        score_delta / result[0].score < 0.15,
        "the score difference of similar strings should be less than 15%"
    );

    let result = searcher.search_full_doc("warp on web").unwrap();
    let score_delta = result[0].score - result[1].score;
    assert!(
        score_delta / result[0].score < 0.15,
        "the score difference of similar strings should be less than 15%"
    );
}

#[test]
fn test_searcher_async() {
    define_search_schema!(
        schema_name: TEST_SCHEMA,
        config_name: SchemaConfig,
        search_doc: SearchDoc,
        identifying_doc: IdentifyingDoc,
        search_fields: [name: 1.0],
        id_fields: [id: u64]
    );

    let search_strings = [
        "Fix clippy formatting after commit",
        "Undo the last git commit",
        "Run cargo fmt on changed files",
        "Run warp-on-web",
        "Run fresh warp-local and clear warp-dev permissions",
        "Give user unlimited AI",
    ];
    let background_executor = Arc::new(Background::default());
    let searcher_async =
        TEST_SCHEMA.create_async_searcher(MIN_MEMORY_BUDGET, background_executor.clone());
    searcher_async
        .build_index_async(
            search_strings
                .iter()
                .enumerate()
                .map(|(id, name)| SearchDoc {
                    name: (*name).to_owned(),
                    id: id as u64,
                }),
        )
        .unwrap();
    std::thread::sleep(std::time::Duration::from_millis(200));

    let result = searcher_async.get_all_doc_ids().unwrap();
    assert_eq!(
        result.len(),
        6,
        "the index should be populated with all documents"
    );

    let result = searcher_async.search_full_doc("unlimited").unwrap();
    assert_eq!(
        result.len(),
        1,
        "there should be exactly 1 match for 'unlimited'"
    );
    assert_eq!(
        result[0].values.name, "Give user unlimited AI",
        "should match the search string"
    );
    assert_eq!(
        result[0].highlights.name,
        (10..19).collect_vec(),
        "should highlight the correct positions"
    );
    let result = searcher_async.search_id("Fix clippy formatting").unwrap();
    assert!(!result.is_empty(), "the document should exist");

    searcher_async
        .delete_document_async(IdentifyingDoc { id: 0 })
        .unwrap();
    searcher_async
        .delete_document_async(IdentifyingDoc { id: 1 })
        .unwrap();
    searcher_async
        .insert_document_async(SearchDoc {
            name: "Undo the last git commit".to_owned(),
            id: 10,
        })
        .unwrap();
    std::thread::sleep(std::time::Duration::from_millis(200));

    let result = searcher_async.search_id("Fix clippy formatting").unwrap();
    assert!(result.is_empty(), "the document should be deleted");

    let result = searcher_async.search_full_doc("Undo").unwrap();
    assert_eq!(
        result.len(),
        1,
        "there should be exactly 1 match for 'Undo'"
    );
    assert_eq!(
        result[0].values.id, 10,
        "a new document should be inserted with id = 10"
    );
    assert_eq!(
        result[0].highlights.name,
        (0..4).collect_vec(),
        "should highlight the correct positions"
    );

    let result = searcher_async
        .get_all_documents()
        .unwrap()
        .into_iter()
        .filter(|doc| doc.id == 4)
        .collect_vec();
    assert_eq!(result.len(), 1, "there should be exactly 1 match for id 4");
    assert_eq!(
        result[0].name, "Run fresh warp-local and clear warp-dev permissions",
        "the original document with id 4 should be unchanged"
    );

    searcher_async
        .insert_document_async(SearchDoc {
            name: "Updated name".to_owned(),
            id: 4,
        })
        .unwrap();
    std::thread::sleep(std::time::Duration::from_millis(200));

    let result = searcher_async
        .get_all_documents()
        .unwrap()
        .into_iter()
        .filter(|doc| doc.id == 4)
        .collect_vec();
    assert_eq!(result.len(), 1, "there should be exactly 1 match for id 4");
    assert_eq!(
        result[0].name, "Updated name",
        "the document with id 4 should be updated on insert"
    );

    searcher_async.clear_search_index_async().unwrap();
    std::thread::sleep(std::time::Duration::from_millis(200));

    let result = searcher_async.get_all_doc_ids().unwrap();
    assert_eq!(
        result.len(),
        0,
        "the index should be cleared and contain no documents"
    );
}
