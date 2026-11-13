use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use futures::channel::oneshot;
use remote_server::HostId;
use remote_server::proto::{RipgrepSearchMatch, RipgrepSearchSubmatch, RipgrepSearchSuccess};
use string_offset::ByteOffset;
use warp_ripgrep::search::Submatch;
use warp_util::local_or_remote_path::LocalOrRemotePath;
use warpui::App;

use super::{ActiveSearch, GlobalSearch, MAX_STORED_LINE_TEXT_BYTES, SearchSource, SourceResult};
use crate::workspace::view::global_search::GlobalSearchMatch;
use crate::workspace::view::global_search::view::GlobalSearchEvent;

fn host() -> HostId {
    HostId::new("test-host".to_string())
}

fn proto_match(
    path: &str,
    line_number: u32,
    line_text: &str,
    submatches: Vec<(u64, u64)>,
) -> RipgrepSearchMatch {
    RipgrepSearchMatch {
        file_path: path.to_string(),
        line_number,
        line_text: line_text.to_string(),
        submatches: submatches
            .into_iter()
            .map(|(byte_start, byte_end)| RipgrepSearchSubmatch {
                byte_start,
                byte_end,
            })
            .collect(),
    }
}

#[test]
fn remote_matches_become_remote_locations_on_the_host() {
    let success = RipgrepSearchSuccess {
        matches: vec![proto_match(
            "/repo/src/main.rs",
            7,
            "fn main() {}",
            vec![(3, 7)],
        )],
        capped: false,
    };

    let results = GlobalSearch::remote_matches_to_global(&host(), success);

    assert_eq!(results.len(), 1);
    match &results[0].location {
        LocalOrRemotePath::Remote(remote) => {
            assert_eq!(remote.host_id, host());
            assert_eq!(remote.path.as_str(), "/repo/src/main.rs");
        }
        LocalOrRemotePath::Local(_) => panic!("expected a remote location"),
    }
    assert_eq!(results[0].line_number, 7);
    assert_eq!(results[0].column_num, Some(4));
    assert_eq!(results[0].line_text, "fn main() {}");
}

#[test]
fn remote_matches_expand_one_row_per_submatch() {
    let success = RipgrepSearchSuccess {
        matches: vec![proto_match(
            "/repo/a.rs",
            1,
            "foo foo",
            vec![(0, 3), (4, 7)],
        )],
        capped: false,
    };

    let results = GlobalSearch::remote_matches_to_global(&host(), success);

    assert_eq!(results.len(), 2);
    assert!(results.iter().all(|m| m.submatches.len() == 1));
}

#[test]
fn remote_matches_with_invalid_paths_are_dropped() {
    let success = RipgrepSearchSuccess {
        matches: vec![
            proto_match("relative/path.rs", 1, "x", vec![(0, 1)]),
            proto_match("/repo/ok.rs", 2, "x", vec![(0, 1)]),
        ],
        capped: false,
    };

    let results = GlobalSearch::remote_matches_to_global(&host(), success);

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].location.display_path(), "/repo/ok.rs");
}

#[test]
fn remote_match_leading_whitespace_is_trimmed_per_submatch() {
    // Leading whitespace before the submatch is trimmed and offsets adjusted,
    // matching local search behavior.
    let success = RipgrepSearchSuccess {
        matches: vec![proto_match("/repo/a.rs", 1, "    foo", vec![(4, 7)])],
        capped: false,
    };

    let results = GlobalSearch::remote_matches_to_global(&host(), success);

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].line_text, "foo");
    assert_eq!(results[0].column_num, Some(5));
    assert_eq!(results[0].submatches[0].byte_start.as_usize(), 0);
    assert_eq!(results[0].submatches[0].byte_end.as_usize(), 3);
}

#[test]
fn remote_match_column_counts_characters_not_bytes() {
    let success = RipgrepSearchSuccess {
        matches: vec![proto_match("/repo/a.rs", 1, "€foo", vec![(3, 6)])],
        capped: false,
    };

    let results = GlobalSearch::remote_matches_to_global(&host(), success);

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].column_num, Some(2));
}

#[test]
fn expanded_matches_are_bounded_before_batching() {
    let prefix = "x".repeat(MAX_STORED_LINE_TEXT_BYTES * 4);
    let line_text = format!("{prefix}TARGET");
    let match_start = prefix.len();
    let match_end = line_text.len();
    let raw_match = GlobalSearchMatch {
        location: LocalOrRemotePath::Local(PathBuf::from("/repo/a.rs")),
        line_number: 7,
        column_num: None,
        line_text,
        submatches: vec![Submatch {
            byte_start: ByteOffset::from(match_start),
            byte_end: ByteOffset::from(match_end),
        }],
    };

    let results = GlobalSearch::expand_submatches(raw_match);

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].line_number, 7);
    assert_eq!(results[0].column_num, Some(match_start + 1));
    assert!(results[0].line_text.len() <= MAX_STORED_LINE_TEXT_BYTES);
    let submatch = &results[0].submatches[0];
    assert_eq!(
        &results[0].line_text[submatch.byte_start.as_usize()..submatch.byte_end.as_usize()],
        "TARGET"
    );
}

#[test]
fn local_heap_limit_status_marks_the_completed_search_as_capped() {
    App::test((), |mut app| async move {
        let model = app.add_model(|_| GlobalSearch::new());
        let (sender, receiver) = oneshot::channel();
        let sender = Arc::new(Mutex::new(Some(sender)));
        app.update(|ctx| {
            ctx.subscribe_to_model(&model, move |_, event, _| {
                if let GlobalSearchEvent::Completed { capped, .. } = event
                    && let Some(sender) = sender.lock().unwrap().take()
                {
                    let _ = sender.send(*capped);
                }
            });
        });

        model.update(&mut app, |model, ctx| {
            model.active_search = Some(ActiveSearch {
                search_id: 7,
                remaining_sources: 1,
                completed_sources: 0,
                local_source_failed: false,
                remote_source_failures: 0,
                total_match_count: 0,
                capped: false,
            });
            model.handle_source_completed(
                7,
                SearchSource::Local,
                Some(SourceResult {
                    match_count: 0,
                    capped: true,
                }),
                ctx,
            );
        });

        assert!(receiver.await.unwrap());
    });
}
