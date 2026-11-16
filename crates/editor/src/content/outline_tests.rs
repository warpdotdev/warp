use itertools::Itertools;
use string_offset::CharOffset;
use warpui_core::App;

use crate::content::buffer::Buffer;
use crate::content::outline::BlockOutline;
use crate::content::selection_model::BufferSelectionModel;
use crate::content::text::{BlockType, BufferBlockStyle, IndentBehavior, TextStyles};

#[test]
fn test_no_blocks() {
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            let _ = buffer.edit_internal_first_selection(
                CharOffset::from(1)..CharOffset::from(1),
                "regular text",
                TextStyles::default(),
                selection.clone(),
                ctx,
            );
            assert_eq!(buffer.outline_blocks().count(), 0);
        });
    });
}

#[test]
fn test_block_at_start() {
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            let _ = buffer.edit_internal_first_selection(
                CharOffset::from(1)..CharOffset::from(1),
                "BlockText",
                TextStyles::default(),
                selection.clone(),
                ctx,
            );
            buffer.block_style_range(
                CharOffset::from(1)..CharOffset::from(6),
                BufferBlockStyle::CodeBlock {
                    code_block_type: Default::default(),
                },
                selection.clone(),
                ctx,
            );
            assert_eq!(buffer.debug(), "<code:Shell>Block<text>Text");

            let outline = buffer.outline_blocks().collect_vec();
            assert_eq!(
                outline,
                vec![BlockOutline {
                    start: CharOffset::from(0),
                    end: CharOffset::from(6),
                    block_type: BlockType::Text(BufferBlockStyle::CodeBlock {
                        code_block_type: Default::default()
                    })
                }]
            )
        });
    });
}

#[test]
fn test_block_at_end() {
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            let _ = buffer.edit_internal_first_selection(
                CharOffset::from(1)..CharOffset::from(1),
                "TextBlock",
                TextStyles::default(),
                selection.clone(),
                ctx,
            );
            buffer.block_style_range(
                CharOffset::from(5)..CharOffset::from(10),
                BufferBlockStyle::CodeBlock {
                    code_block_type: Default::default(),
                },
                selection.clone(),
                ctx,
            );
            assert_eq!(buffer.debug(), "<text>Text<code:Shell>Block<text>");

            let outline = buffer.outline_blocks().collect_vec();
            assert_eq!(
                outline,
                vec![BlockOutline {
                    start: CharOffset::from(5),
                    end: CharOffset::from(11),
                    block_type: BlockType::Text(BufferBlockStyle::CodeBlock {
                        code_block_type: Default::default()
                    })
                }]
            )
        });
    });
}

#[test]
fn test_multiple_blocks() {
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            let _ = buffer.edit_internal_first_selection(
                CharOffset::from(1)..CharOffset::from(1),
                "textFirsttextSecondtext",
                TextStyles::default(),
                selection.clone(),
                ctx,
            );
            buffer.block_style_range(
                CharOffset::from(14)..CharOffset::from(20),
                BufferBlockStyle::CodeBlock {
                    code_block_type: Default::default(),
                },
                selection.clone(),
                ctx,
            );
            buffer.block_style_range(
                CharOffset::from(5)..CharOffset::from(10),
                BufferBlockStyle::CodeBlock {
                    code_block_type: Default::default(),
                },
                selection.clone(),
                ctx,
            );
            assert_eq!(
                buffer.debug(),
                "<text>text<code:Shell>First<text>text<code:Shell>Second<text>text"
            );

            assert_eq!(
                buffer.outline_blocks().collect_vec(),
                vec![
                    BlockOutline {
                        start: CharOffset::from(5),
                        end: CharOffset::from(11),
                        block_type: BlockType::Text(BufferBlockStyle::CodeBlock {
                            code_block_type: Default::default()
                        })
                    },
                    BlockOutline {
                        start: CharOffset::from(16),
                        end: CharOffset::from(23),
                        block_type: BlockType::Text(BufferBlockStyle::CodeBlock {
                            code_block_type: Default::default()
                        })
                    }
                ]
            )
        });
    });
}

#[test]
fn test_adjacent_blocks() {
    // This is a regression test for when two blocks of the same kind are adjacent to each other.
    // This can occur when inserting a block right before another of the same kind.
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            let _ = buffer.edit_internal_first_selection(
                CharOffset::from(1)..CharOffset::from(1),
                "textFirstSecondtext",
                TextStyles::default(),
                selection.clone(),
                ctx,
            );
            buffer.block_style_range(
                CharOffset::from(10)..CharOffset::from(16),
                BufferBlockStyle::CodeBlock {
                    code_block_type: Default::default(),
                },
                selection.clone(),
                ctx,
            );
            buffer.block_style_range(
                CharOffset::from(5)..CharOffset::from(10),
                BufferBlockStyle::CodeBlock {
                    code_block_type: Default::default(),
                },
                selection.clone(),
                ctx,
            );
            assert_eq!(
                buffer.debug(),
                "<text>text<code:Shell>First<code:Shell>Second<text>text"
            );

            assert_eq!(
                buffer.outline_blocks().collect_vec(),
                vec![
                    BlockOutline {
                        start: CharOffset::from(5),
                        end: CharOffset::from(11),
                        block_type: BlockType::Text(BufferBlockStyle::CodeBlock {
                            code_block_type: Default::default()
                        })
                    },
                    BlockOutline {
                        start: CharOffset::from(11),
                        end: CharOffset::from(18),
                        block_type: BlockType::Text(BufferBlockStyle::CodeBlock {
                            code_block_type: Default::default()
                        })
                    }
                ]
            );
        });
    });
}
