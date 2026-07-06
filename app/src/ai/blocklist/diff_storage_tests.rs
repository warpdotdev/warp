use std::collections::HashMap;
use std::sync::Arc;

use ai::agent::action_result::AnyFileContent;
use ai::agent::FileLocations;
use futures::FutureExt as _;
use warpui::{App, Entity, ModelHandle};

use super::*;

/// Minimal [`DiffStorage`] impl: canned snapshots and save outcomes, so tests
/// drive completion through the [`DiffStorageHelper`] flow.
struct TestSurface {
    files: Vec<FileSnapshot>,
    save_results: Vec<Result<(), Arc<FileSaveError>>>,
}

impl TestSurface {
    fn new(files: Vec<FileSnapshot>, save_results: Vec<Result<(), Arc<FileSaveError>>>) -> Self {
        Self {
            files,
            save_results,
        }
    }
}

impl DiffStorage for TestSurface {
    fn snapshot_pending_files(&self, _app: &AppContext) -> Vec<FileSnapshot> {
        self.files.clone()
    }

    fn start_saving(&mut self, _app: &mut AppContext) -> Vec<SaveFuture> {
        std::mem::take(&mut self.save_results)
            .into_iter()
            .map(|result| futures::future::ready(result).boxed() as SaveFuture)
            .collect()
    }
}

impl Entity for TestSurface {
    type Event = ();
}

fn updated_file(path: &str, content: &str) -> FileSnapshot {
    FileSnapshot {
        updated: Some(UpdatedFileState {
            path: path.to_owned(),
            changed_lines: std::iter::once(1..2).collect(),
            final_content: content.to_owned(),
            was_edited: false,
        }),
        deleted_paths: Vec::new(),
        diff_base: String::new(),
        diff_new: content.to_owned(),
        diff_name: path.to_owned(),
    }
}

fn add_surface(
    app: &mut App,
    files: Vec<FileSnapshot>,
    save_results: Vec<Result<(), Arc<FileSaveError>>>,
) -> ModelHandle<TestSurface> {
    app.add_model(|_| TestSurface::new(files, save_results))
}

#[test]
fn accept_resolves_with_computed_diffs_once_saves_complete() {
    App::test((), |mut app| async move {
        let surface = add_surface(
            &mut app,
            vec![updated_file("/tmp/x.rs", "fn main() {}\n")],
            vec![Ok(())],
        );
        let future = surface.update(&mut app, |surface, ctx| surface.accept_and_save(ctx));

        let RequestFileEditsResult::Success {
            diff,
            updated_files,
            deleted_files,
            lines_added,
            lines_removed,
        } = future.await
        else {
            panic!("expected accept to succeed");
        };
        assert!(diff.contains("+fn main() {}"));
        assert_eq!(lines_added, 1);
        assert_eq!(lines_removed, 0);
        assert_eq!(deleted_files, Vec::<String>::new());
        assert_eq!(updated_files.len(), 1);
        assert_eq!(updated_files[0].file_context.file_name, "/tmp/x.rs");
    });
}

#[test]
fn accept_reports_save_failure_for_the_whole_edit() {
    App::test((), |mut app| async move {
        let surface = add_surface(
            &mut app,
            vec![updated_file("/tmp/x.rs", "content\n")],
            vec![Err(Arc::new(FileSaveError::RemoteError(
                "disk full".to_owned(),
            )))],
        );
        let future = surface.update(&mut app, |surface, ctx| surface.accept_and_save(ctx));

        let RequestFileEditsResult::DiffApplicationFailed { error } = future.await else {
            panic!("expected a failed save to fail the edit");
        };
        assert!(error.contains("disk full"));
    });
}

#[test]
fn deleted_paths_are_reported_as_deleted_files() {
    App::test((), |mut app| async move {
        let surface = add_surface(
            &mut app,
            vec![FileSnapshot {
                updated: None,
                deleted_paths: vec!["/tmp/gone.rs".to_owned()],
                diff_base: "old content\n".to_owned(),
                diff_new: String::new(),
                diff_name: "/tmp/gone.rs".to_owned(),
            }],
            vec![Ok(())],
        );
        let future = surface.update(&mut app, |surface, ctx| surface.accept_and_save(ctx));

        let RequestFileEditsResult::Success {
            updated_files,
            deleted_files,
            lines_removed,
            ..
        } = future.await
        else {
            panic!("expected delete to succeed");
        };
        assert!(updated_files.is_empty());
        assert_eq!(deleted_files, vec!["/tmp/gone.rs".to_owned()]);
        assert_eq!(lines_removed, 1);
    });
}

#[test]
fn updated_file_contexts_from_content_map_returns_changed_lines_with_context() {
    let updated_files = vec![(
        FileLocations {
            name: "src/main.rs".to_string(),
            lines: std::iter::once(12..13).collect(),
        },
        true,
    )];
    let content = (1..=30)
        .map(|line| format!("line {line}"))
        .collect::<Vec<_>>()
        .join("\n");
    let content_map = HashMap::from([("src/main.rs".to_string(), content)]);

    let contexts = updated_file_contexts_from_content_map(&updated_files, &content_map);

    assert_eq!(contexts.len(), 1);
    assert!(contexts[0].was_edited_by_user);
    assert_eq!(contexts[0].file_context.file_name, "src/main.rs");
    assert_eq!(contexts[0].file_context.line_range, Some(2..23));
    assert_eq!(contexts[0].file_context.line_count, 30);
    assert_eq!(
        contexts[0].file_context.content,
        AnyFileContent::StringContent(
            (2..=22)
                .map(|line| format!("line {line}"))
                .collect::<Vec<_>>()
                .join("\n")
        )
    );
}

#[test]
fn updated_file_contexts_from_content_map_preserves_full_file_when_no_ranges() {
    let updated_files = vec![(
        FileLocations {
            name: "src/main.rs".to_string(),
            lines: vec![],
        },
        false,
    )];
    let content = "line 1\nline 2\n".to_string();
    let content_map = HashMap::from([("src/main.rs".to_string(), content.clone())]);

    let contexts = updated_file_contexts_from_content_map(&updated_files, &content_map);

    assert_eq!(contexts.len(), 1);
    assert!(!contexts[0].was_edited_by_user);
    assert_eq!(contexts[0].file_context.line_range, None);
    assert_eq!(contexts[0].file_context.line_count, 2);
    assert_eq!(
        contexts[0].file_context.content,
        AnyFileContent::StringContent(content)
    );
}
