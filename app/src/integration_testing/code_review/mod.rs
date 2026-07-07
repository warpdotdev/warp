use warpui::integration::{AssertionCallback, AssertionOutcome, TestStep};
use warpui::{async_assert, App, ViewHandle, WindowId};

use crate::code_review::code_review_view::{CodeReviewView, CodeReviewVisibleAnchorForTest};

/// Expected scroll region type for assertions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScrollRegion {
    Header,
    CurrentLine,
    RemovedLine,
    Footer,
}

fn try_single_code_review_view(
    app: &App,
    window_id: WindowId,
) -> Option<ViewHandle<CodeReviewView>> {
    let views = app.views_of_type::<CodeReviewView>(window_id)?;
    if views.len() == 1 {
        Some(views[0].clone())
    } else {
        None
    }
}

fn single_code_review_view(app: &App, window_id: WindowId) -> ViewHandle<CodeReviewView> {
    try_single_code_review_view(app, window_id)
        .expect("expected exactly one code review view in the window")
}

pub fn assert_code_review_loaded() -> AssertionCallback {
    Box::new(|app, window_id| {
        let Some(code_review_view) = try_single_code_review_view(app, window_id) else {
            return AssertionOutcome::failure(
                "code review view not yet available in the window".to_string(),
            );
        };
        code_review_view.read(app, |code_review_view, _| {
            async_assert!(
                code_review_view.has_file_states()
                    && code_review_view.all_editors_loaded_for_test(),
                "expected code review to have loaded file states and editor buffers"
            )
        })
    })
}

pub fn assert_code_review_anchor(
    expected_file_path: impl Into<String>,
    expected_text: impl Into<String>,
    expected_line_number: Option<usize>,
) -> AssertionCallback {
    let expected_file_path = expected_file_path.into();
    let expected_text = expected_text.into();

    Box::new(move |app, window_id| {
        let Some(code_review_view) = try_single_code_review_view(app, window_id) else {
            return AssertionOutcome::failure(
                "code review view not yet available in the window".to_string(),
            );
        };
        code_review_view.read(app, |code_review_view, ctx| {
            let Some(anchor) = code_review_view.visible_anchor_for_test(ctx) else {
                return AssertionOutcome::failure(
                    "expected a visible code review anchor but none was available".to_string(),
                );
            };

            assert_anchor(
                &anchor,
                &expected_file_path,
                &expected_text,
                expected_line_number,
            )
        })
    })
}

pub fn scroll_code_review_to_line(file_path: impl Into<String>, line_number: usize) -> TestStep {
    let file_path = file_path.into();

    TestStep::new("Scroll code review to a file line").with_action(move |app, window_id, _| {
        let code_review_view = single_code_review_view(app, window_id);
        code_review_view.update(app, |code_review_view, ctx| {
            let _ = code_review_view.scroll_to_line_for_test(&file_path, line_number, ctx);
        });
    })
}

pub fn assert_code_review_line_text(
    expected_file_path: impl Into<String>,
    line_number: usize,
    expected_text: impl Into<String>,
) -> AssertionCallback {
    let expected_file_path = expected_file_path.into();
    let expected_text = expected_text.into();

    Box::new(move |app, window_id| {
        let Some(code_review_view) = try_single_code_review_view(app, window_id) else {
            return AssertionOutcome::failure(
                "code review view not yet available in the window".to_string(),
            );
        };
        code_review_view.read(app, |code_review_view, ctx| {
            let Some(line_text) =
                code_review_view.line_text_for_test(&expected_file_path, line_number, ctx)
            else {
                return AssertionOutcome::failure(format!(
                    "expected code review line {line_number} for {expected_file_path:?} to be available",
                ));
            };

            if line_text != expected_text {
                return AssertionOutcome::failure(format!(
                    "expected line {line_number} in {expected_file_path:?} to be {expected_text:?}, got {line_text:?}",
                ));
            }

            AssertionOutcome::Success
        })
    })
}

fn assert_anchor(
    anchor: &CodeReviewVisibleAnchorForTest,
    expected_file_path: &str,
    expected_text: &str,
    expected_line_number: Option<usize>,
) -> AssertionOutcome {
    if anchor.file_path != expected_file_path {
        return AssertionOutcome::failure(format!(
            "expected anchor file to be {expected_file_path:?}, got {:?}",
            anchor.file_path
        ));
    }
    if anchor.line_text != expected_text {
        return AssertionOutcome::failure(format!(
            "expected anchor text to be {expected_text:?}, got {:?}",
            anchor.line_text
        ));
    }
    if let Some(expected_line_number) = expected_line_number {
        if anchor.line_number != expected_line_number {
            return AssertionOutcome::failure(format!(
                "expected anchor line number to be {expected_line_number}, got {}",
                anchor.line_number
            ));
        }
    }

    AssertionOutcome::Success
}

pub fn scroll_code_review_to_header(file_path: impl Into<String>) -> TestStep {
    let file_path = file_path.into();

    TestStep::new("Scroll code review to header region").with_action(move |app, window_id, _| {
        let code_review_view = single_code_review_view(app, window_id);
        code_review_view.update(app, |code_review_view, ctx| {
            code_review_view.scroll_to_header_for_test(&file_path, ctx);
        });
    })
}

pub fn scroll_code_review_to_footer(file_path: impl Into<String>) -> TestStep {
    let file_path = file_path.into();

    TestStep::new("Scroll code review to footer region").with_action(move |app, window_id, _| {
        let code_review_view = single_code_review_view(app, window_id);
        code_review_view.update(app, |code_review_view, ctx| {
            code_review_view.scroll_to_footer_for_test(&file_path, ctx);
        });
    })
}

pub fn scroll_code_review_to_deleted_range(
    file_path: impl Into<String>,
    near_line: usize,
) -> TestStep {
    let file_path = file_path.into();

    TestStep::new("Scroll code review to deleted range").with_action(move |app, window_id, _| {
        let code_review_view = single_code_review_view(app, window_id);
        code_review_view.update(app, |code_review_view, ctx| {
            code_review_view.scroll_to_deleted_range_for_test(&file_path, near_line, ctx);
        });
    })
}

pub fn assert_code_review_scroll_region(expected_region: ScrollRegion) -> AssertionCallback {
    let expected_str = match expected_region {
        ScrollRegion::Header => "header",
        ScrollRegion::CurrentLine => "current_line",
        ScrollRegion::RemovedLine => "removed_line",
        ScrollRegion::Footer => "footer",
    };

    Box::new(move |app, window_id| {
        let Some(code_review_view) = try_single_code_review_view(app, window_id) else {
            return AssertionOutcome::failure(
                "code review view not yet available in the window".to_string(),
            );
        };
        code_review_view.read(app, |code_review_view, ctx| {
            let actual = code_review_view.scroll_region_for_test(ctx);
            if actual != expected_str {
                return AssertionOutcome::failure(format!(
                    "expected scroll region to be {expected_str:?}, got {actual:?}"
                ));
            }
            AssertionOutcome::Success
        })
    })
}
