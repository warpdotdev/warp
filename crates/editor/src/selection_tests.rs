use std::sync::Arc;

use serde_yaml::Value;
use string_offset::CharOffset;
use sum_tree::SumTree;
use vec1::vec1;
use warp_core::features::FeatureFlag;
use warpui_core::assets::asset_cache::AssetSource;
use warpui_core::text::word_boundaries::WordBoundariesPolicy;
use warpui_core::units::IntoPixels;
use warpui_core::{App, ModelAsRef};

use super::{SelectionModel, TextDirection, TextUnit};
use crate::content::buffer::tests::TestEmbeddedItem;
use crate::content::buffer::{
    AutoScrollBehavior, Buffer, BufferEditAction, BufferSelectAction, EditOrigin,
    InitialBufferState, SelectionOffsets,
};
use crate::content::selection_model::BufferSelectionModel;
use crate::content::text::{BufferBlockStyle, IndentBehavior, IndentUnit};
use crate::render::model::test_utils::{TEST_STYLES, laid_out_paragraph};
use crate::render::model::{BlockItem, COMMAND_SPACING, ImageBlockConfig, RenderState};
use crate::selection::SelectionMode;

impl SelectionModel {
    /// The cursor location.
    pub fn cursor(&self, ctx: &impl ModelAsRef) -> CharOffset {
        self.selection_model
            .as_ref(ctx)
            .selection_to_first_offset_range()
            .end
    }
}

fn selection_model_with_rendered_mermaid(
    app: &mut App,
) -> warpui_core::ModelHandle<SelectionModel> {
    app.add_model(|ctx| {
        let buffer = ctx.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let buffer_selection = ctx.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(ctx, |buffer, ctx| {
            buffer.update_content(
                BufferEditAction::Insert {
                    text: "AA\n12345CC\n",
                    style: Default::default(),
                    override_text_style: None,
                },
                EditOrigin::UserTyped,
                buffer_selection.clone(),
                ctx,
            );
        });

        let render = ctx.add_model(|_| {
            let mut render = RenderState::new_for_test(
                TEST_STYLES,
                f32::MAX.into_pixels(),
                f32::MAX.into_pixels(),
            );

            let mut content = SumTree::new();
            content.push(laid_out_paragraph("AA\n", &TEST_STYLES, f32::MAX));
            content.push(BlockItem::MermaidDiagram {
                content_length: 5.into(),
                asset_source: AssetSource::Bundled {
                    path: "bundled/svg/test.svg",
                },
                config: ImageBlockConfig {
                    width: 120.0.into_pixels(),
                    height: 40.0.into_pixels(),
                    spacing: COMMAND_SPACING,
                },
            });
            content.push(laid_out_paragraph("CC\n", &TEST_STYLES, f32::MAX));
            render.set_content(content);
            render
        });
        SelectionModel::new(buffer, render, buffer_selection, None, ctx)
    })
}

#[test]
fn test_move_right_skips_rendered_mermaid_block() {
    App::test((), |mut app| async move {
        let _flag = FeatureFlag::EditableMarkdownMermaid.override_enabled(true);
        let selection = selection_model_with_rendered_mermaid(&mut app);

        selection.update(&mut app, |selection, ctx| {
            selection.set_cursor(3.into(), ctx);
            selection.update_selection(
                BufferSelectAction::MoveRight,
                AutoScrollBehavior::Selection,
                ctx,
            );
            assert_eq!(selection.cursor(ctx), 8.into());
        });
    });
}

#[test]
fn test_extend_right_expands_across_rendered_mermaid_block() {
    App::test((), |mut app| async move {
        let _flag = FeatureFlag::EditableMarkdownMermaid.override_enabled(true);
        let selection = selection_model_with_rendered_mermaid(&mut app);

        selection.update(&mut app, |selection, ctx| {
            selection.set_cursor(3.into(), ctx);
            selection.update_selection(
                BufferSelectAction::ExtendRight,
                AutoScrollBehavior::Selection,
                ctx,
            );
            assert_eq!(
                selection.selections(ctx),
                vec1![SelectionOffsets {
                    head: 8.into(),
                    tail: 3.into(),
                }]
            );
        });
    });
}

#[test]
fn test_move_left_skips_rendered_mermaid_block() {
    App::test((), |mut app| async move {
        let _flag = FeatureFlag::EditableMarkdownMermaid.override_enabled(true);
        let selection = selection_model_with_rendered_mermaid(&mut app);

        selection.update(&mut app, |selection, ctx| {
            selection.set_cursor(8.into(), ctx);
            selection.update_selection(
                BufferSelectAction::MoveLeft,
                AutoScrollBehavior::Selection,
                ctx,
            );
            assert_eq!(selection.cursor(ctx), 3.into());
        });
    });
}

#[test]
fn test_extend_left_expands_across_rendered_mermaid_block() {
    App::test((), |mut app| async move {
        let _flag = FeatureFlag::EditableMarkdownMermaid.override_enabled(true);
        let selection = selection_model_with_rendered_mermaid(&mut app);

        selection.update(&mut app, |selection, ctx| {
            selection.set_cursor(8.into(), ctx);
            selection.update_selection(
                BufferSelectAction::ExtendLeft,
                AutoScrollBehavior::Selection,
                ctx,
            );
            assert_eq!(
                selection.selections(ctx),
                vec1![SelectionOffsets {
                    head: 3.into(),
                    tail: 8.into(),
                }]
            );
        });
    });
}

#[test]
fn test_extend_left_reverses_shift_selection_across_rendered_mermaid_block() {
    App::test((), |mut app| async move {
        let _flag = FeatureFlag::EditableMarkdownMermaid.override_enabled(true);
        let selection = selection_model_with_rendered_mermaid(&mut app);

        selection.update(&mut app, |selection, ctx| {
            selection.set_cursor(3.into(), ctx);
            selection.update_selection(
                BufferSelectAction::ExtendRight,
                AutoScrollBehavior::Selection,
                ctx,
            );
            selection.update_selection(
                BufferSelectAction::ExtendLeft,
                AutoScrollBehavior::Selection,
                ctx,
            );
            assert_eq!(
                selection.selections(ctx),
                vec1![SelectionOffsets {
                    head: 3.into(),
                    tail: 3.into(),
                }]
            );
        });
    });
}

#[test]
fn test_extend_right_reverses_shift_selection_across_rendered_mermaid_block() {
    App::test((), |mut app| async move {
        let _flag = FeatureFlag::EditableMarkdownMermaid.override_enabled(true);
        let selection = selection_model_with_rendered_mermaid(&mut app);

        selection.update(&mut app, |selection, ctx| {
            selection.set_cursor(8.into(), ctx);
            selection.update_selection(
                BufferSelectAction::ExtendLeft,
                AutoScrollBehavior::Selection,
                ctx,
            );
            selection.update_selection(
                BufferSelectAction::ExtendRight,
                AutoScrollBehavior::Selection,
                ctx,
            );
            assert_eq!(
                selection.selections(ctx),
                vec1![SelectionOffsets {
                    head: 8.into(),
                    tail: 8.into(),
                }]
            );
        });
    });
}

#[test]
fn test_horizontal_movement_resets_goal_column() {
    App::test((), |mut app| async move {
        let selection = app.add_model(|ctx| {
            let buffer = ctx.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
            let buffer_selection = ctx.add_model(|_| BufferSelectionModel::new(buffer.clone()));
            let render = ctx.add_model(|_| {
                RenderState::new_for_test(
                    TEST_STYLES,
                    f32::MAX.into_pixels(),
                    f32::MAX.into_pixels(),
                )
            });
            SelectionModel::new(buffer, render, buffer_selection, None, ctx)
        });

        // Moving via the high-level navigation APIs should reset the goal column.
        selection.update(&mut app, |selection, ctx| {
            selection.goal_xs = Some(vec1::vec1![12.34.into_pixels()]);
            selection.move_selection(TextDirection::Forwards, TextUnit::LineBoundary, ctx);
            assert_eq!(selection.goal_xs, None);
        });

        // Moving via a buffer-level action should as well (as long as it's via the selection model).
        selection.update(&mut app, |selection, ctx| {
            selection.goal_xs = Some(vec1::vec1![12.34.into_pixels()]);
            selection.update_selection(
                BufferSelectAction::MoveLeft,
                AutoScrollBehavior::Selection,
                ctx,
            );
            assert!(selection.goal_xs.is_none());
        });

        // Editing resets the goal too.
        selection.update(&mut app, |selection, ctx| {
            selection.goal_xs = Some(vec1::vec1![12.34.into_pixels()]);
            selection.content.update(ctx, |buffer, ctx| {
                buffer.update_content(
                    BufferEditAction::Insert {
                        text: "test",
                        style: Default::default(),
                        override_text_style: None,
                    },
                    EditOrigin::UserTyped,
                    selection.selection_model.clone(),
                    ctx,
                );
            });
        });
        selection.read(&app, |selection, _| assert!(selection.goal_xs.is_none()));
    });
}

#[test]
fn test_vertical_movement_with_goal() {
    App::test((), |mut app| async move {
        let selection = app.add_model(|ctx| {
            let buffer = ctx.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
            let buffer_selection = ctx.add_model(|_| BufferSelectionModel::new(buffer.clone()));

            buffer.update(ctx, |buffer, ctx| {
                buffer.update_content(
                    BufferEditAction::Insert {
                        text: "This is a long line\nShort\nThis is long again",
                        style: Default::default(),
                        override_text_style: None,
                    },
                    EditOrigin::UserTyped,
                    buffer_selection.clone(),
                    ctx,
                );
            });

            let render = ctx.add_model(|_| {
                let mut render = RenderState::new_for_test(
                    TEST_STYLES,
                    f32::MAX.into_pixels(),
                    f32::MAX.into_pixels(),
                );

                let mut content = SumTree::new();
                content.push(laid_out_paragraph(
                    "This is a long line\n",
                    &TEST_STYLES,
                    f32::MAX,
                ));
                content.push(laid_out_paragraph("Short\n", &TEST_STYLES, f32::MAX));
                content.push(laid_out_paragraph(
                    "This is long again\n",
                    &TEST_STYLES,
                    f32::MAX,
                ));
                render.set_content(content);
                render
            });
            SelectionModel::new(buffer, render, buffer_selection, None, ctx)
        });

        selection.update(&mut app, |selection, ctx| {
            // Position the cursor in the first line, just before the "a".
            selection.set_cursor(9.into(), ctx);

            // Moving down should clamp to the end of the short line.
            selection.move_selection(TextDirection::Forwards, TextUnit::Line, ctx);
            assert_eq!(selection.cursor(ctx), 26.into());

            // Moving down again should use the goal column.
            selection.move_selection(TextDirection::Forwards, TextUnit::Line, ctx);
            assert_eq!(selection.cursor(ctx), 35.into());

            // Moving up to the first line should restore the original cursor.
            selection.move_selection(TextDirection::Backwards, TextUnit::Line, ctx);
            selection.move_selection(TextDirection::Backwards, TextUnit::Line, ctx);
            assert_eq!(selection.cursor(ctx), 9.into());
        });
    });
}

#[test]
fn test_word_and_line_boundary_movement_with_block_item() {
    App::test((), |mut app| async move {
        let selection = app.add_model(|ctx| {
            let buffer = ctx.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
            let buffer_selection = ctx.add_model(|_| BufferSelectionModel::new(buffer.clone()));

            buffer.update(ctx, |buffer, ctx| {
                *buffer = Buffer::from_markdown(
                    r#"text
```warp-embedded-object
id: workflow-123
```
```warp-embedded-object
id: workflow-123
type: workflow
author: kevin
```"#,
                    Some(
                        |mut mapping| match mapping.remove(&Value::String("id".to_string())) {
                            Some(Value::String(hashed_id)) => {
                                Some(Arc::new(TestEmbeddedItem { id: hashed_id }))
                            }
                            _ => None,
                        },
                    ),
                    Box::new(|_, _| IndentBehavior::Ignore),
                    buffer_selection.clone(),
                    ctx,
                );
            });

            let render = ctx.add_model(|_| {
                RenderState::new_for_test(
                    TEST_STYLES,
                    f32::MAX.into_pixels(),
                    f32::MAX.into_pixels(),
                )
            });
            SelectionModel::new(buffer, render, buffer_selection, None, ctx)
        });

        selection.update(&mut app, |selection, ctx| {
            // Place the cursor after a block item. Moving back a word should step over just the block item.
            selection.set_cursor(6.into(), ctx);
            selection.move_selection(
                TextDirection::Backwards,
                TextUnit::Word(WordBoundariesPolicy::Default),
                ctx,
            );
            assert_eq!(selection.cursor(ctx), 5.into());

            // Keep moving the selection backwards should select the word as normal.
            selection.move_selection(
                TextDirection::Backwards,
                TextUnit::Word(WordBoundariesPolicy::Default),
                ctx,
            );
            assert_eq!(selection.cursor(ctx), 1.into());

            // Place the cursor before a block item. Moving forward a word should step over the block item.
            selection.set_cursor(5.into(), ctx);
            selection.move_selection(
                TextDirection::Forwards,
                TextUnit::Word(WordBoundariesPolicy::Default),
                ctx,
            );
            assert_eq!(selection.cursor(ctx), 6.into());

            // Place the cursor after a block item. Moving backward to the line boundary should step over the block item.
            selection.set_cursor(6.into(), ctx);
            selection.move_selection(TextDirection::Backwards, TextUnit::LineBoundary, ctx);
            assert_eq!(selection.cursor(ctx), 5.into());

            // Moving backwards from the paragraph boundary should also step over the block item.
            selection.set_cursor(6.into(), ctx);
            selection.move_selection(TextDirection::Backwards, TextUnit::ParagraphBoundary, ctx);
            assert_eq!(selection.cursor(ctx), 5.into());
        });
    });
}

#[test]
fn test_paragraph_navigation() {
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let buffer_selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            let state = InitialBufferState::markdown(
                "Line one\nLine two\n```\nfirst code\nsecond code\n```\n* list\n* list",
            );
            buffer.update_content(
                BufferEditAction::ReplaceWith(state),
                EditOrigin::SystemEdit,
                buffer_selection.clone(),
                ctx,
            );
        });

        let render = app.add_model(|_| {
            RenderState::new_for_test(TEST_STYLES, f32::MAX.into_pixels(), f32::MAX.into_pixels())
        });

        let selection =
            app.add_model(|ctx| SelectionModel::new(buffer, render, buffer_selection, None, ctx));

        selection.update(&mut app, |selection, ctx| {
            // Within text blocks, paragraph navigation should jump between newlines.
            selection.set_cursor(2.into(), ctx);
            selection.move_selection(TextDirection::Forwards, TextUnit::ParagraphBoundary, ctx);
            assert_eq!(selection.cursor(ctx), 9.into());
            selection.set_cursor(2.into(), ctx);
            selection.move_selection(TextDirection::Backwards, TextUnit::ParagraphBoundary, ctx);
            assert_eq!(selection.cursor(ctx), 1.into());

            // Within code blocks, paragraph navigation should operate on newlines as well.
            selection.set_cursor(22.into(), ctx);
            selection.move_selection(TextDirection::Forwards, TextUnit::ParagraphBoundary, ctx);
            assert_eq!(selection.cursor(ctx), 29.into());
            selection.set_cursor(22.into(), ctx);
            selection.move_selection(TextDirection::Backwards, TextUnit::ParagraphBoundary, ctx);
            assert_eq!(selection.cursor(ctx), 19.into());

            // In single-line blocks, paragraph navigation uses block boundaries.
            selection.set_cursor(50.into(), ctx);
            selection.move_selection(TextDirection::Forwards, TextUnit::ParagraphBoundary, ctx);
            assert_eq!(selection.cursor(ctx), 51.into());
            selection.set_cursor(50.into(), ctx);
            selection.move_selection(TextDirection::Backwards, TextUnit::ParagraphBoundary, ctx);
            assert_eq!(selection.cursor(ctx), 47.into());
        })
    });
}

#[test]
fn test_indented_code_line_boundaries() {
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| {
            Buffer::new(Box::new(|block_style, _| match block_style {
                BufferBlockStyle::CodeBlock { .. } => {
                    IndentBehavior::TabIndent(IndentUnit::Space(4))
                }
                _ => IndentBehavior::Ignore,
            }))
        });
        let buffer_selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            let state = InitialBufferState::markdown("```\n    test\n```");
            buffer.update_content(
                BufferEditAction::ReplaceWith(state),
                EditOrigin::SystemEdit,
                buffer_selection.clone(),
                ctx,
            );
        });

        let render = app.add_model(|_| {
            let mut render = RenderState::new_for_test(
                TEST_STYLES,
                f32::MAX.into_pixels(),
                f32::MAX.into_pixels(),
            );
            let mut content = SumTree::new();
            // This will be laid out as plain text, not code, but that doesn't affect the test.
            content.push(laid_out_paragraph("    test\n", &TEST_STYLES, f32::MAX));
            render.set_content(content);
            render
        });

        let selection =
            app.add_model(|ctx| SelectionModel::new(buffer, render, buffer_selection, None, ctx));

        selection.update(&mut app, |selection, ctx| {
            // Start at the beginning of the buffer.
            selection.set_cursor(1.into(), ctx);

            // Move forward to the start of `test`, then to the end of the line.
            selection.move_selection(TextDirection::Forwards, TextUnit::LineBoundary, ctx);
            assert_eq!(selection.cursor(ctx), 5.into());
            selection.move_selection(TextDirection::Forwards, TextUnit::LineBoundary, ctx);
            assert_eq!(selection.cursor(ctx), 9.into());

            // Now, move backwards to the start of `test`, then to the start of the line.
            selection.move_selection(TextDirection::Backwards, TextUnit::LineBoundary, ctx);
            assert_eq!(selection.cursor(ctx), 5.into());
            selection.move_selection(TextDirection::Backwards, TextUnit::LineBoundary, ctx);
            assert_eq!(selection.cursor(ctx), 1.into());

            // Position the cursor inside the indentation and move backwards to the start of the line.
            selection.set_cursor(3.into(), ctx);
            selection.move_selection(TextDirection::Backwards, TextUnit::LineBoundary, ctx);
            assert_eq!(selection.cursor(ctx), 1.into());

            // From the same starting cursor, move forwards to the indented start.
            selection.set_cursor(3.into(), ctx);
            selection.move_selection(TextDirection::Forwards, TextUnit::LineBoundary, ctx);
            assert_eq!(selection.cursor(ctx), 5.into());
        });
    })
}

#[test]
fn test_new_cursor_at() {
    App::test((), |mut app| async move {
        let selection = app.add_model(|ctx| {
            let buffer = ctx.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
            let buffer_selection = ctx.add_model(|_| BufferSelectionModel::new(buffer.clone()));

            buffer.update(ctx, |buffer, ctx| {
                buffer.update_content(
                    BufferEditAction::Insert {
                        text: "This is a long line\nShort\nThis is long again",
                        style: Default::default(),
                        override_text_style: None,
                    },
                    EditOrigin::UserTyped,
                    buffer_selection.clone(),
                    ctx,
                );
            });

            let render = ctx.add_model(|_| {
                RenderState::new_for_test(
                    TEST_STYLES,
                    f32::MAX.into_pixels(),
                    f32::MAX.into_pixels(),
                )
            });
            SelectionModel::new(buffer, render, buffer_selection, None, ctx)
        });

        selection.update(&mut app, |selection, ctx| {
            // Start at the beginning of the buffer.
            selection.set_cursor(1.into(), ctx);

            assert_eq!(selection.cursors(ctx).len(), 1);
            assert_eq!(selection.cursors(ctx)[0], 1.into());

            selection.add_cursor(5.into(), ctx);

            assert_eq!(selection.cursors(ctx).len(), 2);
            assert_eq!(selection.cursors(ctx)[0], 1.into());
            assert_eq!(selection.cursors(ctx)[1], 5.into());
        });
    })
}

#[test]
fn test_multiselect_vertical_movement_with_goal() {
    App::test((), |mut app| async move {
        let selection = app.add_model(|ctx| {
            let buffer = ctx.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
            let buffer_selection = ctx.add_model(|_| BufferSelectionModel::new(buffer.clone()));

            buffer.update(ctx, |buffer, ctx| {
                buffer.update_content(
                    BufferEditAction::Insert {
                        text: "This is a long line\nShort\nThis is long again\nShort\nThis is again a long one.",
                        style: Default::default(),
                        override_text_style: None,
                    },
                    EditOrigin::UserTyped,
                    buffer_selection.clone(),
                    ctx,
                );
            });

            let render = ctx.add_model(|_| {
                let mut render = RenderState::new_for_test(
                    TEST_STYLES,
                    f32::MAX.into_pixels(),
                    f32::MAX.into_pixels(),
                );

                let mut content = SumTree::new();
                content.push(laid_out_paragraph(
                    "This is a long line\n",
                    &TEST_STYLES,
                    f32::MAX,
                ));
                content.push(laid_out_paragraph("Short\n", &TEST_STYLES, f32::MAX));
                content.push(laid_out_paragraph(
                    "This is long again\n",
                    &TEST_STYLES,
                    f32::MAX,
                ));
                content.push(laid_out_paragraph("Short\n", &TEST_STYLES, f32::MAX));
                content.push(laid_out_paragraph(
                    "This is again a long one.\n",
                    &TEST_STYLES,
                    f32::MAX,
                ));
                render.set_content(content);
                render
            });
            SelectionModel::new(buffer, render, buffer_selection, None, ctx)
        });

        selection.update(&mut app, |selection, ctx| {
            // Position the cursor in the first line, just before the "a".
            selection.set_cursor(9.into(), ctx);
            //Position a 2nd cursor in the 3rd line, just after "long"
            selection.add_cursor(39.into(), ctx);

            // Moving down should clamp to the end of the short lines.
            selection.move_selection(TextDirection::Forwards, TextUnit::Line, ctx);
            assert_eq!(selection.cursors(ctx), vec1::vec1![26.into(), 51.into()]);

            // Moving down again should use the goal column.
            selection.move_selection(TextDirection::Forwards, TextUnit::Line, ctx);
            assert_eq!(selection.cursors(ctx), vec1::vec1![35.into(), 64.into()]);

            // Moving up to the first line should restore the original cursor.
            selection.move_selection(TextDirection::Backwards, TextUnit::Line, ctx);
            selection.move_selection(TextDirection::Backwards, TextUnit::Line, ctx);
            assert_eq!(selection.cursors(ctx), vec1::vec1![9.into(), 39.into()]);
        });
    });
}

#[test]
fn test_multiselect_vertical_extension_with_goal() {
    App::test((), |mut app| async move {
        let selection = app.add_model(|ctx| {
            let buffer = ctx.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
            let buffer_selection = ctx.add_model(|_| BufferSelectionModel::new(buffer.clone()));

            buffer.update(ctx, |buffer, ctx| {
                buffer.update_content(
                    BufferEditAction::Insert {
                        text: "This is a long line\nShort\nThis is long again\nShort\nThis is again a long one.",
                        style: Default::default(),
                        override_text_style: None,
                    },
                    EditOrigin::UserTyped,
                    buffer_selection.clone(),
                    ctx,
                );
            });

            let render = ctx.add_model(|_| {
                let mut render = RenderState::new_for_test(
                    TEST_STYLES,
                    f32::MAX.into_pixels(),
                    f32::MAX.into_pixels(),
                );

                let mut content = SumTree::new();
                content.push(laid_out_paragraph(
                    "This is a long line\n",
                    &TEST_STYLES,
                    f32::MAX,
                ));
                content.push(laid_out_paragraph("Short\n", &TEST_STYLES, f32::MAX));
                content.push(laid_out_paragraph(
                    "This is long again\n",
                    &TEST_STYLES,
                    f32::MAX,
                ));
                content.push(laid_out_paragraph("Short\n", &TEST_STYLES, f32::MAX));
                content.push(laid_out_paragraph(
                    "This is again a long one.\n",
                    &TEST_STYLES,
                    f32::MAX,
                ));
                render.set_content(content);
                render
            });
            SelectionModel::new(buffer, render, buffer_selection, None, ctx)
        });

        selection.update(&mut app, |selection, ctx| {
            // Position the cursor in the first line, just before the "a".
            selection.set_cursor(9.into(), ctx);
            //Position a 2nd cursor in the 3rd line, just after "long"
            selection.add_cursor(39.into(), ctx);

            // Moving down should clamp to the end of the short lines.
            selection.extend_selection(TextDirection::Forwards, TextUnit::Line, ctx);
            assert_eq!(
                selection.selections(ctx),
                vec1::vec1![
                    SelectionOffsets {
                        tail: 9.into(),
                        head: 26.into()
                    },
                    SelectionOffsets {
                        tail: 39.into(),
                        head: 51.into()
                    }
                ]
            );

            // Moving down again should use the goal column.
            selection.extend_selection(TextDirection::Forwards, TextUnit::Line, ctx);
            assert_eq!(
                selection.selections(ctx),
                vec1::vec1![
                    SelectionOffsets {
                        tail: 9.into(),
                        head: 35.into()
                    },
                    SelectionOffsets {
                        tail: 39.into(),
                        head: 64.into()
                    }
                ]
            );

            // Moving up to the first line should restore the original cursor.
            selection.extend_selection(TextDirection::Backwards, TextUnit::Line, ctx);
            selection.extend_selection(TextDirection::Backwards, TextUnit::Line, ctx);
            assert_eq!(
                selection.selections(ctx),
                vec1::vec1![
                    SelectionOffsets {
                        tail: 9.into(),
                        head: 9.into()
                    },
                    SelectionOffsets {
                        tail: 39.into(),
                        head: 39.into()
                    }
                ]
            );
        });
    });
}

#[test]
fn test_multiselect_vertical_extension_overlap() {
    App::test((), |mut app| async move {
        let selection = app.add_model(|ctx| {
            let buffer = ctx.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
            let buffer_selection = ctx.add_model(|_| BufferSelectionModel::new(buffer.clone()));

            buffer.update(ctx, |buffer, ctx| {
                buffer.update_content(
                    BufferEditAction::Insert {
                        text: "This is a long line\nShort\nThis is long again\nShort\nThis is again a long one.",
                        style: Default::default(),
                        override_text_style: None,
                    },
                    EditOrigin::UserTyped,
                    buffer_selection.clone(),
                    ctx,
                );
            });

            let render = ctx.add_model(|_| {
                let mut render = RenderState::new_for_test(
                    TEST_STYLES,
                    f32::MAX.into_pixels(),
                    f32::MAX.into_pixels(),
                );

                let mut content = SumTree::new();
                content.push(laid_out_paragraph(
                    "This is a long line\n",
                    &TEST_STYLES,
                    f32::MAX,
                ));
                content.push(laid_out_paragraph("Short\n", &TEST_STYLES, f32::MAX));
                content.push(laid_out_paragraph(
                    "This is long again\n",
                    &TEST_STYLES,
                    f32::MAX,
                ));
                content.push(laid_out_paragraph("Short\n", &TEST_STYLES, f32::MAX));
                content.push(laid_out_paragraph(
                    "This is again a long one.\n",
                    &TEST_STYLES,
                    f32::MAX,
                ));
                render.set_content(content);
                render
            });
            SelectionModel::new(buffer, render, buffer_selection, None, ctx)
        });

        selection.update(&mut app, |selection, ctx| {
            // Position the cursor in the first line, just before the "a".
            selection.set_cursor(9.into(), ctx);
            //Position a 2nd cursor in the 2nd line, just after "S"
            selection.add_cursor(22.into(), ctx);

            // Moving down should clamp to the end of the short lines.
            // Note that these overlap but shouldn't be merged until the next action.
            selection.extend_selection(TextDirection::Forwards, TextUnit::Line, ctx);
            assert_eq!(
                selection.selections(ctx),
                vec1::vec1![
                    SelectionOffsets {
                        tail: 9.into(),
                        head: 26.into()
                    },
                    SelectionOffsets {
                        tail: 22.into(),
                        head: 28.into()
                    }
                ]
            );

            // Moving down again should merge the selections.
            selection.extend_selection(TextDirection::Forwards, TextUnit::Line, ctx);
            assert_eq!(
                selection.selections(ctx),
                vec1::vec1![SelectionOffsets {
                    tail: 9.into(),
                    head: 51.into()
                },]
            );

            // Moving up to the first line should restore the original cursor.
            selection.extend_selection(TextDirection::Backwards, TextUnit::Line, ctx);
            selection.extend_selection(TextDirection::Backwards, TextUnit::Line, ctx);
            assert_eq!(
                selection.selections(ctx),
                vec1::vec1![SelectionOffsets {
                    tail: 9.into(),
                    head: 26.into()
                },]
            );
        });
    });
}

#[test]
fn test_drag_semantic_selection() {
    App::test((), |mut app| async move {
        let selection = app.add_model(|ctx| {
            let buffer = ctx.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
            let buffer_selection = ctx.add_model(|_| BufferSelectionModel::new(buffer.clone()));

            buffer.update(ctx, |buffer, ctx| {
                *buffer = Buffer::from_plain_text(
                    "First\nSecond\nThird\nFourth",
                    None,
                    Box::new(|_, _| IndentBehavior::Ignore),
                    buffer_selection.clone(),
                    ctx,
                );
            });

            let render = ctx.add_model(|_| {
                RenderState::new_for_test(
                    TEST_STYLES,
                    f32::MAX.into_pixels(),
                    f32::MAX.into_pixels(),
                )
            });

            SelectionModel::new(buffer, render, buffer_selection, None, ctx)
        });

        // Select the third line.
        selection.update(&mut app, |selection, ctx| {
            selection.begin_selection(16.into(), SelectionMode::Line, true, ctx);
            assert_eq!(
                selection.selections(ctx),
                vec1![SelectionOffsets {
                    head: 19.into(),
                    tail: 14.into()
                }]
            );
        });

        // Drag down to the next line.
        selection.update(&mut app, |selection, ctx| {
            // 23 is in the middle of the last line.
            selection.update_pending_selection(23.into(), ctx);

            assert_eq!(
                selection.selections(ctx),
                vec1![SelectionOffsets {
                    head: 26.into(),
                    tail: 14.into()
                }]
            );
        });

        // Drag up to the first line. This should de-select the last line and reverse the selection
        // direction.
        selection.update(&mut app, |selection, ctx| {
            selection.update_pending_selection(3.into(), ctx);

            assert_eq!(
                selection.selections(ctx),
                vec![SelectionOffsets {
                    head: 1.into(),
                    tail: 19.into()
                }]
            );
        });
    });
}

#[test]
fn test_end_semantic_selection() {
    App::test((), |mut app| async move {
        let selection = app.add_model(|ctx| {
            let buffer = ctx.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
            let buffer_selection = ctx.add_model(|_| BufferSelectionModel::new(buffer.clone()));

            buffer.update(ctx, |buffer, ctx| {
                *buffer = Buffer::from_plain_text(
                    "First Second Third Fourth",
                    None,
                    Box::new(|_, _| IndentBehavior::Ignore),
                    buffer_selection.clone(),
                    ctx,
                );
            });

            let render = ctx.add_model(|_| {
                RenderState::new_for_test(
                    TEST_STYLES,
                    f32::MAX.into_pixels(),
                    f32::MAX.into_pixels(),
                )
            });

            SelectionModel::new(buffer, render, buffer_selection, None, ctx)
        });

        // Select the second word.
        selection.update(&mut app, |selection, ctx| {
            selection.begin_selection(
                10.into(),
                SelectionMode::Word(WordBoundariesPolicy::Default),
                true,
                ctx,
            );
            assert_eq!(
                selection.selections(ctx),
                vec1![SelectionOffsets {
                    head: 13.into(),
                    tail: 7.into()
                }]
            );
        });

        // Release without any dragging.
        selection.update(&mut app, |selection, ctx| {
            selection.end_selection(ctx);

            assert_eq!(
                selection.selections(ctx),
                vec1![SelectionOffsets {
                    head: 13.into(),
                    tail: 7.into()
                }]
            );
        });

        // Subsequent calls to extend the selection should have no effect.
        selection.update(&mut app, |selection, ctx| {
            selection.update_pending_selection(3.into(), ctx);

            assert_eq!(
                selection.selections(ctx),
                vec![SelectionOffsets {
                    head: 13.into(),
                    tail: 7.into()
                }]
            );
        });
    });
}
