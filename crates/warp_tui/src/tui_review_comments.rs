use ai::agent::action::{
    AIAgentActionType, CommentSide, InsertReviewComment, InsertedCommentLocation,
    format_review_comment_thread, group_review_comment_threads,
};
use markdown_parser::parse_markdown;
use warp::tui_export::{AIActionStatus, AIAgentAction};
use warpui_core::AppContext;
use warpui_core::elements::tui::{Modifier, TuiContainer, TuiElement, TuiFlex, TuiText};

use crate::agent_block_sections::render_fallback_tool_call_section;
use crate::tui_builder::TuiUiBuilder;
use crate::tui_markdown::{TuiMarkdownBlockHooks, TuiMarkdownPalette, render_formatted_text};

pub(crate) fn render_review_comments_tool_call(
    action: &AIAgentAction,
    status: Option<&AIActionStatus>,
    output_streaming: bool,
    app: &AppContext,
) -> Option<Box<dyn TuiElement>> {
    let AIAgentActionType::InsertCodeReviewComments { comments, .. } = &action.action else {
        return None;
    };

    let status_row = render_fallback_tool_call_section(action, status, output_streaming, None, app);
    if !status.is_some_and(AIActionStatus::is_success) || comments.is_empty() {
        return Some(status_row);
    }

    let builder = TuiUiBuilder::from_app(app);
    let palette = TuiMarkdownPalette::from_builder(&builder);
    let mut column = TuiFlex::column().child(status_row);
    for thread in group_review_comment_threads(comments) {
        let root = thread.root();
        let body = format_review_comment_thread(&thread);
        let rendered_body = match parse_markdown(&body) {
            Ok(formatted) => {
                render_formatted_text(&formatted, palette, &TuiMarkdownBlockHooks::default())
            }
            Err(_) => TuiText::new(body).with_style(palette.body).finish(),
        };

        let mut comment = TuiFlex::column()
            .child(
                TuiText::new(format_comment_target(root))
                    .with_style(builder.muted_text_style().add_modifier(Modifier::BOLD))
                    .finish(),
            )
            .child(rendered_body);
        if let Some(url) = &root.html_url {
            comment = comment.child(
                TuiText::new(url.clone())
                    .with_style(
                        builder
                            .accent_text_style()
                            .add_modifier(Modifier::UNDERLINED),
                    )
                    .finish(),
            );
        }

        column = column.child(
            TuiContainer::new(comment.finish())
                .with_padding_top(1)
                .with_padding_left(2)
                .finish(),
        );
    }

    Some(column.finish())
}

fn format_comment_target(comment: &InsertReviewComment) -> String {
    let Some(location) = &comment.comment_location else {
        return "Pull request".to_owned();
    };
    format_location(location)
}

fn format_location(location: &InsertedCommentLocation) -> String {
    let Some(line) = &location.line else {
        return location.relative_file_path.clone();
    };
    let range = &line.comment_line_range;
    let line_label = if range.start == 0 {
        range.end.to_string()
    } else if range.end == 0 || range.start == range.end {
        range.start.to_string()
    } else {
        format!("{}-{}", range.start, range.end)
    };
    let side = match line.side {
        Some(CommentSide::Right) => "new",
        Some(CommentSide::Left) => "old",
        None => "diff",
    };
    format!("{}:{line_label} ({side})", location.relative_file_path)
}

#[cfg(test)]
#[path = "tui_review_comments_tests.rs"]
mod tests;
