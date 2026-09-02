use std::sync::Arc;
use std::time::Duration;

use instant::Instant;
use itertools::Itertools;
use parking_lot::Mutex;
use tantivy::tokenizer::{TextAnalyzer, Token};
use warpui_core::r#async::executor::Background;

use crate::define_search_schema;
use crate::searcher::{
    AsyncSearcher, CustomTokenizer, FullTextSearchDocumentEntry, FullTextSearchFieldValue,
    MIN_MEMORY_BUDGET, PendingRebuild, QueuedItem, SearchDocumentEntry, SearchSchemaConfig,
    SearcherEvent, SearcherProducerState, SimpleFullTextSearcher, merge_with_rebuild,
};

/// Builds an [`AsyncSearcher`] with no background writer draining its channel, so a test can
/// drive the real async write API and then decide exactly when the queued work is applied, with
/// [`drain_pending_chunks`] and [`apply_chunks`] standing in for the background writer.
///
/// The returned receiver is the events channel the background writer would own. Nothing reads
/// from it; it is handed back so the test can keep it alive, since publishing fails once it is
/// closed, and so [`drain_pending_chunks`] (or the test itself) can read back what was published.
fn async_searcher_without_background_writer<C: SearchSchemaConfig>(
    searcher: SimpleFullTextSearcher<C>,
) -> (AsyncSearcher<C>, async_channel::Receiver<QueuedItem>) {
    let (tx, rx) = async_channel::unbounded();
    let producer_state = Arc::new(Mutex::new(SearcherProducerState {
        next_sequence: 0,
        pending_rebuild: None,
    }));
    (
        AsyncSearcher {
            searcher,
            tx,
            producer_state,
        },
        rx,
    )
}

/// Drains everything published so far -- both the events channel and the pending rebuild slot --
/// and merges it into per-commit chunks, exactly as the background writer does with a batch it
/// has drained. Mirrors the real writer's take-before-drain order (see `process_searcher_events`).
fn drain_pending_chunks<C: SearchSchemaConfig>(
    searcher: &AsyncSearcher<C>,
    events_rx: &async_channel::Receiver<QueuedItem>,
) -> Vec<Vec<SearcherEvent>> {
    let rebuild = searcher.producer_state.lock().pending_rebuild.take();
    let mut batch = Vec::new();
    while let Ok(item) = events_rx.try_recv() {
        if let QueuedItem::Event(event) = item {
            batch.push(event);
        }
    }
    merge_with_rebuild(batch, rebuild)
}

/// Applies `chunks` through the synchronous writer, committing each chunk on its own, exactly as
/// the background writer does.
fn apply_chunks<C: SearchSchemaConfig>(
    searcher: &AsyncSearcher<C>,
    chunks: Vec<Vec<SearcherEvent>>,
) {
    for chunk in chunks {
        searcher
            .searcher
            .writer
            .lock()
            .execute_operations(chunk)
            .unwrap();
    }
}

/// Renders resolved events as readable operations, so an ordering assertion fails with a legible
/// diff. Inserted documents are identified by their `name` field.
fn describe_events(events: &[SearcherEvent]) -> Vec<String> {
    events
        .iter()
        .map(|event| match event {
            SearcherEvent::IndexCleared => "clear".to_owned(),
            SearcherEvent::DocumentInserted(entry) => format!("insert {}", document_name(entry)),
            SearcherEvent::DocumentDeleted(entry) => format!("delete {entry:?}"),
        })
        .collect()
}

fn document_name(entry: &FullTextSearchDocumentEntry) -> String {
    match entry.get("name") {
        Some(FullTextSearchFieldValue::Str(name)) => name.clone(),
        other => panic!("expected a string `name` field, got {other:?}"),
    }
}

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

/// Regression test for a memory leak where a burst of full-index rebuild requests (e.g. from a
/// filesystem watcher repeatedly firing config-reload events) would each enqueue a full
/// clear-and-reinsert of every document on `AsyncSearcher`'s unbounded background channel, with
/// nothing to coalesce or bound them. A sustained burst would grow the channel's backing queue
/// without limit. `rebuild_index_async` must ensure that only the most recently requested
/// document set is ever queued, and that the index still ends up with the correct final content.
///
/// The burst is fired through the real `rebuild_index_async` API, but at a searcher with no
/// background writer attached, so the test observes exactly what the burst left queued and then
/// applies precisely that itself. Both halves are therefore deterministic: the coalescing bound
/// is read off the queue rather than inferred, and the resulting index content is read back after
/// a known set of commits instead of waiting on a background thread to converge.
#[test]
fn test_searcher_async_rebuild_coalesces_burst() {
    define_search_schema!(
        schema_name: TEST_SCHEMA,
        config_name: SchemaConfig,
        search_doc: SearchDoc,
        identifying_doc: IdentifyingDoc,
        search_fields: [name: 1.0],
        id_fields: [id: u64]
    );

    const BURST_SIZE: usize = 500;
    const DOCS_PER_REBUILD: u64 = 50;

    let (searcher_async, events_rx) =
        async_searcher_without_background_writer(TEST_SCHEMA.create_searcher(MIN_MEMORY_BUDGET));

    for burst in 0..BURST_SIZE {
        let documents = (0..DOCS_PER_REBUILD).map(|id| SearchDoc {
            name: format!("burst {burst} doc {id}"),
            id,
        });
        searcher_async.rebuild_index_async(documents).unwrap();
    }

    // However many rebuilds were requested, only the most recently requested document set should
    // ever be retained, and the wake-up marker itself must stay coalesced to at most one queued
    // item rather than one per request -- see `QueuedItem::RebuildMarker`.
    let mut queued_items = Vec::new();
    while let Ok(item) = events_rx.try_recv() {
        queued_items.push(item);
    }
    assert!(
        queued_items
            .iter()
            .all(|item| matches!(item, QueuedItem::RebuildMarker)),
        "a pure burst of rebuild requests should never publish a real event to the events channel"
    );
    assert!(
        queued_items.len() <= 1,
        "a burst of {BURST_SIZE} rebuild requests should coalesce their wake-up marker to at most one queued item, found {}",
        queued_items.len()
    );
    {
        let state = searcher_async.producer_state.lock();
        let rebuild = state
            .pending_rebuild
            .as_ref()
            .expect("a rebuild should be pending");
        assert_eq!(
            rebuild.documents.len(),
            DOCS_PER_REBUILD as usize,
            "the pending rebuild should hold a single snapshot of {DOCS_PER_REBUILD} documents, not one accumulated from all {BURST_SIZE} requests"
        );
    }

    let chunks = drain_pending_chunks(&searcher_async, &events_rx);
    assert_eq!(
        chunks.len(),
        1,
        "the coalesced rebuild should resolve to a single commit"
    );
    assert_eq!(
        chunks[0].len(),
        DOCS_PER_REBUILD as usize + 1,
        "the rebuild should expand to a clear plus one insert per document of a single snapshot, not of all {BURST_SIZE} requested ones"
    );
    assert!(
        matches!(chunks[0].first(), Some(SearcherEvent::IndexCleared)),
        "a rebuild must clear the index before re-inserting its snapshot"
    );

    apply_chunks(&searcher_async, chunks);

    // The snapshot that survives must be the most recently requested one. Every burst produces
    // the same document count, so the document names are what distinguish it.
    let documents = searcher_async.get_all_documents().unwrap();
    assert_eq!(
        documents.len(),
        DOCS_PER_REBUILD as usize,
        "the index should hold exactly the final rebuild's snapshot, got: {documents:?}"
    );
    let last_burst_prefix = format!("burst {} doc ", BURST_SIZE - 1);
    assert!(
        documents
            .iter()
            .all(|doc| doc.name.starts_with(&last_burst_prefix)),
        "every document should come from the final rebuild's snapshot, got: {documents:?}"
    );
}

/// Regression test for a correctness bug in an earlier version of the rebuild coalescer: when a
/// second rebuild superseded a first, not-yet-applied rebuild, the coalescer reused the first
/// rebuild's position in the operation queue for the second rebuild's (newer) document set. Any
/// insert/delete call made in between -- as Warp Drive's per-object updates do -- ended up placed
/// *after* the superseding rebuild in the resolved operation list, even though it was requested
/// *before* that rebuild. Because inserts overwrite by composite key, the stale interleaved
/// update would silently win over the newer rebuild's value for the same document.
///
/// `rebuild_index_async` must instead preserve request order: an insert/delete made before a
/// rebuild must never be applied after (and so clobber) that rebuild, and one made after a
/// rebuild must never be silently overwritten by it.
///
/// Request order is asserted directly -- on the queue, and on the per-commit chunks it resolves
/// into -- at a searcher with no background writer attached. Those chunks are then applied
/// through the synchronous writer and the index read back, which is what covers the Tantivy
/// semantics that force a rebuild into a commit of its own: `delete_all_documents` only removes
/// already-committed documents, so an insert requested before a rebuild has to be committed
/// before the rebuild's clear runs, or it survives that clear.
#[test]
fn test_searcher_async_rebuild_preserves_operation_order_with_interleaved_updates() {
    define_search_schema!(
        schema_name: TEST_SCHEMA,
        config_name: SchemaConfig,
        search_doc: SearchDoc,
        identifying_doc: IdentifyingDoc,
        search_fields: [name: 1.0],
        id_fields: [id: u64]
    );

    let (searcher_async, events_rx) =
        async_searcher_without_background_writer(TEST_SCHEMA.create_searcher(MIN_MEMORY_BUDGET));

    let indexed_documents = || {
        searcher_async
            .get_all_documents()
            .unwrap()
            .into_iter()
            .map(|doc| (doc.id, doc.name))
            .sorted()
            .collect_vec()
    };

    // Request a rebuild (R1), then -- before the background writer gets a chance to apply it --
    // incremental updates land (as Warp Drive's per-object insert calls do), then a second
    // rebuild (R2) is requested whose snapshot holds a newer value for document 1 and no longer
    // contains document 2 at all. R2 supersedes R1 in the coalescer, but the interleaved updates
    // must still be treated as older than R2, since they were requested before it.
    searcher_async
        .rebuild_index_async([SearchDoc {
            name: "r1 stale".to_owned(),
            id: 1,
        }])
        .unwrap();
    searcher_async
        .insert_document_async(SearchDoc {
            name: "stale interleaved update".to_owned(),
            id: 1,
        })
        .unwrap();
    searcher_async
        .insert_document_async(SearchDoc {
            name: "interleaved doc dropped by r2".to_owned(),
            id: 2,
        })
        .unwrap();
    searcher_async
        .rebuild_index_async([SearchDoc {
            name: "r2 fresh".to_owned(),
            id: 1,
        }])
        .unwrap();

    {
        let state = searcher_async.producer_state.lock();
        let rebuild = state
            .pending_rebuild
            .as_ref()
            .expect("the newer rebuild (r2) should be pending, replacing r1");
        assert_eq!(
            rebuild.documents.len(),
            1,
            "r2's snapshot should hold exactly the document it was requested with, not r1's"
        );
    }

    let chunks = drain_pending_chunks(&searcher_async, &events_rx);
    assert_eq!(
        chunks
            .iter()
            .map(|chunk| describe_events(chunk))
            .collect_vec(),
        vec![
            vec![
                "insert stale interleaved update".to_owned(),
                "insert interleaved doc dropped by r2".to_owned(),
            ],
            vec!["clear".to_owned(), "insert r2 fresh".to_owned()],
        ],
        "the interleaved inserts (requested before r2) should be committed before, and separately from, the newer rebuild"
    );

    apply_chunks(&searcher_async, chunks);
    assert_eq!(
        indexed_documents(),
        vec![(1, "r2 fresh".to_owned())],
        "the newer rebuild must win over the interleaved updates that preceded it"
    );

    // Conversely, an update requested *after* a rebuild must not be silently overwritten by it,
    // even when the two are drained into the same batch: it is the more recent request, so it has
    // to be applied after the rebuild.
    searcher_async
        .rebuild_index_async([SearchDoc {
            name: "r3 rebuild".to_owned(),
            id: 1,
        }])
        .unwrap();
    searcher_async
        .insert_document_async(SearchDoc {
            name: "fresh update after rebuild".to_owned(),
            id: 1,
        })
        .unwrap();

    let chunks = drain_pending_chunks(&searcher_async, &events_rx);
    assert_eq!(
        chunks
            .iter()
            .map(|chunk| describe_events(chunk))
            .collect_vec(),
        vec![
            vec!["clear".to_owned(), "insert r3 rebuild".to_owned()],
            vec!["insert fresh update after rebuild".to_owned()],
        ],
        "an update requested after a rebuild should resolve into a chunk after it"
    );

    apply_chunks(&searcher_async, chunks);
    assert_eq!(
        indexed_documents(),
        vec![(1, "fresh update after rebuild".to_owned())],
        "an update requested after a rebuild must win over that rebuild"
    );
}

/// Regression test for the wake-up marker introduced when the separate rebuild-notify channel
/// was folded into the events channel: [`QueuedItem::RebuildMarker`] must itself stay coalesced
/// to at most one outstanding item, even when a rebuild is requested, then superseded by
/// another, while the first rebuild's marker is still sitting unconsumed on the channel.
#[test]
fn test_searcher_async_rebuild_marker_stays_coalesced_across_supersession() {
    define_search_schema!(
        schema_name: TEST_SCHEMA,
        config_name: SchemaConfig,
        search_doc: SearchDoc,
        identifying_doc: IdentifyingDoc,
        search_fields: [name: 1.0],
        id_fields: [id: u64]
    );

    let (searcher_async, events_rx) =
        async_searcher_without_background_writer(TEST_SCHEMA.create_searcher(MIN_MEMORY_BUDGET));

    // The first rebuild transitions the marker from not-outstanding to outstanding, so it
    // enqueues exactly one. The second, requested before that marker has been consumed,
    // supersedes the pending document set but must not enqueue a second one.
    searcher_async
        .rebuild_index_async([SearchDoc {
            name: "first".to_owned(),
            id: 1,
        }])
        .unwrap();
    searcher_async
        .rebuild_index_async([SearchDoc {
            name: "second".to_owned(),
            id: 1,
        }])
        .unwrap();

    let mut queued_items = Vec::new();
    while let Ok(item) = events_rx.try_recv() {
        queued_items.push(item);
    }
    assert_eq!(
        queued_items.len(),
        1,
        "superseding a rebuild while its marker is still outstanding must not enqueue a second marker"
    );
    assert!(matches!(queued_items[0], QueuedItem::RebuildMarker));

    // The pending rebuild itself must reflect the latest request, not the first.
    {
        let state = searcher_async.producer_state.lock();
        let rebuild = state
            .pending_rebuild
            .as_ref()
            .expect("a rebuild should be pending");
        assert_eq!(rebuild.documents.len(), 1);
    }

    let chunks = drain_pending_chunks(&searcher_async, &events_rx);
    apply_chunks(&searcher_async, chunks);
    let documents = searcher_async.get_all_documents().unwrap();
    assert_eq!(
        documents.iter().map(|doc| &doc.name).collect_vec(),
        vec!["second"],
        "the superseding rebuild's document set must be the one that was applied"
    );

    // Once the marker has been consumed (by `drain_pending_chunks`, mirroring the background
    // writer), a fresh rebuild request must enqueue its own marker again rather than assuming
    // one is still outstanding.
    searcher_async
        .rebuild_index_async([SearchDoc {
            name: "third".to_owned(),
            id: 1,
        }])
        .unwrap();
    let mut queued_items = Vec::new();
    while let Ok(item) = events_rx.try_recv() {
        queued_items.push(item);
    }
    assert_eq!(
        queued_items.len(),
        1,
        "a rebuild requested after the marker was consumed must enqueue a fresh marker"
    );
}

/// Polls `converged` at a short interval until it returns `true` or `deadline` elapses. Returns
/// whether it converged in time.
fn poll_until(deadline: Duration, mut converged: impl FnMut() -> bool) -> bool {
    let start = Instant::now();
    loop {
        if converged() {
            return true;
        }
        if start.elapsed() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// Regression test for a latency hazard identified while simplifying the wake-up accounting for
/// [`QueuedItem::RebuildMarker`]: since the background writer takes the pending rebuild and
/// drains the events channel as two separate (non-atomic) steps, a rebuild whose marker gets
/// drained -- and so discarded, since the marker carries no data -- before the *next* cycle
/// exists to notice the rebuild it announced would otherwise sit unapplied until the next real
/// event or the 5-second idle timeout (`SEARCH_IDLE_TIMEOUT`). `process_searcher_events` closes
/// this by checking whether a rebuild is already pending *before* waiting on the channel at all,
/// at the top of every cycle.
///
/// Rather than racing real wall-clock timing against the background writer to try to land in
/// that narrow window (which is unreliable: in practice the writer is back to idly waiting long
/// before a next request arrives, so the race almost never reproduces), this stores a pending
/// rebuild directly in `producer_state` without going through `rebuild_index_async` at all --
/// deliberately bypassing the marker mechanism entirely, so no marker is ever sent for it. That
/// is exactly the state a rebuild would be left in if its marker had been silently swallowed:
/// `pending_rebuild` is `Some`, but nothing is going to arrive on the channel to announce it.
/// This still requires the real background writer (not the synchronous test harness used
/// elsewhere in this file), since the fix lives in that writer's wait/skip logic. The assertion
/// is a bounded poll well under the idle timeout, so this test fails (times out) if the fix
/// regresses, rather than passing on a technicality.
#[test]
fn test_searcher_async_rebuild_is_not_delayed_when_its_marker_is_never_sent() {
    define_search_schema!(
        schema_name: TEST_SCHEMA,
        config_name: SchemaConfig,
        search_doc: SearchDoc,
        identifying_doc: IdentifyingDoc,
        search_fields: [name: 1.0],
        id_fields: [id: u64]
    );

    let background_executor = Arc::new(Background::default());
    let searcher_async =
        TEST_SCHEMA.create_async_searcher(MIN_MEMORY_BUDGET, background_executor.clone());

    {
        let mut state = searcher_async.producer_state.lock();
        let sequence = state.next_sequence;
        state.next_sequence += 1;
        state.pending_rebuild = Some(PendingRebuild {
            sequence,
            documents: vec![
                SearchDoc {
                    name: "stranded rebuild".to_owned(),
                    id: 1,
                }
                .into_document_entry(),
            ],
        });
    }

    let converged = poll_until(Duration::from_secs(2), || {
        searcher_async
            .get_all_documents()
            .is_ok_and(|docs| docs.len() == 1 && docs[0].name == "stranded rebuild")
    });
    assert!(
        converged,
        "a pending rebuild whose wake-up marker was never sent must still be applied promptly, \
         not delayed until the idle timeout"
    );
}
