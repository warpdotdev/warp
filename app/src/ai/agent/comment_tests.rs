use crate::ai::agent::comment::{ReviewComment, ReviewDiff};
use crate::code_review::comments::CommentId;

fn comment(content: &str, head_title: Option<&str>) -> ReviewComment {
    ReviewComment {
        id: CommentId::default(),
        content: content.to_string(),
        diff: ReviewDiff {
            file_path: None,
            line_number: None,
        },
        head_title: head_title.map(str::to_string),
    }
}

#[test]
fn summary_shows_comment_text_when_short() {
    let c = comment("Please rename this variable", None);
    assert_eq!(c.summary(80), "Please rename this variable");
}

#[test]
fn summary_truncates_long_text_with_ellipsis() {
    let c = comment("abcdefghij", None);
    // truncate_from_end keeps max_chars - 1 chars and appends the ellipsis glyph.
    assert_eq!(c.summary(4), "abc…");
}

#[test]
fn summary_collapses_whitespace_into_single_line() {
    let c = comment("first line\n\nsecond   line\tthird", None);
    assert_eq!(c.summary(80), "first line second line third");
}

#[test]
fn summary_falls_back_to_title_when_content_is_empty() {
    // Empty (whitespace-only) content and no file/line -> falls back to head_title.
    let c = comment("   \n  ", Some("PR summary"));
    assert_eq!(c.summary(80), "PR summary");

    // No head title either -> generic title.
    let c = comment("", None);
    assert_eq!(c.summary(80), "Review Comment");
}
