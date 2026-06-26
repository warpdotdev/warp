use std::iter;

use warp_util::path::CleanPathResult;

use super::super::TerminalView;
use super::{path_without_trailing_sentence_punctuation, GridHighlightedLink};
use crate::terminal::model::grid::grid_handler::PossiblePath;
use crate::terminal::model::index::Point;
use crate::terminal::model::terminal_model::WithinModel;

#[test]
fn strips_only_sentence_periods() {
    // A trailing period after a real file name is sentence punctuation.
    assert_eq!(
        path_without_trailing_sentence_punctuation("notes/README.md.").map(|trimmed| trimmed.path),
        Some("notes/README.md")
    );
    assert_eq!(
        path_without_trailing_sentence_punctuation(".gitignore.").map(|trimmed| trimmed.path),
        Some(".gitignore")
    );
    assert_eq!(
        path_without_trailing_sentence_punctuation("C:/Users/c/warp-md-test.md.")
            .map(|trimmed| trimmed.path),
        Some("C:/Users/c/warp-md-test.md")
    );

    // No trailing period -> nothing to trim.
    assert_eq!(
        path_without_trailing_sentence_punctuation("notes/README.md").map(|trimmed| trimmed.path),
        None
    );

    // `.`/`..` path components must be preserved, not treated as punctuation.
    assert_eq!(
        path_without_trailing_sentence_punctuation(".").map(|trimmed| trimmed.path),
        None
    );
    assert_eq!(
        path_without_trailing_sentence_punctuation("..").map(|trimmed| trimmed.path),
        None
    );
    assert_eq!(
        path_without_trailing_sentence_punctuation("foo/.").map(|trimmed| trimmed.path),
        None
    );
    assert_eq!(
        path_without_trailing_sentence_punctuation("foo/..").map(|trimmed| trimmed.path),
        None
    );
    assert_eq!(
        path_without_trailing_sentence_punctuation("foo..").map(|trimmed| trimmed.path),
        None
    );
}

#[test]
fn strips_trailing_fullwidth_sentence_punctuation() {
    let trimmed = path_without_trailing_sentence_punctuation("notes/README.md，")
        .expect("fullwidth comma should be stripped");
    assert_eq!(trimmed.path, "notes/README.md");
    assert_eq!(trimmed.removed_width, 2);

    let trimmed = path_without_trailing_sentence_punctuation("notes/README.md。！？")
        .expect("CJK sentence punctuation should be stripped");
    assert_eq!(trimmed.path, "notes/README.md");
    assert_eq!(trimmed.removed_width, 6);
}

// Regression test for https://github.com/warpdotdev/warp/issues/11477:
// a `.md` path at the end of a sentence captured the trailing period, so the
// resolved file and the highlight range ended in `.md.` and the file failed
// markdown classification. The trailing period must be excluded from both.
#[test]
fn compute_valid_paths_excludes_trailing_sentence_period() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("warp-md-test.md");
    std::fs::write(&file, "# Hello\n").unwrap();

    // The captured token as it would appear in `Drafted at <abs path>.`
    let token = format!("{}.", file.to_string_lossy());
    let end_col = token.chars().count() - 1;
    let candidate = WithinModel::AltScreen(PossiblePath {
        path: CleanPathResult {
            path: token,
            line_and_column_num: None,
        },
        range: Point { row: 0, col: 0 }..=Point {
            row: 0,
            col: end_col,
        },
    });

    let link = TerminalView::compute_valid_paths(
        dir.path().to_str().unwrap(),
        iter::once(candidate),
        1000,
        None,
    )
    .expect("the markdown file should be detected as a link");

    let GridHighlightedLink::File(file_link) = link else {
        panic!("expected a file link");
    };
    let file_link = file_link.get_inner();

    // The resolved file excludes the trailing period (so it classifies as `.md`)...
    assert_eq!(
        file_link.absolute_path.file_name().unwrap(),
        "warp-md-test.md"
    );
    // ...and the highlighted range stops before the trailing period.
    assert_eq!(
        *file_link.link.range().end(),
        Point {
            row: 0,
            col: end_col - 1,
        }
    );
}

#[test]
fn compute_valid_paths_excludes_trailing_fullwidth_sentence_punctuation() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("warp-md-test.md");
    std::fs::write(&file, "# Hello\n").unwrap();

    let token = format!("{}，", file.to_string_lossy());
    let punctuation_width = 2;
    let end_col = token.chars().count();
    let candidate = WithinModel::AltScreen(PossiblePath {
        path: CleanPathResult {
            path: token,
            line_and_column_num: None,
        },
        range: Point { row: 0, col: 0 }..=Point {
            row: 0,
            col: end_col,
        },
    });

    let link = TerminalView::compute_valid_paths(
        dir.path().to_str().unwrap(),
        iter::once(candidate),
        1000,
        None,
    )
    .expect("the markdown file should be detected as a link");

    let GridHighlightedLink::File(file_link) = link else {
        panic!("expected a file link");
    };
    let file_link = file_link.get_inner();

    assert_eq!(
        file_link.absolute_path.file_name().unwrap(),
        "warp-md-test.md"
    );
    assert_eq!(
        *file_link.link.range().end(),
        Point {
            row: 0,
            col: end_col - punctuation_width,
        }
    );
}

#[test]
fn compute_valid_paths_keeps_trailing_fullwidth_punctuation_when_it_is_the_filename() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("warp-md-test.md，");
    std::fs::write(&file, "# Hello\n").unwrap();

    let token = file.to_string_lossy().to_string();
    let end_col = token.chars().count();
    let candidate = WithinModel::AltScreen(PossiblePath {
        path: CleanPathResult {
            path: token,
            line_and_column_num: None,
        },
        range: Point { row: 0, col: 0 }..=Point {
            row: 0,
            col: end_col,
        },
    });

    let link = TerminalView::compute_valid_paths(
        dir.path().to_str().unwrap(),
        iter::once(candidate),
        1000,
        None,
    )
    .expect("the file with fullwidth punctuation should be detected as a link");

    let GridHighlightedLink::File(file_link) = link else {
        panic!("expected a file link");
    };
    let file_link = file_link.get_inner();

    assert_eq!(
        file_link.absolute_path.file_name().unwrap(),
        "warp-md-test.md，"
    );
    assert_eq!(
        *file_link.link.range().end(),
        Point {
            row: 0,
            col: end_col,
        }
    );
}
