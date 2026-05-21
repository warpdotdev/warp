use std::ops::Range;
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};

use line_ending::LineEnding;
use markdown_parser::{
    FormattedIndentTextInline, FormattedText, FormattedTextFragment, FormattedTextLine, parse_html,
    parse_markdown,
};
use pathfinder_color::ColorU;
use rand::SeedableRng;
use rand::rngs::StdRng;
use serde_yaml::{Mapping, Value};
use vec1::{Vec1, vec1};
use warpui::{App, AppContext, ModelContext, ModelHandle, ReadModel};

use crate::content::buffer::{
    AutoScrollBehavior, BufferEditAction, BufferSelectAction, EditOrigin, EmbeddedItemConversion,
    InitialBufferState, SelectionOffsets, StyledBlockBoundaryBehavior, StyledBufferBlock,
    StyledTextBlock, TabIndentation, ToBufferByteOffset, ToBufferPoint,
};
use crate::content::core::{CoreEditorAction, CoreEditorActionType};
use crate::content::cursor::BufferSumTree;
use crate::content::edit::PreciseDelta;
use crate::content::markdown::MarkdownStyle;
use crate::content::selection::TextStyleBias;
use crate::content::selection_model::BufferSelectionModel;
use crate::content::text::{
    BlockHeaderSize, BlockType, BufferBlockItem, BufferBlockStyle, CodeBlockType, IndentBehavior,
    IndentUnit, TABLE_BLOCK_MARKDOWN_LANG, TextStyles, TextStylesWithMetadata,
};
use crate::content::undo::{
    NonAtomicType, ReversibleEditorActions, ReversibleSelectionState, UndoActionType, UndoArg,
};
use crate::render::layout::TextLayout;
use crate::render::model::{
    EmbeddedItem, EmbeddedItemHTMLRepresentation, EmbeddedItemRichFormat, LaidOutEmbeddedItem,
    RenderedSelectionSet,
};
use string_offset::ByteOffset;
use string_offset::CharOffset;
use warpui::elements::ListIndentLevel;
use warpui::text::point::Point;

use crate::content::buffer::{Buffer, StyledBufferRun};

use super::{BufferEvent, EditResult, ToBufferCharOffset};
use warp_util::content_version::ContentVersion;

#[derive(Debug)]
pub struct TestEmbeddedItem {
    pub(crate) id: String,
}

impl EmbeddedItem for TestEmbeddedItem {
    // We don't need to implement this for testing.
    fn layout(&self, _: &TextLayout, _: &AppContext) -> Box<dyn LaidOutEmbeddedItem> {
        unimplemented!()
    }

    fn to_mapping(&self, style: MarkdownStyle) -> Mapping {
        let mut mapping = Mapping::from_iter([(
            Value::String("id".to_string()),
            Value::String(self.hashed_id().to_string()),
        )]);
        if let MarkdownStyle::Export { .. } = style {
            mapping.insert("export".into(), true.into());
        }
        mapping
    }

    fn hashed_id(&self) -> &str {
        self.id.as_str()
    }

    fn to_rich_format(&self, _app: &AppContext) -> EmbeddedItemRichFormat<'_> {
        EmbeddedItemRichFormat {
            plain_text: self.hashed_id().to_string(),
            html: EmbeddedItemHTMLRepresentation {
                element_name: "pre",
                content: self.hashed_id().to_string(),
                attributes: Default::default(),
            },
        }
    }
}

fn set_selections<T: Into<CharOffset>>(
    selection_model: &mut BufferSelectionModel,
    selections: Vec1<Range<T>>,
) {
    selection_model.set_selection_offsets(selections.mapped(|range| SelectionOffsets {
        head: range.end.into(),
        tail: range.start.into(),
    }));
}

impl Buffer {
    // Helper function to push a nonatomic action to the undo stack.
    // This is needed to skip the timer check in unit test since it could cause
    // flakiness.
    fn push_undo_item_nonatomic(
        &mut self,
        prev_selection_range: RenderedSelectionSet,
        arg: UndoArg,
        action: NonAtomicType,
        selection_model: ModelHandle<BufferSelectionModel>,
        ctx: &mut ModelContext<Self>,
    ) {
        self.undo_stack.push_non_atomic_edit(
            ReversibleEditorActions {
                actions: arg.actions,
                replacement_range: arg.replacement_range,
                selections: ReversibleSelectionState {
                    next: prev_selection_range,
                    reverse: self.to_rendered_selection_set(selection_model, ctx),
                },
            },
            action,
            self.content_version,
        );
    }

    // Helper function to get the html for one range.
    fn range_as_html(&self, range: Range<CharOffset>, ctx: &AppContext) -> Option<String> {
        self.ranges_as_html(vec1![range], ctx)
    }

    // Helper function to set a single selection range.
    fn set_selection(
        &mut self,
        range: Range<CharOffset>,
        selection_model: ModelHandle<BufferSelectionModel>,
        ctx: &mut ModelContext<Self>,
    ) {
        self.add_cursor(range.start, true, selection_model.clone(), ctx);
        self.set_last_head(range.end, selection_model, ctx);
    }

    /// Helper to replace previous uses of buffer.style_link_internal
    fn select_and_style_link(
        &mut self,
        range: Range<CharOffset>,
        text: String,
        url: String,
        selection_model: ModelHandle<BufferSelectionModel>,
        ctx: &mut ModelContext<Self>,
    ) -> EditResult {
        self.set_selection(range, selection_model.clone(), ctx);
        self.style_link_internal(text, url, selection_model, ctx)
    }

    pub fn mock_from_markdown(
        markdown: &str,
        embedded_item_conversion: Option<EmbeddedItemConversion>,
        tab_indentation: TabIndentation,
        ctx: &mut App,
    ) -> (ModelHandle<Self>, ModelHandle<BufferSelectionModel>) {
        let buffer = ctx.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection = ctx.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(ctx, |buffer, ctx| {
            *buffer = Buffer::from_markdown(
                markdown,
                embedded_item_conversion,
                tab_indentation,
                selection.clone(),
                ctx,
            );
        });

        (buffer, selection)
    }
}

#[test]
fn test_edit_plain_text() {
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            let _ = buffer.edit_internal("test", TextStyles::default(), selection.clone(), ctx);
            assert_eq!(buffer.content.debug(), "<text>test");

            buffer.set_selection(
                CharOffset::from(1)..CharOffset::from(4),
                selection.clone(),
                ctx,
            );
            let _ = buffer.edit_internal("be", TextStyles::default(), selection.clone(), ctx);
            assert_eq!(buffer.content.debug(), "<text>bet");

            buffer.set_selection(
                CharOffset::from(2)..CharOffset::from(3),
                selection.clone(),
                ctx,
            );
            let _ = buffer.edit_internal("", TextStyles::default(), selection.clone(), ctx);
            assert_eq!(buffer.content.debug(), "<text>bt");

            buffer.set_selection(
                CharOffset::from(1)..CharOffset::from(1),
                selection.clone(),
                ctx,
            );
            let _ = buffer.edit_internal("ke", TextStyles::default(), selection.clone(), ctx);
            assert_eq!(buffer.content.debug(), "<text>kebt");

            buffer.set_selection(
                CharOffset::from(2)..CharOffset::from(4),
                selection.clone(),
                ctx,
            );
            let _ = buffer.edit_internal("m", TextStyles::default(), selection.clone(), ctx);
            assert_eq!(buffer.content.debug(), "<text>kmt");
        });
    });
}

#[test]
fn test_edit_end_of_text() {
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            let _ = buffer.edit_internal("test", TextStyles::default(), selection.clone(), ctx);
            assert_eq!(buffer.content.debug(), "<text>test");

            let _ = buffer.enter(false, TextStyles::default(), selection.clone(), ctx);
            assert_eq!(buffer.content.debug(), "<text>test\\n");

            let _ = buffer.enter(false, TextStyles::default(), selection.clone(), ctx);
            assert_eq!(buffer.content.debug(), "<text>test\\n\\n");
        });
    });
}

#[test]
fn test_edit_styled_text() {
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            let _ = buffer.edit_internal("te", TextStyles::default(), selection.clone(), ctx);
            assert_eq!(buffer.text().as_str(), "te");
            assert_eq!(buffer.content.debug(), "<text>te");

            buffer.set_selection(
                CharOffset::from(3)..CharOffset::from(3),
                selection.clone(),
                ctx,
            );
            // Correct buffer state: te<b>st<b>.
            let _ =
                buffer.edit_internal("st", TextStyles::default().bold(), selection.clone(), ctx);
            assert_eq!(buffer.content.debug(), "<text>te<b_s>st<b_e>");

            // Correct buffer state: te<i>x<i><b>t<b>.
            buffer.set_selection(
                CharOffset::from(3)..CharOffset::from(4),
                selection.clone(),
                ctx,
            );
            let _ =
                buffer.edit_internal("x", TextStyles::default().italic(), selection.clone(), ctx);
            assert_eq!(buffer.content.debug(), "<text>te<i_s>x<b_s><i_e>t<b_e>");

            // Correct buffer state: t<b>i<b><i>x<i><b>t<b>.
            buffer.set_selection(
                CharOffset::from(2)..CharOffset::from(3),
                selection.clone(),
                ctx,
            );
            let _ = buffer.edit_internal("i", TextStyles::default().bold(), selection.clone(), ctx);
            assert_eq!(
                buffer.content.debug(),
                "<text>t<b_s>i<b_e><i_s>x<b_s><i_e>t<b_e>"
            );

            // Correct buffer state: t<b>i<b>a<b>t<b>.
            buffer.set_selection(
                CharOffset::from(3)..CharOffset::from(4),
                selection.clone(),
                ctx,
            );
            let _ = buffer.edit_internal("a", TextStyles::default(), selection.clone(), ctx);
            assert_eq!(buffer.content.debug(), "<text>t<b_s>i<b_e>a<b_s>t<b_e>");

            // Correct buffer state: t<b>at<b>.
            buffer.set_selection(
                CharOffset::from(2)..CharOffset::from(4),
                selection.clone(),
                ctx,
            );
            let _ = buffer.edit_internal("a", TextStyles::default().bold(), selection.clone(), ctx);
            assert_eq!(buffer.content.debug(), "<text>t<b_s>at<b_e>");

            // Correct buffer state: t<b>axt<b>.
            buffer.set_selection(
                CharOffset::from(3)..CharOffset::from(3),
                selection.clone(),
                ctx,
            );
            let _ = buffer.edit_internal("x", TextStyles::default().bold(), selection.clone(), ctx);
            assert_eq!(buffer.content.debug(), "<text>t<b_s>axt<b_e>");

            // Correct buffer state: t<b>axting<b>.
            buffer.set_selection(
                CharOffset::from(5)..CharOffset::from(5),
                selection.clone(),
                ctx,
            );
            let _ =
                buffer.edit_internal("ing", TextStyles::default().bold(), selection.clone(), ctx);
            assert_eq!(buffer.content.debug(), "<text>t<b_s>axting<b_e>");
        });
    });
}

#[test]
fn test_edit_with_anchors() {
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            buffer.edit_internal("test", TextStyles::default(), selection.clone(), ctx);
            assert_eq!(buffer.text().as_str(), "test");
        });

        // Anchors are clamped to the end of the buffer
        let end = selection.update(&mut app, |selection, ctx| selection.anchor(500.into(), ctx));

        selection.read(&app, |selection, _| {
            assert_eq!(selection.resolve_anchor(&end), Some(5.into()));
        });

        buffer.update(&mut app, |buffer, ctx| {
            // When inserting text, the anchor is updated.
            buffer.set_selection(
                CharOffset::from(5)..CharOffset::from(5),
                selection.clone(),
                ctx,
            );
            buffer.edit_internal("ing", TextStyles::default(), selection.clone(), ctx);
            assert_eq!(buffer.text().as_str(), "testing");
        });

        selection.read(&app, |selection, _| {
            assert_eq!(selection.resolve_anchor(&end), Some(8.into()));
        });

        buffer.update(&mut app, |buffer, ctx| {
            // Likewise, it's updated when deleting, at the cursor or before it.
            buffer.set_selection(
                CharOffset::from(7)..CharOffset::from(8),
                selection.clone(),
                ctx,
            );
            buffer.edit_internal("", TextStyles::default(), selection.clone(), ctx);
            assert_eq!(buffer.text().as_str(), "testin");
        });

        selection.read(&app, |selection, _| {
            assert_eq!(selection.resolve_anchor(&end), Some(7.into()));
        });

        buffer.update(&mut app, |buffer, ctx| {
            buffer.set_selection(
                CharOffset::from(3)..CharOffset::from(5),
                selection.clone(),
                ctx,
            );
            buffer.edit_internal("", TextStyles::default(), selection.clone(), ctx);
            assert_eq!(buffer.text().as_str(), "tein");
        });

        selection.read(&app, |selection, _| {
            assert_eq!(selection.resolve_anchor(&end), Some(5.into()));
        });

        // If there are multiple anchors, all stay in sync.
        let middle = selection.update(&mut app, |selection, ctx| selection.anchor(2.into(), ctx));

        buffer.update(&mut app, |buffer, ctx| {
            buffer.set_selection(
                CharOffset::from(1)..CharOffset::from(1),
                selection.clone(),
                ctx,
            );
            buffer.edit_internal("pro", TextStyles::default(), selection.clone(), ctx);
            assert_eq!(buffer.text().as_str(), "protein");
        });

        selection.read(&app, |selection, _| {
            assert_eq!(selection.resolve_anchor(&middle), Some(5.into()));
            assert_eq!(selection.resolve_anchor(&end), Some(8.into()));
        });

        buffer.update(&mut app, |buffer, ctx| {
            // Anchors may be invalidated by deletion.
            buffer.set_selection(
                CharOffset::from(4)..CharOffset::from(6),
                selection.clone(),
                ctx,
            );
            buffer.edit_internal("", TextStyles::default(), selection.clone(), ctx);
            assert_eq!(buffer.text().as_str(), "proin");
        });

        selection.read(&app, |selection, _| {
            assert_eq!(selection.resolve_anchor(&middle), None);
            assert_eq!(selection.resolve_anchor(&end), Some(6.into()));
        });
    });
}

#[test]
fn test_replace_above_with_anchors() {
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            buffer.edit_internal(
                "Hello world.\nGoodbye world.",
                TextStyles::default(),
                selection.clone(),
                ctx,
            );
            assert_eq!(buffer.text().as_str(), "Hello world.\nGoodbye world.");
        });

        let anchor = selection.update(&mut app, |selection, ctx| selection.anchor(14.into(), ctx));

        selection.read(&app, |selection, _| {
            assert_eq!(selection.resolve_anchor(&anchor), Some(14.into()));
        });

        // Second anchor at the start of the buffer.
        let anchor_2 = selection.update(&mut app, |selection, ctx| selection.anchor(1.into(), ctx));

        selection.read(&app, |selection, _| {
            assert_eq!(selection.resolve_anchor(&anchor_2), Some(1.into()));
        });

        // Third anchor at the end of the buffer.
        let anchor_3 =
            selection.update(&mut app, |selection, ctx| selection.anchor(28.into(), ctx));

        selection.read(&app, |selection, _| {
            assert_eq!(selection.resolve_anchor(&anchor_3), Some(28.into()));
        });

        buffer.update(&mut app, |buffer, ctx| {
            buffer.replace(
                InitialBufferState::markdown("Howdy.\nGoodbye world."),
                selection.clone(),
                ctx,
            );
            assert_eq!(buffer.text().as_str(), "Howdy.\nGoodbye world.");
        });

        // Validate the buffer state.
        selection.read(&app, |selection, _| {
            buffer.read(&app, |buffer, _| {
                buffer.validate(&selection.anchors);
            });
        });
    });
}

#[test]
fn test_replace_with_anchors() {
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            buffer.edit_internal("test", TextStyles::default(), selection.clone(), ctx);
        });

        let anchor = selection.update(&mut app, |selection, ctx| selection.anchor(3.into(), ctx));

        buffer.update(&mut app, |buffer, ctx| {
            // Replace on top of the anchor, with a net increase in character count.
            buffer.set_selection(
                CharOffset::from(2)..CharOffset::from(4),
                selection.clone(),
                ctx,
            );
            buffer.edit_internal("oas", TextStyles::default(), selection.clone(), ctx);
            assert_eq!(buffer.text().as_str(), "toast");
        });

        selection.read(&app, |selection, _| {
            assert_eq!(selection.resolve_anchor(&anchor), Some(3.into()));
        });

        buffer.update(&mut app, |buffer, ctx| {
            // Now, replace on top of the anchor with a net decrease in character count.
            buffer.set_selection(
                CharOffset::from(1)..CharOffset::from(4),
                selection.clone(),
                ctx,
            );
            buffer.edit_internal("p", TextStyles::default(), selection.clone(), ctx);
            assert_eq!(buffer.text().as_str(), "pst");
        });

        selection.read(&app, |selection, _| {
            assert_eq!(selection.resolve_anchor(&anchor), None);
        });
    });
}

#[test]
fn test_edit_delta_ranges() {
    // This tests that editing returns the correct modification ranges, which is
    // important for updating the rendering model.
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            let delta = buffer
                .edit_internal("Test", Default::default(), selection.clone(), ctx)
                .delta
                .expect("Edit range should exist");

            // This is a fresh insertion, so the range in the original content is zero-length.
            assert_eq!(delta.old_offset, CharOffset::from(1)..CharOffset::from(1));

            // Inserting into the middle of a line modifies the whole line.
            buffer.set_selection(
                CharOffset::from(3)..CharOffset::from(3),
                selection.clone(),
                ctx,
            );
            let delta = buffer
                .edit_internal("x", Default::default(), selection.clone(), ctx)
                .delta
                .expect("Edit range should exist");
            assert_eq!(buffer.text().as_str(), "Texst");
            assert_eq!(delta.old_offset, CharOffset::from(1)..CharOffset::from(5));

            // A newline modifies both the line before and the line after it.
            buffer.set_selection(
                CharOffset::from(6)..CharOffset::from(6),
                selection.clone(),
                ctx,
            );
            let delta = buffer
                .edit_internal("\n", Default::default(), selection.clone(), ctx)
                .delta
                .expect("Edit range should exist");
            assert_eq!(buffer.text().as_str(), "Texst\n");
            assert_eq!(delta.old_offset, CharOffset::from(1)..CharOffset::from(6));

            // Typing at the start of a line, however, does not modify the previous line.
            // Note that in both this case and the newline one, the original edit range
            // is the very end of the buffer. However, the affected range for rendering is
            // quite different.
            buffer.set_selection(
                CharOffset::from(7)..CharOffset::from(7),
                selection.clone(),
                ctx,
            );
            let delta = buffer
                .edit_internal("Hi", Default::default(), selection.clone(), ctx)
                .delta
                .expect("Edit range should exist");
            assert_eq!(buffer.text().as_str(), "Texst\nHi");
            assert_eq!(delta.old_offset, CharOffset::from(7)..CharOffset::from(7));

            // A newline that splits an existing line modifies just that line, even though
            // it creates two new ones.
            buffer.set_selection(
                CharOffset::from(3)..CharOffset::from(3),
                selection.clone(),
                ctx,
            );
            let delta = buffer
                .edit_internal("\n", Default::default(), selection.clone(), ctx)
                .delta
                .expect("Edit range should exist");
            assert_eq!(buffer.text().as_str(), "Te\nxst\nHi");
            assert_eq!(delta.old_offset, CharOffset::from(1)..CharOffset::from(7));
            assert_eq!(delta.new_lines.len(), 2);

            // Deleting that same newline replaces the two lines with one.
            buffer.set_selection(
                CharOffset::from(3)..CharOffset::from(4),
                selection.clone(),
                ctx,
            );
            let delta = buffer
                .edit_internal("", Default::default(), selection.clone(), ctx)
                .delta
                .expect("Edit range should exist");
            assert_eq!(buffer.text().as_str(), "Texst\nHi");
            assert_eq!(delta.old_offset, CharOffset::from(1)..CharOffset::from(8));
            assert_eq!(delta.new_lines.len(), 1);

            // Applying a no-op operation should return a None delta.
            buffer.set_selection(
                CharOffset::from(2)..CharOffset::from(3),
                selection.clone(),
                ctx,
            );
            let _ = buffer
                .style_internal(TextStyles::default().bold(), selection.clone(), ctx)
                .delta
                .expect("Edit range should exist");
            let delta = buffer
                .style_internal(TextStyles::default().bold(), selection.clone(), ctx)
                .delta;
            assert!(delta.is_none());
        });
    });
}

#[test]
fn test_text_styling() {
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            let _ = buffer.edit_internal_first_selection(
                CharOffset::from(1)..CharOffset::from(1),
                "hello",
                Default::default(),
                selection.clone(),
                ctx,
            );
            assert_eq!(buffer.content.debug(), "<text>hello");

            // Correct buffer state: h<b>el<b>lo.
            buffer.set_selection(
                CharOffset::from(2)..CharOffset::from(4),
                selection.clone(),
                ctx,
            );
            let _ = buffer.style_internal(TextStyles::default().bold(), selection.clone(), ctx);
            assert_eq!(buffer.content.debug(), "<text>h<b_s>el<b_e>lo");

            // Correct buffer state: h<b>e<i>ll<i><b>o.
            buffer.set_selection(
                CharOffset::from(3)..CharOffset::from(5),
                selection.clone(),
                ctx,
            );
            let _ = buffer.style_internal(
                TextStyles::default().bold().italic(),
                selection.clone(),
                ctx,
            );
            assert_eq!(buffer.content.debug(), "<text>h<b_s>e<i_s>ll<b_e><i_e>o");

            // Correct buffer state: <b>he<i>ll<i><b>o.
            buffer.set_selection(
                CharOffset::from(1)..CharOffset::from(3),
                selection.clone(),
                ctx,
            );
            let _ = buffer.style_internal(TextStyles::default().bold(), selection.clone(), ctx);
            assert_eq!(buffer.content.debug(), "<text><b_s>he<i_s>ll<b_e><i_e>o");

            let _ = buffer.edit_internal_first_selection(
                CharOffset::from(1)..CharOffset::from(6),
                "test",
                Default::default(),
                selection.clone(),
                ctx,
            );
            let _ = buffer.block_style_range(
                CharOffset::from(3)..CharOffset::from(5),
                BufferBlockStyle::CodeBlock {
                    code_block_type: Default::default(),
                },
                selection.clone(),
                ctx,
            );
            buffer.set_selection(
                CharOffset::from(1)..CharOffset::from(3),
                selection.clone(),
                ctx,
            );
            let _ = buffer.style_internal(TextStyles::default().bold(), selection.clone(), ctx);
            assert_eq!(
                buffer.content.debug(),
                "<text><b_s>te<b_e><code:Shell>st<text>"
            );

            buffer.set_selection(
                CharOffset::from(1)..CharOffset::from(3),
                selection.clone(),
                ctx,
            );
            let _ =
                buffer.style_internal(TextStyles::default().inline_code(), selection.clone(), ctx);
            assert_eq!(
                buffer.content.debug(),
                "<text><b_s><c_s>te<b_e><c_e><code:Shell>st<text>"
            );
        });
    });
}

#[test]
fn test_text_unstyling() {
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            let _ = buffer.edit_internal_first_selection(
                CharOffset::from(1)..CharOffset::from(1),
                "hello",
                Default::default(),
                selection.clone(),
                ctx,
            );
            assert_eq!(buffer.content.debug(), "<text>hello");

            // Correct buffer state: h<b>ell<b>o.
            buffer.set_selection(
                CharOffset::from(2)..CharOffset::from(5),
                selection.clone(),
                ctx,
            );
            let _ = buffer.style_internal(TextStyles::default().bold(), selection.clone(), ctx);
            assert_eq!(buffer.content.debug(), "<text>h<b_s>ell<b_e>o");

            // Correct buffer state: h<b>e<i>ll<i><b>o.
            buffer.set_selection(
                CharOffset::from(3)..CharOffset::from(5),
                selection.clone(),
                ctx,
            );
            let _ = buffer.style_internal(TextStyles::default().italic(), selection.clone(), ctx);
            assert_eq!(buffer.content.debug(), "<text>h<b_s>e<i_s>ll<b_e><i_e>o");

            // Correct buffer state: h<b>e<b>l<b><i>l<i><b>o.
            buffer.set_selection(
                CharOffset::from(3)..CharOffset::from(4),
                selection.clone(),
                ctx,
            );
            let _ = buffer.unstyle_internal(
                TextStyles::default().bold().italic(),
                selection.clone(),
                ctx,
            );
            assert_eq!(
                buffer.content.debug(),
                "<text>h<b_s>e<b_e>l<b_s><i_s>l<b_e><i_e>o"
            );

            // Correct buffer state: h<b>e<b>l<i>l<i>o.
            buffer.set_selection(
                CharOffset::from(4)..CharOffset::from(5),
                selection.clone(),
                ctx,
            );
            let _ = buffer.unstyle_internal(TextStyles::default().bold(), selection.clone(), ctx);
            assert_eq!(buffer.content.debug(), "<text>h<b_s>e<b_e>l<i_s>l<i_e>o");

            buffer.set_selection(
                CharOffset::from(2)..CharOffset::from(4),
                selection.clone(),
                ctx,
            );
            let _ = buffer.style_internal(
                TextStyles::default().strikethrough(),
                selection.clone(),
                ctx,
            );
            assert_eq!(
                buffer.content.debug(),
                "<text>h<b_s><s_s>e<b_e>l<i_s><s_e>l<i_e>o"
            );

            buffer.set_selection(
                CharOffset::from(2)..CharOffset::from(3),
                selection.clone(),
                ctx,
            );
            let _ = buffer.unstyle_internal(
                TextStyles::default().strikethrough(),
                selection.clone(),
                ctx,
            );
            assert_eq!(
                buffer.content.debug(),
                "<text>h<b_s>e<b_e><s_s>l<i_s><s_e>l<i_e>o"
            );
        });
    });
}

#[test]
fn test_text_unstyling_not_full_range() {
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            let _ = buffer.edit_internal_first_selection(
                CharOffset::from(1)..CharOffset::from(1),
                "hello",
                TextStyles::default(),
                selection.clone(),
                ctx,
            );
            assert_eq!(buffer.content.debug(), "<text>hello");

            // Correct buffer state: h<b>ell<b>o.
            buffer.set_selection(
                CharOffset::from(2)..CharOffset::from(5),
                selection.clone(),
                ctx,
            );
            let _ = buffer.style_internal(TextStyles::default().bold(), selection.clone(), ctx);
            assert_eq!(buffer.content.debug(), "<text>h<b_s>ell<b_e>o");

            // Correct buffer state: h<b>e<i>ll<i><b>o.
            buffer.set_selection(
                CharOffset::from(3)..CharOffset::from(5),
                selection.clone(),
                ctx,
            );
            let _ = buffer.style_internal(TextStyles::default().italic(), selection.clone(), ctx);
            assert_eq!(buffer.content.debug(), "<text>h<b_s>e<i_s>ll<b_e><i_e>o");

            // Correct buffer state: he<b><i>ll<i><b>o.
            buffer.set_selection(
                CharOffset::from(1)..CharOffset::from(3),
                selection.clone(),
                ctx,
            );
            let _ = buffer.unstyle_internal(TextStyles::default().bold(), selection.clone(), ctx);
            assert_eq!(buffer.content.debug(), "<text>he<i_s><b_s>ll<b_e><i_e>o");

            // Correct buffer state: he<b><i>l<i>l<b>o.
            buffer.set_selection(
                CharOffset::from(4)..CharOffset::from(6),
                selection.clone(),
                ctx,
            );
            let _ = buffer.unstyle_internal(TextStyles::default().italic(), selection.clone(), ctx);
            assert_eq!(buffer.content.debug(), "<text>he<i_s><b_s>l<i_e>l<b_e>o");

            // Correct buffer state: hel<b>l<b>o.
            buffer.set_selection(
                CharOffset::from(2)..CharOffset::from(4),
                selection.clone(),
                ctx,
            );
            let _ = buffer.unstyle_internal(
                TextStyles::default().bold().italic(),
                selection.clone(),
                ctx,
            );
            assert_eq!(buffer.content.debug(), "<text>hel<b_s>l<b_e>o");
        });
    });
}

#[test]
fn test_range_fully_styled() {
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            let _ = buffer.edit_internal_first_selection(
                CharOffset::from(1)..CharOffset::from(1),
                "hello",
                TextStyles::default(),
                selection.clone(),
                ctx,
            );
            assert_eq!(buffer.text().as_str(), "hello");

            // Correct buffer state: h<b>ell<b>o.
            buffer.set_selection(
                CharOffset::from(2)..CharOffset::from(5),
                selection.clone(),
                ctx,
            );
            let _ = buffer.style_internal(TextStyles::default().bold(), selection.clone(), ctx);
            assert_eq!(buffer.content.debug(), "<text>h<b_s>ell<b_e>o");

            // Correct buffer state: h<b>e<i>ll<i><b>o.
            buffer.set_selection(
                CharOffset::from(3)..CharOffset::from(5),
                selection.clone(),
                ctx,
            );
            let _ = buffer.style_internal(TextStyles::default().italic(), selection.clone(), ctx);
            assert_eq!(buffer.content.debug(), "<text>h<b_s>e<i_s>ll<b_e><i_e>o");

            assert!(
                !buffer
                    .ranges_fully_styled(vec1![1.into()..3.into()], TextStyles::default().bold())
            );
            assert!(
                buffer.ranges_fully_styled(vec1![2.into()..4.into()], TextStyles::default().bold())
            );
            assert!(buffer.ranges_fully_styled(
                vec1![3.into()..5.into()],
                TextStyles::default().bold().italic()
            ));
            assert!(
                !buffer
                    .ranges_fully_styled(vec1![4.into()..6.into()], TextStyles::default().italic())
            );
        });
    });
}

#[test]
fn test_range_styles() {
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            buffer.edit_internal_first_selection(
                CharOffset::from(1)..CharOffset::from(1),
                "boldbothitalicplain",
                Default::default(),
                selection.clone(),
                ctx,
            );
            let bold_range = CharOffset::from(1)..CharOffset::from(5);
            let both_range = CharOffset::from(5)..CharOffset::from(9);
            let italic_range = CharOffset::from(9)..CharOffset::from(15);

            buffer.set_selection(bold_range.clone(), selection.clone(), ctx);
            buffer.style_internal(TextStyles::default().bold(), selection.clone(), ctx);
            buffer.set_selection(both_range.clone(), selection.clone(), ctx);
            buffer.style_internal(
                TextStyles::default().bold().italic(),
                selection.clone(),
                ctx,
            );
            buffer.set_selection(italic_range.clone(), selection.clone(), ctx);
            buffer.style_internal(TextStyles::default().italic(), selection.clone(), ctx);

            let cases = [
                // Fully within one style range.
                (2, 3, TextStylesWithMetadata::default().bold()),
                // Partly just bold, and partly both - a new style starts in the range.
                (4, 6, TextStylesWithMetadata::default().bold()),
                // Partly bold, partly both, and partly italic - styles start and end in the range.
                (4, 11, TextStylesWithMetadata::default()),
                // Fully both.
                (6, 8, TextStylesWithMetadata::default().bold().italic()),
                // Partly both, and partly italic - a style ends in the range.
                (7, 11, TextStylesWithMetadata::default().italic()),
                // Partly italic and partly plain.
                (13, 17, TextStylesWithMetadata::default()),
                // Fully plain.
                (17, 18, TextStylesWithMetadata::default()),
            ];

            for (start, end, expected_styles) in cases {
                let range_styles = buffer.range_text_styles(start.into()..end.into());
                assert_eq!(
                    range_styles, expected_styles,
                    "Incorrect styles over {start}..{end}",
                );
            }
        });
    });
}

#[test]
fn test_buffer_line_metadata() {
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            let target_text = "test\nhello\nword";
            let _ = buffer.edit_internal_first_selection(
                CharOffset::from(1)..CharOffset::from(1),
                target_text,
                TextStyles::default(),
                selection.clone(),
                ctx,
            );

            assert_eq!(
                buffer.len(),
                CharOffset::from(target_text.chars().count()) + 1
            );
            assert_eq!(buffer.max_point(), Point::new(3, 4));
            assert_eq!(buffer.line_len(1), 4);
            assert_eq!(buffer.line_len(2), 5);
            assert_eq!(buffer.line_len(3), 4);
        });
    });
}

#[test]
fn test_random() {
    // Use a fixed seed for stable output.
    let mut rng = StdRng::seed_from_u64(123456);
    let buffer = Buffer::random(&mut rng, 50);
    assert!(buffer.len().as_usize() <= 50);
    assert_eq!(
        buffer.text().as_str(),
        "d3\nSOf\ngZjvGHqkBxl2583x69F13\n\n8wlTivQFFQ9cY"
    );
    assert_eq!(
        buffer.content.debug(),
        "<text>d3\\nSOf\\ngZjvGHqkBxl25<c_s>83x69F<i_s>13\\n\\n8wlTivQFFQ9cY<i_e><c_e>"
    );
}

#[test]
fn test_edit_delta() {
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            // Insert content into an empty buffer. EditDelta should return all the inserted
            // stylized text runs.
            let delta = buffer
                .edit_internal("te\nst", TextStyles::default(), selection.clone(), ctx)
                .delta
                .expect("Should exist");
            assert_eq!(buffer.content.debug(), "<text>te\\nst");
            assert_eq!(
                delta.precise_deltas,
                vec![PreciseDelta {
                    replaced_range: CharOffset::from(1)..CharOffset::from(1),
                    replaced_points: Point::new(1, 0)..Point::new(1, 0),
                    replaced_byte_range: ByteOffset::from(1)..ByteOffset::from(1),
                    new_byte_length: 5,
                    new_end_point: Point::new(2, 2),
                    resolved_range: CharOffset::from(1)..CharOffset::from(6),
                }]
            );
            assert_eq!(
                delta.new_lines,
                vec![
                    StyledBufferBlock::Text(StyledTextBlock {
                        block: vec![StyledBufferRun {
                            run: "te\n".to_string(),
                            text_styles: TextStylesWithMetadata::default(),
                            block_style: BufferBlockStyle::PlainText
                        },],
                        style: BufferBlockStyle::PlainText,
                        content_length: CharOffset::from(3)
                    }),
                    StyledBufferBlock::Text(StyledTextBlock {
                        block: vec![StyledBufferRun {
                            run: "st".to_string(),
                            text_styles: TextStylesWithMetadata::default(),
                            block_style: BufferBlockStyle::PlainText
                        },],
                        style: BufferBlockStyle::PlainText,
                        content_length: CharOffset::from(2)
                    })
                ]
            );

            // Remove a newline character to collapse two lines into one. EditDelta should return
            // the collapsed line.
            buffer.set_selection(
                CharOffset::from(2)..CharOffset::from(4),
                selection.clone(),
                ctx,
            );
            let delta = buffer
                .edit_internal("n", TextStyles::default(), selection.clone(), ctx)
                .delta
                .expect("Should exist");
            assert_eq!(buffer.content.debug(), "<text>tnst");
            assert_eq!(
                delta.precise_deltas,
                vec![PreciseDelta {
                    replaced_range: CharOffset::from(2)..CharOffset::from(4),
                    replaced_points: Point::new(1, 1)..Point::new(2, 0),
                    replaced_byte_range: ByteOffset::from(2)..ByteOffset::from(4),
                    new_byte_length: 1,
                    new_end_point: Point::new(1, 2),
                    resolved_range: CharOffset::from(2)..CharOffset::from(3),
                }]
            );
            assert_eq!(
                delta.new_lines,
                vec![StyledBufferBlock::Text(StyledTextBlock {
                    block: vec![StyledBufferRun {
                        run: "tnst".to_string(),
                        text_styles: TextStylesWithMetadata::default(),
                        block_style: BufferBlockStyle::PlainText
                    },],
                    style: BufferBlockStyle::PlainText,
                    content_length: CharOffset::from(4)
                }),]
            );

            // Insert multiple lines into the buffer. EditDelta should return all the inserted line.
            buffer.set_selection(
                CharOffset::from(2)..CharOffset::from(4),
                selection.clone(),
                ctx,
            );
            let delta = buffer
                .edit_internal("n\n\n", TextStyles::default(), selection.clone(), ctx)
                .delta
                .expect("Should exist");
            assert_eq!(buffer.content.debug(), "<text>tn\\n\\nt");
            assert_eq!(
                delta.precise_deltas,
                vec![PreciseDelta {
                    replaced_range: CharOffset::from(2)..CharOffset::from(4),
                    replaced_points: Point::new(1, 1)..Point::new(1, 3),
                    replaced_byte_range: ByteOffset::from(2)..ByteOffset::from(4),
                    new_byte_length: 3,
                    new_end_point: Point::new(3, 0),
                    resolved_range: CharOffset::from(2)..CharOffset::from(5),
                }]
            );
            assert_eq!(
                delta.new_lines,
                vec![
                    StyledBufferBlock::Text(StyledTextBlock {
                        block: vec![StyledBufferRun {
                            run: "tn\n".to_string(),
                            text_styles: TextStylesWithMetadata::default(),
                            block_style: BufferBlockStyle::PlainText
                        },],
                        style: BufferBlockStyle::PlainText,
                        content_length: CharOffset::from(3)
                    }),
                    StyledBufferBlock::Text(StyledTextBlock {
                        block: vec![StyledBufferRun {
                            run: "\n".to_string(),
                            text_styles: TextStylesWithMetadata::default(),
                            block_style: BufferBlockStyle::PlainText
                        },],
                        style: BufferBlockStyle::PlainText,
                        content_length: CharOffset::from(1)
                    }),
                    StyledBufferBlock::Text(StyledTextBlock {
                        block: vec![StyledBufferRun {
                            run: "t".to_string(),
                            text_styles: TextStylesWithMetadata::default(),
                            block_style: BufferBlockStyle::PlainText
                        },],
                        style: BufferBlockStyle::PlainText,
                        content_length: CharOffset::from(1)
                    }),
                ]
            );

            // Change the content of one line. EditDelta should only return the updated line.
            buffer.set_selection(
                CharOffset::from(2)..CharOffset::from(3),
                selection.clone(),
                ctx,
            );
            let delta = buffer
                .edit_internal("s", TextStyles::default(), selection.clone(), ctx)
                .delta
                .expect("Should exist");
            assert_eq!(buffer.content.debug(), "<text>ts\\n\\nt");
            assert_eq!(
                delta.precise_deltas,
                vec![PreciseDelta {
                    replaced_range: CharOffset::from(2)..CharOffset::from(3),
                    replaced_points: Point::new(1, 1)..Point::new(1, 2),
                    replaced_byte_range: ByteOffset::from(2)..ByteOffset::from(3),
                    new_byte_length: 1,
                    new_end_point: Point::new(1, 2),
                    resolved_range: CharOffset::from(2)..CharOffset::from(3),
                }]
            );
            assert_eq!(
                delta.new_lines,
                vec![StyledBufferBlock::Text(StyledTextBlock {
                    block: vec![StyledBufferRun {
                        run: "ts\n".to_string(),
                        text_styles: TextStylesWithMetadata::default(),
                        block_style: BufferBlockStyle::PlainText
                    },],
                    style: BufferBlockStyle::PlainText,
                    content_length: CharOffset::from(3)
                }),]
            );

            // Edit just the middle line.
            buffer.set_selection(
                CharOffset::from(4)..CharOffset::from(4),
                selection.clone(),
                ctx,
            );
            let delta = buffer
                .edit_internal("hi", TextStyles::default(), selection.clone(), ctx)
                .delta
                .expect("Should exist");
            assert_eq!(buffer.content.debug(), "<text>ts\\nhi\\nt");
            assert_eq!(
                delta.precise_deltas,
                vec![PreciseDelta {
                    replaced_range: CharOffset::from(4)..CharOffset::from(4),
                    replaced_points: Point::new(2, 0)..Point::new(2, 0),
                    replaced_byte_range: ByteOffset::from(4)..ByteOffset::from(4),
                    new_byte_length: 2,
                    new_end_point: Point::new(2, 2),
                    resolved_range: CharOffset::from(4)..CharOffset::from(6),
                }]
            );
            assert_eq!(
                delta.new_lines,
                vec![StyledBufferBlock::Text(StyledTextBlock {
                    block: vec![StyledBufferRun {
                        run: "hi\n".to_string(),
                        text_styles: TextStylesWithMetadata::default(),
                        block_style: BufferBlockStyle::PlainText
                    },],
                    style: BufferBlockStyle::PlainText,
                    content_length: CharOffset::from(3)
                }),]
            );

            // Style the content of one line. EditDelta should only return the updated line.
            buffer.set_selection(
                CharOffset::from(2)..CharOffset::from(3),
                selection.clone(),
                ctx,
            );
            let delta = buffer
                .style_internal(TextStyles::default().bold(), selection.clone(), ctx)
                .delta
                .expect("Should exist");
            assert_eq!(
                delta.precise_deltas,
                vec![PreciseDelta {
                    replaced_range: CharOffset::from(2)..CharOffset::from(3),
                    replaced_points: Point::new(1, 1)..Point::new(1, 2),
                    replaced_byte_range: ByteOffset::from(2)..ByteOffset::from(3),
                    new_byte_length: 1,
                    new_end_point: Point::new(1, 2),
                    resolved_range: CharOffset::from(2)..CharOffset::from(3),
                }]
            );
            assert_eq!(
                delta.new_lines,
                vec![StyledBufferBlock::Text(StyledTextBlock {
                    block: vec![
                        StyledBufferRun {
                            run: "t".to_string(),
                            text_styles: TextStylesWithMetadata::default(),
                            block_style: BufferBlockStyle::PlainText
                        },
                        StyledBufferRun {
                            run: "s".to_string(),
                            text_styles: TextStylesWithMetadata::default().bold(),
                            block_style: BufferBlockStyle::PlainText
                        },
                        StyledBufferRun {
                            run: "\n".to_string(),
                            text_styles: TextStylesWithMetadata::default(),
                            block_style: BufferBlockStyle::PlainText
                        },
                    ],
                    style: BufferBlockStyle::PlainText,
                    content_length: CharOffset::from(3)
                }),]
            );
        });
    });
}

#[test]
fn test_multi_delta_stale_replaced_range() {
    // Demonstrates that when apply_core_edit_actions processes multiple actions where a
    // later action is at a lower buffer offset, the earlier action's replaced_range becomes
    // stale in the final buffer's coordinate system. This causes incorrect text when reading
    // replacement content from the final buffer (as notify_lsp_of_content_change does).
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            // Set up buffer: "AABBCCEE" (8 chars at CharOffset 1..9)
            let _ = buffer.edit_internal("AABBCCEE", TextStyles::default(), selection.clone(), ctx);
            assert_eq!(buffer.text().as_str(), "AABBCCEE");

            // Apply two actions in reverse positional order (high offset first):
            //   Action 0: Replace "CC" (offset 5..7) with "DDDD" (+2 chars)
            //   Action 1: Replace "AA" (offset 1..3) with "X"  (-1 char)
            //
            // After action 0: buffer = "AABBDDDDEE"
            // After action 1: buffer = "XBBDDDDEE"  (final)
            let plain = |text: &str| {
                super::convert_text_with_style_to_formatted_text(
                    text,
                    TextStyles::default(),
                    BufferBlockStyle::PlainText,
                )
            };
            let actions = vec![
                CoreEditorAction::new(
                    CharOffset::from(5)..CharOffset::from(7),
                    CoreEditorActionType::Insert {
                        text: plain("DDDD"),
                        source: EditOrigin::UserInitiated,
                        override_next_style: false,
                        insert_on_selection: true,
                    },
                ),
                CoreEditorAction::new(
                    CharOffset::from(1)..CharOffset::from(3),
                    CoreEditorActionType::Insert {
                        text: plain("X"),
                        source: EditOrigin::UserInitiated,
                        override_next_style: false,
                        insert_on_selection: true,
                    },
                ),
            ];

            let result = buffer.apply_core_edit_actions(actions);
            let delta = result.delta.expect("Should have delta");

            assert_eq!(buffer.text().as_str(), "XBBDDDDEE");
            assert_eq!(delta.precise_deltas.len(), 2);

            // Delta 0 replaced "CC" at offset 5..7 with "DDDD" (4 chars).
            let delta0 = &delta.precise_deltas[0];
            assert_eq!(
                delta0.replaced_range,
                CharOffset::from(5)..CharOffset::from(7)
            );
            assert_eq!(delta0.resolved_length(), CharOffset::from(4));

            // Delta 1 replaced "AA" at offset 1..3 with "X" (1 char).
            let delta1 = &delta.precise_deltas[1];
            assert_eq!(
                delta1.replaced_range,
                CharOffset::from(1)..CharOffset::from(3)
            );
            assert_eq!(delta1.resolved_length(), CharOffset::from(1));

            // BUG: The stale replaced_range for delta 0 points to the wrong location
            // in the final buffer.
            //
            // Delta 0's replaced_range.start (5) is in the post-delta-0 coordinate system.
            // After delta 1 removed 1 char before it, "DDDD" shifted to offset 4..8 in the
            // final buffer. But the stale computation reads from 5..9:
            //
            //   Final buffer "XBBDDDDEE":
            //     offset 4: D, 5: D, 6: D, 7: D, 8: E, 9: E
            //   text_in_range(5..9) = "DDDE" -- WRONG, should be "DDDD"
            let stale_range =
                delta0.replaced_range.start..delta0.replaced_range.start + delta0.resolved_length();
            let stale_text = buffer.text_in_range(stale_range).into_string();
            assert_eq!(
                stale_text, "DDDE",
                "Stale replaced_range reads wrong text from the final buffer"
            );

            // resolved_range gives the correct range in the final buffer (4..8).
            assert_eq!(
                delta0.resolved_range,
                CharOffset::from(4)..CharOffset::from(8)
            );
            let correct_text = buffer
                .text_in_range(delta0.resolved_range.clone())
                .into_string();
            assert_eq!(
                correct_text, "DDDD",
                "resolved_range reads correct text from the final buffer"
            );

            // Delta 1 is the last action applied, so its resolved_range matches
            // the simple computation (replaced_range.start + resolved_length).
            assert_eq!(
                delta1.resolved_range,
                CharOffset::from(1)..CharOffset::from(2)
            );
            let delta1_text = buffer
                .text_in_range(delta1.resolved_range.clone())
                .into_string();
            assert_eq!(delta1_text, "X");
        });
    });
}

#[test]
fn test_selection_movement() {
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            let _ = buffer.edit_internal_first_selection(
                CharOffset::from(1)..CharOffset::from(1),
                "te\nst",
                TextStyles::default(),
                selection.clone(),
                ctx,
            );
            assert_eq!(buffer.content.debug(), "<text>te\\nst");
            assert_eq!(
                selection.as_ref(ctx).selection_to_first_offset_range(),
                CharOffset::from(6)..CharOffset::from(6)
            );

            // This should be no-op.
            buffer.update_selection(
                selection.clone(),
                BufferSelectAction::MoveRight,
                AutoScrollBehavior::Selection,
                ctx,
            );
            assert_eq!(
                selection.as_ref(ctx).selection_to_first_offset_range(),
                CharOffset::from(6)..CharOffset::from(6)
            );

            buffer.update_selection(
                selection.clone(),
                BufferSelectAction::MoveLeft,
                AutoScrollBehavior::Selection,
                ctx,
            );
            assert_eq!(
                selection.as_ref(ctx).selection_to_first_offset_range(),
                CharOffset::from(5)..CharOffset::from(5)
            );

            buffer.extend_selection_left(2, selection.clone(), ctx);
            assert_eq!(
                selection.as_ref(ctx).selection_to_first_offset_range(),
                CharOffset::from(3)..CharOffset::from(5)
            );

            // The head of selection is 3. Extending it right by 3 should move the head to 6.
            buffer.extend_selection_right(3, selection.clone(), ctx);
            assert_eq!(
                selection.as_ref(ctx).selection_to_first_offset_range(),
                CharOffset::from(5)..CharOffset::from(6)
            );

            buffer.update_selection(
                selection.clone(),
                BufferSelectAction::MoveRight,
                AutoScrollBehavior::Selection,
                ctx,
            );
            assert_eq!(
                selection.as_ref(ctx).selection_to_first_offset_range(),
                CharOffset::from(6)..CharOffset::from(6)
            );

            buffer.set_selection(
                CharOffset::from(1)..CharOffset::from(3),
                selection.clone(),
                ctx,
            );
            buffer.update_selection(
                selection.clone(),
                BufferSelectAction::MoveLeft,
                AutoScrollBehavior::Selection,
                ctx,
            );
            assert_eq!(
                selection.as_ref(ctx).selection_to_first_offset_range(),
                CharOffset::from(1)..CharOffset::from(1)
            );

            selection.update(ctx, |selection, _| {
                selection.set_single_cursor(CharOffset::from(1));
            });
            // Should be no-op.
            buffer.update_selection(
                selection.clone(),
                BufferSelectAction::MoveLeft,
                AutoScrollBehavior::Selection,
                ctx,
            );
            assert_eq!(
                selection.as_ref(ctx).selection_to_first_offset_range(),
                CharOffset::from(1)..CharOffset::from(1)
            );

            buffer.update_selection(
                selection.clone(),
                BufferSelectAction::MoveRight,
                AutoScrollBehavior::Selection,
                ctx,
            );
            assert_eq!(
                selection.as_ref(ctx).selection_to_first_offset_range(),
                CharOffset::from(2)..CharOffset::from(2)
            );
        });
    });
}

#[test]
fn test_active_selection_style() {
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            let _ = buffer.edit_internal_first_selection(
                CharOffset::from(1)..CharOffset::from(1),
                "te\nst",
                TextStyles::default(),
                selection.clone(),
                ctx,
            );
            assert_eq!(buffer.content.debug(), "<text>te\\nst");
            assert_eq!(
                selection.as_ref(ctx).selection_to_first_offset_range(),
                CharOffset::from(6)..CharOffset::from(6)
            );

            buffer.extend_selection_left(2, selection.clone(), ctx);
            assert_eq!(
                selection.as_ref(ctx).selection_to_first_offset_range(),
                CharOffset::from(4)..CharOffset::from(6)
            );
            let text_style: TextStyles = buffer
                .active_style_with_metadata_at_selection(selection.as_ref(ctx))
                .into();
            assert_eq!(text_style, TextStyles::default());

            // When selection is a range of characters, active style should be the style of the first character.
            buffer.style_internal(TextStyles::default().bold(), selection.clone(), ctx);
            let text_style: TextStyles = buffer
                .active_style_with_metadata_at_selection(selection.as_ref(ctx))
                .into();
            assert_eq!(text_style, TextStyles::default().bold());

            // When selection is a single cursor, active style should be the style of the character before the cursor.
            buffer.update_selection(
                selection.clone(),
                BufferSelectAction::MoveLeft,
                AutoScrollBehavior::Selection,
                ctx,
            );
            assert_eq!(
                selection.as_ref(ctx).selection_to_first_offset_range(),
                CharOffset::from(4)..CharOffset::from(4)
            );
            let text_style: TextStyles = buffer
                .active_style_with_metadata_at_selection(selection.as_ref(ctx))
                .into();
            assert_eq!(text_style, TextStyles::default());

            // When there are multiple selections, active style should be the common style of all selections, if any.
            assert_eq!(buffer.content.debug(), "<text>te\\n<b_s>st<b_e>");

            buffer.add_cursor(5.into(), true, selection.clone(), ctx);
            buffer.add_cursor(1.into(), false, selection.clone(), ctx);
            let text_style: TextStyles = buffer
                .active_style_with_metadata_at_selection(selection.as_ref(ctx))
                .into();
            assert_eq!(text_style, TextStyles::default());

            buffer.add_cursor(5.into(), true, selection.clone(), ctx);
            buffer.add_cursor(6.into(), false, selection.clone(), ctx);
            let text_style: TextStyles = buffer
                .active_style_with_metadata_at_selection(selection.as_ref(ctx))
                .into();
            assert_eq!(text_style, TextStyles::default().bold());
        });
    });
}

#[test]
fn test_selection_text_styles() {
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            let _ = buffer.edit_internal_first_selection(
                CharOffset::from(1)..CharOffset::from(1),
                "te\nst",
                TextStyles::default(),
                selection.clone(),
                ctx,
            );

            buffer.extend_selection_left(2, selection.clone(), ctx);
            buffer.style_internal(TextStyles::default().bold(), selection.clone(), ctx);

            assert_eq!(buffer.content.debug(), "<text>te\\n<b_s>st<b_e>");
        });

        // When selection is a range of characters, selection style should be the style common to all of the selection.
        let text_style: TextStyles = selection
            .read(&app, |selection, app| selection.selection_text_styles(app))
            .into();
        assert_eq!(text_style, TextStyles::default().bold());

        // Make a selection that is partially bold.
        selection.update(&mut app, |selection, _| {
            selection.set_selection_offsets(vec1![SelectionOffsets {
                head: 6.into(),
                tail: 1.into(),
            }]);
        });
        let text_style: TextStyles = selection
            .read(&app, |selection, app| selection.selection_text_styles(app))
            .into();
        assert_eq!(text_style, TextStyles::default());

        // When there is more than one selection, the style is the styles common to all selections.
        // Make two selections, only one bold.
        selection.update(&mut app, |selection, _| {
            selection.set_selection_offsets(vec1![
                SelectionOffsets {
                    head: 6.into(),
                    tail: 5.into(),
                },
                SelectionOffsets {
                    head: 2.into(),
                    tail: 1.into(),
                }
            ]);
        });
        let text_style: TextStyles = selection
            .read(&app, |selection, app| selection.selection_text_styles(app))
            .into();
        assert_eq!(text_style, TextStyles::default());

        // Make two selections, both bold.
        selection.update(&mut app, |selection, _| {
            selection.set_selection_offsets(vec1![
                SelectionOffsets {
                    head: 6.into(),
                    tail: 5.into(),
                },
                SelectionOffsets {
                    head: 5.into(),
                    tail: 4.into(),
                }
            ]);
        });
        let text_style: TextStyles = selection
            .read(&app, |selection, app| selection.selection_text_styles(app))
            .into();
        assert_eq!(text_style, TextStyles::default().bold());
    });
}

#[test]
fn test_block_style() {
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            let _ = buffer.edit_internal_first_selection(
                CharOffset::from(1)..CharOffset::from(1),
                "test\nline\nsecond",
                TextStyles::default(),
                selection.clone(),
                ctx,
            );
            buffer.set_selection(CharOffset::from(1)..CharOffset::from(3), selection.clone(), ctx);
            let _ = buffer.style_internal(TextStyles::default().bold(), selection.clone(), ctx);
            assert_eq!(
                buffer.content.debug(),
                "<text><b_s>te<b_e>st\\nline\\nsecond"
            );

            // Styling s in test to a runnable code block.
            let delta = buffer
                .block_style_range(
                    CharOffset::from(3)..CharOffset::from(4),
                    BufferBlockStyle::CodeBlock {
                        code_block_type: Default::default(),
                    },
                    selection.clone(),
                    ctx,
                )
                .delta
                .expect("Should exist");
            assert_eq!(
                buffer.content.debug(),
                "<text><b_s>te<b_e><code:Shell>s<text>t\\nline\\nsecond"
            );
            assert_eq!(delta.old_offset, CharOffset::from(1)..CharOffset::from(6));
            assert_eq!(
                delta.new_lines,
                vec![
                    StyledBufferBlock::Text(StyledTextBlock {
                        block: vec![
                            StyledBufferRun {
                                run: "te".to_string(),
                                text_styles: TextStylesWithMetadata::default().bold(),
                                block_style: BufferBlockStyle::PlainText
                            },
                            StyledBufferRun {
                                run: "\n".to_string(),
                                text_styles: TextStylesWithMetadata::default(),
                                block_style: BufferBlockStyle::PlainText
                            },
                        ],
                        style: BufferBlockStyle::PlainText,
                        content_length: CharOffset::from(3),
                    }),
                    StyledBufferBlock::Text(StyledTextBlock {
                        block: vec![StyledBufferRun {
                            run: "s\n".to_string(),
                            text_styles: TextStylesWithMetadata::default(),
                            block_style: BufferBlockStyle::CodeBlock {
                                code_block_type: Default::default()
                            }
                        },],
                        style: BufferBlockStyle::CodeBlock {
                            code_block_type: Default::default()

                },
                        content_length: CharOffset::from(2),
                    }),
                    StyledBufferBlock::Text(StyledTextBlock {
                        block: vec![StyledBufferRun {
                            run: "t\n".to_string(),
                            text_styles: TextStylesWithMetadata::default(),
                            block_style: BufferBlockStyle::PlainText
                        },],
                        style: BufferBlockStyle::PlainText,
                        content_length: CharOffset::from(2),
                    }),
                ]
            );

            // Styling li in line to a runnable code block.
            let delta = buffer
                .block_style_range(
                    CharOffset::from(8)..CharOffset::from(10),
                    BufferBlockStyle::CodeBlock {
                        code_block_type: Default::default(),
                    },
                    selection.clone(),
                    ctx,
                )
                .delta
                .expect("Should exist");
            assert_eq!(delta.old_offset, CharOffset::from(8)..CharOffset::from(13));
            assert_eq!(
                buffer.content.debug(),
                "<text><b_s>te<b_e><code:Shell>s<text>t<code:Shell>li<text>ne\\nsecond"
            );
            assert_eq!(
                delta.new_lines,
                vec![
                    StyledBufferBlock::Text(StyledTextBlock {
                        block: vec![StyledBufferRun {
                            run: "li\n".to_string(),
                            text_styles: TextStylesWithMetadata::default(),
                            block_style: BufferBlockStyle::CodeBlock {
                                code_block_type: Default::default()
                            }
                        },],
                        style: BufferBlockStyle::CodeBlock {
                            code_block_type: Default::default()
                },
                        content_length: CharOffset::from(3),
                    }),
                    StyledBufferBlock::Text(StyledTextBlock {
                        block: vec![StyledBufferRun {
                            run: "ne\n".to_string(),
                            text_styles: TextStylesWithMetadata::default(),
                            block_style: BufferBlockStyle::PlainText
                        },],
                        style: BufferBlockStyle::PlainText,
                        content_length: CharOffset::from(3),
                    }),
                ]
            );

            // Styling t in test to a runnable code block.
            let delta = buffer
                .block_style_range(
                    CharOffset::from(4)..CharOffset::from(7),
                    BufferBlockStyle::CodeBlock {
                        code_block_type: Default::default(),
                    },
                    selection.clone(),
                    ctx,
                )
                .delta
                .expect("Should exist");
            assert_eq!(
                buffer.content.debug(),
                "<text><b_s>te<b_e><code:Shell>s\\nt<code:Shell>li<text>ne\\nsecond"
            );
            assert_eq!(delta.old_offset, CharOffset::from(4)..CharOffset::from(8));
            assert_eq!(
                delta.new_lines,
                vec![StyledBufferBlock::Text(StyledTextBlock {
                    block: vec![
                        StyledBufferRun {
                            run: "s\n".to_string(),
                            text_styles: TextStylesWithMetadata::default(),
                            block_style: BufferBlockStyle::CodeBlock {
                                code_block_type: Default::default()
                            }
                        },
                        StyledBufferRun {
                            run: "t\n".to_string(),
                            text_styles: TextStylesWithMetadata::default(),
                            block_style: BufferBlockStyle::CodeBlock {
                                code_block_type: Default::default()
                            }
                        },
                    ],
                    style: BufferBlockStyle::CodeBlock {
                        code_block_type: Default::default()
                    },
                    content_length: CharOffset::from(4),
                }),]
            );

            // Styling the last line to a runnable code block.
            let delta = buffer
                .block_style_range(
                    CharOffset::from(14)..CharOffset::from(20),
                    BufferBlockStyle::CodeBlock {
                        code_block_type: Default::default(),
                    },
                    selection.clone(),
                    ctx,
                )
                .delta
                .expect("Should exist");
            assert_eq!(
                buffer.content.debug(),
                "<text><b_s>te<b_e><code:Shell>s\\nt<code:Shell>li<text>ne<code:Shell>second<text>"
            );
            assert_eq!(delta.old_offset, CharOffset::from(14)..CharOffset::from(20));
            assert_eq!(
                delta.new_lines,
                vec![StyledBufferBlock::Text(StyledTextBlock {
                    block: vec![StyledBufferRun {
                        run: "second\n".to_string(),
                        text_styles: TextStylesWithMetadata::default(),
                        block_style: BufferBlockStyle::CodeBlock {
                            code_block_type: Default::default()
                        }
                    },],
                    style: BufferBlockStyle::CodeBlock {
                        code_block_type: Default::default()
                    },
                    content_length: CharOffset::from(7),
                }),]
            );

            // Styling the line right after a code block should create a new code block.
            let delta = buffer
                .block_style_range(
                    CharOffset::from(11)..CharOffset::from(12),
                    BufferBlockStyle::CodeBlock {
                        code_block_type: Default::default(),
                    },
                    selection.clone(),
                    ctx,
                )
                .delta
                .expect("Should exist");
            assert_eq!(
                buffer.content.debug(),
                "<text><b_s>te<b_e><code:Shell>s\\nt<code:Shell>li<code:Shell>n<text>e<code:Shell>second<text>"
            );
            assert_eq!(delta.old_offset, CharOffset::from(11)..CharOffset::from(14));
            assert_eq!(
                delta.new_lines,
                vec![
                    StyledBufferBlock::Text(StyledTextBlock {
                        block: vec![StyledBufferRun {
                            run: "n\n".to_string(),
                            text_styles: TextStylesWithMetadata::default(),
                            block_style: BufferBlockStyle::CodeBlock {
                                code_block_type: Default::default()
                            }
                        },],
                        style: BufferBlockStyle::CodeBlock {
                            code_block_type: Default::default()
                        },
                        content_length: CharOffset::from(2),
                    }),
                    StyledBufferBlock::Text(StyledTextBlock {
                        block: vec![StyledBufferRun {
                            run: "e\n".to_string(),
                            text_styles: TextStylesWithMetadata::default(),
                            block_style: BufferBlockStyle::PlainText
                        },],
                        style: BufferBlockStyle::PlainText,
                        content_length: CharOffset::from(2),
                    })
                ]
            );
        });
    });
}

#[test]
fn test_containing_block_start() {
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            buffer.edit_internal_first_selection(
                1.into()..1.into(),
                "text\ncode",
                Default::default(),
                selection.clone(),
                ctx,
            );
            buffer.block_style_range(
                6.into()..10.into(),
                BufferBlockStyle::CodeBlock {
                    code_block_type: CodeBlockType::Shell,
                },
                selection.clone(),
                ctx,
            );
            assert_eq!(buffer.debug(), "<text>text<code:Shell>code<text>");

            // The first text block
            assert_eq!(buffer.containing_block_start(1.into()), 1.into());
            assert_eq!(buffer.containing_block_start(2.into()), 1.into());

            // The code block
            assert_eq!(buffer.containing_block_start(6.into()), 6.into());
            assert_eq!(buffer.containing_block_start(8.into()), 6.into());
            assert_eq!(buffer.containing_block_start(10.into()), 6.into());

            // The trailing plain text.
            assert_eq!(buffer.containing_block_start(11.into()), 11.into());
        });
    });
}

#[test]
fn test_style_unstyle_block_overlapping() {
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            let _ = buffer.edit_internal_first_selection(
                CharOffset::from(1)..CharOffset::from(1),
                "test\nline\nsecond\nblock",
                TextStyles::default(),
                selection.clone(),
                ctx,
            );
            let _ = buffer.block_style_range(
                CharOffset::from(2)..CharOffset::from(4),
                BufferBlockStyle::CodeBlock {
                    code_block_type: Default::default(),
                },
                selection.clone(),
                ctx,
            );
            assert_eq!(
                buffer.content.debug(),
                "<text>t<code:Shell>es<text>t\\nline\\nsecond\\nblock"
            );

            let delta = buffer
                .block_style_range(
                    CharOffset::from(1)..CharOffset::from(4),
                    BufferBlockStyle::CodeBlock {
                        code_block_type: Default::default(),
                    },
                    selection.clone(),
                    ctx,
                )
                .delta
                .expect("Should exist");
            assert_eq!(
                buffer.content.debug(),
                "<code:Shell>t\\nes<text>t\\nline\\nsecond\\nblock"
            );
            assert_eq!(delta.old_offset, CharOffset::from(1)..CharOffset::from(6));
            assert_eq!(
                delta.new_lines,
                vec![StyledBufferBlock::Text(StyledTextBlock {
                    block: vec![
                        StyledBufferRun {
                            run: "t\n".to_string(),
                            text_styles: TextStylesWithMetadata::default(),
                            block_style: BufferBlockStyle::CodeBlock {
                                code_block_type: Default::default()
                            }
                        },
                        StyledBufferRun {
                            run: "es\n".to_string(),
                            text_styles: TextStylesWithMetadata::default(),
                            block_style: BufferBlockStyle::CodeBlock {
                                code_block_type: Default::default()
                            }
                        }
                    ],
                    style: BufferBlockStyle::CodeBlock {
                        code_block_type: Default::default()
                    },
                    content_length: CharOffset::from(5),
                }),]
            );

            let _ = buffer.block_style_range(
                CharOffset::from(8)..CharOffset::from(12),
                BufferBlockStyle::CodeBlock {
                    code_block_type: Default::default(),
                },
                selection.clone(),
                ctx,
            );
            assert_eq!(
                buffer.content.debug(),
                "<code:Shell>t\\nes<text>t<code:Shell>line<text>second\\nblock"
            );

            let delta = buffer
                .block_style_range(
                    CharOffset::from(6)..CharOffset::from(7),
                    BufferBlockStyle::CodeBlock {
                        code_block_type: Default::default(),
                    },
                    selection.clone(),
                    ctx,
                )
                .delta
                .expect("Should exist");
            assert_eq!(
                buffer.content.debug(),
                "<code:Shell>t\\nes<code:Shell>t<code:Shell>line<text>second\\nblock"
            );
            assert_eq!(delta.old_offset, CharOffset::from(6)..CharOffset::from(8));
            assert_eq!(
                delta.new_lines,
                vec![StyledBufferBlock::Text(StyledTextBlock {
                    block: vec![StyledBufferRun {
                        run: "t\n".to_string(),
                        text_styles: TextStylesWithMetadata::default(),
                        block_style: BufferBlockStyle::CodeBlock {
                            code_block_type: Default::default()
                        }
                    },],
                    style: BufferBlockStyle::CodeBlock {
                        code_block_type: Default::default()
                    },
                    content_length: CharOffset::from(2)
                }),]
            );

            let delta = buffer
                .block_style_range(
                    CharOffset::from(13)..CharOffset::from(15),
                    BufferBlockStyle::CodeBlock {
                        code_block_type: Default::default(),
                    },
                    selection.clone(),
                    ctx,
                )
                .delta
                .expect("Should exist");
            assert_eq!(
                buffer.content.debug(),
                "<code:Shell>t\\nes<code:Shell>t<code:Shell>line<code:Shell>se<text>cond\\nblock"
            );
            assert_eq!(delta.old_offset, CharOffset::from(13)..CharOffset::from(20));
            assert_eq!(
                delta.new_lines,
                vec![
                    StyledBufferBlock::Text(StyledTextBlock {
                        block: vec![StyledBufferRun {
                            run: "se\n".to_string(),
                            text_styles: TextStylesWithMetadata::default(),
                            block_style: BufferBlockStyle::CodeBlock {
                                code_block_type: Default::default()
                            }
                        },],
                        style: BufferBlockStyle::CodeBlock {
                            code_block_type: Default::default()
                        },
                        content_length: CharOffset::from(3),
                    }),
                    StyledBufferBlock::Text(StyledTextBlock {
                        block: vec![StyledBufferRun {
                            run: "cond\n".to_string(),
                            text_styles: TextStylesWithMetadata::default(),
                            block_style: BufferBlockStyle::PlainText
                        },],
                        style: BufferBlockStyle::PlainText,
                        content_length: CharOffset::from(5),
                    }),
                ]
            );

            let delta = buffer
                .block_style_range(
                    CharOffset::from(3)..CharOffset::from(5),
                    BufferBlockStyle::PlainText,
                    selection.clone(),
                    ctx,
                )
                .delta
                .expect("Should exist");
            assert_eq!(
                buffer.content.debug(),
                "<code:Shell>t<text>es<code:Shell>t<code:Shell>line<code:Shell>se<text>cond\\nblock"
            );
            assert_eq!(delta.old_offset, CharOffset::from(1)..CharOffset::from(6));
            assert_eq!(
                delta.new_lines,
                vec![
                    StyledBufferBlock::Text(StyledTextBlock {
                        block: vec![StyledBufferRun {
                            run: "t\n".to_string(),
                            text_styles: TextStylesWithMetadata::default(),
                            block_style: BufferBlockStyle::CodeBlock {
                                code_block_type: Default::default()
                            }
                        },],
                        style: BufferBlockStyle::CodeBlock {
                            code_block_type: Default::default()
                        },
                        content_length: CharOffset::from(2),
                    }),
                    StyledBufferBlock::Text(StyledTextBlock {
                        block: vec![StyledBufferRun {
                            run: "es\n".to_string(),
                            text_styles: TextStylesWithMetadata::default(),
                            block_style: BufferBlockStyle::PlainText
                        },],
                        style: BufferBlockStyle::PlainText,
                        content_length: CharOffset::from(3),
                    }),
                ]
            );

            let delta = buffer
                .block_style_range(
                    CharOffset::from(8)..CharOffset::from(11),
                    BufferBlockStyle::PlainText,
                    selection.clone(),
                    ctx,
                )
                .delta
                .expect("Should exist");
            assert_eq!(
                buffer.content.debug(),
                "<code:Shell>t<text>es<code:Shell>t<text>lin<code:Shell>e<code:Shell>se<text>cond\\nblock"
            );
            assert_eq!(delta.old_offset, CharOffset::from(8)..CharOffset::from(13));
            assert_eq!(
                delta.new_lines,
                vec![
                    StyledBufferBlock::Text(StyledTextBlock {
                        block: vec![StyledBufferRun {
                            run: "lin\n".to_string(),
                            text_styles: TextStylesWithMetadata::default(),
                            block_style: BufferBlockStyle::PlainText
                        },],
                        style: BufferBlockStyle::PlainText,
                        content_length: CharOffset::from(4),
                    }),
                    StyledBufferBlock::Text(StyledTextBlock {
                        block: vec![StyledBufferRun {
                            run: "e\n".to_string(),
                            text_styles: TextStylesWithMetadata::default(),
                            block_style: BufferBlockStyle::CodeBlock {
                                code_block_type: Default::default()
                            }
                        },],
                        style: BufferBlockStyle::CodeBlock {
                            code_block_type: Default::default()
                        },
                        content_length: CharOffset::from(2),
                    }),
                ]
            );
        });
    });
}

#[test]
fn test_containing_code_block() {
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            let _ = buffer.edit_internal_first_selection(
                CharOffset::from(1)..CharOffset::from(1),
                "test\nline\nsecond",
                TextStyles::default(),
                selection.clone(),
                ctx,
            );
            assert_eq!(buffer.content.debug(), "<text>test\\nline\\nsecond");

            // Styling s in test to a runnable code block.
            let _ = buffer.block_style_range(
                CharOffset::from(3)..CharOffset::from(4),
                BufferBlockStyle::CodeBlock {
                    code_block_type: Default::default(),
                },
                selection.clone(),
                ctx,
            );
            // Styling 'line\ns' to a runnable code block.
            let _ = buffer.block_style_range(
                CharOffset::from(8)..CharOffset::from(14),
                BufferBlockStyle::CodeBlock {
                    code_block_type: Default::default(),
                },
                selection.clone(),
                ctx,
            );
            assert_eq!(
                buffer.content.debug(),
                "<text>te<code:Shell>s<text>t<code:Shell>line\\ns<text>econd"
            );

            assert_eq!(
                buffer.containing_block_start(CharOffset::from(3)),
                CharOffset::from(1)
            );
            assert_eq!(
                buffer.containing_block_end(CharOffset::from(3)),
                CharOffset::from(4)
            );

            assert_eq!(
                buffer.containing_block_start(CharOffset::from(5)),
                CharOffset::from(4)
            );
            assert_eq!(
                buffer.containing_block_end(CharOffset::from(5)),
                CharOffset::from(6)
            );

            assert_eq!(
                buffer.containing_block_start(CharOffset::from(10)),
                CharOffset::from(8)
            );
            assert_eq!(
                buffer.containing_block_end(CharOffset::from(10)),
                CharOffset::from(15)
            );
        });
    });
}

#[test]
fn test_containing_offset() {
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            buffer.edit_internal_first_selection(
                CharOffset::from(1)..CharOffset::from(1),
                "first\nsecond\nthird",
                Default::default(),
                selection.clone(),
                ctx,
            );

            for (offset, expected_start, expected_end) in vec![
                // Extremes of the buffer should still behave as expected.
                (1, 1, 7),
                (19, 14, 20),
                // Likewise, we can seek from the start of each line and from its last character.
                (6, 1, 7),    // First newline.
                (7, 7, 14),   // First character of second line.
                (13, 7, 14),  // Second newline.
                (14, 14, 20), // First character of last line.
                // We can also seek to the boundaries from within a line.
                (4, 1, 7),
                (9, 7, 14),
                (16, 14, 20),
                // If out of bounds, we should clamp to the last line.
                (21, 14, 20),
            ] {
                let offset = CharOffset::from(offset);
                let expected_start = CharOffset::from(expected_start);
                let expected_end = CharOffset::from(expected_end);
                let start = buffer.containing_line_start(offset);
                assert_eq!(
            start, expected_start,
            "Expected line containing {offset} to start at {expected_start}, but got {start}"
        );

                let end = buffer.containing_line_end(offset);
                assert_eq!(
                    end, expected_end,
                    "Expected line containing {offset} to end at {expected_end}, but got {end}"
                );
            }
        });
    });
}

#[test]
fn test_delete_unpaired_block_style_marker() {
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            let _ = buffer.edit_internal_first_selection(
                CharOffset::from(1)..CharOffset::from(1),
                "test",
                TextStyles::default(),
                selection.clone(),
                ctx,
            );
            assert_eq!(buffer.content.debug(), "<text>test");

            // Styling s in test to a runnable code block.
            // <text>te<code:Shell>s<text>t
            let _ = buffer.block_style_range(
                CharOffset::from(3)..CharOffset::from(4),
                BufferBlockStyle::CodeBlock {
                    code_block_type: Default::default(),
                },
                selection.clone(),
                ctx,
            );
            assert_eq!(buffer.content.debug(), "<text>te<code:Shell>s<text>t");

            // Remove the starting marker only.
            // tess\nt
            let delta = buffer
                .edit_internal_first_selection(
                    CharOffset::from(3)..CharOffset::from(4),
                    "s",
                    Default::default(),
                    selection.clone(),
                    ctx,
                )
                .delta
                .expect("Should exist");
            assert_eq!(delta.old_offset, CharOffset::from(1)..CharOffset::from(6));
            assert_eq!(
                delta.new_lines,
                vec![StyledBufferBlock::Text(StyledTextBlock {
                    block: vec![StyledBufferRun {
                        run: "tess\n".to_string(),
                        text_styles: TextStylesWithMetadata::default(),
                        block_style: BufferBlockStyle::PlainText
                    },],
                    style: BufferBlockStyle::PlainText,
                    content_length: CharOffset::from(5)
                }),]
            );
            assert_eq!(buffer.content.debug(), "<text>tess\\nt");

            // Styling s in tesst to a runnable code block.
            // <text>te<code:Shell>s<text>s\nt
            let _ = buffer.block_style_range(
                CharOffset::from(3)..CharOffset::from(4),
                BufferBlockStyle::CodeBlock {
                    code_block_type: Default::default(),
                },
                selection.clone(),
                ctx,
            );
            // Remove the start marker of the next text block. This should merge it into the code block.
            // <text>te<code:Shell>sss<text>t
            let delta = buffer
                .edit_internal_first_selection(
                    CharOffset::from(5)..CharOffset::from(6),
                    "s",
                    Default::default(),
                    selection.clone(),
                    ctx,
                )
                .delta
                .expect("Should exist");
            assert_eq!(delta.old_offset, CharOffset::from(4)..CharOffset::from(8));
            assert_eq!(
                delta.new_lines,
                vec![StyledBufferBlock::Text(StyledTextBlock {
                    block: vec![StyledBufferRun {
                        run: "sss\n".to_string(),
                        text_styles: TextStylesWithMetadata::default(),
                        block_style: BufferBlockStyle::CodeBlock {
                            code_block_type: Default::default()
                        }
                    },],
                    style: BufferBlockStyle::CodeBlock {
                        code_block_type: Default::default(),
                    },
                    content_length: CharOffset::from(4)
                }),]
            );
            assert_eq!(buffer.content.debug(), "<text>te<code:Shell>sss<text>t");

            // Adding a newline in the code block.
            let _ = buffer.edit_internal_first_selection(
                CharOffset::from(5)..CharOffset::from(5),
                "\n",
                Default::default(),
                selection.clone(),
                ctx,
            );
            assert_eq!(buffer.content.debug(), "<text>te<code:Shell>s\\nss<text>t");
            // Remove the start marker.
            let delta = buffer
                .edit_internal_first_selection(
                    CharOffset::from(3)..CharOffset::from(4),
                    "",
                    Default::default(),
                    selection.clone(),
                    ctx,
                )
                .delta
                .expect("Should exist");
            assert_eq!(delta.old_offset, CharOffset::from(1)..CharOffset::from(9));
            assert_eq!(
                delta.new_lines,
                vec![
                    StyledBufferBlock::Text(StyledTextBlock {
                        block: vec![StyledBufferRun {
                            run: "tes\n".to_string(),
                            text_styles: TextStylesWithMetadata::default(),
                            block_style: BufferBlockStyle::PlainText
                        },],
                        style: BufferBlockStyle::PlainText,
                        content_length: CharOffset::from(4)
                    }),
                    StyledBufferBlock::Text(StyledTextBlock {
                        block: vec![StyledBufferRun {
                            run: "ss\n".to_string(),
                            text_styles: TextStylesWithMetadata::default(),
                            block_style: BufferBlockStyle::PlainText
                        },],
                        style: BufferBlockStyle::PlainText,
                        content_length: CharOffset::from(3)
                    }),
                ]
            );
            assert_eq!(buffer.content.debug(), "<text>tes\\nss\\nt");
        });
    });
}

#[test]
fn test_content_from_block_start_to_selection_start() {
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            let _ = buffer.edit_internal_first_selection(
                CharOffset::from(1)..CharOffset::from(1),
                "te\nst",
                TextStyles::default(),
                selection.clone(),
                ctx,
            );

            // Cursor is at the end of the buffer -- should return "st" from the second line.
            assert_eq!(
                buffer.content_from_block_start_to_selection_start(
                    selection
                        .as_ref(ctx)
                        .selection_to_first_offset_range()
                        .start
                ),
                "st".to_string()
            );

            // Cursor is at the end of the first line -- should return "te".
            selection.update(ctx, |selection, _| {
                selection.set_single_cursor(CharOffset::from(3));
            });
            assert_eq!(
                buffer.content_from_block_start_to_selection_start(
                    selection
                        .as_ref(ctx)
                        .selection_to_first_offset_range()
                        .start
                ),
                "te".to_string()
            );

            // Cursor is in the middle of the first line -- should return "t".
            selection.update(ctx, |selection, _| {
                selection.set_single_cursor(CharOffset::from(2));
            });
            assert_eq!(
                buffer.content_from_block_start_to_selection_start(
                    selection
                        .as_ref(ctx)
                        .selection_to_first_offset_range()
                        .start
                ),
                "t".to_string()
            );

            // Cursor is in the beginning of the second line -- should return empty string.
            selection.update(ctx, |selection, _| {
                selection.set_single_cursor(CharOffset::from(4));
            });
            assert_eq!(
                buffer.content_from_block_start_to_selection_start(
                    selection
                        .as_ref(ctx)
                        .selection_to_first_offset_range()
                        .start
                ),
                "".to_string()
            );

            // Cursor is in the beginning of the buffer -- should return empty string.
            selection.update(ctx, |selection, _| {
                selection.set_single_cursor(CharOffset::from(1));
            });
            assert_eq!(
                buffer.content_from_block_start_to_selection_start(
                    selection
                        .as_ref(ctx)
                        .selection_to_first_offset_range()
                        .start
                ),
                "".to_string()
            );
        });
    });
}

#[test]
fn test_remove_prefix_and_style() {
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            let _ = buffer.edit_internal_first_selection(
                CharOffset::from(1)..CharOffset::from(1),
                "```",
                TextStyles::default(),
                selection.clone(),
                ctx,
            );

            // Remove prefix and style an empty line.
            let delta = buffer
                .remove_prefix_and_style_blocks(
                    BlockType::Text(BufferBlockStyle::CodeBlock {
                        code_block_type: Default::default(),
                    }),
                    selection.clone(),
                    ctx,
                )
                .delta
                .expect("Should exist");
            assert_eq!(buffer.content.debug(), "<code:Shell><text>");
            assert_eq!(delta.old_offset, CharOffset::from(1)..CharOffset::from(4));
            assert_eq!(
                delta.new_lines,
                vec![StyledBufferBlock::Text(StyledTextBlock {
                    block: vec![StyledBufferRun {
                        run: "\n".to_string(),
                        text_styles: TextStylesWithMetadata::default(),
                        block_style: BufferBlockStyle::CodeBlock {
                            code_block_type: Default::default()
                        }
                    },],
                    style: BufferBlockStyle::CodeBlock {
                        code_block_type: Default::default(),
                    },
                    content_length: CharOffset::from(1)
                }),]
            );

            // Remove prefix and style one line in a multi-line paragraph.
            let _ = buffer.edit_internal_first_selection(
                CharOffset::from(2)..CharOffset::from(2),
                "```abc\ndef",
                TextStyles::default(),
                selection.clone(),
                ctx,
            );

            selection.update(ctx, |selection, _| {
                selection.set_single_cursor(5.into());
            });
            let prev_selection = buffer.to_rendered_selection_set(selection.clone(), ctx);
            let edit_result = buffer.remove_prefix_and_style_blocks(
                BlockType::Text(BufferBlockStyle::CodeBlock {
                    code_block_type: Default::default(),
                }),
                selection.clone(),
                ctx,
            );

            let curr_selection = buffer.to_rendered_selection_set(selection.clone(), ctx);
            buffer.push_undo_item(
                prev_selection,
                curr_selection,
                edit_result.undo_item.expect("Should have undo item"),
                UndoActionType::Atomic,
            );

            let delta = edit_result.delta.expect("Should exist");
            assert_eq!(
                buffer.content.debug(),
                "<code:Shell><code:Shell>abc<text>def"
            );
            assert_eq!(delta.old_offset, CharOffset::from(2)..CharOffset::from(9));
            assert_eq!(
                delta.new_lines,
                vec![StyledBufferBlock::Text(StyledTextBlock {
                    block: vec![StyledBufferRun {
                        run: "abc\n".to_string(),
                        text_styles: TextStylesWithMetadata::default(),
                        block_style: BufferBlockStyle::CodeBlock {
                            code_block_type: Default::default()
                        }
                    },],
                    style: BufferBlockStyle::CodeBlock {
                        code_block_type: Default::default(),
                    },
                    content_length: CharOffset::from(4)
                }),]
            );

            // Should undo to the state before prefix was removed.
            let _ = buffer.undo(selection.clone(), ctx);
            assert_eq!(buffer.content.debug(), "<code:Shell><text>```abc\\ndef");

            let _ = buffer.redo(selection.clone(), ctx);
            assert_eq!(
                buffer.content.debug(),
                "<code:Shell><code:Shell>abc<text>def"
            );
        });
    });
}

#[test]
fn test_remove_prefix_and_insert_block_item() {
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            buffer.edit_internal_first_selection(
                CharOffset::from(1)..CharOffset::from(1),
                "T\nBlock\nT\nBlock",
                TextStyles::default(),
                selection.clone(),
                ctx,
            );

            buffer.block_style_range(
                CharOffset::from(3)..CharOffset::from(8),
                BufferBlockStyle::CodeBlock {
                    code_block_type: CodeBlockType::Shell,
                },
                selection.clone(),
                ctx,
            );

            buffer.block_style_range(
                CharOffset::from(11)..CharOffset::from(16),
                BufferBlockStyle::CodeBlock {
                    code_block_type: CodeBlockType::Shell,
                },
                selection.clone(),
                ctx,
            );
            assert_eq!(
                buffer.debug(),
                "<text>T<code:Shell>Block<text>T<code:Shell>Block<text>"
            );

            // Remove prefix and style an empty line.
            selection.update(ctx, |selection, _| {
                selection.set_single_cursor(2.into());
            });
            let delta = buffer
                .remove_prefix_and_style_blocks(
                    BlockType::Item(BufferBlockItem::HorizontalRule),
                    selection.clone(),
                    ctx,
                )
                .delta
                .expect("Should exist");
            assert_eq!(
                buffer.content.debug(),
                "<hr><code:Shell>Block<text>T<code:Shell>Block<text>"
            );
            assert_eq!(delta.old_offset, CharOffset::from(0)..CharOffset::from(9));
            assert_eq!(
                delta.new_lines,
                vec![
                    StyledBufferBlock::Item(BufferBlockItem::HorizontalRule),
                    StyledBufferBlock::Text(StyledTextBlock {
                        block: vec![StyledBufferRun {
                            run: "Block\n".to_string(),
                            text_styles: TextStylesWithMetadata::default(),
                            block_style: BufferBlockStyle::CodeBlock {
                                code_block_type: Default::default()
                            }
                        },],
                        style: BufferBlockStyle::CodeBlock {
                            code_block_type: Default::default(),
                        },
                        content_length: CharOffset::from(6)
                    }),
                ]
            );

            selection.update(ctx, |selection, _| {
                selection.set_single_cursor(9.into());
            });
            let prev_selection = buffer.to_rendered_selection_set(selection.clone(), ctx);
            let edit_result = buffer.remove_prefix_and_style_blocks(
                BlockType::Item(BufferBlockItem::HorizontalRule),
                selection.clone(),
                ctx,
            );

            let curr_selection = buffer.to_rendered_selection_set(selection.clone(), ctx);
            buffer.push_undo_item(
                prev_selection,
                curr_selection,
                edit_result.undo_item.expect("Should have undo item"),
                UndoActionType::Atomic,
            );

            let delta = edit_result.delta.expect("Should exist");
            assert_eq!(
                buffer.content.debug(),
                "<hr><code:Shell>Block<hr><code:Shell>Block<text>"
            );
            assert_eq!(delta.old_offset, CharOffset::from(2)..CharOffset::from(16));
            assert_eq!(
                delta.new_lines,
                vec![
                    StyledBufferBlock::Text(StyledTextBlock {
                        block: vec![StyledBufferRun {
                            run: "Block\n".to_string(),
                            text_styles: TextStylesWithMetadata::default(),
                            block_style: BufferBlockStyle::CodeBlock {
                                code_block_type: Default::default()
                            }
                        },],
                        style: BufferBlockStyle::CodeBlock {
                            code_block_type: Default::default(),
                        },
                        content_length: CharOffset::from(6)
                    }),
                    StyledBufferBlock::Item(BufferBlockItem::HorizontalRule),
                    StyledBufferBlock::Text(StyledTextBlock {
                        block: vec![StyledBufferRun {
                            run: "Block\n".to_string(),
                            text_styles: TextStylesWithMetadata::default(),
                            block_style: BufferBlockStyle::CodeBlock {
                                code_block_type: Default::default()
                            }
                        },],
                        style: BufferBlockStyle::CodeBlock {
                            code_block_type: Default::default(),
                        },
                        content_length: CharOffset::from(6)
                    }),
                ]
            );

            // Should undo to the state before prefix was removed.
            let _ = buffer.undo(selection.clone(), ctx);
            assert_eq!(
                buffer.content.debug(),
                "<hr><code:Shell>Block<text>T<code:Shell>Block<text>"
            );

            let _ = buffer.redo(selection.clone(), ctx);
            assert_eq!(
                buffer.content.debug(),
                "<hr><code:Shell>Block<hr><code:Shell>Block<text>"
            );
        });
    });
}

#[test]
fn test_styled_runs_multiple_styles() {
    // This tests that we split nested/overlapping style runs correctly.
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            buffer.edit_internal_first_selection(
                CharOffset::from(1)..CharOffset::from(1),
                "hello world",
                Default::default(),
                selection.clone(),
                ctx,
            );

            // Bold "llo wo" and italicize "world"
            buffer.set_selection(
                CharOffset::from(3)..CharOffset::from(9),
                selection.clone(),
                ctx,
            );
            buffer.style_internal(TextStyles::default().bold(), selection.clone(), ctx);
            buffer.set_selection(
                CharOffset::from(7)..CharOffset::from(12),
                selection.clone(),
                ctx,
            );
            buffer.style_internal(TextStyles::default().italic(), selection.clone(), ctx);

            assert_eq!(
                buffer.styled_blocks_in_range(
                    CharOffset::from(1)..buffer.max_charoffset(),
                    StyledBlockBoundaryBehavior::Exclusive
                ),
                vec![StyledBufferBlock::Text(StyledTextBlock {
                    block: vec![
                        StyledBufferRun {
                            run: "he".into(),
                            text_styles: Default::default(),
                            block_style: BufferBlockStyle::PlainText
                        },
                        StyledBufferRun {
                            run: "llo ".into(),
                            text_styles: TextStylesWithMetadata::default().bold(),
                            block_style: BufferBlockStyle::PlainText
                        },
                        StyledBufferRun {
                            run: "wo".into(),
                            text_styles: TextStylesWithMetadata::default().bold().italic(),
                            block_style: BufferBlockStyle::PlainText
                        },
                        StyledBufferRun {
                            run: "rld".into(),
                            text_styles: TextStylesWithMetadata::default().italic(),
                            block_style: BufferBlockStyle::PlainText
                        },
                    ],
                    style: BufferBlockStyle::PlainText,
                    content_length: CharOffset::from(11)
                })]
            );
        });
    });
}

#[test]
fn test_styled_runs_range() {
    // This tests that we apply the start and end offsets.
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            buffer.edit_internal_first_selection(
                CharOffset::from(1)..CharOffset::from(1),
                "this is a sentence",
                Default::default(),
                selection.clone(),
                ctx,
            );
            // Bold "is"
            buffer.set_selection(
                CharOffset::from(6)..CharOffset::from(8),
                selection.clone(),
                ctx,
            );
            buffer.style_internal(TextStyles::default().bold(), selection.clone(), ctx);

            assert_eq!(
                buffer.styled_blocks_in_range(
                    CharOffset::from(1)..CharOffset::from(7),
                    StyledBlockBoundaryBehavior::Exclusive
                ),
                vec![StyledBufferBlock::Text(StyledTextBlock {
                    block: vec![
                        StyledBufferRun {
                            run: "this ".into(),
                            text_styles: Default::default(),
                            block_style: BufferBlockStyle::PlainText
                        },
                        // This should be cut off by the upper limit.
                        StyledBufferRun {
                            run: "i".into(),
                            text_styles: TextStylesWithMetadata::default().bold(),
                            block_style: BufferBlockStyle::PlainText
                        }
                    ],
                    style: BufferBlockStyle::PlainText,
                    content_length: CharOffset::from(7)
                })]
            );

            assert_eq!(
                buffer.styled_blocks_in_range(
                    CharOffset::from(5)..buffer.max_charoffset(),
                    StyledBlockBoundaryBehavior::Exclusive
                ),
                vec![StyledBufferBlock::Text(StyledTextBlock {
                    block: vec![
                        // This should be cut off by the starting offset.
                        StyledBufferRun {
                            run: " ".into(),
                            text_styles: Default::default(),
                            block_style: BufferBlockStyle::PlainText
                        },
                        StyledBufferRun {
                            run: "is".into(),
                            text_styles: TextStylesWithMetadata::default().bold(),
                            block_style: BufferBlockStyle::PlainText
                        },
                        StyledBufferRun {
                            run: " a sentence".into(),
                            text_styles: Default::default(),
                            block_style: BufferBlockStyle::PlainText
                        },
                    ],
                    style: BufferBlockStyle::PlainText,
                    content_length: CharOffset::from(14)
                })]
            );
        });
    });
}

#[test]
fn test_styled_runs_splits_newlines() {
    // This tests that style runs are broken by newlines, even if the styling
    // itself continues.
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            buffer.edit_internal_first_selection(
                CharOffset::from(1)..CharOffset::from(1),
                "hello\nworld",
                Default::default(),
                selection.clone(),
                ctx,
            );
            // Italicize llo\nwo
            buffer.set_selection(
                CharOffset::from(3)..CharOffset::from(9),
                selection.clone(),
                ctx,
            );
            buffer.style_internal(TextStyles::default().italic(), selection.clone(), ctx);
            assert_eq!(
                buffer.styled_blocks_in_range(
                    CharOffset::from(2)..buffer.max_charoffset(),
                    StyledBlockBoundaryBehavior::Exclusive
                ),
                vec![
                    StyledBufferBlock::Text(StyledTextBlock {
                        block: vec![
                            StyledBufferRun {
                                run: "e".into(),
                                text_styles: Default::default(),
                                block_style: BufferBlockStyle::PlainText
                            },
                            StyledBufferRun {
                                run: "llo\n".into(),
                                text_styles: TextStylesWithMetadata::default().italic(),
                                block_style: BufferBlockStyle::PlainText
                            },
                        ],
                        style: BufferBlockStyle::PlainText,
                        content_length: CharOffset::from(5)
                    }),
                    StyledBufferBlock::Text(StyledTextBlock {
                        block: vec![
                            // This is italic, along with the previous run, but it split by the newline.
                            StyledBufferRun {
                                run: "wo".into(),
                                text_styles: TextStylesWithMetadata::default().italic(),
                                block_style: BufferBlockStyle::PlainText
                            },
                            StyledBufferRun {
                                run: "rld".into(),
                                text_styles: Default::default(),
                                block_style: BufferBlockStyle::PlainText
                            }
                        ],
                        style: BufferBlockStyle::PlainText,
                        content_length: CharOffset::from(5)
                    })
                ]
            );
        });
    });
}

#[test]
fn test_styled_runs_blocks() {
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            buffer.edit_internal_first_selection(
                CharOffset::from(1)..CharOffset::from(1),
                "some\ntext",
                Default::default(),
                selection.clone(),
                ctx,
            );
            buffer.block_style_range(
                CharOffset::from(3)..CharOffset::from(8),
                BufferBlockStyle::CodeBlock {
                    code_block_type: Default::default(),
                },
                selection.clone(),
                ctx,
            );

            assert_eq!(
                buffer.styled_blocks_in_range(
                    CharOffset::from(1)..buffer.max_charoffset(),
                    StyledBlockBoundaryBehavior::Exclusive
                ),
                vec![
                    StyledBufferBlock::Text(StyledTextBlock {
                        block: vec![StyledBufferRun {
                            run: "so\n".into(),
                            text_styles: Default::default(),
                            block_style: BufferBlockStyle::PlainText
                        },],
                        style: BufferBlockStyle::PlainText,
                        content_length: CharOffset::from(3)
                    }),
                    StyledBufferBlock::Text(StyledTextBlock {
                        block: vec![
                            // This run is split by both the code block start and the internal newline.
                            StyledBufferRun {
                                run: "me\n".into(),
                                text_styles: Default::default(),
                                block_style: BufferBlockStyle::CodeBlock {
                                    code_block_type: Default::default()
                                },
                            },
                            StyledBufferRun {
                                run: "te\n".into(),
                                text_styles: Default::default(),
                                block_style: BufferBlockStyle::CodeBlock {
                                    code_block_type: Default::default()
                                },
                            },
                        ],
                        style: BufferBlockStyle::CodeBlock {
                            code_block_type: Default::default(),
                        },
                        content_length: CharOffset::from(6)
                    }),
                    StyledBufferBlock::Text(StyledTextBlock {
                        block: vec![StyledBufferRun {
                            run: "xt".into(),
                            text_styles: Default::default(),
                            block_style: BufferBlockStyle::PlainText
                        },],
                        style: BufferBlockStyle::PlainText,
                        content_length: CharOffset::from(2)
                    }),
                ]
            );
        });
    });
}

#[test]
fn test_styled_blocks_on_buffer_start() {
    // This tests styled block for the edge case of querying just the
    // leading block item (0..1).
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            buffer.edit_internal_first_selection(
                CharOffset::from(1)..CharOffset::from(1),
                "\n",
                Default::default(),
                selection.clone(),
                ctx,
            );

            buffer.insert_block_item(
                BufferBlockItem::HorizontalRule,
                CharOffset::from(1)..CharOffset::from(1),
            );
            assert_eq!(buffer.content.debug(), "<hr><text>");
            assert_eq!(
                buffer.styled_blocks_in_range(
                    CharOffset::from(0)..CharOffset::from(1),
                    StyledBlockBoundaryBehavior::Inclusive
                ),
                vec![StyledBufferBlock::Item(BufferBlockItem::HorizontalRule),]
            );
            assert_eq!(
                buffer.styled_blocks_in_range(
                    CharOffset::from(0)..CharOffset::from(1),
                    StyledBlockBoundaryBehavior::Exclusive
                ),
                vec![]
            );
        });
    });
}

#[test]
fn test_inline_markdown_roundtrips() {
    // Because of our formatting requirements, not all Markdown round-trips. These cases should,
    // however.
    let inputs = &[
        "A **bold** string",
        "A [*styled* link](https://example.com)",
        "**An [exterior](https://example.com) link**",
        "An `inline code` span",
        "Nested ***bold** and italic*",
        "Nested ***italic* and bold**",
        "A [link with `code`](https://example.com) and text",
        "*Complicated **text*** with *nest**ing***",
        "This `is not a [link](https://example.com) due to` precedence",
        "A **`bold code span`** too",
        "[link1](https://warp.dev)[**link2**](https://example.com)",
        "Combined *~~italic and strikethrough~~*",
        "Overlapping *~~abc~~def*",
        "This is <u>underlined</u>",
    ];

    for input in inputs {
        let formatted = parse_markdown(input).unwrap();
        assert_eq!(
            Buffer::export_to_markdown(formatted, None, MarkdownStyle::Internal),
            *input
        );
    }
}

#[test]
fn test_export_markdown_blocks() {
    let markdown =
        "A `styled`\n***range** of text* and\n```warp-runnable-command\ncode\nblock\n```\n";
    let formatted = parse_markdown(markdown).unwrap();
    assert_eq!(
        Buffer::export_to_markdown(formatted, None, MarkdownStyle::Internal),
        markdown
    );
}

#[test]
fn text_export_markdown_styled_inline_code() {
    // Markdown ignores formatting within code spans. Because of this, our options are:
    // 1. Silently strip out formatting within code spans:
    //    <c_s>first <b_s>word<b_e> last<c_e> ==> `first word last`
    // 2. Split up code spans so that we can wrap them with formatting:
    //   <c_s>first <b_s>word<b_e> last<c_e> ==> `first` **`word`** `last`
    // 3. Preserve the Markdown formatting within the span:
    //   <c_s>first <b_s>word<b_e> last<c_e> ==> `first **word** last`
    // Like Notion, we currently go with (3) on the assumption that:
    // * Users will generally expect that they can't format within a code span
    // * Preserving the Markdown characters is then the least-surprising behavior.

    App::test((), |mut app| async move {
        let (buffer, selection) = Buffer::mock_from_markdown(
            "`first word last`",
            None,
            Box::new(|_, _| IndentBehavior::Ignore),
            &mut app,
        );

        buffer.update(&mut app, |buffer, ctx| {
            buffer.set_selection(
                CharOffset::from(7)..CharOffset::from(11),
                selection.clone(),
                ctx,
            );
            buffer.style_internal(TextStyles::default().bold(), selection.clone(), ctx);
            assert_eq!(buffer.debug(), "<text><c_s>first <b_s>word<b_e> last<c_e>");
            assert_eq!(buffer.markdown(), "`first **word** last`");
        });
    });
}

#[test]
fn test_markdown_styled_whitespace() {
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            buffer.edit_internal_first_selection(
                CharOffset::from(1)..CharOffset::from(1),
                "mixed  emphasis    end",
                Default::default(),
                selection.clone(),
                ctx,
            );
            buffer.set_selection(
                CharOffset::from(1)..CharOffset::from(20),
                selection.clone(),
                ctx,
            );
            buffer.style_internal(TextStyles::default().italic(), selection.clone(), ctx);
            buffer.set_selection(
                CharOffset::from(6)..CharOffset::from(20),
                selection.clone(),
                ctx,
            );
            buffer.style_internal(TextStyles::default().bold(), selection.clone(), ctx);
            // Even though the spaces should be styled, this isn't representable in Markdown.
            // Instead, the formatting is shifted around the whitespace.
            assert_eq!(
                buffer.debug(),
                "<text><i_s>mixed<b_s>  emphasis    <i_e><b_e>end"
            );
            assert_eq!(buffer.markdown(), "*mixed  **emphasis***    end");
        });

        let markdown_text = buffer.read(&app, |buffer, _| buffer.markdown());
        let (buffer2, _selection) = Buffer::mock_from_markdown(
            &markdown_text,
            None,
            Box::new(|_, _| IndentBehavior::Ignore),
            &mut app,
        );
        buffer2.read(&app, |buffer, _| {
            assert_eq!(
                buffer.debug(),
                "<text><i_s>mixed  <b_s>emphasis<b_e><i_e>    end"
            );
        });

        // Regression test for handling multibyte characters around the rearranged whitespace (CLD-962).
        let buffer3 = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection3 = app.add_model(|_| BufferSelectionModel::new(buffer3.clone()));

        buffer3.update(&mut app, |buffer, ctx| {
            buffer.edit_internal_first_selection(
                CharOffset::from(1)..CharOffset::from(1),
                "styled🧑‍💻    not",
                Default::default(),
                selection3.clone(),
                ctx,
            );
            buffer.set_selection(
                CharOffset::from(1)..CharOffset::from(14),
                selection3.clone(),
                ctx,
            );
            buffer.style_internal(TextStyles::default().italic(), selection3.clone(), ctx);
            assert_eq!(buffer.debug(), "<text><i_s>styled🧑‍💻    <i_e>not");
            assert_eq!(buffer.markdown(), "*styled🧑‍💻*    not");
        });
    });
}

#[test]
fn test_export_markdown_sublists() {
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            buffer.edit_internal_first_selection(
                CharOffset::from(1)..CharOffset::from(1),
                "Line\nFirst\nSecond\nThird\nLast",
                Default::default(),
                selection.clone(),
                ctx,
            );

            let _ = buffer.block_style_range(
                CharOffset::from(1)..CharOffset::from(5),
                BufferBlockStyle::ordered_list(ListIndentLevel::One),
                selection.clone(),
                ctx,
            );

            let _ = buffer.block_style_range(
                CharOffset::from(6)..CharOffset::from(11),
                BufferBlockStyle::ordered_list(ListIndentLevel::Two),
                selection.clone(),
                ctx,
            );

            let _ = buffer.block_style_range(
                CharOffset::from(12)..CharOffset::from(18),
                BufferBlockStyle::ordered_list(ListIndentLevel::Three),
                selection.clone(),
                ctx,
            );

            let _ = buffer.block_style_range(
                CharOffset::from(19)..CharOffset::from(24),
                BufferBlockStyle::ordered_list(ListIndentLevel::One),
                selection.clone(),
                ctx,
            );

            let _ = buffer.block_style_range(
                CharOffset::from(25)..CharOffset::from(29),
                BufferBlockStyle::ordered_list(ListIndentLevel::Two),
                selection.clone(),
                ctx,
            );

            assert_eq!(
                buffer.debug(),
                "<ol0>Line<ol1>First<ol2>Second<ol0>Third<ol1>Last<text>"
            );

            assert_eq!(
                buffer.markdown(),
                "1. Line\n    1. First\n        1. Second\n2. Third\n    1. Last\n"
            );
        });
    });
}

#[test]
fn test_markdown_omits_placeholders() {
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            buffer.edit_internal_first_selection(
                CharOffset::from(1)..CharOffset::from(1),
                "markdown",
                Default::default(),
                selection.clone(),
                ctx,
            );
            buffer.insert_placeholder(3.into(), "placeholder", selection.clone(), ctx);
            buffer.set_selection(
                CharOffset::from(1)..CharOffset::from(7),
                selection.clone(),
                ctx,
            );
            buffer.style_internal(TextStyles::default().italic(), selection.clone(), ctx);
            assert_eq!(
                buffer.debug(),
                "<text><i_s>ma<placeholder_s>placeholder<placeholder_e>rkd<i_e>own"
            );

            assert_eq!(buffer.markdown(), "*markd*own");
        });
    });
}

#[test]
fn test_markdown_escapes() {
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            buffer.edit_internal_first_selection(
                CharOffset::from(1)..CharOffset::from(1),
                "This is *not* markdownThis is $code*!*",
                Default::default(),
                selection.clone(),
                ctx,
            );
            buffer.block_style_range(
                CharOffset::from(23)..CharOffset::from(39),
                BufferBlockStyle::CodeBlock {
                    code_block_type: Default::default(),
                },
                selection.clone(),
                ctx,
            );
            assert_eq!(
                buffer.debug(),
                "<text>This is *not* markdown<code:Shell>This is $code*!*<text>"
            );
            // The Markdown special characters should be escaped.
            // // Punctuation in code blocks should not be escaped.
            assert_eq!(
                buffer.markdown(),
                "This is \\*not\\* markdown\n```warp-runnable-command\nThis is $code*!*\n```\n"
            );
        });

        // The buffer should roundtrip as well.
        let markdown_text = buffer.read(&app, |buffer, _| buffer.markdown());
        let (parsed, _selection) = Buffer::mock_from_markdown(
            &markdown_text,
            None,
            Box::new(|_, _| IndentBehavior::Ignore),
            &mut app,
        );
        parsed.read(&app, |buffer, _| {
            assert_eq!(
                buffer.debug(),
                "<text>This is *not* markdown<code:Shell>This is $code*!*<text>"
            );
        });
    });
}

#[test]
fn test_import_markdown() {
    App::test((), |mut app| async move {
        let markdown_string =
            "test\n```warp-runnable-command\nparagragh\n```\nSome text\nSome ***bold and italic***";
        let (buffer, _selection) = Buffer::mock_from_markdown(
            markdown_string,
            None,
            Box::new(|_, _| IndentBehavior::Ignore),
            &mut app,
        );
        buffer.read(&app, |buffer, _| {
            assert_eq!(
                buffer.content.debug(),
                "<text>test<code:Shell>paragragh<text>Some text\\nSome <b_s><i_s>bold and italic<b_e><i_e>"
            );
            // Ensure we could go a round trip without altering markdowns.
            assert_eq!(buffer.markdown(), markdown_string);
        });

        let markdown_string = "***bold and italic";
        let (buffer, _selection) = Buffer::mock_from_markdown(
            markdown_string,
            None,
            Box::new(|_, _| IndentBehavior::Ignore),
            &mut app,
        );
        buffer.read(&app, |buffer, _| {
            assert_eq!(buffer.content.debug(), "<text>***bold and italic");
            assert_eq!(buffer.markdown(), "\\*\\*\\*bold and italic");
        });

        let markdown_string = "p1\n```text\nsome\ncode\n```\na\n```text\nsome\nother\ncode\n```\n";
        let (buffer, _selection) = Buffer::mock_from_markdown(
            markdown_string,
            None,
            Box::new(|_, _| IndentBehavior::Ignore),
            &mut app,
        );
        buffer.read(&app, |buffer, _| {
            assert_eq!(
                buffer.content.debug(),
                "<text>p1<code:Code>some\\ncode<text>a<code:Code>some\\nother\\ncode<text>"
            );
            assert_eq!(buffer.markdown(), markdown_string);
        });

        let markdown_string = "aaa\n```warp-runnable-command\nafb\n```\n*b**b***\n```warp-runnable-command\nb\nlll\n```\n";
        let (buffer, _selection) = Buffer::mock_from_markdown(
            markdown_string,
            None,
            Box::new(|_, _| IndentBehavior::Ignore),
            &mut app,
        );
        buffer.read(&app, |buffer, _| {
            assert_eq!(
                buffer.content.debug(),
                "<text>aaa<code:Shell>afb<text><i_s>b<b_s>b<b_e><i_e><code:Shell>b\\nlll<text>"
            );
            assert_eq!(buffer.markdown(), markdown_string);
        });

        let markdown_string = "```warp-runnable-command\ntest\nblock\n```\n";
        let (buffer, _selection) = Buffer::mock_from_markdown(
            markdown_string,
            None,
            Box::new(|_, _| IndentBehavior::Ignore),
            &mut app,
        );
        buffer.read(&app, |buffer, _| {
            assert_eq!(buffer.content.debug(), "<code:Shell>test\\nblock<text>");
            assert_eq!(buffer.markdown(), markdown_string);
        });

        let markdown_string = "### Header\n## Header\nText";
        let (buffer, _selection) = Buffer::mock_from_markdown(
            markdown_string,
            None,
            Box::new(|_, _| IndentBehavior::Ignore),
            &mut app,
        );
        buffer.read(&app, |buffer, _| {
            assert_eq!(
                buffer.content.debug(),
                "<header3>Header<header2>Header<text>Text"
            );
            assert_eq!(buffer.markdown(), markdown_string);
        });

        let markdown_string = "* List\n* List\nText";
        let (buffer, _selection) = Buffer::mock_from_markdown(
            markdown_string,
            None,
            Box::new(|_, _| IndentBehavior::Ignore),
            &mut app,
        );
        buffer.read(&app, |buffer, _| {
            assert_eq!(buffer.content.debug(), "<ul0>List<ul0>List<text>Text");
            assert_eq!(buffer.markdown(), markdown_string);
        });

        let markdown_string = "* List\n    * List\n        * List\n";
        let (buffer, _selection) = Buffer::mock_from_markdown(
            markdown_string,
            None,
            Box::new(|_, _| IndentBehavior::Ignore),
            &mut app,
        );
        app.read_model(&buffer, |buffer, _| {
            assert_eq!(buffer.content.debug(), "<ul0>List<ul1>List<ul2>List<text>");
            assert_eq!(buffer.markdown(), markdown_string);
        });

        // Per the spec, only the first ordered list item's number is used.
        // https://spec.commonmark.org/0.30/#start-number
        let markdown_string = "3. First\n2. Second\n1. Third";
        let (buffer, _selection) = Buffer::mock_from_markdown(
            markdown_string,
            None,
            Box::new(|_, _| IndentBehavior::Ignore),
            &mut app,
        );
        // When importing from Markdown, we ensure that the buffer ends as plain text.
        buffer.read(&app, |buffer, _| {
            assert_eq!(
                buffer.content.debug(),
                "<ol0@3>First<ol0>Second<ol0>Third<text>"
            );
            assert_eq!(buffer.markdown(), "3. First\n4. Second\n5. Third\n");
        });
    });
}

#[test]
fn test_import_empty_markdown() {
    App::test((), |mut app| async move {
        // This is a regression test for CLD-601, where parsing an empty Markdown file would put the
        // buffer in an invalid state.
        let (buffer, selection) =
            Buffer::mock_from_markdown("", None, Box::new(|_, _| IndentBehavior::Ignore), &mut app);
        buffer.read(&app, |buffer, _| {
            assert_eq!(buffer.content.debug(), "<text>");
        });
        selection.read(&app, |selection, _| {
            assert_eq!(selection.first_selection_head(), CharOffset::from(1));
        });
        buffer.read(&app, |buffer, _| {
            selection.read(&app, |selection, _| {
                buffer.validate(&selection.anchors);
            });
        });

        let (buffer, selection) = Buffer::mock_from_markdown(
            "has text",
            None,
            Box::new(|_, _| IndentBehavior::Ignore),
            &mut app,
        );

        buffer.update(&mut app, |buffer, ctx| {
            buffer.replace(InitialBufferState::markdown(""), selection.clone(), ctx);

            assert_eq!(buffer.content.debug(), "<text>");
        });

        selection.read(&app, |selection, _| {
            assert_eq!(selection.first_selection_head(), CharOffset::from(1));
        });

        buffer.read(&app, |buffer, _| {
            selection.read(&app, |selection, _| {
                buffer.validate(&selection.anchors);
            });
        });
    });
}

#[test]
fn test_import_markdown_code() {
    App::test((), |mut app| async move {
        // This tests that we categorize Markdown code blocks as expected.
        let (buffer, selection) = Buffer::mock_from_markdown(
            r#"```
default code
```
```sh
sh code
```
```rust
rust code
```
```warp-runnable-command
warp code
```"#,
            None,
            Box::new(|_, _| IndentBehavior::Ignore),
            &mut app,
        );

        buffer.read(&app, |buffer, _| {
            assert_eq!(buffer.debug(), "<code:Shell>default code<code:Shell>sh code<code:Rust>rust code<code:Shell>warp code<text>");
        });
        buffer.read(&app, |buffer, _| {
            selection.read(&app, |selection, _| {
                buffer.validate(&selection.anchors);
            });
        });
    });
}

#[test]
fn test_import_markdown_embedded() {
    App::test((), |mut app| async move {
        let (buffer, selection) = Buffer::mock_from_markdown(
            r#"```
default code
```
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
            &mut app,
        );

        buffer.read(&app, |buffer, _| {
            assert_eq!(
                buffer.debug(),
                "<code:Shell>default code<embed_workflow-123><embed_workflow-123><text>"
            );
        });
        buffer.read(&app, |buffer, _| {
            selection.read(&app, |selection, _| {
                buffer.validate(&selection.anchors);
            });
        });
    });
}

#[test]
fn test_unstyle_block_noop() {
    // This tests converting a block back to text when no action is needed (it's
    // already text).
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            buffer.edit_internal_first_selection(
                CharOffset::from(1)..CharOffset::from(1),
                "Hello\nWorld",
                Default::default(),
                selection.clone(),
                ctx,
            );
            buffer.set_selection(
                CharOffset::from(1)..CharOffset::from(3),
                selection.clone(),
                ctx,
            );
            buffer.style_internal(TextStyles::default().bold(), selection.clone(), ctx);

            let selection_range = CharOffset::from(4)..CharOffset::from(9);

            // Set a selection so we can confirm it's unaffected.
            buffer.set_selection(selection_range.clone(), selection.clone(), ctx);
            let delta = buffer
                .block_style_range(
                    selection_range,
                    BufferBlockStyle::PlainText,
                    selection.clone(),
                    ctx,
                )
                .delta;
            assert!(delta.is_none());
        });
    });
}

#[test]
fn test_unstyle_block_partial_left() {
    // Test unstyling when there is a block overlapping to the left of the selection.
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            buffer.edit_internal_first_selection(
                CharOffset::from(1)..CharOffset::from(1),
                "HelloWorld\nExtra",
                Default::default(),
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
            assert_eq!(buffer.text().as_str(), "Hello\nWorld\nExtra");

            // Operate on `lo\nWo`
            let selection_range = CharOffset::from(4)..CharOffset::from(9);
            buffer.set_selection(selection_range.clone(), selection.clone(), ctx);

            let delta = buffer
                .block_style_range(
                    selection_range,
                    BufferBlockStyle::PlainText,
                    selection.clone(),
                    ctx,
                )
                .delta
                .expect("Should exist");

            // After this operation, the selection moves right 1 to reflect the new
            // newline before `lo`.
            assert_eq!(
                selection.as_ref(ctx).selection_to_first_offset_range(),
                CharOffset::from(5)..CharOffset::from(10)
            );

            assert_eq!(
                buffer.content.debug(),
                "<code:Shell>Hel<text>lo\\nWorld\\nExtra"
            );

            // The "Hello" runnable command and the "World" paragraph are affected.
            assert_eq!(delta.old_offset, CharOffset::from(1)..CharOffset::from(7));

            // The "Hel" runnable command and the "lo" and "World" paragraphs are re-rendered.
            assert_eq!(
                delta.new_lines,
                vec![
                    StyledBufferBlock::Text(StyledTextBlock {
                        block: vec![StyledBufferRun {
                            run: "Hel\n".to_string(),
                            text_styles: Default::default(),
                            block_style: BufferBlockStyle::CodeBlock {
                                code_block_type: Default::default()
                            }
                        }],
                        style: BufferBlockStyle::CodeBlock {
                            code_block_type: Default::default(),
                        },
                        content_length: CharOffset::from(4)
                    }),
                    StyledBufferBlock::Text(StyledTextBlock {
                        block: vec![StyledBufferRun {
                            run: "lo\n".to_string(),
                            text_styles: Default::default(),
                            block_style: BufferBlockStyle::PlainText
                        }],
                        style: BufferBlockStyle::PlainText,
                        content_length: CharOffset::from(3)
                    }),
                ]
            );
        });
    });
}

#[test]
fn test_unstyle_block_partial_right() {
    // Tests unstyling when there is a block overlapping to the right of the selection.
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            buffer.edit_internal_first_selection(
                CharOffset::from(1)..CharOffset::from(1),
                "Extra\nHello\nWorld\nExtra",
                Default::default(),
                selection.clone(),
                ctx,
            );
            // Make "World" a runnable command.
            buffer.block_style_range(
                CharOffset::from(13)..CharOffset::from(18),
                BufferBlockStyle::CodeBlock {
                    code_block_type: Default::default(),
                },
                selection.clone(),
                ctx,
            );
            assert_eq!(
                buffer.content.debug(),
                "<text>Extra\\nHello<code:Shell>World<text>Extra"
            );

            // Operate on `lo\nWo`.
            let selection_range = CharOffset::from(10)..CharOffset::from(15);
            buffer.set_selection(selection_range.clone(), selection.clone(), ctx);

            let delta = buffer
                .block_style_range(
                    selection_range,
                    BufferBlockStyle::PlainText,
                    selection.clone(),
                    ctx,
                )
                .delta
                .expect("Should exist");

            // After the operation, the selection extends by 1 to account for the additional newline.
            assert_eq!(
                selection.as_ref(ctx).selection_to_first_offset_range(),
                CharOffset::from(10)..CharOffset::from(16)
            );

            assert_eq!(
                buffer.content.debug(),
                "<text>Extra\\nHello\\nWo<code:Shell>rld<text>Extra"
            );

            // The edit range expands to `Hello\nWorld\n`.
            assert_eq!(delta.old_offset, CharOffset::from(13)..CharOffset::from(19));

            assert_eq!(
                delta.new_lines,
                vec![
                    StyledBufferBlock::Text(StyledTextBlock {
                        block: vec![
                            // This changed from a runnable command to a new paragraph.
                            StyledBufferRun {
                                run: "Wo\n".to_string(),
                                text_styles: Default::default(),
                                block_style: BufferBlockStyle::PlainText
                            },
                        ],
                        style: BufferBlockStyle::PlainText,
                        content_length: CharOffset::from(3)
                    }),
                    StyledBufferBlock::Text(StyledTextBlock {
                        block: vec![
                            // This part of the runnable command is re-rendered
                            StyledBufferRun {
                                run: "rld\n".to_string(),
                                text_styles: Default::default(),
                                block_style: BufferBlockStyle::CodeBlock {
                                    code_block_type: Default::default()
                                }
                            },
                        ],
                        style: BufferBlockStyle::CodeBlock {
                            code_block_type: Default::default(),
                        },
                        content_length: CharOffset::from(4)
                    })
                ]
            );
        });
    });
}

#[test]
fn test_unstyle_block_multi_line() {
    // Tests unstyling a line in a block with multiple lines.
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            buffer.edit_internal_first_selection(
                CharOffset::from(1)..CharOffset::from(1),
                "Before\nText\nSecond\nAfter",
                Default::default(),
                selection.clone(),
                ctx,
            );

            // Style Text to a runnable command and back again.
            buffer.block_style_range(
                CharOffset::from(8)..CharOffset::from(19),
                BufferBlockStyle::CodeBlock {
                    code_block_type: Default::default(),
                },
                selection.clone(),
                ctx,
            );

            let selection_range = CharOffset::from(8)..CharOffset::from(12);
            buffer.set_selection(selection_range.clone(), selection.clone(), ctx);

            let delta = buffer
                .block_style_range(
                    selection_range.clone(),
                    BufferBlockStyle::PlainText,
                    selection.clone(),
                    ctx,
                )
                .delta
                .expect("Should exist");
            // There's no net character change, so the selection should not move.
            assert_eq!(
                selection.as_ref(ctx).selection_to_first_offset_range(),
                selection_range
            );

            assert_eq!(
                buffer.content.debug(),
                "<text>Before\\nText<code:Shell>Second<text>After"
            );

            // Effectively, only the `Text` block was affected. This is slightly different
            // from the selection range because it also includes the trailing newline.
            assert_eq!(delta.old_offset, CharOffset::from(8)..CharOffset::from(20));

            // Only the converted block needs to be re-rendered.
            assert_eq!(
                delta.new_lines,
                vec![
                    StyledBufferBlock::Text(StyledTextBlock {
                        block: vec![StyledBufferRun {
                            run: "Text\n".to_string(),
                            text_styles: Default::default(),
                            block_style: BufferBlockStyle::PlainText
                        }],
                        style: BufferBlockStyle::PlainText,
                        content_length: CharOffset::from(5)
                    }),
                    StyledBufferBlock::Text(StyledTextBlock {
                        block: vec![StyledBufferRun {
                            run: "Second\n".to_string(),
                            text_styles: Default::default(),
                            block_style: BufferBlockStyle::CodeBlock {
                                code_block_type: Default::default()
                            }
                        }],
                        style: BufferBlockStyle::CodeBlock {
                            code_block_type: Default::default(),
                        },
                        content_length: CharOffset::from(7)
                    })
                ]
            );
        });
    });
}

#[test]
fn test_edit_to_unstyle_block_multi_line() {
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            buffer.edit_internal_first_selection(
                CharOffset::from(1)..CharOffset::from(1),
                "Before\nText\nSecond\nAfter",
                Default::default(),
                selection.clone(),
                ctx,
            );

            buffer.block_style_range(
                CharOffset::from(1)..CharOffset::from(7),
                BufferBlockStyle::UnorderedList {
                    indent_level: ListIndentLevel::One,
                },
                selection.clone(),
                ctx,
            );
            buffer.block_style_range(
                CharOffset::from(8)..CharOffset::from(19),
                BufferBlockStyle::CodeBlock {
                    code_block_type: Default::default(),
                },
                selection.clone(),
                ctx,
            );
            assert_eq!(
                buffer.content.debug(),
                "<ul0>Before<code:Shell>Text\\nSecond<text>After"
            );

            let delta = buffer
                .edit_internal_first_selection(
                    CharOffset::from(7)..CharOffset::from(8),
                    "",
                    Default::default(),
                    selection.clone(),
                    ctx,
                )
                .delta
                .expect("Should exist");

            // Since the previous block only supports single line, we should only convert
            // the first line to match the previous block's style.
            assert_eq!(
                buffer.content.debug(),
                "<ul0>BeforeText<text>Second\\nAfter"
            );

            assert_eq!(delta.old_offset, CharOffset::from(1)..CharOffset::from(20));
            assert_eq!(
                delta.new_lines,
                vec![
                    StyledBufferBlock::Text(StyledTextBlock {
                        block: vec![StyledBufferRun {
                            run: "BeforeText\n".to_string(),
                            text_styles: Default::default(),
                            block_style: BufferBlockStyle::UnorderedList {
                                indent_level: ListIndentLevel::One
                            }
                        }],
                        style: BufferBlockStyle::UnorderedList {
                            indent_level: ListIndentLevel::One,
                        },
                        content_length: CharOffset::from(11)
                    }),
                    StyledBufferBlock::Text(StyledTextBlock {
                        block: vec![StyledBufferRun {
                            run: "Second\n".to_string(),
                            text_styles: Default::default(),
                            block_style: BufferBlockStyle::PlainText
                        }],
                        style: BufferBlockStyle::PlainText,
                        content_length: CharOffset::from(7)
                    })
                ]
            );
        });
    });
}

#[test]
fn test_unstyle_block_exact() {
    // Tests unstyling a block exactly.
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            buffer.edit_internal_first_selection(
                CharOffset::from(1)..CharOffset::from(1),
                "Before\nText\nAfter",
                Default::default(),
                selection.clone(),
                ctx,
            );

            // Style Text to a runnable command and back again.
            let selection_range = CharOffset::from(8)..CharOffset::from(12);

            buffer.block_style_range(
                selection_range.clone(),
                BufferBlockStyle::CodeBlock {
                    code_block_type: Default::default(),
                },
                selection.clone(),
                ctx,
            );
            buffer.set_selection(selection_range.clone(), selection.clone(), ctx);

            let delta = buffer
                .block_style_range(
                    selection_range.clone(),
                    BufferBlockStyle::PlainText,
                    selection.clone(),
                    ctx,
                )
                .delta
                .expect("Should exist");
            // There's no net character change, so the selection should not move.
            assert_eq!(
                selection.as_ref(ctx).selection_to_first_offset_range(),
                selection_range
            );

            assert_eq!(buffer.content.debug(), "<text>Before\\nText\\nAfter");

            // Effectively, only the `Text` block was affected. This is slightly different
            // from the selection range because it also includes the trailing newline.
            assert_eq!(delta.old_offset, CharOffset::from(8)..CharOffset::from(13));

            // Only the converted block needs to be re-rendered.
            assert_eq!(
                delta.new_lines,
                vec![StyledBufferBlock::Text(StyledTextBlock {
                    block: vec![StyledBufferRun {
                        run: "Text\n".to_string(),
                        text_styles: Default::default(),
                        block_style: BufferBlockStyle::PlainText
                    }],
                    style: BufferBlockStyle::PlainText,
                    content_length: CharOffset::from(5)
                })]
            );
        });
    });
}

#[test]
fn test_unstyle_block_within() {
    // Tests unstyling the middle chunk of a block.
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            buffer.edit_internal_first_selection(
                CharOffset::from(1)..CharOffset::from(1),
                "Extra\nThe\nBlock\nExtra",
                Default::default(),
                selection.clone(),
                ctx,
            );
            buffer.block_style_range(
                CharOffset::from(7)..CharOffset::from(16),
                BufferBlockStyle::CodeBlock {
                    code_block_type: Default::default(),
                },
                selection.clone(),
                ctx,
            );
            assert_eq!(
                buffer.content.debug(),
                "<text>Extra<code:Shell>The\\nBlock<text>Extra"
            );

            // Operate on `e\nBlo`.
            let selection_range = CharOffset::from(9)..CharOffset::from(14);
            buffer.set_selection(selection_range.clone(), selection.clone(), ctx);

            let delta = buffer
                .block_style_range(
                    selection_range,
                    BufferBlockStyle::PlainText,
                    selection.clone(),
                    ctx,
                )
                .delta
                .expect("Should exist");

            assert_eq!(
                buffer.content.debug(),
                "<text>Extra<code:Shell>Th<text>e\\nBlo<code:Shell>ck<text>Extra"
            );
            assert_eq!(buffer.text().as_str(), "Extra\nTh\ne\nBlo\nck\nExtra");
            // The selection shifts to start after the newline before "e" and end after
            // the newline between "o" and "ck"
            assert_eq!(
                selection.as_ref(ctx).selection_to_first_offset_range(),
                CharOffset::from(10)..CharOffset::from(16)
            );

            // The affected range is that of the old command block, `The\nBlock`.
            assert_eq!(delta.old_offset, CharOffset::from(7)..CharOffset::from(17));

            // The new content to render is the 4 blocks created from the former 2 lines
            // of runnable commands.
            assert_eq!(
                delta.new_lines,
                vec![
                    StyledBufferBlock::Text(StyledTextBlock {
                        block: vec![StyledBufferRun {
                            run: "Th\n".to_string(),
                            text_styles: Default::default(),
                            block_style: BufferBlockStyle::CodeBlock {
                                code_block_type: Default::default()
                            }
                        }],
                        style: BufferBlockStyle::CodeBlock {
                            code_block_type: Default::default(),
                        },
                        content_length: CharOffset::from(3)
                    }),
                    StyledBufferBlock::Text(StyledTextBlock {
                        block: vec![StyledBufferRun {
                            run: "e\n".to_string(),
                            text_styles: Default::default(),
                            block_style: BufferBlockStyle::PlainText
                        }],
                        style: BufferBlockStyle::PlainText,
                        content_length: CharOffset::from(2)
                    }),
                    StyledBufferBlock::Text(StyledTextBlock {
                        block: vec![StyledBufferRun {
                            run: "Blo\n".to_string(),
                            text_styles: Default::default(),
                            block_style: BufferBlockStyle::PlainText
                        }],
                        style: BufferBlockStyle::PlainText,
                        content_length: CharOffset::from(4)
                    }),
                    StyledBufferBlock::Text(StyledTextBlock {
                        block: vec![StyledBufferRun {
                            run: "ck\n".to_string(),
                            text_styles: Default::default(),
                            block_style: BufferBlockStyle::CodeBlock {
                                code_block_type: Default::default()
                            },
                        }],
                        style: BufferBlockStyle::CodeBlock {
                            code_block_type: Default::default(),
                        },
                        content_length: CharOffset::from(3)
                    })
                ]
            );
        });
    });
}

#[test]
fn test_unstyle_block_surrounded() {
    // Test unstyling when there's a block fully within the selection range.
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            buffer.edit_internal_first_selection(
                CharOffset::from(1)..CharOffset::from(1),
                "Before\nBlock\nAfter",
                Default::default(),
                selection.clone(),
                ctx,
            );
            buffer.block_style_range(
                CharOffset::from(8)..CharOffset::from(13),
                BufferBlockStyle::CodeBlock {
                    code_block_type: Default::default(),
                },
                selection.clone(),
                ctx,
            );

            // Operate on the whole "ore\nBlock\nAft" range.
            let selection_range = CharOffset::from(4)..CharOffset::from(17);
            buffer.set_selection(selection_range.clone(), selection.clone(), ctx);

            let delta = buffer
                .block_style_range(
                    selection_range.clone(),
                    BufferBlockStyle::PlainText,
                    selection.clone(),
                    ctx,
                )
                .delta
                .expect("Should exist");

            // The selection range shouldn't change in this case, since no characters are
            // added or removed.
            assert_eq!(
                selection.as_ref(ctx).selection_to_first_offset_range(),
                selection_range
            );

            assert_eq!(buffer.content.debug(), "<text>Before\\nBlock\\nAfter");

            // The whole buffer is affected, due to the selection range.
            assert_eq!(delta.old_offset, CharOffset::from(8)..CharOffset::from(14));
            // This means we should return all blocks in the buffer.
            assert_eq!(
                delta.new_lines,
                vec![StyledBufferBlock::Text(StyledTextBlock {
                    block: vec![StyledBufferRun {
                        run: "Block\n".to_string(),
                        text_styles: Default::default(),
                        block_style: BufferBlockStyle::PlainText,
                    }],
                    style: BufferBlockStyle::PlainText,
                    content_length: CharOffset::from(6)
                }),]
            );
        });
    });
}

#[test]
fn test_enter_at_block_start() {
    // This tests that Enter at the start of a list or heading block preserves its styling and
    // inserts a new line above the block.

    App::test((), |mut app| async move {
        let (buffer, selection) = Buffer::mock_from_markdown(
            "Text\n3. List\nText\n# Heading",
            None,
            Box::new(|_, _| IndentBehavior::Ignore),
            &mut app,
        );

        buffer.update(&mut app, |buffer, ctx| {
            assert_eq!(
                buffer.debug(),
                "<text>Text<ol0@3>List<text>Text<header1>Heading<text>"
            );

            // Enter at the start of the list item.
            selection.update(ctx, |selection, _| {
                selection.set_single_cursor(CharOffset::from(6));
            });
            let prev_selection = buffer.to_rendered_selection_set(selection.clone(), ctx);
            let result = buffer.enter(false, Default::default(), selection.clone(), ctx);
            let current_selection = buffer.to_rendered_selection_set(selection.clone(), ctx);

            buffer.push_undo_item(
                prev_selection,
                current_selection,
                result.undo_item.expect("Should be undoable"),
                UndoActionType::Atomic,
            );

            let delta = result.delta.expect("Need edit delta to re-render");
            // The block after the cursor is re-rendered to splice in the new, empty list item.
            assert_eq!(delta.old_offset, CharOffset::from(6)..CharOffset::from(11));
            assert_eq!(
                delta.new_lines,
                vec![
                    StyledBufferBlock::Text(StyledTextBlock {
                        block: vec![StyledBufferRun {
                            run: "\n".to_string(),
                            text_styles: Default::default(),
                            block_style: BufferBlockStyle::OrderedList {
                                number: Some(3),
                                indent_level: ListIndentLevel::One
                            }
                        }],
                        style: BufferBlockStyle::OrderedList {
                            number: Some(3),
                            indent_level: ListIndentLevel::One
                        },
                        content_length: CharOffset::from(1)
                    }),
                    StyledBufferBlock::Text(StyledTextBlock {
                        block: vec![StyledBufferRun {
                            run: "List\n".to_string(),
                            text_styles: Default::default(),
                            block_style: BufferBlockStyle::ordered_list(ListIndentLevel::One)
                        }],
                        style: BufferBlockStyle::ordered_list(ListIndentLevel::One),
                        content_length: CharOffset::from(5)
                    })
                ]
            );

            assert_eq!(
                buffer.debug(),
                "<text>Text<ol0@3><ol0>List<text>Text<header1>Heading<text>"
            );
            assert_eq!(
                selection.as_ref(ctx).selection_to_first_offset_range(),
                CharOffset::from(7)..CharOffset::from(7)
            );

            buffer.undo(selection.clone(), ctx);
            assert_eq!(
                buffer.debug(),
                "<text>Text<ol0@3>List<text>Text<header1>Heading<text>"
            );
        });
    });
}

#[test]
fn test_enter_at_code_block_start() {
    App::test((), |mut app| async move {
        let (buffer, selection) = Buffer::mock_from_markdown(
            "```\nThis is code\nMore code\n```\nText",
            None,
            Box::new(|_, _| IndentBehavior::Ignore),
            &mut app,
        );

        buffer.update(&mut app, |buffer, ctx| {
            assert_eq!(
                buffer.debug(),
                "<code:Shell>This is code\\nMore code<text>Text"
            );

            // Enter at the start of the code block.
            selection.update(ctx, |selection, _| {
                selection.set_single_cursor(1.into());
            });
            let prev_selection = buffer.to_rendered_selection_set(selection.clone(), ctx);
            let result = buffer.enter(false, Default::default(), selection.clone(), ctx);
            let current_selection = buffer.to_rendered_selection_set(selection.clone(), ctx);
            buffer.push_undo_item(
                prev_selection,
                current_selection,
                result.undo_item.expect("Should be undoable"),
                UndoActionType::Atomic,
            );

            let delta = result.delta.expect("Need edit delta to re-render");
            // A new block is spliced in at the start of the buffer, but the code block doesn't change.
            assert_eq!(delta.old_offset, 0.into()..1.into());
            assert_eq!(
                delta.new_lines,
                vec![StyledBufferBlock::Text(StyledTextBlock {
                    block: vec![StyledBufferRun {
                        run: "\n".to_string(),
                        text_styles: Default::default(),
                        block_style: BufferBlockStyle::PlainText
                    }],
                    style: BufferBlockStyle::PlainText,
                    content_length: CharOffset::from(1)
                })]
            );

            assert_eq!(
                buffer.debug(),
                "<text><code:Shell>This is code\\nMore code<text>Text"
            );
            assert_eq!(
                selection.as_ref(ctx).selection_to_first_offset_range(),
                2.into()..2.into()
            );

            buffer.undo(selection.clone(), ctx);
            assert_eq!(
                buffer.debug(),
                "<code:Shell>This is code\\nMore code<text>Text"
            );

            buffer.redo(selection.clone(), ctx);
            assert_eq!(
                buffer.debug(),
                "<text><code:Shell>This is code\\nMore code<text>Text"
            );

            // Enter again, which should insert another blank line.
            assert_eq!(
                selection.as_ref(ctx).selection_to_first_offset_range(),
                2.into()..2.into()
            );
            buffer.enter(false, Default::default(), selection.clone(), ctx);
            assert_eq!(
                buffer.debug(),
                "<text>\\n<code:Shell>This is code\\nMore code<text>Text"
            );
        });
    });
}

#[test]
fn test_enter_at_starting_styled_block() {
    // This is a specific edge case when hitting Enter at the start of a block at the very
    // beginning of the buffer.

    App::test((), |mut app| async move {
        let (buffer, selection) = Buffer::mock_from_markdown(
            "# Initial heading\nText",
            None,
            Box::new(|_, _| IndentBehavior::Ignore),
            &mut app,
        );

        buffer.update(&mut app, |buffer, ctx| {
            assert_eq!(buffer.debug(), "<header1>Initial heading<text>Text");

            // Enter at the start of the buffer must create a new plain-text block.
            selection.update(ctx, |selection, _| {
                selection.set_single_cursor(CharOffset::from(1));
            });
            let prev_selection = buffer.to_rendered_selection_set(selection.clone(), ctx);
            let result = buffer.enter(false, Default::default(), selection.clone(), ctx);
            let current_selection = buffer.to_rendered_selection_set(selection.clone(), ctx);
            buffer.push_undo_item(
                prev_selection,
                current_selection,
                result.undo_item.expect("Should be undoable"),
                UndoActionType::Atomic,
            );

            let delta = result.delta.expect("Need edit delta to re-render");
            // In this case, there's no previous block to re-render. Instead, we just have the new one.
            assert_eq!(delta.old_offset, CharOffset::zero()..CharOffset::from(1));
            assert_eq!(
                delta.new_lines,
                vec![StyledBufferBlock::Text(StyledTextBlock {
                    block: vec![StyledBufferRun {
                        run: "\n".to_string(),
                        text_styles: Default::default(),
                        block_style: BufferBlockStyle::PlainText
                    }],
                    style: BufferBlockStyle::PlainText,
                    content_length: CharOffset::from(1)
                })]
            );

            assert_eq!(buffer.debug(), "<text><header1>Initial heading<text>Text");
            assert_eq!(
                selection.as_ref(ctx).selection_to_first_offset_range(),
                CharOffset::from(2)..CharOffset::from(2)
            );

            buffer.undo(selection.clone(), ctx);
            assert_eq!(buffer.debug(), "<header1>Initial heading<text>Text");

            buffer.redo(selection.clone(), ctx);
            assert_eq!(buffer.debug(), "<text><header1>Initial heading<text>Text");
        });
    });
}

#[test]
fn test_enter_at_starting_plain_text() {
    App::test((), |mut app| async move {
        let (buffer, selection) = Buffer::mock_from_markdown(
            "example",
            None,
            Box::new(|_, _| IndentBehavior::Ignore),
            &mut app,
        );

        buffer.update(&mut app, |buffer, ctx| {
            // Enter at the start of the first line.
            selection.update(ctx, |selection, _| {
                selection.set_single_cursor(CharOffset::from(1));
            });
            let prev_selection = buffer.to_rendered_selection_set(selection.clone(), ctx);
            let result = buffer.enter(false, Default::default(), selection.clone(), ctx);
            let current_selection = buffer.to_rendered_selection_set(selection.clone(), ctx);
            buffer.push_undo_item(
                prev_selection,
                current_selection,
                result.undo_item.unwrap(),
                UndoActionType::Atomic,
            );

            // Because we're editing an existing line, it should be re-rendered along with the new one.
            let delta = result.delta.unwrap();
            assert_eq!(delta.old_offset, CharOffset::from(1)..CharOffset::from(8));
            assert_eq!(
                delta.new_lines,
                vec![
                    StyledBufferBlock::Text(StyledTextBlock {
                        block: vec![StyledBufferRun {
                            run: "\n".to_string(),
                            text_styles: Default::default(),
                            block_style: BufferBlockStyle::PlainText
                        }],
                        style: BufferBlockStyle::PlainText,
                        content_length: CharOffset::from(1)
                    }),
                    StyledBufferBlock::Text(StyledTextBlock {
                        block: vec![StyledBufferRun {
                            run: "example".to_string(),
                            text_styles: Default::default(),
                            block_style: BufferBlockStyle::PlainText
                        }],
                        style: BufferBlockStyle::PlainText,
                        content_length: CharOffset::from(7)
                    })
                ]
            );

            assert_eq!(buffer.debug(), "<text>\\nexample");

            buffer.undo(selection.clone(), ctx);
            assert_eq!(buffer.debug(), "<text>example");

            buffer.redo(selection.clone(), ctx);
            assert_eq!(buffer.debug(), "<text>\\nexample");
        });
    });
}

#[test]
fn test_enter_at_start_of_empty_text() {
    App::test((), |mut app| async move {
        let (buffer, selection) = Buffer::mock_from_markdown(
            "* list\n",
            None,
            Box::new(|_, _| IndentBehavior::Ignore),
            &mut app,
        );

        buffer.update(&mut app, |buffer, ctx| {
            assert_eq!(buffer.debug(), "<ul0>list<text>");

            // Enter at the start of the empty plain-text block.
            selection.update(ctx, |selection, _| {
                selection.set_single_cursor(CharOffset::from(6));
            });
            let prev_selection = buffer.to_rendered_selection_set(selection.clone(), ctx);
            let result = buffer.enter(false, Default::default(), selection.clone(), ctx);
            let current_selection = buffer.to_rendered_selection_set(selection.clone(), ctx);
            buffer.push_undo_item(
                prev_selection,
                current_selection,
                result.undo_item.unwrap(),
                UndoActionType::Atomic,
            );

            assert_eq!(buffer.debug(), "<ul0>list<text>\\n");

            buffer.undo(selection.clone(), ctx);
            assert_eq!(buffer.debug(), "<ul0>list<text>");

            buffer.redo(selection.clone(), ctx);
            assert_eq!(buffer.debug(), "<ul0>list<text>\\n");
        });
    });
}

#[test]
fn test_enter_after_empty_block() {
    // This is a regression test for the issue described in https://github.com/warpdotdev/warp-internal/pull/6953#discussion_r1319189935.
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            buffer.edit_internal_first_selection(
                CharOffset::from(1)..CharOffset::from(1),
                "Text\nHeadingEnd",
                Default::default(),
                selection.clone(),
                ctx,
            );
            buffer.block_style_range(
                CharOffset::from(6)..CharOffset::from(13),
                BufferBlockStyle::Header {
                    header_size: BlockHeaderSize::Header1,
                },
                selection.clone(),
                ctx,
            );
            buffer.block_style_range(
                CharOffset::from(5)..CharOffset::from(6),
                BufferBlockStyle::Header {
                    header_size: BlockHeaderSize::Header1,
                },
                selection.clone(),
                ctx,
            );
            assert_eq!(
                buffer.debug(),
                "<text>Text<header1><header1>Heading<text>End"
            );

            selection.update(ctx, |selection, _| {
                selection.set_single_cursor(CharOffset::from(7));
            });
            buffer.enter(false, Default::default(), selection.clone(), ctx);
            assert_eq!(
                buffer.debug(),
                "<text>Text<header1><text><header1>Heading<text>End"
            );
        });
    });
}

#[test]
fn test_undo_enter_at_buffer_start() {
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            buffer.edit_internal_first_selection(
                1.into()..1.into(),
                "Start",
                Default::default(),
                selection.clone(),
                ctx,
            );
            buffer.block_style_range(
                1.into()..6.into(),
                BufferBlockStyle::Header {
                    header_size: BlockHeaderSize::Header1,
                },
                selection.clone(),
                ctx,
            );
            assert_eq!(buffer.debug(), "<header1>Start<text>");

            // Press Enter at the start of the header, which edits the start of the buffer.
            selection.update(ctx, |selection, _| {
                selection.set_single_cursor(1.into());
            });
            let prev_selection = buffer.to_rendered_selection_set(selection.clone(), ctx);
            let edit_result = buffer.enter(false, Default::default(), selection.clone(), ctx);
            let current_selection = buffer.to_rendered_selection_set(selection.clone(), ctx);
            buffer.push_undo_item(
                prev_selection,
                current_selection,
                edit_result.undo_item.unwrap(),
                UndoActionType::Atomic,
            );

            let delta = edit_result.delta.unwrap();
            assert_eq!(delta.old_offset, CharOffset::zero()..CharOffset::from(1));
            assert_eq!(
                delta.new_lines,
                vec![StyledBufferBlock::Text(StyledTextBlock {
                    block: vec![StyledBufferRun {
                        run: "\n".to_string(),
                        block_style: BufferBlockStyle::PlainText,
                        text_styles: Default::default()
                    }],
                    style: BufferBlockStyle::PlainText,
                    content_length: CharOffset::from(1)
                })]
            );
            assert_eq!(buffer.debug(), "<text><header1>Start<text>");

            // Now, undo it.
            let edit_result = buffer.undo(selection.clone(), ctx);
            let delta = edit_result.delta.unwrap();
            assert_eq!(delta.old_offset, CharOffset::zero()..CharOffset::from(2));
            // There should be no new lines, since they were deleted.
            assert_eq!(delta.new_lines, vec![]);
        });
    });
}
#[test]
fn test_newline_at_empty_list() {
    App::test((), |mut app| async move {
        let (buffer, selection) = Buffer::mock_from_markdown(
            "* First\n    * \n* Second",
            None,
            Box::new(|_, _| IndentBehavior::Ignore),
            &mut app,
        );

        buffer.update(&mut app, |buffer, ctx| {
            assert_eq!(buffer.debug(), "<ul0>First<ul1><ul0>Second<text>");

            // Forced newline at the start of the empty, indented list item.
            selection.update(ctx, |selection, _| {
                selection.set_single_cursor(7.into());
            });
            let prev_selection = buffer.to_rendered_selection_set(selection.clone(), ctx);
            let result = buffer.enter(true, Default::default(), selection.clone(), ctx);
            let current_selection = buffer.to_rendered_selection_set(selection.clone(), ctx);
            buffer.push_undo_item(
                prev_selection,
                current_selection,
                result.undo_item.unwrap(),
                UndoActionType::Atomic,
            );

            // This should insert an empty list item, NOT un-indent the existing item.
            assert_eq!(buffer.debug(), "<ul0>First<ul1><ul1><ul0>Second<text>");
            assert_eq!(selection.as_ref(ctx).first_selection_head(), 8.into());

            buffer.undo(selection.clone(), ctx);
            assert_eq!(buffer.debug(), "<ul0>First<ul1><ul0>Second<text>");
            assert_eq!(selection.as_ref(ctx).first_selection_head(), 7.into());

            buffer.redo(selection.clone(), ctx);
            assert_eq!(buffer.debug(), "<ul0>First<ul1><ul1><ul0>Second<text>");
            assert_eq!(selection.as_ref(ctx).first_selection_head(), 8.into());
        });
    });
}

#[test]
fn test_newline_at_code_block_start() {
    App::test((), |mut app| async move {
        let (buffer, selection) = Buffer::mock_from_markdown(
            "Test\n```\necho hello\n```\n",
            None,
            Box::new(|_, _| IndentBehavior::Ignore),
            &mut app,
        );

        buffer.update(&mut app, |buffer, ctx| {
            assert_eq!(buffer.debug(), "<text>Test<code:Shell>echo hello<text>");

            // Forced newline at the start of the code block.
            selection.update(ctx, |selection, _| {
                selection.set_single_cursor(6.into());
            });
            let prev_selection = buffer.to_rendered_selection_set(selection.clone(), ctx);
            let result = buffer.enter(true, Default::default(), selection.clone(), ctx);
            let current_selection = buffer.to_rendered_selection_set(selection.clone(), ctx);
            buffer.push_undo_item(
                prev_selection,
                current_selection,
                result.undo_item.unwrap(),
                UndoActionType::Atomic,
            );

            // This should insert a new line within the code block.
            assert_eq!(buffer.debug(), "<text>Test<code:Shell>\\necho hello<text>");
            assert_eq!(selection.as_ref(ctx).first_selection_head(), 7.into());

            buffer.undo(selection.clone(), ctx);
            assert_eq!(buffer.debug(), "<text>Test<code:Shell>echo hello<text>");
            assert_eq!(selection.as_ref(ctx).first_selection_head(), 6.into());

            buffer.redo(selection.clone(), ctx);
            assert_eq!(buffer.debug(), "<text>Test<code:Shell>\\necho hello<text>");
            assert_eq!(selection.as_ref(ctx).first_selection_head(), 7.into());
        });
    });
}

#[test]
fn test_insert_html() {
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            buffer.edit_internal_first_selection(
                CharOffset::from(1)..CharOffset::from(1),
                "Before\nBlock\nAfter",
                Default::default(),
                selection.clone(),
                ctx,
            );
            buffer.block_style_range(
                CharOffset::from(8)..CharOffset::from(13),
                BufferBlockStyle::CodeBlock {
                    code_block_type: Default::default(),
                },
                selection.clone(),
                ctx,
            );

            let formatted_text = parse_html("<p><strong>test text</strong></p>").expect("Should parse");
            buffer.replace_with_formatted_text(CharOffset::from(7)..CharOffset::from(7), formatted_text, EditOrigin::UserInitiated, selection.clone(), ctx);
            assert_eq!(
                buffer.content.debug(),
                "<text>Before<b_s>test text<b_e><code:Shell>Block<text>After"
            );

            let formatted_text =
                parse_html("<pre><code:Shell>block 1\nblock 2</code></pre> other <pre>block 3</pre>")
                    .expect("Should parse");
            buffer.replace_with_formatted_text(CharOffset::from(1)..CharOffset::from(16), formatted_text, EditOrigin::UserInitiated, selection.clone(), ctx);
            assert_eq!(
                buffer.content.debug(),
                "<code:Shell>block 1\\nblock 2<text> other <code:Shell>block 3<code:Shell>Block<text>After"
            );

            // Inserting block style into an already active block with the same style should be the same as inserting plain text.
            let formatted_text = parse_html("<pre>block content</pre>").expect("Should parse");
            buffer.replace_with_formatted_text(CharOffset::from(1)..CharOffset::from(1), formatted_text, EditOrigin::UserInitiated, selection.clone(), ctx);
            assert_eq!(
                buffer.content.debug(),
                "<code:Shell>block contentblock 1\\nblock 2<text> other <code:Shell>block 3<code:Shell>Block<text>After"
            );

            let formatted_text = parse_html("<ul><li>abc</li><li>def</li></ul>").expect("Should parse");
            buffer.replace_with_formatted_text(CharOffset::from(3)..CharOffset::from(3), formatted_text, EditOrigin::UserInitiated, selection.clone(), ctx);
            assert_eq!(
                buffer.content.debug(),
                "<code:Shell>blabc<ul0>def<code:Shell>ock contentblock 1\\nblock 2<text> other <code:Shell>block 3<code:Shell>Block<text>After"
            );
        });
    });
}

#[test]
fn test_insert_code_block_with_language() {
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            buffer.edit_internal_first_selection(
                CharOffset::from(1)..CharOffset::from(1),
                "Before\n",
                Default::default(),
                selection.clone(),
                ctx,
            );

            let formatted_text =
                parse_html("<pre><code class=\"language-jsx\">Some code</code></pre>")
                    .expect("Should parse");
            buffer.replace_with_formatted_text(
                CharOffset::from(8)..CharOffset::from(8),
                formatted_text,
                EditOrigin::UserInitiated,
                selection.clone(),
                ctx,
            );
            assert_eq!(
                buffer.content.debug(),
                "<text>Before<code:JavaScript>Some code<text>"
            );
        });
    });
}

#[test]
fn test_insert_formatted_text_empty_buffer() {
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            let formatted_text = parse_markdown("\n```\nblock\n```\n").expect("Should parse");
            let delta = buffer
                .replace_with_formatted_text(
                    CharOffset::from(0)..CharOffset::from(1),
                    formatted_text.clone(),
                    EditOrigin::UserInitiated,
                    selection.clone(),
                    ctx,
                )
                .delta
                .expect("Should exist");

            assert_eq!(buffer.content.debug(), "<text><code:Shell>block<text>");
            assert_eq!(delta.old_offset, CharOffset::from(0)..CharOffset::from(1));
            assert_eq!(
                delta.new_lines,
                vec![
                    StyledBufferBlock::Text(StyledTextBlock {
                        block: vec![StyledBufferRun {
                            run: "\n".into(),
                            text_styles: Default::default(),
                            block_style: BufferBlockStyle::PlainText
                        },],
                        style: BufferBlockStyle::PlainText,
                        content_length: CharOffset::from(1)
                    }),
                    StyledBufferBlock::Text(StyledTextBlock {
                        block: vec![StyledBufferRun {
                            run: "block\n".into(),
                            text_styles: Default::default(),
                            block_style: BufferBlockStyle::CodeBlock {
                                code_block_type: Default::default()
                            }
                        },],
                        style: BufferBlockStyle::CodeBlock {
                            code_block_type: Default::default(),
                        },
                        content_length: CharOffset::from(6)
                    }),
                ]
            );
        });
    });
}

#[test]
fn test_insert_code_block_in_text_lines() {
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            buffer.edit_internal_first_selection(
                CharOffset::from(1)..CharOffset::from(1),
                "Before\nLine\nAfter\nSecond\n",
                Default::default(),
                selection.clone(),
                ctx,
            );

            let formatted_text = parse_html("<pre>block</pre>").expect("Should parse");
            // Inserting code block in between plain text line should split it in half.
            let delta = buffer
                .replace_with_formatted_text(
                    CharOffset::from(3)..CharOffset::from(3),
                    formatted_text.clone(),
                    EditOrigin::UserInitiated,
                    selection.clone(),
                    ctx,
                )
                .delta
                .expect("Should exist");
            assert_eq!(
                buffer.content.debug(),
                "<text>Beblockfore\\nLine\\nAfter\\nSecond\\n"
            );
            assert_eq!(delta.old_offset, CharOffset::from(1)..CharOffset::from(8));
            assert_eq!(
                delta.new_lines,
                vec![StyledBufferBlock::Text(StyledTextBlock {
                    block: vec![StyledBufferRun {
                        run: "Beblockfore\n".into(),
                        text_styles: Default::default(),
                        block_style: BufferBlockStyle::PlainText
                    },],
                    style: BufferBlockStyle::PlainText,
                    content_length: CharOffset::from(12)
                }),]
            );

            // Inserting code block before a new plain text line should replace the starting block marker.
            let delta = buffer
                .replace_with_formatted_text(
                    CharOffset::from(13)..CharOffset::from(17),
                    formatted_text.clone(),
                    EditOrigin::UserInitiated,
                    selection.clone(),
                    ctx,
                )
                .delta
                .expect("Should exist");
            assert_eq!(
                buffer.content.debug(),
                "<text>Beblockfore<code:Shell>block<text>After\\nSecond\\n"
            );
            assert_eq!(delta.old_offset, CharOffset::from(13)..CharOffset::from(18));
            assert_eq!(
                delta.new_lines,
                vec![StyledBufferBlock::Text(StyledTextBlock {
                    block: vec![StyledBufferRun {
                        run: "block\n".into(),
                        text_styles: Default::default(),
                        block_style: BufferBlockStyle::CodeBlock {
                            code_block_type: Default::default()
                        }
                    },],
                    style: BufferBlockStyle::CodeBlock {
                        code_block_type: Default::default(),
                    },
                    content_length: CharOffset::from(6)
                }),]
            );

            // Inserting code block at the end of the buffer should add a trailing newline.
            let delta = buffer
                .replace_with_formatted_text(
                    CharOffset::from(32)..CharOffset::from(32),
                    formatted_text,
                    EditOrigin::UserInitiated,
                    selection.clone(),
                    ctx,
                )
                .delta
                .expect("Should exist");
            assert_eq!(
                buffer.content.debug(),
                "<text>Beblockfore<code:Shell>block<text>After\\nSecond<code:Shell>block<text>"
            );
            assert_eq!(delta.old_offset, CharOffset::from(32)..CharOffset::from(32));
            assert_eq!(
                delta.new_lines,
                vec![StyledBufferBlock::Text(StyledTextBlock {
                    block: vec![StyledBufferRun {
                        run: "block\n".into(),
                        text_styles: Default::default(),
                        block_style: BufferBlockStyle::CodeBlock {
                            code_block_type: Default::default()
                        }
                    },],
                    style: BufferBlockStyle::CodeBlock {
                        code_block_type: Default::default(),
                    },
                    content_length: CharOffset::from(6)
                }),]
            );
        });
    });
}

#[test]
fn test_placeholder_insertion() {
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            buffer.edit_internal_first_selection(
                CharOffset::from(1)..CharOffset::from(1),
                "sometext",
                Default::default(),
                selection.clone(),
                ctx,
            );

            selection.update(ctx, |selection, _| {
                selection.set_single_cursor(5.into());
            });
            let delta = buffer
                .insert_placeholder(5.into(), "placeholder", selection.clone(), ctx)
                .delta
                .expect("Should exist");
            assert_eq!(
                buffer.content.debug(),
                "<text>some<placeholder_s>placeholder<placeholder_e>text"
            );
            // Inserting a placeholder should re-render the line.
            assert_eq!(delta.old_offset, CharOffset::from(1)..CharOffset::from(9));
            assert_eq!(
                delta.new_lines,
                vec![StyledBufferBlock::Text(StyledTextBlock {
                    block: vec![
                        StyledBufferRun {
                            run: "some".into(),
                            text_styles: Default::default(),
                            block_style: BufferBlockStyle::PlainText
                        },
                        StyledBufferRun {
                            run: "placeholder".into(),
                            text_styles: TextStylesWithMetadata::default().for_placeholder(),
                            block_style: BufferBlockStyle::PlainText
                        },
                        StyledBufferRun {
                            run: "text".into(),
                            text_styles: Default::default(),
                            block_style: BufferBlockStyle::PlainText
                        },
                    ],
                    style: BufferBlockStyle::PlainText,
                    content_length: CharOffset::from(9)
                })]
            );

            // The placeholder was inserted at the cursor, so it should move back.
            assert_eq!(
                selection.as_ref(ctx).selection_to_first_offset_range(),
                CharOffset::from(6)..CharOffset::from(6)
            );
        });
    });
}

#[test]
fn test_placeholder_inherits_styles() {
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            buffer.edit_internal_first_selection(
                CharOffset::from(1)..CharOffset::from(1),
                "sometext",
                Default::default(),
                selection.clone(),
                ctx,
            );
            buffer.insert_placeholder(5.into(), "placeholder", selection.clone(), ctx);
            buffer.set_selection(
                CharOffset::from(3)..CharOffset::from(8),
                selection.clone(),
                ctx,
            );
            let delta = buffer
                .style_internal(TextStyles::default().bold(), selection.clone(), ctx)
                .delta
                .expect("Should exist");
            assert_eq!(
                buffer.content.debug(),
                "<text>so<b_s>me<placeholder_s>placeholder<placeholder_e>te<b_e>xt"
            );

            assert_eq!(
                delta.new_lines,
                vec![StyledBufferBlock::Text(StyledTextBlock {
                    block: vec![
                        StyledBufferRun {
                            run: "so".into(),
                            text_styles: Default::default(),
                            block_style: BufferBlockStyle::PlainText
                        },
                        StyledBufferRun {
                            run: "me".into(),
                            text_styles: TextStylesWithMetadata::default().bold(),
                            block_style: BufferBlockStyle::PlainText
                        },
                        StyledBufferRun {
                            run: "placeholder".into(),
                            text_styles: TextStylesWithMetadata::default().bold().for_placeholder(),
                            block_style: BufferBlockStyle::PlainText
                        },
                        StyledBufferRun {
                            run: "te".into(),
                            text_styles: TextStylesWithMetadata::default().bold(),
                            block_style: BufferBlockStyle::PlainText
                        },
                        StyledBufferRun {
                            run: "xt".into(),
                            text_styles: Default::default(),
                            block_style: BufferBlockStyle::PlainText
                        }
                    ],
                    style: BufferBlockStyle::PlainText,
                    content_length: CharOffset::from(9)
                })]
            );
        });
    });
}

#[test]
fn test_read_html() {
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            buffer.edit_internal_first_selection(
                CharOffset::from(1)..CharOffset::from(1),
                "Before\nBlock\nAfter",
                Default::default(),
                selection.clone(),
                ctx,
            );
            buffer.block_style_range(
                CharOffset::from(8)..CharOffset::from(13),
                BufferBlockStyle::CodeBlock {
                    code_block_type: Default::default(),
                },
                selection.clone(),
                ctx,
            );
            buffer.set_selection(CharOffset::from(3)..CharOffset::from(7), selection.clone(), ctx);
            buffer.style_internal(TextStyles::default().bold(), selection.clone(), ctx);
            assert_eq!(
                buffer.content.debug(),
                "<text>Be<b_s>fore<b_e><code:Shell>Block<text>After"
            );

            buffer.set_selection(CharOffset::from(3)..CharOffset::from(5), selection.clone(), ctx);
            assert_eq!(
                buffer.selected_text_as_html(selection.clone(), ctx),
                Some("<p><strong>fo</strong></p>".to_string())
            );

            buffer.set_selection(CharOffset::from(8)..CharOffset::from(11), selection.clone(), ctx);
            assert_eq!(
                buffer.selected_text_as_html(selection.clone(), ctx),
                Some(
                    "<pre><code class=\"language-warp-runnable-command\">Blo</code></pre>".to_string()
                )
            );

            buffer.set_selection(CharOffset::from(4)..CharOffset::from(11), selection.clone(), ctx);
            assert_eq!(
                buffer.selected_text_as_html(selection.clone(), ctx),
                Some("<p><strong>ore</strong></p><pre><code class=\"language-warp-runnable-command\">Blo</code></pre>".to_string())
            );

            buffer.set_selection(CharOffset::from(11)..CharOffset::from(16), selection.clone(), ctx);
            assert_eq!(
                buffer.selected_text_as_html(selection.clone(), ctx),
                Some(
                    "<pre><code class=\"language-warp-runnable-command\">ck</code></pre><p>Af</p>"
                        .to_string()
                )
            );

            buffer.block_style_range(
                CharOffset::from(14)..CharOffset::from(19),
                BufferBlockStyle::Header {
                    header_size: BlockHeaderSize::Header1,
                },
                selection.clone(),
                ctx,
            );

            buffer.set_selection(CharOffset::from(15)..CharOffset::from(16), selection.clone(), ctx);
            assert_eq!(
                buffer.selected_text_as_html(selection.clone(), ctx),
                Some("<h1>f</h1>".to_string())
            );

            buffer.set_selection(CharOffset::from(11)..CharOffset::from(19), selection.clone(), ctx);
            assert_eq!(
                buffer.selected_text_as_html(selection.clone(), ctx),
                Some(
                    "<pre><code class=\"language-warp-runnable-command\">ck</code></pre><h1>After</h1>"
                        .to_string()
                )
            );

            buffer.block_style_range(
                CharOffset::from(14)..CharOffset::from(19),
                BufferBlockStyle::UnorderedList {
                    indent_level: ListIndentLevel::One,
                },
                selection.clone(),
                ctx,
            );
            buffer.set_selection(CharOffset::from(14)..CharOffset::from(19), selection.clone(), ctx);
            assert_eq!(
                buffer.selected_text_as_html(selection.clone(), ctx),
                Some("<ul><li>After</li></ul>".to_string())
            );
        });
    });
}

#[test]
fn test_read_html_nested_list() {
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            buffer.edit_internal_first_selection(
                CharOffset::from(1)..CharOffset::from(1),
                "Before\nBlock\nAfter\nLine",
                Default::default(),
                selection.clone(),
                ctx,
            );

            buffer.block_style_range(
                CharOffset::from(1)..CharOffset::from(7),
                BufferBlockStyle::UnorderedList {
                    indent_level: ListIndentLevel::One,
                },
                selection.clone(),
                ctx,
            );

            buffer.block_style_range(
                CharOffset::from(8)..CharOffset::from(13),
                BufferBlockStyle::UnorderedList {
                    indent_level: ListIndentLevel::Two,
                },
                selection.clone(),
                ctx,
            );

            buffer.block_style_range(
                CharOffset::from(14)..CharOffset::from(19),
                BufferBlockStyle::UnorderedList {
                    indent_level: ListIndentLevel::Three,
                },
                selection.clone(),
                ctx,
            );

            buffer.block_style_range(
                CharOffset::from(20)..CharOffset::from(24),
                BufferBlockStyle::UnorderedList {
                    indent_level: ListIndentLevel::One,
                },
                selection.clone(),
                ctx,
            );

            assert_eq!(
                buffer.content.debug(),
                "<ul0>Before<ul1>Block<ul2>After<ul0>Line<text>"
            );

            buffer.set_selection(CharOffset::from(1)..CharOffset::from(24), selection.clone(), ctx);
            assert_eq!(
                buffer.selected_text_as_html(selection.clone(), ctx),
                Some(
                    "<ul><li>Before<ul><li>Block<ul><li>After</li></ul></li></ul></li><li>Line</li></ul>"
                        .to_string()
                )
            );
        });
    });
}

#[test]
fn test_copy_partial_list() {
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            // If part of a list is copied, make sure we still close it.
            buffer.edit_internal_first_selection(
                CharOffset::from(1)..CharOffset::from(1),
                "a\nb\ncone\ntwo\nthree",
                Default::default(),
                selection.clone(),
                ctx,
            );
            buffer.block_style_range(
                CharOffset::from(1)..CharOffset::from(6),
                BufferBlockStyle::UnorderedList {
                    indent_level: ListIndentLevel::One,
                },
                selection.clone(),
                ctx,
            );
            buffer.block_style_range(
                CharOffset::from(7)..CharOffset::from(20),
                BufferBlockStyle::ordered_list(ListIndentLevel::One),
                selection.clone(),
                ctx,
            );
            assert_eq!(
                buffer.debug(),
                "<ul0>a<ul0>b<ul0>c<ol0>one<ol0>two<ol0>three<text>"
            );

            assert_eq!(
                buffer
                    .range_as_html(CharOffset::from(3)..CharOffset::from(6), ctx)
                    .expect("Can serialize to HTML"),
                "<ul><li>b</li><li>c</li></ul>"
            );

            assert_eq!(
                buffer
                    .range_as_html(CharOffset::from(11)..CharOffset::from(14), ctx)
                    .expect("Can serialize HTML"),
                "<ol><li>two</li></ol>"
            );
        });
    });
}

#[test]
fn test_read_html_nested_task_list() {
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            buffer.edit_internal_first_selection(
                CharOffset::from(1)..CharOffset::from(1),
                "Before\nBlock\nAfter\nLine",
                Default::default(),
                selection.clone(),
                ctx,
            );

            buffer.block_style_range(
                CharOffset::from(1)..CharOffset::from(7),
                BufferBlockStyle::TaskList {
                    indent_level: ListIndentLevel::One,
                    complete: true,
                },
                selection.clone(),
                ctx,
            );

            buffer.block_style_range(
                CharOffset::from(8)..CharOffset::from(13),
                BufferBlockStyle::TaskList {
                    indent_level: ListIndentLevel::Two,
                    complete: false,
                },
                selection.clone(),
                ctx,
            );

            buffer.block_style_range(
                CharOffset::from(14)..CharOffset::from(19),
                BufferBlockStyle::TaskList {
                    indent_level: ListIndentLevel::Three,
                    complete: true,
                },
                selection.clone(),
                ctx,
            );

            buffer.block_style_range(
                CharOffset::from(20)..CharOffset::from(24),
                BufferBlockStyle::TaskList {
                    indent_level: ListIndentLevel::One,
                    complete: false,
                },
                selection.clone(),
                ctx,
            );

            assert_eq!(
                buffer.content.debug(),
                "<cl0:true>Before<cl1:false>Block<cl2:true>After<cl0:false>Line<text>"
            );

            buffer.set_selection(CharOffset::from(1)..CharOffset::from(24), selection.clone(), ctx);
            assert_eq!(
                buffer.selected_text_as_html(selection.clone(), ctx),
                Some(
                    "<ul><li><input type=\"checkbox\" checked=\"\">Before<ul><li><input type=\"checkbox\">Block<ul><li><input type=\"checkbox\" checked=\"\">After</li></ul></li></ul></li><li><input type=\"checkbox\">Line</li></ul>"
                        .to_string()
                )
            );
        });
    });
}

#[test]
fn test_adjacent_lists_as_html() {
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            buffer.edit_internal_first_selection(
                CharOffset::from(1)..CharOffset::from(1),
                "Start\nOne\nTwo\nThree\nFour\nEnd",
                Default::default(),
                selection.clone(),
                ctx,
            );
            buffer.block_style_range(
                CharOffset::from(7)..CharOffset::from(14),
                BufferBlockStyle::ordered_list(ListIndentLevel::One),
                selection.clone(),
                ctx,
            );
            buffer.block_style_range(
                CharOffset::from(15)..CharOffset::from(25),
                BufferBlockStyle::UnorderedList {
                    indent_level: ListIndentLevel::One,
                },
                selection.clone(),
                ctx,
            );
            assert_eq!(
                buffer.debug(),
                "<text>Start<ol0>One<ol0>Two<ul0>Three<ul0>Four<text>End"
            );

            // This test makes sure that we start and end ul/ol elements appropriately.
            buffer.set_selection(CharOffset::zero()..buffer.max_charoffset(), selection.clone(), ctx);
            assert_eq!(
                buffer
                    .selected_text_as_html(selection.clone(), ctx)
                    .expect("Should convert to HTML"),
                "<p>Start</p><ol><li>One</li><li>Two</li></ol><ul><li>Three</li><li>Four</li></ul><p>End</p>"
            );
        });
    });
}

#[test]
fn test_embedded_item_as_html() {
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| {
            Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)).with_embedded_item_conversion(
                |mut mapping| match mapping.remove(&Value::String("id".to_string())) {
                    Some(Value::String(hashed_id)) => {
                        Some(Arc::new(TestEmbeddedItem { id: hashed_id }))
                    }
                    _ => None,
                },
            )
        });
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            buffer.edit_internal_first_selection(
                CharOffset::from(1)..CharOffset::from(1),
                "Before\nAfter",
                Default::default(),
                selection.clone(),
                ctx,
            );

            buffer.replace_with_formatted_text(
                CharOffset::from(7)..CharOffset::from(7),
                FormattedText::new(vec![FormattedTextLine::Embedded(Mapping::from_iter([(
                    Value::String("id".to_string()),
                    Value::String("workflow-123".to_string()),
                )]))]),
                EditOrigin::UserInitiated,
                selection.clone(),
                ctx,
            );
            buffer.replace_with_formatted_text(
                CharOffset::from(8)..CharOffset::from(8),
                FormattedText::new(vec![FormattedTextLine::Embedded(Mapping::from_iter([(
                    Value::String("id".to_string()),
                    Value::String("workflow-234".to_string()),
                )]))]),
                EditOrigin::UserInitiated,
                selection.clone(),
                ctx,
            );

            assert_eq!(
                buffer.content.debug(),
                "<text>Before<embed_workflow-123><embed_workflow-234><text>After"
            );
            buffer.set_selection(
                CharOffset::from(1)..buffer.max_charoffset(),
                selection.clone(),
                ctx,
            );
            assert_eq!(
                buffer
                    .selected_text_as_html(selection.clone(), ctx)
                    .expect("Should convert to HTML"),
                "<p>Before</p><pre>workflow-123</pre><pre>workflow-234</pre><p>After</p>"
            );

            assert_eq!(
                buffer.text_in_ranges_with_expanded_embedded_items(
                    vec1![CharOffset::from(1)..buffer.max_charoffset()],
                    ctx
                ),
                "Before\n```\nworkflow-123\n```\n```\nworkflow-234\n```\nAfter"
            );
        });
    });
}

#[test]
fn test_undo_redo_plain_text() {
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            let prev_selection = buffer.to_rendered_selection_set(selection.clone(), ctx);
            let edit_result = buffer.edit_internal(
                "test\n\nline",
                TextStyles::default(),
                selection.clone(),
                ctx,
            );
            let curr_selection = buffer.to_rendered_selection_set(selection.clone(), ctx);
            buffer.push_undo_item(
                prev_selection,
                curr_selection,
                edit_result.undo_item.expect("Should exist"),
                UndoActionType::Atomic,
            );
            let delta = buffer
                .undo(selection.clone(), ctx)
                .delta
                .expect("Edit delta should exist");
            assert_eq!(buffer.content.debug(), "<text>");
            assert_eq!(delta.old_offset, CharOffset::from(1)..CharOffset::from(11));
            assert_eq!(delta.new_lines, vec![]);

            let delta = buffer
                .redo(selection.clone(), ctx)
                .delta
                .expect("Edit delta should exist");
            assert_eq!(buffer.content.debug(), "<text>test\\n\\nline");
            assert_eq!(delta.old_offset, CharOffset::from(1)..CharOffset::from(1));
            assert_eq!(
                delta.new_lines,
                vec![
                    StyledBufferBlock::Text(StyledTextBlock {
                        block: vec![StyledBufferRun {
                            run: "test\n".into(),
                            text_styles: TextStylesWithMetadata::default(),
                            block_style: BufferBlockStyle::PlainText
                        }],
                        style: BufferBlockStyle::PlainText,
                        content_length: CharOffset::from(5)
                    }),
                    StyledBufferBlock::Text(StyledTextBlock {
                        block: vec![StyledBufferRun {
                            run: "\n".into(),
                            text_styles: TextStylesWithMetadata::default(),
                            block_style: BufferBlockStyle::PlainText
                        }],
                        style: BufferBlockStyle::PlainText,
                        content_length: CharOffset::from(1)
                    }),
                    StyledBufferBlock::Text(StyledTextBlock {
                        block: vec![StyledBufferRun {
                            run: "line".into(),
                            text_styles: TextStylesWithMetadata::default(),
                            block_style: BufferBlockStyle::PlainText
                        }],
                        style: BufferBlockStyle::PlainText,
                        content_length: CharOffset::from(4)
                    })
                ]
            );

            buffer.set_selection(
                CharOffset::from(3)..CharOffset::from(7),
                selection.clone(),
                ctx,
            );
            let prev_selection = buffer.to_rendered_selection_set(selection.clone(), ctx);
            let edit_result =
                buffer.edit_internal("", TextStyles::default(), selection.clone(), ctx);
            assert_eq!(buffer.content.debug(), "<text>teline");
            let curr_selection = buffer.to_rendered_selection_set(selection.clone(), ctx);
            buffer.push_undo_item(
                prev_selection,
                curr_selection,
                edit_result.undo_item.expect("Should exist"),
                UndoActionType::Atomic,
            );

            let delta = buffer
                .undo(selection.clone(), ctx)
                .delta
                .expect("Edit delta should exist");
            assert_eq!(buffer.content.debug(), "<text>test\\n\\nline");
            assert_eq!(delta.old_offset, CharOffset::from(1)..CharOffset::from(7));
            assert_eq!(
                delta.new_lines,
                vec![
                    StyledBufferBlock::Text(StyledTextBlock {
                        block: vec![StyledBufferRun {
                            run: "test\n".into(),
                            text_styles: TextStylesWithMetadata::default(),
                            block_style: BufferBlockStyle::PlainText
                        }],
                        style: BufferBlockStyle::PlainText,
                        content_length: CharOffset::from(5)
                    }),
                    StyledBufferBlock::Text(StyledTextBlock {
                        block: vec![StyledBufferRun {
                            run: "\n".into(),
                            text_styles: TextStylesWithMetadata::default(),
                            block_style: BufferBlockStyle::PlainText
                        }],
                        style: BufferBlockStyle::PlainText,
                        content_length: CharOffset::from(1)
                    }),
                    StyledBufferBlock::Text(StyledTextBlock {
                        block: vec![StyledBufferRun {
                            run: "line".into(),
                            text_styles: TextStylesWithMetadata::default(),
                            block_style: BufferBlockStyle::PlainText
                        }],
                        style: BufferBlockStyle::PlainText,
                        content_length: CharOffset::from(4)
                    })
                ]
            );

            let _ = buffer.undo(selection.clone(), ctx);
            assert_eq!(buffer.content.debug(), "<text>");

            let _ = buffer.redo(selection.clone(), ctx);
            assert_eq!(buffer.content.debug(), "<text>test\\n\\nline");

            let _ = buffer.redo(selection.clone(), ctx);
            assert_eq!(buffer.content.debug(), "<text>teline");
        });
    });
}

#[test]
fn test_undo_redo_block() {
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            let prev_selection = buffer.to_rendered_selection_set(selection.clone(), ctx);
            let edit_result = buffer.edit_internal_first_selection(
                CharOffset::from(1)..CharOffset::from(1),
                "test\n\nblock",
                TextStyles::default(),
                selection.clone(),
                ctx,
            );
            let curr_selection = buffer.to_rendered_selection_set(selection.clone(), ctx);
            buffer.push_undo_item(
                prev_selection,
                curr_selection,
                edit_result.undo_item.expect("Should exist"),
                UndoActionType::Atomic,
            );

            let prev_selection = buffer.to_rendered_selection_set(selection.clone(), ctx);
            let edit_result = buffer.block_style_range(
                CharOffset::from(7)..CharOffset::from(9),
                BufferBlockStyle::CodeBlock {
                    code_block_type: Default::default(),
                },
                selection.clone(),
                ctx,
            );
            let curr_selection = buffer.to_rendered_selection_set(selection.clone(), ctx);
            buffer.push_undo_item(
                prev_selection,
                curr_selection,
                edit_result.undo_item.expect("Should exist"),
                UndoActionType::Atomic,
            );
            assert_eq!(
                buffer.content.debug(),
                "<text>test\\n<code:Shell>bl<text>ock"
            );

            let delta = buffer
                .undo(selection.clone(), ctx)
                .delta
                .expect("Edit delta should exist");
            assert_eq!(buffer.content.debug(), "<text>test\\n\\nblock");
            assert_eq!(delta.old_offset, CharOffset::from(7)..CharOffset::from(13));
            assert_eq!(
                delta.new_lines,
                vec![StyledBufferBlock::Text(StyledTextBlock {
                    block: vec![StyledBufferRun {
                        run: "block".into(),
                        text_styles: TextStylesWithMetadata::default(),
                        block_style: BufferBlockStyle::PlainText
                    }],
                    style: BufferBlockStyle::PlainText,
                    content_length: CharOffset::from(5)
                }),]
            );

            let delta = buffer
                .redo(selection.clone(), ctx)
                .delta
                .expect("Edit delta should exist");
            assert_eq!(
                buffer.content.debug(),
                "<text>test\\n<code:Shell>bl<text>ock"
            );
            assert_eq!(delta.old_offset, CharOffset::from(7)..CharOffset::from(12));
            assert_eq!(
                delta.new_lines,
                vec![
                    StyledBufferBlock::Text(StyledTextBlock {
                        block: vec![StyledBufferRun {
                            run: "bl\n".into(),
                            text_styles: TextStylesWithMetadata::default(),
                            block_style: BufferBlockStyle::CodeBlock {
                                code_block_type: Default::default()
                            }
                        }],
                        style: BufferBlockStyle::CodeBlock {
                            code_block_type: Default::default(),
                        },
                        content_length: CharOffset::from(3)
                    }),
                    StyledBufferBlock::Text(StyledTextBlock {
                        block: vec![StyledBufferRun {
                            run: "ock".into(),
                            text_styles: TextStylesWithMetadata::default(),
                            block_style: BufferBlockStyle::PlainText
                        }],
                        style: BufferBlockStyle::PlainText,
                        content_length: CharOffset::from(3)
                    }),
                ]
            );

            let prev_selection = buffer.to_rendered_selection_set(selection.clone(), ctx);
            let edit_result = buffer.edit_internal_first_selection(
                CharOffset::from(6)..CharOffset::from(7),
                "",
                TextStyles::default(),
                selection.clone(),
                ctx,
            );
            let curr_selection = buffer.to_rendered_selection_set(selection.clone(), ctx);
            buffer.push_undo_item(
                prev_selection,
                curr_selection,
                edit_result.undo_item.expect("Should exist"),
                UndoActionType::Atomic,
            );
            assert_eq!(buffer.content.debug(), "<text>test\\nbl\\nock");

            let delta = buffer
                .undo(selection.clone(), ctx)
                .delta
                .expect("Edit delta should exist");
            assert_eq!(
                buffer.content.debug(),
                "<text>test\\n<code:Shell>bl<text>ock"
            );
            assert_eq!(delta.old_offset, CharOffset::from(6)..CharOffset::from(9));
            assert_eq!(
                delta.new_lines,
                vec![
                    StyledBufferBlock::Text(StyledTextBlock {
                        block: vec![StyledBufferRun {
                            run: "\n".into(),
                            text_styles: TextStylesWithMetadata::default(),
                            block_style: BufferBlockStyle::PlainText
                        }],
                        style: BufferBlockStyle::PlainText,
                        content_length: CharOffset::from(1)
                    }),
                    StyledBufferBlock::Text(StyledTextBlock {
                        block: vec![StyledBufferRun {
                            run: "bl\n".into(),
                            text_styles: TextStylesWithMetadata::default(),
                            block_style: BufferBlockStyle::CodeBlock {
                                code_block_type: Default::default()
                            }
                        }],
                        style: BufferBlockStyle::CodeBlock {
                            code_block_type: Default::default(),
                        },
                        content_length: CharOffset::from(3)
                    }),
                ]
            );

            let _ = buffer
                .redo(selection.clone(), ctx)
                .delta
                .expect("Edit delta should exist");
            assert_eq!(buffer.content.debug(), "<text>test\\nbl\\nock");
        });
    });
}

#[test]
fn test_undo_redo_multi_block_deletion() {
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            let prev_selection = buffer.to_rendered_selection_set(selection.clone(), ctx);
            let edit_result = buffer.edit_internal_first_selection(
                CharOffset::from(1)..CharOffset::from(1),
                "test\nline\nblock\nafter",
                TextStyles::default(),
                selection.clone(),
                ctx,
            );
            let curr_selection = buffer.to_rendered_selection_set(selection.clone(), ctx);
            buffer.push_undo_item(
                prev_selection,
                curr_selection,
                edit_result.undo_item.expect("Should exist"),
                UndoActionType::Atomic,
            );

            let prev_selection = buffer.to_rendered_selection_set(selection.clone(), ctx);
            let edit_result = buffer.block_style_range(
                CharOffset::from(6)..CharOffset::from(10),
                BufferBlockStyle::CodeBlock {
                    code_block_type: Default::default(),
                },
                selection.clone(),
                ctx,
            );
            let curr_selection = buffer.to_rendered_selection_set(selection.clone(), ctx);
            buffer.push_undo_item(
                prev_selection,
                curr_selection,
                edit_result.undo_item.expect("Should exist"),
                UndoActionType::Atomic,
            );
            assert_eq!(
                buffer.content.debug(),
                "<text>test<code:Shell>line<text>block\\nafter"
            );

            let prev_selection = buffer.to_rendered_selection_set(selection.clone(), ctx);
            let edit_result = buffer.edit_internal_first_selection(
                CharOffset::from(8)..CharOffset::from(13),
                "",
                TextStyles::default(),
                selection.clone(),
                ctx,
            );
            let curr_selection = buffer.to_rendered_selection_set(selection.clone(), ctx);
            buffer.push_undo_item(
                prev_selection,
                curr_selection,
                edit_result.undo_item.expect("Should exist"),
                UndoActionType::Atomic,
            );
            assert_eq!(
                buffer.content.debug(),
                "<text>test<code:Shell>liock<text>after"
            );

            let delta = buffer
                .undo(selection.clone(), ctx)
                .delta
                .expect("Edit delta should exist");
            assert_eq!(
                buffer.content.debug(),
                "<text>test<code:Shell>line<text>block\\nafter"
            );
            assert_eq!(delta.old_offset, CharOffset::from(6)..CharOffset::from(12));
            assert_eq!(
                delta.new_lines,
                vec![
                    StyledBufferBlock::Text(StyledTextBlock {
                        block: vec![StyledBufferRun {
                            run: "line\n".into(),
                            text_styles: TextStylesWithMetadata::default(),
                            block_style: BufferBlockStyle::CodeBlock {
                                code_block_type: Default::default()
                            }
                        }],
                        style: BufferBlockStyle::CodeBlock {
                            code_block_type: Default::default(),
                        },
                        content_length: CharOffset::from(5)
                    }),
                    StyledBufferBlock::Text(StyledTextBlock {
                        block: vec![StyledBufferRun {
                            run: "block\n".into(),
                            text_styles: TextStylesWithMetadata::default(),
                            block_style: BufferBlockStyle::PlainText
                        }],
                        style: BufferBlockStyle::PlainText,
                        content_length: CharOffset::from(6)
                    }),
                ]
            );

            let _ = buffer
                .redo(selection.clone(), ctx)
                .delta
                .expect("Edit delta should exist");
            assert_eq!(
                buffer.content.debug(),
                "<text>test<code:Shell>liock<text>after"
            );

            let _ = buffer.undo(selection.clone(), ctx);

            let prev_selection = buffer.to_rendered_selection_set(selection.clone(), ctx);
            let edit_result = buffer.block_style_range(
                CharOffset::from(11)..CharOffset::from(16),
                BufferBlockStyle::CodeBlock {
                    code_block_type: Default::default(),
                },
                selection.clone(),
                ctx,
            );
            let curr_selection = buffer.to_rendered_selection_set(selection.clone(), ctx);
            buffer.push_undo_item(
                prev_selection,
                curr_selection,
                edit_result.undo_item.expect("Should exist"),
                UndoActionType::Atomic,
            );
            assert_eq!(
                buffer.content.debug(),
                "<text>test<code:Shell>line<code:Shell>block<text>after"
            );

            let prev_selection = buffer.to_rendered_selection_set(selection.clone(), ctx);
            let edit_result = buffer.edit_internal_first_selection(
                CharOffset::from(8)..CharOffset::from(16),
                "",
                TextStyles::default(),
                selection.clone(),
                ctx,
            );
            let curr_selection = buffer.to_rendered_selection_set(selection.clone(), ctx);
            buffer.push_undo_item(
                prev_selection,
                curr_selection,
                edit_result.undo_item.expect("Should exist"),
                UndoActionType::Atomic,
            );
            assert_eq!(
                buffer.content.debug(),
                "<text>test<code:Shell>li<text>after"
            );

            let delta = buffer
                .undo(selection.clone(), ctx)
                .delta
                .expect("Edit delta should exist");
            assert_eq!(
                buffer.content.debug(),
                "<text>test<code:Shell>line<code:Shell>block<text>after"
            );
            assert_eq!(delta.old_offset, CharOffset::from(6)..CharOffset::from(9));
            assert_eq!(
                delta.new_lines,
                vec![
                    StyledBufferBlock::Text(StyledTextBlock {
                        block: vec![StyledBufferRun {
                            run: "line\n".into(),
                            text_styles: TextStylesWithMetadata::default(),
                            block_style: BufferBlockStyle::CodeBlock {
                                code_block_type: Default::default()
                            }
                        }],
                        style: BufferBlockStyle::CodeBlock {
                            code_block_type: Default::default(),
                        },
                        content_length: CharOffset::from(5)
                    }),
                    StyledBufferBlock::Text(StyledTextBlock {
                        block: vec![StyledBufferRun {
                            run: "block\n".into(),
                            text_styles: TextStylesWithMetadata::default(),
                            block_style: BufferBlockStyle::CodeBlock {
                                code_block_type: Default::default()
                            }
                        }],
                        style: BufferBlockStyle::CodeBlock {
                            code_block_type: Default::default(),
                        },
                        content_length: CharOffset::from(6)
                    }),
                ]
            );

            let _ = buffer
                .redo(selection.clone(), ctx)
                .delta
                .expect("Edit delta should exist");
            assert_eq!(
                buffer.content.debug(),
                "<text>test<code:Shell>li<text>after"
            );
        });
    });
}

#[test]
fn test_styling_mixed_block_types_exact() {
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            let _ = buffer.edit_internal_first_selection(
                CharOffset::from(1)..CharOffset::from(1),
                "test\nline\nsecond",
                TextStyles::default(),
                selection.clone(),
                ctx,
            );
            assert_eq!(buffer.content.debug(), "<text>test\\nline\\nsecond");

            let _ = buffer.block_style_range(
                CharOffset::from(6)..CharOffset::from(10),
                BufferBlockStyle::CodeBlock {
                    code_block_type: Default::default(),
                },
                selection.clone(),
                ctx,
            );
            assert_eq!(
                buffer.content.debug(),
                "<text>test<code:Shell>line<text>second"
            );

            let prev_selection = buffer.to_rendered_selection_set(selection.clone(), ctx);
            let edit_result = buffer.block_style_range(
                CharOffset::from(11)..CharOffset::from(17),
                BufferBlockStyle::Header {
                    header_size: BlockHeaderSize::Header1,
                },
                selection.clone(),
                ctx,
            );
            assert_eq!(
                buffer.content.debug(),
                "<text>test<code:Shell>line<header1>second<text>"
            );

            let curr_selection = buffer.to_rendered_selection_set(selection.clone(), ctx);
            buffer.push_undo_item(
                prev_selection,
                curr_selection,
                edit_result.undo_item.expect("Should exist"),
                UndoActionType::Atomic,
            );

            let delta = edit_result.delta.expect("Should exist");
            assert_eq!(delta.old_offset, CharOffset::from(11)..CharOffset::from(17));
            assert_eq!(
                delta.new_lines,
                vec![StyledBufferBlock::Text(StyledTextBlock {
                    block: vec![StyledBufferRun {
                        run: "second\n".to_string(),
                        text_styles: TextStylesWithMetadata::default(),
                        block_style: BufferBlockStyle::Header {
                            header_size: BlockHeaderSize::Header1
                        }
                    },],
                    style: BufferBlockStyle::Header {
                        header_size: BlockHeaderSize::Header1,
                    },
                    content_length: CharOffset::from(7)
                }),]
            );

            buffer.undo(selection.clone(), ctx);
            assert_eq!(
                buffer.content.debug(),
                "<text>test<code:Shell>line<text>second"
            );
            buffer.redo(selection.clone(), ctx);
            assert_eq!(
                buffer.content.debug(),
                "<text>test<code:Shell>line<header1>second<text>"
            );
        });
    });
}

#[test]
fn test_styling_mixed_block_types_surrounded() {
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            let _ = buffer.edit_internal_first_selection(
                CharOffset::from(1)..CharOffset::from(1),
                "test\nline\nsecond",
                TextStyles::default(),
                selection.clone(),
                ctx,
            );
            assert_eq!(buffer.content.debug(), "<text>test\\nline\\nsecond");

            let _ = buffer.block_style_range(
                CharOffset::from(6)..CharOffset::from(10),
                BufferBlockStyle::CodeBlock {
                    code_block_type: Default::default(),
                },
                selection.clone(),
                ctx,
            );
            assert_eq!(
                buffer.content.debug(),
                "<text>test<code:Shell>line<text>second"
            );

            let prev_selection = buffer.to_rendered_selection_set(selection.clone(), ctx);
            let edit_result = buffer.block_style_range(
                CharOffset::from(3)..CharOffset::from(17),
                BufferBlockStyle::Header {
                    header_size: BlockHeaderSize::Header1,
                },
                selection.clone(),
                ctx,
            );
            assert_eq!(
                buffer.content.debug(),
                "<text>te<header1>st<header1>line<header1>second<text>"
            );

            let curr_selection = buffer.to_rendered_selection_set(selection.clone(), ctx);
            buffer.push_undo_item(
                prev_selection,
                curr_selection,
                edit_result.undo_item.expect("Should exist"),
                UndoActionType::Atomic,
            );

            let delta = edit_result.delta.expect("Should exist");
            assert_eq!(delta.old_offset, CharOffset::from(1)..CharOffset::from(17));
            assert_eq!(
                delta.new_lines,
                vec![
                    StyledBufferBlock::Text(StyledTextBlock {
                        block: vec![StyledBufferRun {
                            run: "te\n".to_string(),
                            text_styles: TextStylesWithMetadata::default(),
                            block_style: BufferBlockStyle::PlainText
                        },],
                        style: BufferBlockStyle::PlainText,
                        content_length: CharOffset::from(3)
                    }),
                    StyledBufferBlock::Text(StyledTextBlock {
                        block: vec![StyledBufferRun {
                            run: "st\n".to_string(),
                            text_styles: TextStylesWithMetadata::default(),
                            block_style: BufferBlockStyle::Header {
                                header_size: BlockHeaderSize::Header1
                            }
                        },],
                        style: BufferBlockStyle::Header {
                            header_size: BlockHeaderSize::Header1,
                        },
                        content_length: CharOffset::from(3)
                    }),
                    StyledBufferBlock::Text(StyledTextBlock {
                        block: vec![StyledBufferRun {
                            run: "line\n".to_string(),
                            text_styles: TextStylesWithMetadata::default(),
                            block_style: BufferBlockStyle::Header {
                                header_size: BlockHeaderSize::Header1
                            }
                        },],
                        style: BufferBlockStyle::Header {
                            header_size: BlockHeaderSize::Header1,
                        },
                        content_length: CharOffset::from(5)
                    }),
                    StyledBufferBlock::Text(StyledTextBlock {
                        block: vec![StyledBufferRun {
                            run: "second\n".to_string(),
                            text_styles: TextStylesWithMetadata::default(),
                            block_style: BufferBlockStyle::Header {
                                header_size: BlockHeaderSize::Header1
                            }
                        },],
                        style: BufferBlockStyle::Header {
                            header_size: BlockHeaderSize::Header1,
                        },
                        content_length: CharOffset::from(7)
                    }),
                ]
            );

            buffer.undo(selection.clone(), ctx);
            assert_eq!(
                buffer.content.debug(),
                "<text>test<code:Shell>line<text>second"
            );
            buffer.redo(selection.clone(), ctx);
            assert_eq!(
                buffer.content.debug(),
                "<text>te<header1>st<header1>line<header1>second<text>"
            );
        });
    });
}

#[test]
fn test_styling_mixed_block_types_overlapping() {
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            let _ = buffer.edit_internal_first_selection(
                CharOffset::from(1)..CharOffset::from(1),
                "test\nline\nsecond",
                TextStyles::default(),
                selection.clone(),
                ctx,
            );
            assert_eq!(buffer.content.debug(), "<text>test\\nline\\nsecond");

            let _ = buffer.block_style_range(
                CharOffset::from(6)..CharOffset::from(10),
                BufferBlockStyle::CodeBlock {
                    code_block_type: Default::default(),
                },
                selection.clone(),
                ctx,
            );
            assert_eq!(
                buffer.content.debug(),
                "<text>test<code:Shell>line<text>second"
            );

            let prev_selection = buffer.to_rendered_selection_set(selection.clone(), ctx);
            let edit_result = buffer.block_style_range(
                CharOffset::from(8)..CharOffset::from(15),
                BufferBlockStyle::Header {
                    header_size: BlockHeaderSize::Header1,
                },
                selection.clone(),
                ctx,
            );
            assert_eq!(
                buffer.content.debug(),
                "<text>test<code:Shell>li<header1>ne<header1>seco<text>nd"
            );

            let curr_selection = buffer.to_rendered_selection_set(selection.clone(), ctx);
            buffer.push_undo_item(
                prev_selection,
                curr_selection,
                edit_result.undo_item.expect("Should exist"),
                UndoActionType::Atomic,
            );

            let delta = edit_result.delta.expect("Should exist");
            assert_eq!(delta.old_offset, CharOffset::from(6)..CharOffset::from(17));
            assert_eq!(
                delta.new_lines,
                vec![
                    StyledBufferBlock::Text(StyledTextBlock {
                        block: vec![StyledBufferRun {
                            run: "li\n".to_string(),
                            text_styles: TextStylesWithMetadata::default(),
                            block_style: BufferBlockStyle::CodeBlock {
                                code_block_type: Default::default()
                            }
                        },],
                        style: BufferBlockStyle::CodeBlock {
                            code_block_type: Default::default(),
                        },
                        content_length: CharOffset::from(3)
                    }),
                    StyledBufferBlock::Text(StyledTextBlock {
                        block: vec![StyledBufferRun {
                            run: "ne\n".to_string(),
                            text_styles: TextStylesWithMetadata::default(),
                            block_style: BufferBlockStyle::Header {
                                header_size: BlockHeaderSize::Header1
                            }
                        },],
                        style: BufferBlockStyle::Header {
                            header_size: BlockHeaderSize::Header1,
                        },
                        content_length: CharOffset::from(3)
                    }),
                    StyledBufferBlock::Text(StyledTextBlock {
                        block: vec![StyledBufferRun {
                            run: "seco\n".to_string(),
                            text_styles: TextStylesWithMetadata::default(),
                            block_style: BufferBlockStyle::Header {
                                header_size: BlockHeaderSize::Header1
                            }
                        },],
                        style: BufferBlockStyle::Header {
                            header_size: BlockHeaderSize::Header1,
                        },
                        content_length: CharOffset::from(5)
                    }),
                    StyledBufferBlock::Text(StyledTextBlock {
                        block: vec![StyledBufferRun {
                            run: "nd".to_string(),
                            text_styles: TextStylesWithMetadata::default(),
                            block_style: BufferBlockStyle::PlainText
                        },],
                        style: BufferBlockStyle::PlainText,
                        content_length: CharOffset::from(2)
                    }),
                ]
            );

            buffer.undo(selection.clone(), ctx);
            assert_eq!(
                buffer.content.debug(),
                "<text>test<code:Shell>line<text>second"
            );
            buffer.redo(selection.clone(), ctx);
            assert_eq!(
                buffer.content.debug(),
                "<text>test<code:Shell>li<header1>ne<header1>seco<text>nd"
            );
        });
    });
}

#[test]
fn test_styling_mixed_block_types_within() {
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            let _ = buffer.edit_internal_first_selection(
                CharOffset::from(1)..CharOffset::from(1),
                "test\nline\nsecond",
                TextStyles::default(),
                selection.clone(),
                ctx,
            );
            assert_eq!(buffer.content.debug(), "<text>test\\nline\\nsecond");

            let _ = buffer.block_style_range(
                CharOffset::from(6)..CharOffset::from(17),
                BufferBlockStyle::CodeBlock {
                    code_block_type: Default::default(),
                },
                selection.clone(),
                ctx,
            );
            assert_eq!(
                buffer.content.debug(),
                "<text>test<code:Shell>line\\nsecond<text>"
            );

            let prev_selection = buffer.to_rendered_selection_set(selection.clone(), ctx);
            let edit_result = buffer.block_style_range(
                CharOffset::from(8)..CharOffset::from(12),
                BufferBlockStyle::Header {
                    header_size: BlockHeaderSize::Header1,
                },
                selection.clone(),
                ctx,
            );
            assert_eq!(
                buffer.content.debug(),
                "<text>test<code:Shell>li<header1>ne<header1>s<code:Shell>econd<text>"
            );

            let curr_selection = buffer.to_rendered_selection_set(selection.clone(), ctx);
            buffer.push_undo_item(
                prev_selection,
                curr_selection,
                edit_result.undo_item.expect("Should exist"),
                UndoActionType::Atomic,
            );

            let delta = edit_result.delta.expect("Should exist");
            assert_eq!(delta.old_offset, CharOffset::from(6)..CharOffset::from(18));
            assert_eq!(
                delta.new_lines,
                vec![
                    StyledBufferBlock::Text(StyledTextBlock {
                        block: vec![StyledBufferRun {
                            run: "li\n".to_string(),
                            text_styles: TextStylesWithMetadata::default(),
                            block_style: BufferBlockStyle::CodeBlock {
                                code_block_type: Default::default()
                            }
                        },],
                        style: BufferBlockStyle::CodeBlock {
                            code_block_type: Default::default(),
                        },
                        content_length: CharOffset::from(3)
                    }),
                    StyledBufferBlock::Text(StyledTextBlock {
                        block: vec![StyledBufferRun {
                            run: "ne\n".to_string(),
                            text_styles: TextStylesWithMetadata::default(),
                            block_style: BufferBlockStyle::Header {
                                header_size: BlockHeaderSize::Header1
                            }
                        },],
                        style: BufferBlockStyle::Header {
                            header_size: BlockHeaderSize::Header1,
                        },
                        content_length: CharOffset::from(3)
                    }),
                    StyledBufferBlock::Text(StyledTextBlock {
                        block: vec![StyledBufferRun {
                            run: "s\n".to_string(),
                            text_styles: TextStylesWithMetadata::default(),
                            block_style: BufferBlockStyle::Header {
                                header_size: BlockHeaderSize::Header1
                            }
                        },],
                        style: BufferBlockStyle::Header {
                            header_size: BlockHeaderSize::Header1,
                        },
                        content_length: CharOffset::from(2)
                    }),
                    StyledBufferBlock::Text(StyledTextBlock {
                        block: vec![StyledBufferRun {
                            run: "econd\n".to_string(),
                            text_styles: TextStylesWithMetadata::default(),
                            block_style: BufferBlockStyle::CodeBlock {
                                code_block_type: Default::default()
                            }
                        },],
                        style: BufferBlockStyle::CodeBlock {
                            code_block_type: Default::default(),
                        },
                        content_length: CharOffset::from(6)
                    }),
                ]
            );

            buffer.undo(selection.clone(), ctx);
            assert_eq!(
                buffer.content.debug(),
                "<text>test<code:Shell>line\\nsecond<text>"
            );
            buffer.redo(selection.clone(), ctx);
            assert_eq!(
                buffer.content.debug(),
                "<text>test<code:Shell>li<header1>ne<header1>s<code:Shell>econd<text>"
            );
        });
    });
}

#[test]
fn test_unstyle_unordered_list_partial() {
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            let _ = buffer.edit_internal_first_selection(
                CharOffset::from(1)..CharOffset::from(1),
                "abc\ndef",
                TextStyles::default(),
                selection.clone(),
                ctx,
            );

            let _ = buffer.block_style_range(
                CharOffset::from(1)..CharOffset::from(4),
                BufferBlockStyle::UnorderedList {
                    indent_level: ListIndentLevel::One,
                },
                selection.clone(),
                ctx,
            );

            let prev_selection = buffer.to_rendered_selection_set(selection.clone(), ctx);
            let edit_result = buffer.block_style_range(
                CharOffset::from(2)..CharOffset::from(4),
                BufferBlockStyle::PlainText,
                selection.clone(),
                ctx,
            );
            assert_eq!(buffer.content.debug(), "<ul0>a<text>bc\\ndef");

            let curr_selection = buffer.to_rendered_selection_set(selection.clone(), ctx);
            buffer.push_undo_item(
                prev_selection,
                curr_selection,
                edit_result.undo_item.expect("Should exist"),
                UndoActionType::Atomic,
            );

            let delta = edit_result.delta.expect("Should exist");
            assert_eq!(delta.old_offset, CharOffset::from(1)..CharOffset::from(5));
            assert_eq!(
                delta.new_lines,
                vec![
                    StyledBufferBlock::Text(StyledTextBlock {
                        block: vec![StyledBufferRun {
                            run: "a\n".to_string(),
                            text_styles: TextStylesWithMetadata::default(),
                            block_style: BufferBlockStyle::UnorderedList {
                                indent_level: ListIndentLevel::One
                            }
                        },],
                        style: BufferBlockStyle::UnorderedList {
                            indent_level: ListIndentLevel::One,
                        },
                        content_length: CharOffset::from(2)
                    }),
                    StyledBufferBlock::Text(StyledTextBlock {
                        block: vec![StyledBufferRun {
                            run: "bc\n".to_string(),
                            text_styles: TextStylesWithMetadata::default(),
                            block_style: BufferBlockStyle::PlainText
                        },],
                        style: BufferBlockStyle::PlainText,
                        content_length: CharOffset::from(3)
                    })
                ]
            );

            buffer.undo(selection.clone(), ctx);
            assert_eq!(buffer.content.debug(), "<ul0>abc<text>def");
            buffer.redo(selection.clone(), ctx);
            assert_eq!(buffer.content.debug(), "<ul0>a<text>bc\\ndef");
        });
    });
}

#[test]
fn test_edit_in_header() {
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            let _ = buffer.edit_internal_first_selection(
                CharOffset::from(1)..CharOffset::from(1),
                "test\nline\nsecond",
                TextStyles::default(),
                selection.clone(),
                ctx,
            );
            assert_eq!(buffer.content.debug(), "<text>test\\nline\\nsecond");

            let _ = buffer.block_style_range(
                CharOffset::from(6)..CharOffset::from(10),
                BufferBlockStyle::Header {
                    header_size: BlockHeaderSize::Header1,
                },
                selection.clone(),
                ctx,
            );
            assert_eq!(
                buffer.content.debug(),
                "<text>test<header1>line<text>second"
            );

            let prev_selection = buffer.to_rendered_selection_set(selection.clone(), ctx);
            let edit_result = buffer.edit_internal_first_selection(
                CharOffset::from(8)..CharOffset::from(8),
                "more",
                TextStyles::default(),
                selection.clone(),
                ctx,
            );
            assert_eq!(
                buffer.content.debug(),
                "<text>test<header1>limorene<text>second"
            );

            let curr_selection = buffer.to_rendered_selection_set(selection.clone(), ctx);
            buffer.push_undo_item(
                prev_selection,
                curr_selection,
                edit_result.undo_item.expect("Should exist"),
                UndoActionType::Atomic,
            );

            let delta = edit_result.delta.expect("Should exist");
            assert_eq!(delta.old_offset, CharOffset::from(6)..CharOffset::from(11));
            assert_eq!(
                delta.new_lines,
                vec![StyledBufferBlock::Text(StyledTextBlock {
                    block: vec![StyledBufferRun {
                        run: "limorene\n".to_string(),
                        text_styles: TextStylesWithMetadata::default(),
                        block_style: BufferBlockStyle::Header {
                            header_size: BlockHeaderSize::Header1
                        }
                    },],
                    style: BufferBlockStyle::Header {
                        header_size: BlockHeaderSize::Header1,
                    },
                    content_length: CharOffset::from(9)
                }),]
            );

            buffer.undo(selection.clone(), ctx);
            assert_eq!(
                buffer.content.debug(),
                "<text>test<header1>line<text>second"
            );
            buffer.redo(selection.clone(), ctx);
            assert_eq!(
                buffer.content.debug(),
                "<text>test<header1>limorene<text>second"
            );

            let prev_selection = buffer.to_rendered_selection_set(selection.clone(), ctx);
            let edit_result = buffer.edit_internal_first_selection(
                CharOffset::from(8)..CharOffset::from(8),
                "\n",
                TextStyles::default(),
                selection.clone(),
                ctx,
            );
            assert_eq!(
                buffer.content.debug(),
                "<text>test<header1>li<text>morene\\nsecond"
            );

            let curr_selection = buffer.to_rendered_selection_set(selection.clone(), ctx);
            buffer.push_undo_item(
                prev_selection,
                curr_selection,
                edit_result.undo_item.expect("Should exist"),
                UndoActionType::Atomic,
            );

            let delta = edit_result.delta.expect("Should exist");
            assert_eq!(delta.old_offset, CharOffset::from(6)..CharOffset::from(15));
            assert_eq!(
                delta.new_lines,
                vec![
                    StyledBufferBlock::Text(StyledTextBlock {
                        block: vec![StyledBufferRun {
                            run: "li\n".to_string(),
                            text_styles: TextStylesWithMetadata::default(),
                            block_style: BufferBlockStyle::Header {
                                header_size: BlockHeaderSize::Header1
                            }
                        },],
                        style: BufferBlockStyle::Header {
                            header_size: BlockHeaderSize::Header1,
                        },
                        content_length: CharOffset::from(3)
                    }),
                    StyledBufferBlock::Text(StyledTextBlock {
                        block: vec![StyledBufferRun {
                            run: "morene\n".to_string(),
                            text_styles: TextStylesWithMetadata::default(),
                            block_style: BufferBlockStyle::PlainText
                        },],
                        style: BufferBlockStyle::PlainText,
                        content_length: CharOffset::from(7)
                    })
                ]
            );

            buffer.undo(selection.clone(), ctx);
            assert_eq!(
                buffer.content.debug(),
                "<text>test<header1>limorene<text>second"
            );
            buffer.redo(selection.clone(), ctx);
            assert_eq!(
                buffer.content.debug(),
                "<text>test<header1>li<text>morene\\nsecond"
            );
        });
    });
}

#[test]
fn test_insert_formatted_text_header() {
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            buffer.edit_internal_first_selection(
                CharOffset::from(1)..CharOffset::from(1),
                "test\nline\n",
                Default::default(),
                selection.clone(),
                ctx,
            );
            let _ = buffer.block_style_range(
                CharOffset::from(6)..CharOffset::from(10),
                BufferBlockStyle::CodeBlock {
                    code_block_type: Default::default(),
                },
                selection.clone(),
                ctx,
            );
            assert_eq!(buffer.content.debug(), "<text>test<code:Shell>line<text>");

            let formatted_text = parse_markdown("## New Header\n### Subheader").expect("Should parse");
            buffer.replace_with_formatted_text(
                CharOffset::from(11)..CharOffset::from(11),
                formatted_text.clone(),
                EditOrigin::UserInitiated,
                selection.clone(),
                ctx,
            );

            assert_eq!(
                buffer.content.debug(),
                "<text>test<code:Shell>line<header2>New Header<header3>Subheader<text>"
            );

            buffer.replace_with_formatted_text(CharOffset::from(7)..CharOffset::from(7), formatted_text, EditOrigin::UserInitiated, selection.clone(), ctx);
            assert_eq!(
                buffer.content.debug(),
                "<text>test<code:Shell>lNew Header<header3>Subheader<code:Shell>ine<header2>New Header<header3>Subheader<text>"
            );
        });
    });
}

#[test]
fn test_insert_block_after_block_with_offset() {
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            // Test inserting in an empty buffer.
            let prev_selection = buffer.to_rendered_selection_set(selection.clone(), ctx);
            let edit_result = buffer.insert_block_after_block_with_offset(
                CharOffset::from(1),
                BlockType::Text(BufferBlockStyle::Header {
                    header_size: BlockHeaderSize::Header1,
                }),
                selection.clone(),
                ctx,
            );
            // Note that we added an additional empty plain text line at the bottom.
            assert_eq!(buffer.content.debug(), "<text><header1><text>");
            let curr_selection = buffer.to_rendered_selection_set(selection.clone(), ctx);
            buffer.push_undo_item(
                prev_selection,
                curr_selection,
                edit_result.undo_item.expect("Should exist"),
                UndoActionType::Atomic,
            );

            let delta = edit_result.delta.expect("Should exist");
            assert_eq!(delta.old_offset, CharOffset::from(1)..CharOffset::from(1));
            assert_eq!(
                delta.new_lines,
                vec![
                    StyledBufferBlock::Text(StyledTextBlock {
                        block: vec![StyledBufferRun {
                            run: "\n".to_string(),
                            text_styles: TextStylesWithMetadata::default(),
                            block_style: BufferBlockStyle::PlainText
                        },],
                        style: BufferBlockStyle::PlainText,
                        content_length: CharOffset::from(1)
                    }),
                    StyledBufferBlock::Text(StyledTextBlock {
                        block: vec![StyledBufferRun {
                            run: "\n".to_string(),
                            text_styles: TextStylesWithMetadata::default(),
                            block_style: BufferBlockStyle::Header {
                                header_size: BlockHeaderSize::Header1
                            }
                        },],
                        style: BufferBlockStyle::Header {
                            header_size: BlockHeaderSize::Header1,
                        },
                        content_length: CharOffset::from(1)
                    })
                ]
            );

            buffer.undo(selection.clone(), ctx);
            assert_eq!(buffer.content.debug(), "<text>");
            buffer.redo(selection.clone(), ctx);
            assert_eq!(buffer.content.debug(), "<text><header1><text>");

            // Test inserting a different block type after the header.
            let prev_selection = buffer.to_rendered_selection_set(selection.clone(), ctx);
            let edit_result = buffer.insert_block_after_block_with_offset(
                CharOffset::from(2),
                BlockType::Text(BufferBlockStyle::CodeBlock {
                    code_block_type: Default::default(),
                }),
                selection.clone(),
                ctx,
            );
            assert_eq!(buffer.content.debug(), "<text><header1><code:Shell><text>");
            let curr_selection = buffer.to_rendered_selection_set(selection.clone(), ctx);
            buffer.push_undo_item(
                prev_selection,
                curr_selection,
                edit_result.undo_item.expect("Should exist"),
                UndoActionType::Atomic,
            );

            let delta = edit_result.delta.expect("Should exist");
            assert_eq!(delta.old_offset, CharOffset::from(2)..CharOffset::from(3));
            assert_eq!(
                delta.new_lines,
                vec![
                    StyledBufferBlock::Text(StyledTextBlock {
                        block: vec![StyledBufferRun {
                            run: "\n".to_string(),
                            text_styles: TextStylesWithMetadata::default(),
                            block_style: BufferBlockStyle::Header {
                                header_size: BlockHeaderSize::Header1
                            }
                        },],
                        style: BufferBlockStyle::Header {
                            header_size: BlockHeaderSize::Header1,
                        },
                        content_length: CharOffset::from(1)
                    }),
                    StyledBufferBlock::Text(StyledTextBlock {
                        block: vec![StyledBufferRun {
                            run: "\n".to_string(),
                            text_styles: TextStylesWithMetadata::default(),
                            block_style: BufferBlockStyle::CodeBlock {
                                code_block_type: Default::default()
                            }
                        },],
                        style: BufferBlockStyle::CodeBlock {
                            code_block_type: Default::default(),
                        },
                        content_length: CharOffset::from(1)
                    }),
                ]
            );

            buffer.undo(selection.clone(), ctx);
            assert_eq!(buffer.content.debug(), "<text><header1><text>");
            buffer.redo(selection.clone(), ctx);
            assert_eq!(buffer.content.debug(), "<text><header1><code:Shell><text>");

            // Test inserting the same block type. This should create a new block instead of
            // adding a newline in the old block.
            let prev_selection = buffer.to_rendered_selection_set(selection.clone(), ctx);
            let edit_result = buffer.insert_block_after_block_with_offset(
                CharOffset::from(3),
                BlockType::Text(BufferBlockStyle::CodeBlock {
                    code_block_type: Default::default(),
                }),
                selection.clone(),
                ctx,
            );
            assert_eq!(
                buffer.content.debug(),
                "<text><header1><code:Shell><code:Shell><text>"
            );
            let curr_selection = buffer.to_rendered_selection_set(selection.clone(), ctx);
            buffer.push_undo_item(
                prev_selection,
                curr_selection,
                edit_result.undo_item.expect("Should exist"),
                UndoActionType::Atomic,
            );

            let delta = edit_result.delta.expect("Should exist");
            assert_eq!(delta.old_offset, CharOffset::from(3)..CharOffset::from(4));
            assert_eq!(
                delta.new_lines,
                vec![
                    StyledBufferBlock::Text(StyledTextBlock {
                        block: vec![StyledBufferRun {
                            run: "\n".to_string(),
                            text_styles: TextStylesWithMetadata::default(),
                            block_style: BufferBlockStyle::CodeBlock {
                                code_block_type: Default::default()
                            }
                        },],
                        style: BufferBlockStyle::CodeBlock {
                            code_block_type: Default::default(),
                        },
                        content_length: CharOffset::from(1)
                    }),
                    StyledBufferBlock::Text(StyledTextBlock {
                        block: vec![StyledBufferRun {
                            run: "\n".to_string(),
                            text_styles: TextStylesWithMetadata::default(),
                            block_style: BufferBlockStyle::CodeBlock {
                                code_block_type: Default::default()
                            }
                        },],
                        style: BufferBlockStyle::CodeBlock {
                            code_block_type: Default::default(),
                        },
                        content_length: CharOffset::from(1)
                    }),
                ]
            );

            buffer.undo(selection.clone(), ctx);
            assert_eq!(buffer.content.debug(), "<text><header1><code:Shell><text>");
            buffer.redo(selection.clone(), ctx);
            assert_eq!(
                buffer.content.debug(),
                "<text><header1><code:Shell><code:Shell><text>"
            );
        });
    });
}

#[test]
fn test_linebreak_in_unordered_list() {
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            buffer.edit_internal_first_selection(
                CharOffset::from(1)..CharOffset::from(1),
                "test\nline\nsecond",
                Default::default(),
                selection.clone(),
                ctx,
            );
            let _ = buffer.block_style_range(
                CharOffset::from(6)..CharOffset::from(10),
                BufferBlockStyle::UnorderedList {
                    indent_level: ListIndentLevel::One,
                },
                selection.clone(),
                ctx,
            );
            assert_eq!(buffer.content.debug(), "<text>test<ul0>line<text>second");

            let prev_selection = buffer.to_rendered_selection_set(selection.clone(), ctx);
            let edit_result = buffer.edit_internal_first_selection(
                CharOffset::from(10)..CharOffset::from(10),
                "\n",
                TextStyles::default(),
                selection.clone(),
                ctx,
            );
            assert_eq!(
                buffer.content.debug(),
                "<text>test<ul0>line<ul0><text>second"
            );

            let curr_selection = buffer.to_rendered_selection_set(selection.clone(), ctx);
            buffer.push_undo_item(
                prev_selection,
                curr_selection,
                edit_result.undo_item.expect("Should exist"),
                UndoActionType::Atomic,
            );

            let delta = edit_result.delta.expect("Should exist");
            assert_eq!(delta.old_offset, CharOffset::from(6)..CharOffset::from(11));
            assert_eq!(
                delta.new_lines,
                vec![
                    StyledBufferBlock::Text(StyledTextBlock {
                        block: vec![StyledBufferRun {
                            run: "line\n".to_string(),
                            text_styles: TextStylesWithMetadata::default(),
                            block_style: BufferBlockStyle::UnorderedList {
                                indent_level: ListIndentLevel::One
                            }
                        },],
                        style: BufferBlockStyle::UnorderedList {
                            indent_level: ListIndentLevel::One,
                        },
                        content_length: CharOffset::from(5)
                    }),
                    StyledBufferBlock::Text(StyledTextBlock {
                        block: vec![StyledBufferRun {
                            run: "\n".to_string(),
                            text_styles: TextStylesWithMetadata::default(),
                            block_style: BufferBlockStyle::UnorderedList {
                                indent_level: ListIndentLevel::One
                            }
                        },],
                        style: BufferBlockStyle::UnorderedList {
                            indent_level: ListIndentLevel::One,
                        },
                        content_length: CharOffset::from(1)
                    }),
                ]
            );

            buffer.undo(selection.clone(), ctx);
            assert_eq!(buffer.content.debug(), "<text>test<ul0>line<text>second");
            buffer.redo(selection.clone(), ctx);
            assert_eq!(
                buffer.content.debug(),
                "<text>test<ul0>line<ul0><text>second"
            );

            let prev_selection = buffer.to_rendered_selection_set(selection.clone(), ctx);
            let edit_result = buffer.edit_internal_first_selection(
                CharOffset::from(8)..CharOffset::from(8),
                "\n",
                TextStyles::default(),
                selection.clone(),
                ctx,
            );
            assert_eq!(
                buffer.content.debug(),
                "<text>test<ul0>li<ul0>ne<ul0><text>second"
            );
            let curr_selection = buffer.to_rendered_selection_set(selection.clone(), ctx);
            buffer.push_undo_item(
                prev_selection,
                curr_selection,
                edit_result.undo_item.expect("Should exist"),
                UndoActionType::Atomic,
            );

            let delta = edit_result.delta.expect("Should exist");
            assert_eq!(delta.old_offset, CharOffset::from(6)..CharOffset::from(11));
            assert_eq!(
                delta.new_lines,
                vec![
                    StyledBufferBlock::Text(StyledTextBlock {
                        block: vec![StyledBufferRun {
                            run: "li\n".to_string(),
                            text_styles: TextStylesWithMetadata::default(),
                            block_style: BufferBlockStyle::UnorderedList {
                                indent_level: ListIndentLevel::One
                            }
                        },],
                        style: BufferBlockStyle::UnorderedList {
                            indent_level: ListIndentLevel::One,
                        },
                        content_length: CharOffset::from(3)
                    }),
                    StyledBufferBlock::Text(StyledTextBlock {
                        block: vec![StyledBufferRun {
                            run: "ne\n".to_string(),
                            text_styles: TextStylesWithMetadata::default(),
                            block_style: BufferBlockStyle::UnorderedList {
                                indent_level: ListIndentLevel::One
                            }
                        },],
                        style: BufferBlockStyle::UnorderedList {
                            indent_level: ListIndentLevel::One,
                        },
                        content_length: CharOffset::from(3)
                    }),
                ]
            );

            buffer.undo(selection.clone(), ctx);
            assert_eq!(
                buffer.content.debug(),
                "<text>test<ul0>line<ul0><text>second"
            );
            buffer.redo(selection.clone(), ctx);
            assert_eq!(
                buffer.content.debug(),
                "<text>test<ul0>li<ul0>ne<ul0><text>second"
            );
        });
    });
}

#[test]
fn test_enter_in_list_at_buffer_end() {
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            buffer.edit_internal_first_selection(
                CharOffset::from(1)..CharOffset::from(1),
                "Text\nList",
                Default::default(),
                selection.clone(),
                ctx,
            );
            buffer.block_style_range(
                CharOffset::from(6)..CharOffset::from(10),
                BufferBlockStyle::ordered_list(ListIndentLevel::One),
                selection.clone(),
                ctx,
            );
            assert_eq!(buffer.debug(), "<text>Text<ol0>List<text>");

            // Now, type a newline right at the end of List.
            let result = buffer.edit_internal_first_selection(
                CharOffset::from(10)..CharOffset::from(10),
                "\n",
                Default::default(),
                selection.clone(),
                ctx,
            );

            assert_eq!(buffer.debug(), "<text>Text<ol0>List<ol0><text>");
            let delta = result.delta.expect("Edit delta should exist");
            assert_eq!(delta.old_offset, CharOffset::from(6)..CharOffset::from(11));
            assert_eq!(
                delta.new_lines,
                vec![
                    StyledBufferBlock::Text(StyledTextBlock {
                        block: vec![StyledBufferRun {
                            run: "List\n".to_string(),
                            text_styles: Default::default(),
                            block_style: BufferBlockStyle::ordered_list(ListIndentLevel::One)
                        }],
                        style: BufferBlockStyle::ordered_list(ListIndentLevel::One),
                        content_length: CharOffset::from(5)
                    }),
                    StyledBufferBlock::Text(StyledTextBlock {
                        block: vec![StyledBufferRun {
                            run: "\n".to_string(),
                            text_styles: Default::default(),
                            block_style: BufferBlockStyle::ordered_list(ListIndentLevel::One)
                        }],
                        style: BufferBlockStyle::ordered_list(ListIndentLevel::One),
                        content_length: CharOffset::from(1)
                    }),
                ]
            );
        });
    });
}

#[test]
fn test_backspace_on_block_marker() {
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            buffer.edit_internal_first_selection(
                CharOffset::from(1)..CharOffset::from(1),
                "test\nline\nsecond",
                Default::default(),
                selection.clone(),
                ctx,
            );
            let _ = buffer.block_style_range(
                CharOffset::from(6)..CharOffset::from(10),
                BufferBlockStyle::UnorderedList {
                    indent_level: ListIndentLevel::One,
                },
                selection.clone(),
                ctx,
            );
            assert_eq!(buffer.content.debug(), "<text>test<ul0>line<text>second");

            selection.update(ctx, |selection, _| {
                selection.set_single_cursor(CharOffset::from(6));
            });
            buffer.backspace(&mut None, selection.clone(), ctx);
            assert_eq!(buffer.content.debug(), "<text>test\\nline\\nsecond");
        });
    });
}

#[test]
fn test_delete_clamped() {
    // This tests that Delete actions can't remove the starting block marker.
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            buffer.edit_internal_first_selection(
                CharOffset::from(1)..CharOffset::from(1),
                "some text",
                Default::default(),
                selection.clone(),
                ctx,
            );
            buffer.set_selection(
                CharOffset::from(1)..CharOffset::from(8),
                selection.clone(),
                ctx,
            );
            buffer.style_internal(TextStyles::default().bold(), selection.clone(), ctx);

            // This would happen when, for example, deleting a word (alt-backspace) at the start of the buffer.
            let mut style = None;
            buffer.delete(
                &mut style,
                vec1![CharOffset::zero()..CharOffset::from(5)],
                selection.clone(),
                ctx,
            );

            assert_eq!(buffer.debug(), "<text><b_s> te<b_e>xt");
            assert_eq!(style, Some(TextStylesWithMetadata::default().bold()));
        });
    });
}

#[test]
fn test_nonatomic_undo_insertion() {
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            buffer.edit_internal_first_selection(
                CharOffset::from(1)..CharOffset::from(1),
                "test\nline\nsecond",
                Default::default(),
                selection.clone(),
                ctx,
            );
            let _ = buffer.block_style_range(
                CharOffset::from(6)..CharOffset::from(10),
                BufferBlockStyle::UnorderedList {
                    indent_level: ListIndentLevel::One,
                },
                selection.clone(),
                ctx,
            );
            assert_eq!(buffer.content.debug(), "<text>test<ul0>line<text>second");

            let prev_selection = buffer.to_rendered_selection_set(selection.clone(), ctx);
            let edit_result = buffer.edit_internal_first_selection(
                CharOffset::from(1)..CharOffset::from(1),
                "so",
                TextStyles::default(),
                selection.clone(),
                ctx,
            );
            assert_eq!(buffer.content.debug(), "<text>sotest<ul0>line<text>second");

            buffer.push_undo_item_nonatomic(
                prev_selection,
                edit_result.undo_item.expect("Should exist"),
                NonAtomicType::Insert,
                selection.clone(),
                ctx,
            );

            let prev_selection = buffer.to_rendered_selection_set(selection.clone(), ctx);
            let edit_result = buffer.edit_internal_first_selection(
                CharOffset::from(3)..CharOffset::from(3),
                "\n",
                TextStyles::default(),
                selection.clone(),
                ctx,
            );
            assert_eq!(
                buffer.content.debug(),
                "<text>so\\ntest<ul0>line<text>second"
            );

            buffer.push_undo_item_nonatomic(
                prev_selection,
                edit_result.undo_item.expect("Should exist"),
                NonAtomicType::Insert,
                selection.clone(),
                ctx,
            );

            let prev_selection = buffer.to_rendered_selection_set(selection.clone(), ctx);
            let edit_result = buffer.edit_internal_first_selection(
                CharOffset::from(4)..CharOffset::from(4),
                " ",
                TextStyles::default(),
                selection.clone(),
                ctx,
            );
            assert_eq!(
                buffer.content.debug(),
                "<text>so\\n test<ul0>line<text>second"
            );

            buffer.push_undo_item_nonatomic(
                prev_selection,
                edit_result.undo_item.expect("Should exist"),
                NonAtomicType::Insert,
                selection.clone(),
                ctx,
            );

            let delta = buffer
                .undo(selection.clone(), ctx)
                .delta
                .expect("Should exist");
            assert_eq!(buffer.content.debug(), "<text>test<ul0>line<text>second");

            assert_eq!(delta.old_offset, CharOffset::from(1)..CharOffset::from(10));
            assert_eq!(
                delta.new_lines,
                vec![StyledBufferBlock::Text(StyledTextBlock {
                    block: vec![StyledBufferRun {
                        run: "test\n".to_string(),
                        text_styles: TextStylesWithMetadata::default(),
                        block_style: BufferBlockStyle::PlainText
                    },],
                    style: BufferBlockStyle::PlainText,
                    content_length: CharOffset::from(5)
                }),]
            );

            let delta = buffer
                .redo(selection.clone(), ctx)
                .delta
                .expect("Should exist");
            assert_eq!(
                buffer.content.debug(),
                "<text>so\\n test<ul0>line<text>second"
            );

            assert_eq!(delta.old_offset, CharOffset::from(1)..CharOffset::from(6));
            assert_eq!(
                delta.new_lines,
                vec![
                    StyledBufferBlock::Text(StyledTextBlock {
                        block: vec![StyledBufferRun {
                            run: "so\n".to_string(),
                            text_styles: TextStylesWithMetadata::default(),
                            block_style: BufferBlockStyle::PlainText
                        },],
                        style: BufferBlockStyle::PlainText,
                        content_length: CharOffset::from(3)
                    }),
                    StyledBufferBlock::Text(StyledTextBlock {
                        block: vec![StyledBufferRun {
                            run: " test\n".to_string(),
                            text_styles: TextStylesWithMetadata::default(),
                            block_style: BufferBlockStyle::PlainText
                        },],
                        style: BufferBlockStyle::PlainText,
                        content_length: CharOffset::from(6)
                    })
                ]
            );
        });
    });
}

#[test]
fn test_nonatomic_undo_deletion() {
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            buffer.edit_internal_first_selection(
                CharOffset::from(1)..CharOffset::from(1),
                "test\nline\nsecond",
                Default::default(),
                selection.clone(),
                ctx,
            );
            assert_eq!(buffer.content.debug(), "<text>test\\nline\\nsecond");

            selection.update(ctx, |selection, _| {
                selection.set_single_cursor(CharOffset::from(7));
            });
            let prev_selection = buffer.to_rendered_selection_set(selection.clone(), ctx);
            let edit_result = buffer.backspace(&mut None, selection.clone(), ctx);
            assert_eq!(buffer.content.debug(), "<text>test\\nine\\nsecond");

            buffer.push_undo_item_nonatomic(
                prev_selection,
                edit_result.undo_item.expect("Should exist"),
                NonAtomicType::Backspace,
                selection.clone(),
                ctx,
            );

            let prev_selection = buffer.to_rendered_selection_set(selection.clone(), ctx);
            let edit_result = buffer.backspace(&mut None, selection.clone(), ctx);
            assert_eq!(buffer.content.debug(), "<text>testine\\nsecond");

            buffer.push_undo_item_nonatomic(
                prev_selection,
                edit_result.undo_item.expect("Should exist"),
                NonAtomicType::Backspace,
                selection.clone(),
                ctx,
            );

            let prev_selection = buffer.to_rendered_selection_set(selection.clone(), ctx);
            let edit_result = buffer.backspace(&mut None, selection.clone(), ctx);
            assert_eq!(buffer.content.debug(), "<text>tesine\\nsecond");

            buffer.push_undo_item_nonatomic(
                prev_selection,
                edit_result.undo_item.expect("Should exist"),
                NonAtomicType::Backspace,
                selection.clone(),
                ctx,
            );

            let delta = buffer
                .undo(selection.clone(), ctx)
                .delta
                .expect("Should exist");
            assert_eq!(buffer.content.debug(), "<text>test\\nline\\nsecond");

            assert_eq!(delta.old_offset, CharOffset::from(1)..CharOffset::from(8));
            assert_eq!(
                delta.new_lines,
                vec![
                    StyledBufferBlock::Text(StyledTextBlock {
                        block: vec![StyledBufferRun {
                            run: "test\n".to_string(),
                            text_styles: TextStylesWithMetadata::default(),
                            block_style: BufferBlockStyle::PlainText
                        },],
                        style: BufferBlockStyle::PlainText,
                        content_length: CharOffset::from(5)
                    }),
                    StyledBufferBlock::Text(StyledTextBlock {
                        block: vec![StyledBufferRun {
                            run: "line\n".to_string(),
                            text_styles: TextStylesWithMetadata::default(),
                            block_style: BufferBlockStyle::PlainText
                        },],
                        style: BufferBlockStyle::PlainText,
                        content_length: CharOffset::from(5)
                    })
                ]
            );

            let delta = buffer
                .redo(selection.clone(), ctx)
                .delta
                .expect("Should exist");
            assert_eq!(buffer.content.debug(), "<text>tesine\\nsecond");

            assert_eq!(delta.old_offset, CharOffset::from(1)..CharOffset::from(11));
            assert_eq!(
                delta.new_lines,
                vec![StyledBufferBlock::Text(StyledTextBlock {
                    block: vec![StyledBufferRun {
                        run: "tesine\n".to_string(),
                        text_styles: TextStylesWithMetadata::default(),
                        block_style: BufferBlockStyle::PlainText
                    },],
                    style: BufferBlockStyle::PlainText,
                    content_length: CharOffset::from(7)
                }),]
            );
        });
    });
}

#[test]
fn test_nonatomic_undo_mixed() {
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            buffer.edit_internal_first_selection(
                CharOffset::from(1)..CharOffset::from(1),
                "test\nline\nsecond",
                Default::default(),
                selection.clone(),
                ctx,
            );
            assert_eq!(buffer.content.debug(), "<text>test\\nline\\nsecond");

            selection.update(ctx, |selection, _| {
                selection.set_single_cursor(CharOffset::from(7));
            });
            let prev_selection = buffer.to_rendered_selection_set(selection.clone(), ctx);
            let edit_result = buffer.backspace(&mut None, selection.clone(), ctx);
            assert_eq!(buffer.content.debug(), "<text>test\\nine\\nsecond");

            buffer.push_undo_item_nonatomic(
                prev_selection,
                edit_result.undo_item.expect("Should exist"),
                NonAtomicType::Backspace,
                selection.clone(),
                ctx,
            );

            let prev_selection = buffer.to_rendered_selection_set(selection.clone(), ctx);
            let edit_result = buffer.edit_internal_first_selection(
                CharOffset::from(6)..CharOffset::from(6),
                "d",
                Default::default(),
                selection.clone(),
                ctx,
            );
            assert_eq!(buffer.content.debug(), "<text>test\\ndine\\nsecond");

            buffer.push_undo_item_nonatomic(
                prev_selection,
                edit_result.undo_item.expect("Should exist"),
                NonAtomicType::Insert,
                selection.clone(),
                ctx,
            );

            let prev_selection = buffer.to_rendered_selection_set(selection.clone(), ctx);
            let edit_result = buffer.edit_internal_first_selection(
                CharOffset::from(7)..CharOffset::from(7),
                "e",
                Default::default(),
                selection.clone(),
                ctx,
            );
            assert_eq!(buffer.content.debug(), "<text>test\\ndeine\\nsecond");

            buffer.push_undo_item_nonatomic(
                prev_selection,
                edit_result.undo_item.expect("Should exist"),
                NonAtomicType::Insert,
                selection.clone(),
                ctx,
            );

            buffer.undo(selection.clone(), ctx);
            assert_eq!(buffer.content.debug(), "<text>test\\nine\\nsecond");

            buffer.undo(selection.clone(), ctx);
            assert_eq!(buffer.content.debug(), "<text>test\\nline\\nsecond");

            buffer.redo(selection.clone(), ctx);
            assert_eq!(buffer.content.debug(), "<text>test\\nine\\nsecond");

            buffer.redo(selection.clone(), ctx);
            assert_eq!(buffer.content.debug(), "<text>test\\ndeine\\nsecond");
        });
    });
}

// Regression test for CLD-655
#[test]
fn test_undo_with_invalidated_selection_range() {
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            buffer.edit_internal_first_selection(
                CharOffset::from(1)..CharOffset::from(1),
                "abc\n",
                Default::default(),
                selection.clone(),
                ctx,
            );
            assert_eq!(buffer.content.debug(), "<text>abc\\n");

            let prev_selection = buffer.to_rendered_selection_set(selection.clone(), ctx);
            let edit_result = buffer.edit_internal_first_selection(
                CharOffset::from(5)..CharOffset::from(5),
                "abc",
                TextStyles::default(),
                selection.clone(),
                ctx,
            );
            let curr_selection = buffer.to_rendered_selection_set(selection.clone(), ctx);
            buffer.push_undo_item(
                prev_selection,
                curr_selection,
                edit_result.undo_item.expect("Should exist"),
                UndoActionType::Atomic,
            );
            assert_eq!(buffer.content.debug(), "<text>abc\\nabc");

            selection.update(ctx, |selection, _| {
                selection.set_single_cursor(CharOffset::from(6));
            });

            buffer.undo(selection.clone(), ctx);
            assert_eq!(buffer.content.debug(), "<text>abc\\n");
            assert_eq!(
                selection.as_ref(ctx).selection_to_first_offset_range(),
                CharOffset::from(5)..CharOffset::from(5)
            );
        });
    });
}

#[test]
fn test_link_style_exact() {
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            buffer.edit_internal_first_selection(
                CharOffset::from(1)..CharOffset::from(1),
                "line\nblock",
                Default::default(),
                selection.clone(),
                ctx,
            );
            assert_eq!(buffer.content.debug(), "<text>line\\nblock");

            let prev_selection = buffer.to_rendered_selection_set(selection.clone(), ctx);
            let edit_result = buffer.select_and_style_link(
                CharOffset::from(1)..CharOffset::from(3),
                "li".to_string(),
                "www.google.com".to_string(),
                selection.clone(),
                ctx,
            );
            let curr_selection = buffer.to_rendered_selection_set(selection.clone(), ctx);
            buffer.push_undo_item(
                prev_selection,
                curr_selection,
                edit_result.undo_item.expect("Should exist"),
                UndoActionType::Atomic,
            );
            assert_eq!(
                buffer.content.debug(),
                "<text><a_www.google.com>li<a>ne\\nblock"
            );

            let delta = edit_result.delta.expect("Should exist");
            assert_eq!(delta.old_offset, CharOffset::from(1)..CharOffset::from(6));
            assert_eq!(
                delta.new_lines,
                vec![StyledBufferBlock::Text(StyledTextBlock {
                    block: vec![
                        StyledBufferRun {
                            run: "li".to_string(),
                            text_styles: TextStylesWithMetadata::default()
                                .link("www.google.com".to_string()),
                            block_style: BufferBlockStyle::PlainText
                        },
                        StyledBufferRun {
                            run: "ne\n".to_string(),
                            text_styles: TextStylesWithMetadata::default(),
                            block_style: BufferBlockStyle::PlainText
                        }
                    ],
                    style: BufferBlockStyle::PlainText,
                    content_length: CharOffset::from(5)
                }),]
            );

            buffer.undo(selection.clone(), ctx);
            assert_eq!(buffer.content.debug(), "<text>line\\nblock");

            buffer.redo(selection.clone(), ctx);
            assert_eq!(
                buffer.content.debug(),
                "<text><a_www.google.com>li<a>ne\\nblock"
            );

            let prev_selection = buffer.to_rendered_selection_set(selection.clone(), ctx);
            let edit_result = buffer.select_and_style_link(
                CharOffset::from(3)..CharOffset::from(7),
                "ne\nb".to_string(),
                "www.warp.dev".to_string(),
                selection.clone(),
                ctx,
            );
            let curr_selection = buffer.to_rendered_selection_set(selection.clone(), ctx);
            buffer.push_undo_item(
                prev_selection,
                curr_selection,
                edit_result.undo_item.expect("Should exist"),
                UndoActionType::Atomic,
            );
            assert_eq!(
                buffer.content.debug(),
                "<text><a_www.google.com>li<a><a_www.warp.dev>ne\\nb<a>lock"
            );

            let delta = edit_result.delta.expect("Should exist");
            assert_eq!(delta.old_offset, CharOffset::from(1)..CharOffset::from(11));
            assert_eq!(
                delta.new_lines,
                vec![
                    StyledBufferBlock::Text(StyledTextBlock {
                        block: vec![
                            StyledBufferRun {
                                run: "li".to_string(),
                                text_styles: TextStylesWithMetadata::default()
                                    .link("www.google.com".to_string()),
                                block_style: BufferBlockStyle::PlainText
                            },
                            StyledBufferRun {
                                run: "ne\n".to_string(),
                                text_styles: TextStylesWithMetadata::default()
                                    .link("www.warp.dev".to_string()),
                                block_style: BufferBlockStyle::PlainText
                            }
                        ],
                        style: BufferBlockStyle::PlainText,
                        content_length: CharOffset::from(5)
                    }),
                    StyledBufferBlock::Text(StyledTextBlock {
                        block: vec![
                            StyledBufferRun {
                                run: "b".to_string(),
                                text_styles: TextStylesWithMetadata::default()
                                    .link("www.warp.dev".to_string()),
                                block_style: BufferBlockStyle::PlainText
                            },
                            StyledBufferRun {
                                run: "lock".to_string(),
                                text_styles: TextStylesWithMetadata::default(),
                                block_style: BufferBlockStyle::PlainText
                            }
                        ],
                        style: BufferBlockStyle::PlainText,
                        content_length: CharOffset::from(5)
                    })
                ]
            );

            buffer.undo(selection.clone(), ctx);
            assert_eq!(
                buffer.content.debug(),
                "<text><a_www.google.com>li<a>ne\\nblock"
            );

            buffer.redo(selection.clone(), ctx);
            assert_eq!(
                buffer.content.debug(),
                "<text><a_www.google.com>li<a><a_www.warp.dev>ne\\nb<a>lock"
            );
        });
    });
}

#[test]
fn test_link_style_different_tag() {
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            buffer.edit_internal_first_selection(
                CharOffset::from(1)..CharOffset::from(1),
                "line\nblock",
                Default::default(),
                selection.clone(),
                ctx,
            );
            assert_eq!(buffer.content.debug(), "<text>line\\nblock");

            let prev_selection = buffer.to_rendered_selection_set(selection.clone(), ctx);
            let edit_result = buffer.select_and_style_link(
                CharOffset::from(1)..CharOffset::from(3),
                "g".to_string(),
                "www.google.com".to_string(),
                selection.clone(),
                ctx,
            );
            let curr_selection = buffer.to_rendered_selection_set(selection.clone(), ctx);
            buffer.push_undo_item(
                prev_selection,
                curr_selection,
                edit_result.undo_item.expect("Should exist"),
                UndoActionType::Atomic,
            );
            assert_eq!(
                buffer.content.debug(),
                "<text><a_www.google.com>g<a>ne\\nblock"
            );

            let delta = edit_result.delta.expect("Should exist");
            assert_eq!(delta.old_offset, CharOffset::from(1)..CharOffset::from(6));
            assert_eq!(
                delta.new_lines,
                vec![StyledBufferBlock::Text(StyledTextBlock {
                    block: vec![
                        StyledBufferRun {
                            run: "g".to_string(),
                            text_styles: TextStylesWithMetadata::default()
                                .link("www.google.com".to_string()),
                            block_style: BufferBlockStyle::PlainText
                        },
                        StyledBufferRun {
                            run: "ne\n".to_string(),
                            text_styles: TextStylesWithMetadata::default(),
                            block_style: BufferBlockStyle::PlainText
                        }
                    ],
                    style: BufferBlockStyle::PlainText,
                    content_length: CharOffset::from(4)
                }),]
            );

            buffer.undo(selection.clone(), ctx);
            assert_eq!(buffer.content.debug(), "<text>line\\nblock");

            buffer.redo(selection.clone(), ctx);
            assert_eq!(
                buffer.content.debug(),
                "<text><a_www.google.com>g<a>ne\\nblock"
            );

            let prev_selection = buffer.to_rendered_selection_set(selection.clone(), ctx);
            let edit_result = buffer.select_and_style_link(
                CharOffset::from(3)..CharOffset::from(7),
                "normal long text".to_string(),
                "www.warp.dev".to_string(),
                selection.clone(),
                ctx,
            );
            let curr_selection = buffer.to_rendered_selection_set(selection.clone(), ctx);
            buffer.push_undo_item(
                prev_selection,
                curr_selection,
                edit_result.undo_item.expect("Should exist"),
                UndoActionType::Atomic,
            );
            assert_eq!(
                buffer.content.debug(),
                "<text><a_www.google.com>g<a>n<a_www.warp.dev>normal long text<a>ock"
            );

            let delta = edit_result.delta.expect("Should exist");
            assert_eq!(delta.old_offset, CharOffset::from(1)..CharOffset::from(10));
            assert_eq!(
                delta.new_lines,
                vec![StyledBufferBlock::Text(StyledTextBlock {
                    block: vec![
                        StyledBufferRun {
                            run: "g".to_string(),
                            text_styles: TextStylesWithMetadata::default()
                                .link("www.google.com".to_string()),
                            block_style: BufferBlockStyle::PlainText
                        },
                        StyledBufferRun {
                            run: "n".to_string(),
                            text_styles: TextStylesWithMetadata::default(),
                            block_style: BufferBlockStyle::PlainText
                        },
                        StyledBufferRun {
                            run: "normal long text".to_string(),
                            text_styles: TextStylesWithMetadata::default()
                                .link("www.warp.dev".to_string()),
                            block_style: BufferBlockStyle::PlainText
                        },
                        StyledBufferRun {
                            run: "ock".to_string(),
                            text_styles: TextStylesWithMetadata::default(),
                            block_style: BufferBlockStyle::PlainText
                        }
                    ],
                    style: BufferBlockStyle::PlainText,
                    content_length: CharOffset::from(21)
                }),]
            );

            buffer.undo(selection.clone(), ctx);
            assert_eq!(
                buffer.content.debug(),
                "<text><a_www.google.com>g<a>ne\\nblock"
            );

            buffer.redo(selection.clone(), ctx);
            assert_eq!(
                buffer.content.debug(),
                "<text><a_www.google.com>g<a>n<a_www.warp.dev>normal long text<a>ock"
            );
        });
    });
}

#[test]
fn test_link_style_overlapping() {
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            buffer.edit_internal_first_selection(
                CharOffset::from(1)..CharOffset::from(1),
                "line\nblock",
                Default::default(),
                selection.clone(),
                ctx,
            );
            assert_eq!(buffer.content.debug(), "<text>line\\nblock");
            buffer.select_and_style_link(
                CharOffset::from(1)..CharOffset::from(5),
                "line".to_string(),
                "www.google.com".to_string(),
                selection.clone(),
                ctx,
            );
            assert_eq!(
                buffer.content.debug(),
                "<text><a_www.google.com>line<a>\\nblock"
            );

            let prev_selection = buffer.to_rendered_selection_set(selection.clone(), ctx);
            let edit_result = buffer.select_and_style_link(
                CharOffset::from(3)..CharOffset::from(7),
                "ne\nb".to_string(),
                "www.warp.dev".to_string(),
                selection.clone(),
                ctx,
            );
            let curr_selection = buffer.to_rendered_selection_set(selection.clone(), ctx);
            buffer.push_undo_item(
                prev_selection,
                curr_selection,
                edit_result.undo_item.expect("Should exist"),
                UndoActionType::Atomic,
            );
            assert_eq!(
                buffer.content.debug(),
                "<text><a_www.google.com>li<a><a_www.warp.dev>ne\\nb<a>lock"
            );

            let delta = edit_result.delta.expect("Should exist");
            assert_eq!(delta.old_offset, CharOffset::from(1)..CharOffset::from(11));
            assert_eq!(
                delta.new_lines,
                vec![
                    StyledBufferBlock::Text(StyledTextBlock {
                        block: vec![
                            StyledBufferRun {
                                run: "li".to_string(),
                                text_styles: TextStylesWithMetadata::default()
                                    .link("www.google.com".to_string()),
                                block_style: BufferBlockStyle::PlainText
                            },
                            StyledBufferRun {
                                run: "ne\n".to_string(),
                                text_styles: TextStylesWithMetadata::default()
                                    .link("www.warp.dev".to_string()),
                                block_style: BufferBlockStyle::PlainText
                            }
                        ],
                        style: BufferBlockStyle::PlainText,
                        content_length: CharOffset::from(5)
                    }),
                    StyledBufferBlock::Text(StyledTextBlock {
                        block: vec![
                            StyledBufferRun {
                                run: "b".to_string(),
                                text_styles: TextStylesWithMetadata::default()
                                    .link("www.warp.dev".to_string()),
                                block_style: BufferBlockStyle::PlainText
                            },
                            StyledBufferRun {
                                run: "lock".to_string(),
                                text_styles: TextStylesWithMetadata::default(),
                                block_style: BufferBlockStyle::PlainText
                            }
                        ],
                        style: BufferBlockStyle::PlainText,
                        content_length: CharOffset::from(5)
                    })
                ]
            );

            buffer.undo(selection.clone(), ctx);
            assert_eq!(
                buffer.content.debug(),
                "<text><a_www.google.com>line<a>\\nblock"
            );

            buffer.redo(selection.clone(), ctx);
            assert_eq!(
                buffer.content.debug(),
                "<text><a_www.google.com>li<a><a_www.warp.dev>ne\\nb<a>lock"
            );
        });
    });
}

#[test]
fn test_link_style_surrounded() {
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            buffer.edit_internal_first_selection(
                CharOffset::from(1)..CharOffset::from(1),
                "line\nblock",
                Default::default(),
                selection.clone(),
                ctx,
            );
            assert_eq!(buffer.content.debug(), "<text>line\\nblock");
            buffer.select_and_style_link(
                CharOffset::from(1)..CharOffset::from(5),
                "line".to_string(),
                "www.google.com".to_string(),
                selection.clone(),
                ctx,
            );
            assert_eq!(
                buffer.content.debug(),
                "<text><a_www.google.com>line<a>\\nblock"
            );

            let prev_selection = buffer.to_rendered_selection_set(selection.clone(), ctx);
            let edit_result = buffer.select_and_style_link(
                CharOffset::from(3)..CharOffset::from(5),
                "ne".to_string(),
                "www.warp.dev".to_string(),
                selection.clone(),
                ctx,
            );
            let curr_selection = buffer.to_rendered_selection_set(selection.clone(), ctx);
            buffer.push_undo_item(
                prev_selection,
                curr_selection,
                edit_result.undo_item.expect("Should exist"),
                UndoActionType::Atomic,
            );
            assert_eq!(
                buffer.content.debug(),
                "<text><a_www.google.com>li<a><a_www.warp.dev>ne<a>\\nblock"
            );

            let delta = edit_result.delta.expect("Should exist");
            assert_eq!(delta.old_offset, CharOffset::from(1)..CharOffset::from(6));
            assert_eq!(
                delta.new_lines,
                vec![StyledBufferBlock::Text(StyledTextBlock {
                    block: vec![
                        StyledBufferRun {
                            run: "li".to_string(),
                            text_styles: TextStylesWithMetadata::default()
                                .link("www.google.com".to_string()),
                            block_style: BufferBlockStyle::PlainText
                        },
                        StyledBufferRun {
                            run: "ne".to_string(),
                            text_styles: TextStylesWithMetadata::default()
                                .link("www.warp.dev".to_string()),
                            block_style: BufferBlockStyle::PlainText
                        },
                        StyledBufferRun {
                            run: "\n".to_string(),
                            text_styles: TextStylesWithMetadata::default(),
                            block_style: BufferBlockStyle::PlainText
                        },
                    ],
                    style: BufferBlockStyle::PlainText,
                    content_length: CharOffset::from(5)
                }),]
            );

            buffer.undo(selection.clone(), ctx);
            assert_eq!(
                buffer.content.debug(),
                "<text><a_www.google.com>line<a>\\nblock"
            );

            buffer.redo(selection.clone(), ctx);
            assert_eq!(
                buffer.content.debug(),
                "<text><a_www.google.com>li<a><a_www.warp.dev>ne<a>\\nblock"
            );
        });
    });
}

#[test]
fn test_link_same_url() {
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            buffer.edit_internal_first_selection(
                CharOffset::from(1)..CharOffset::from(1),
                "line\nblock",
                Default::default(),
                selection.clone(),
                ctx,
            );
            assert_eq!(buffer.content.debug(), "<text>line\\nblock");
            buffer.select_and_style_link(
                CharOffset::from(1)..CharOffset::from(5),
                "line".to_string(),
                "www.google.com".to_string(),
                selection.clone(),
                ctx,
            );
            assert_eq!(
                buffer.content.debug(),
                "<text><a_www.google.com>line<a>\\nblock"
            );

            let prev_selection = buffer.to_rendered_selection_set(selection.clone(), ctx);
            let edit_result = buffer.select_and_style_link(
                CharOffset::from(3)..CharOffset::from(7),
                "ne\nb".to_string(),
                "www.google.com".to_string(),
                selection.clone(),
                ctx,
            );
            let curr_selection = buffer.to_rendered_selection_set(selection.clone(), ctx);
            buffer.push_undo_item(
                prev_selection,
                curr_selection,
                edit_result.undo_item.expect("Should exist"),
                UndoActionType::Atomic,
            );
            assert_eq!(
                buffer.content.debug(),
                "<text><a_www.google.com>line\\nb<a>lock"
            );

            let delta = edit_result.delta.expect("Should exist");
            assert_eq!(delta.old_offset, CharOffset::from(1)..CharOffset::from(11));
            assert_eq!(
                delta.new_lines,
                vec![
                    StyledBufferBlock::Text(StyledTextBlock {
                        block: vec![StyledBufferRun {
                            run: "line\n".to_string(),
                            text_styles: TextStylesWithMetadata::default()
                                .link("www.google.com".to_string()),
                            block_style: BufferBlockStyle::PlainText
                        },],
                        style: BufferBlockStyle::PlainText,
                        content_length: CharOffset::from(5)
                    }),
                    StyledBufferBlock::Text(StyledTextBlock {
                        block: vec![
                            StyledBufferRun {
                                run: "b".to_string(),
                                text_styles: TextStylesWithMetadata::default()
                                    .link("www.google.com".to_string()),
                                block_style: BufferBlockStyle::PlainText
                            },
                            StyledBufferRun {
                                run: "lock".to_string(),
                                text_styles: TextStylesWithMetadata::default(),
                                block_style: BufferBlockStyle::PlainText
                            },
                        ],
                        style: BufferBlockStyle::PlainText,
                        content_length: CharOffset::from(5)
                    })
                ]
            );

            buffer.undo(selection.clone(), ctx);
            assert_eq!(
                buffer.content.debug(),
                "<text><a_www.google.com>line<a>\\nblock"
            );

            buffer.redo(selection.clone(), ctx);
            assert_eq!(
                buffer.content.debug(),
                "<text><a_www.google.com>line\\nb<a>lock"
            );
        });
    });
}

#[test]
fn test_style_link_insertion() {
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            buffer.edit_internal_first_selection(
                CharOffset::from(1)..CharOffset::from(1),
                "some text",
                Default::default(),
                selection.clone(),
                ctx,
            );

            // Insert a link, not re-styling any existing text.
            buffer.select_and_style_link(
                CharOffset::from(6)..CharOffset::from(6),
                "link".to_string(),
                "https://example.com".to_string(),
                selection.clone(),
                ctx,
            );

            assert_eq!(
                buffer.debug(),
                "<text>some <a_https://example.com>link<a>text"
            );

            // Edit the link's tag to be shorter.
            buffer.select_and_style_link(
                CharOffset::from(6)..CharOffset::from(10),
                "k".to_string(),
                "https://example.com".to_string(),
                selection.clone(),
                ctx,
            );
            assert_eq!(buffer.debug(), "<text>some <a_https://example.com>k<a>text");

            // Edit the link's tag to be longer.
            buffer.select_and_style_link(
                CharOffset::from(6)..CharOffset::from(7),
                "long".to_string(),
                "https://example.com".to_string(),
                selection.clone(),
                ctx,
            );
            assert_eq!(
                buffer.debug(),
                "<text>some <a_https://example.com>long<a>text"
            );
        });
    });
}

#[test]
fn test_unstyle_link_exact() {
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            buffer.edit_internal_first_selection(
                CharOffset::from(1)..CharOffset::from(1),
                "line\nblock",
                Default::default(),
                selection.clone(),
                ctx,
            );
            assert_eq!(buffer.content.debug(), "<text>line\\nblock");
            buffer.select_and_style_link(
                CharOffset::from(1)..CharOffset::from(5),
                "line".to_string(),
                "www.google.com".to_string(),
                selection.clone(),
                ctx,
            );
            assert_eq!(
                buffer.content.debug(),
                "<text><a_www.google.com>line<a>\\nblock"
            );

            let prev_selection = buffer.to_rendered_selection_set(selection.clone(), ctx);
            buffer.set_selection(
                CharOffset::from(1)..CharOffset::from(5),
                selection.clone(),
                ctx,
            );
            let edit_result = buffer.unstyle_link_internal(selection.clone(), ctx);
            let curr_selection = buffer.to_rendered_selection_set(selection.clone(), ctx);
            buffer.push_undo_item(
                prev_selection,
                curr_selection,
                edit_result.undo_item.expect("Should exist"),
                UndoActionType::Atomic,
            );
            assert_eq!(buffer.content.debug(), "<text>line\\nblock");

            let delta = edit_result.delta.expect("Should exist");
            assert_eq!(delta.old_offset, CharOffset::from(1)..CharOffset::from(6));
            assert_eq!(
                delta.new_lines,
                vec![StyledBufferBlock::Text(StyledTextBlock {
                    block: vec![StyledBufferRun {
                        run: "line\n".to_string(),
                        text_styles: TextStylesWithMetadata::default(),
                        block_style: BufferBlockStyle::PlainText
                    },],
                    style: BufferBlockStyle::PlainText,
                    content_length: CharOffset::from(5)
                }),]
            );

            buffer.undo(selection.clone(), ctx);
            assert_eq!(
                buffer.content.debug(),
                "<text><a_www.google.com>line<a>\\nblock"
            );

            buffer.redo(selection.clone(), ctx);
            assert_eq!(buffer.content.debug(), "<text>line\\nblock");
        });
    });
}

#[test]
fn test_unstyle_link_overlapping() {
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            buffer.edit_internal_first_selection(
                CharOffset::from(1)..CharOffset::from(1),
                "line\nblock",
                Default::default(),
                selection.clone(),
                ctx,
            );
            assert_eq!(buffer.content.debug(), "<text>line\\nblock");
            buffer.select_and_style_link(
                CharOffset::from(1)..CharOffset::from(4),
                "lin".to_string(),
                "www.google.com".to_string(),
                selection.clone(),
                ctx,
            );
            assert_eq!(
                buffer.content.debug(),
                "<text><a_www.google.com>lin<a>e\\nblock"
            );

            buffer.select_and_style_link(
                CharOffset::from(4)..CharOffset::from(7),
                "e\nb".to_string(),
                "www.warp.dev".to_string(),
                selection.clone(),
                ctx,
            );
            assert_eq!(
                buffer.content.debug(),
                "<text><a_www.google.com>lin<a><a_www.warp.dev>e\\nb<a>lock"
            );

            let prev_selection = buffer.to_rendered_selection_set(selection.clone(), ctx);
            buffer.set_selection(
                CharOffset::from(2)..CharOffset::from(5),
                selection.clone(),
                ctx,
            );
            let edit_result = buffer.unstyle_link_internal(selection.clone(), ctx);
            let curr_selection = buffer.to_rendered_selection_set(selection.clone(), ctx);
            buffer.push_undo_item(
                prev_selection,
                curr_selection,
                edit_result.undo_item.expect("Should exist"),
                UndoActionType::Atomic,
            );
            assert_eq!(
                buffer.content.debug(),
                "<text><a_www.google.com>l<a>ine<a_www.warp.dev>\\nb<a>lock"
            );

            let delta = edit_result.delta.expect("Should exist");
            assert_eq!(delta.old_offset, CharOffset::from(1)..CharOffset::from(11));
            assert_eq!(
                delta.new_lines,
                vec![
                    StyledBufferBlock::Text(StyledTextBlock {
                        block: vec![
                            StyledBufferRun {
                                run: "l".to_string(),
                                text_styles: TextStylesWithMetadata::default()
                                    .link("www.google.com".to_string()),
                                block_style: BufferBlockStyle::PlainText
                            },
                            StyledBufferRun {
                                run: "ine".to_string(),
                                text_styles: TextStylesWithMetadata::default(),
                                block_style: BufferBlockStyle::PlainText
                            },
                            StyledBufferRun {
                                run: "\n".to_string(),
                                text_styles: TextStylesWithMetadata::default()
                                    .link("www.warp.dev".to_string()),
                                block_style: BufferBlockStyle::PlainText
                            },
                        ],
                        style: BufferBlockStyle::PlainText,
                        content_length: CharOffset::from(5)
                    }),
                    StyledBufferBlock::Text(StyledTextBlock {
                        block: vec![
                            StyledBufferRun {
                                run: "b".to_string(),
                                text_styles: TextStylesWithMetadata::default()
                                    .link("www.warp.dev".to_string()),
                                block_style: BufferBlockStyle::PlainText
                            },
                            StyledBufferRun {
                                run: "lock".to_string(),
                                text_styles: TextStylesWithMetadata::default(),
                                block_style: BufferBlockStyle::PlainText
                            },
                        ],
                        style: BufferBlockStyle::PlainText,
                        content_length: CharOffset::from(5)
                    })
                ]
            );

            buffer.undo(selection.clone(), ctx);
            assert_eq!(
                buffer.content.debug(),
                "<text><a_www.google.com>lin<a><a_www.warp.dev>e\\nb<a>lock"
            );

            buffer.redo(selection.clone(), ctx);
            assert_eq!(
                buffer.content.debug(),
                "<text><a_www.google.com>l<a>ine<a_www.warp.dev>\\nb<a>lock"
            );
        });
    });
}

#[test]
fn test_unstyle_link_noop() {
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            buffer.edit_internal_first_selection(
                CharOffset::from(1)..CharOffset::from(1),
                "line\nblock",
                Default::default(),
                selection.clone(),
                ctx,
            );
            assert_eq!(buffer.content.debug(), "<text>line\\nblock");
            buffer.select_and_style_link(
                CharOffset::from(1)..CharOffset::from(4),
                "lin".to_string(),
                "www.google.com".to_string(),
                selection.clone(),
                ctx,
            );
            assert_eq!(
                buffer.content.debug(),
                "<text><a_www.google.com>lin<a>e\\nblock"
            );

            buffer.set_selection(
                CharOffset::from(4)..CharOffset::from(5),
                selection.clone(),
                ctx,
            );
            let edit_result = buffer.unstyle_link_internal(selection.clone(), ctx);
            assert_eq!(
                buffer.content.debug(),
                "<text><a_www.google.com>lin<a>e\\nblock"
            );
            assert!(edit_result.delta.is_none());
        });
    });
}

#[test]
fn test_edit_after_hyperlink() {
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            buffer.edit_internal_first_selection(
                CharOffset::from(1)..CharOffset::from(1),
                "line\nblock",
                Default::default(),
                selection.clone(),
                ctx,
            );
            assert_eq!(buffer.content.debug(), "<text>line\\nblock");
            buffer.select_and_style_link(
                CharOffset::from(1)..CharOffset::from(4),
                "lin".to_string(),
                "www.google.com".to_string(),
                selection.clone(),
                ctx,
            );
            assert_eq!(
                buffer.content.debug(),
                "<text><a_www.google.com>lin<a>e\\nblock"
            );

            buffer.edit_internal_first_selection(
                CharOffset::from(4)..CharOffset::from(4),
                "ab",
                TextStyles::default(),
                selection.clone(),
                ctx,
            );
            assert_eq!(
                buffer.content.debug(),
                "<text><a_www.google.com>lin<a>abe\\nblock"
            );
        });
    });
}

#[test]
fn test_code_tab_indent() {
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| {
            Buffer::new(Box::new(|block_style, _| match block_style {
                BufferBlockStyle::CodeBlock { .. } => {
                    IndentBehavior::TabIndent(IndentUnit::Space(4))
                }
                _ => IndentBehavior::Ignore,
            }))
        });
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            buffer.edit_internal_first_selection(
                CharOffset::from(1)..CharOffset::from(1),
                "first\n second",
                Default::default(),
                selection.clone(),
                ctx,
            );
            buffer.block_style_range(
                CharOffset::from(1)..CharOffset::from(14),
                BufferBlockStyle::CodeBlock {
                    code_block_type: CodeBlockType::Code {
                        lang: "Rust".to_string(),
                    },
                },
                selection.clone(),
                ctx,
            );
            assert_eq!(buffer.debug(), "<code:Rust>first\\n second<text>");

            // Partially-select each line and indent them. This adds 4 spaces to the first line, but only 3
            // to the second, resulting in an overall indent of 4 for each (one tab stop).
            buffer.set_selection(
                CharOffset::from(4)..CharOffset::from(10),
                selection.clone(),
                ctx,
            );
            let prev_selection = buffer.to_rendered_selection_set(selection.clone(), ctx);
            let edit_result = buffer.indent(1, selection.clone(), ctx);
            let curr_selection = buffer.to_rendered_selection_set(selection.clone(), ctx);
            buffer.push_undo_item(
                prev_selection,
                curr_selection,
                edit_result.undo_item.unwrap(),
                UndoActionType::Atomic,
            );

            assert_eq!(buffer.debug(), "<code:Rust>    first\\n    second<text>");

            buffer.undo(selection.clone(), ctx);
            assert_eq!(buffer.debug(), "<code:Rust>first\\n second<text>");
            buffer.redo(selection.clone(), ctx);

            // Indent the first line further. This should add 4 spaces, but not affect the second line.
            selection.update(ctx, |selection, _| {
                selection.set_single_cursor(5.into());
            });
            buffer.indent(1, selection.clone(), ctx);
            assert_eq!(
                buffer.debug(),
                "<code:Rust>        first\\n    second<text>"
            );

            // Indent the second line further, but starting 2 characters into the line. This results in
            // adding 2 more spaces, not a full tab.
            selection.update(ctx, |selection, _| {
                selection.set_single_cursor(17.into());
            });
            buffer.indent(1, selection.clone(), ctx);
            assert_eq!(
                buffer.debug(),
                "<code:Rust>        first\\n      second<text>"
            );
        });
    });
}

#[test]
fn test_indent_with_tab_unit() {
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| {
            Buffer::new(Box::new(|block_style, _| match block_style {
                BufferBlockStyle::CodeBlock { .. } => IndentBehavior::TabIndent(IndentUnit::Tab),
                _ => IndentBehavior::Ignore,
            }))
        });
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            buffer.edit_internal_first_selection(
                CharOffset::from(1)..CharOffset::from(1),
                "first\n second",
                Default::default(),
                selection.clone(),
                ctx,
            );
            buffer.block_style_range(
                CharOffset::from(1)..CharOffset::from(14),
                BufferBlockStyle::CodeBlock {
                    code_block_type: CodeBlockType::Code {
                        lang: "Rust".to_string(),
                    },
                },
                selection.clone(),
                ctx,
            );
            assert_eq!(buffer.debug(), "<code:Rust>first\\n second<text>");

            // Partially-select each line and indent them. This adds 4 spaces to the first line, but only 3
            // to the second, resulting in an overall indent of 4 for each (one tab stop).
            buffer.set_selection(
                CharOffset::from(4)..CharOffset::from(10),
                selection.clone(),
                ctx,
            );
            let prev_selection = buffer.to_rendered_selection_set(selection.clone(), ctx);
            let edit_result = buffer.indent(1, selection.clone(), ctx);
            let curr_selection = buffer.to_rendered_selection_set(selection.clone(), ctx);
            buffer.push_undo_item(
                prev_selection,
                curr_selection,
                edit_result.undo_item.unwrap(),
                UndoActionType::Atomic,
            );

            assert_eq!(buffer.debug(), "<code:Rust>\tfirst\\n \tsecond<text>");

            buffer.undo(selection.clone(), ctx);
            assert_eq!(buffer.debug(), "<code:Rust>first\\n second<text>");
            buffer.redo(selection.clone(), ctx);

            // Indent the first line further. This should add 4 spaces, but not affect the second line.
            selection.update(ctx, |selection, _| {
                selection.set_single_cursor(1.into());
            });
            buffer.indent(1, selection.clone(), ctx);
            assert_eq!(buffer.debug(), "<code:Rust>\t\tfirst\\n \tsecond<text>");

            // Indent the second line further, but starting 2 characters into the line. This results in
            // adding 2 more spaces, not a full tab.
            selection.update(ctx, |selection, _| {
                selection.set_single_cursor(10.into());
            });
            buffer.indent(1, selection.clone(), ctx);
            assert_eq!(buffer.debug(), "<code:Rust>\t\tfirst\\n \t\tsecond<text>");

            selection.update(ctx, |selection, _| {
                selection.set_single_cursor(2.into());
            });
            buffer.unindent(selection.clone(), ctx);
            assert_eq!(buffer.debug(), "<code:Rust>\tfirst\\n \t\tsecond<text>");

            selection.update(ctx, |selection, _| {
                selection.set_single_cursor(11.into());
            });
            buffer.backspace(&mut None, selection.clone(), ctx);
            assert_eq!(buffer.debug(), "<code:Rust>\tfirst\\n \tsecond<text>");
        });
    });
}

#[test]
fn test_code_tab_within_line() {
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| {
            Buffer::new(Box::new(|block_style, _| match block_style {
                BufferBlockStyle::CodeBlock { .. } => {
                    IndentBehavior::TabIndent(IndentUnit::Space(4))
                }
                _ => IndentBehavior::Ignore,
            }))
        });
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            buffer.edit_internal_first_selection(
                CharOffset::from(1)..CharOffset::from(1),
                "git log",
                Default::default(),
                selection.clone(),
                ctx,
            );
            buffer.block_style_range(
                CharOffset::from(1)..CharOffset::from(8),
                BufferBlockStyle::CodeBlock {
                    code_block_type: CodeBlockType::Shell,
                },
                selection.clone(),
                ctx,
            );
            assert_eq!(buffer.debug(), "<code:Shell>git log<text>");

            // Since the cursor is 1 character into the line, 3 more spaces are added.
            selection.update(ctx, |selection, _| {
                selection.set_single_cursor(2.into());
            });
            buffer.indent(1, selection.clone(), ctx);
            assert_eq!(buffer.debug(), "<code:Shell>g   it log<text>");

            // The `l` is 8 characters in, so a full 4 spaces are added.
            selection.update(ctx, |selection, _| {
                selection.set_single_cursor(9.into());
            });
            buffer.indent(1, selection.clone(), ctx);
            assert_eq!(buffer.debug(), "<code:Shell>g   it l    og<text>");

            // Tab at the start of the line also adds 4 spaces.
            selection.update(ctx, |selection, _| {
                selection.set_single_cursor(1.into());
            });
            buffer.indent(1, selection.clone(), ctx);
            assert_eq!(buffer.debug(), "<code:Shell>    g   it l    og<text>");
            assert_eq!(
                selection.as_ref(ctx).selection_to_first_offset_range(),
                5.into()..5.into()
            );
        });
    });
}

#[test]
fn test_code_unindent() {
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| {
            Buffer::new(Box::new(|block_style, _| match block_style {
                BufferBlockStyle::CodeBlock { .. } => {
                    IndentBehavior::TabIndent(IndentUnit::Space(4))
                }
                _ => IndentBehavior::Ignore,
            }))
        });
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            buffer.edit_internal_first_selection(
                1.into()..1.into(),
                "    git log\n  ls\n     cat file.txt",
                Default::default(),
                selection.clone(),
                ctx,
            );
            buffer.block_style_range(
                1.into()..buffer.max_charoffset(),
                BufferBlockStyle::CodeBlock {
                    code_block_type: CodeBlockType::Shell,
                },
                selection.clone(),
                ctx,
            );
            assert_eq!(
                buffer.debug(),
                "<code:Shell>    git log\\n  ls\\n     cat file.txt<text>"
            );

            // Unindent all 3 lines.
            let max_charoffset = buffer.max_charoffset();
            buffer.set_selection(1.into()..max_charoffset, selection.clone(), ctx);
            buffer.unindent(selection.clone(), ctx);
            // `git log` was indented by 4 spaces, so all are removed.
            // `ls` was indented by 2 spaces, both of which are removed (but only the spaces).
            // `cat file.txt` was indented by 5 spaces, 1 of which is removed so that it's evenly indented.
            assert_eq!(
                buffer.debug(),
                "<code:Shell>git log\\nls\\n    cat file.txt<text>"
            );

            // Unindent the last line from a single cursor. Even though the character at the cursor is a
            // space, unindent only ever applies to leading indentation.
            selection.update(ctx, |selection, _| {
                selection.set_single_cursor(19.into());
            });
            buffer.unindent(selection.clone(), ctx);
            assert_eq!(
                buffer.debug(),
                "<code:Shell>git log\\nls\\ncat file.txt<text>"
            );
        });
    });
}

#[test]
fn test_code_backspace_unindent() {
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| {
            Buffer::new(Box::new(|block_style, _| match block_style {
                BufferBlockStyle::CodeBlock { .. } => {
                    IndentBehavior::TabIndent(IndentUnit::Space(4))
                }
                _ => IndentBehavior::Ignore,
            }))
        });
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            buffer.edit_internal_first_selection(
                1.into()..1.into(),
                "fn main() {\n        panic!();\n }",
                Default::default(),
                selection.clone(),
                ctx,
            );
            buffer.block_style_range(
                1.into()..buffer.max_charoffset(),
                BufferBlockStyle::CodeBlock {
                    code_block_type: CodeBlockType::Code {
                        lang: "Rust".to_string(),
                    },
                },
                selection.clone(),
                ctx,
            );
            assert_eq!(
                buffer.debug(),
                "<code:Rust>fn main() {\\n        panic!();\\n }<text>"
            );

            // At the start of the panic! line, backspace should remove a tab of indentation.
            selection.update(ctx, |selection, _| {
                selection.set_single_cursor(21.into());
            });
            buffer.backspace(&mut None, selection.clone(), ctx);
            assert_eq!(
                buffer.debug(),
                "<code:Rust>fn main() {\\n    panic!();\\n }<text>"
            );

            // Within the panic! line's whitespace, backspace should remove some of the indentation.
            selection.update(ctx, |selection, _| {
                selection.set_single_cursor(15.into());
            });
            buffer.backspace(&mut None, selection.clone(), ctx);
            assert_eq!(
                buffer.debug(),
                "<code:Rust>fn main() {\\n  panic!();\\n }<text>"
            );

            // At the start of the }, line, backspace should remove the single space.
            selection.update(ctx, |selection, _| {
                selection.set_single_cursor(26.into());
            });
            buffer.backspace(&mut None, selection.clone(), ctx);
            assert_eq!(
                buffer.debug(),
                "<code:Rust>fn main() {\\n  panic!();\\n}<text>"
            );

            // Within a line, backspace should remove the previous character.
            selection.update(ctx, |selection, _| {
                selection.set_single_cursor(5.into());
            });
            buffer.backspace(&mut None, selection.clone(), ctx);
            assert_eq!(
                buffer.debug(),
                "<code:Rust>fn ain() {\\n  panic!();\\n}<text>"
            );

            // At the start of an interior line, backspace should remove the newline.
            selection.update(ctx, |selection, _| {
                selection.set_single_cursor(12.into());
            });
            buffer.backspace(&mut None, selection.clone(), ctx);
            assert_eq!(buffer.debug(), "<code:Rust>fn ain() {  panic!();\\n}<text>");

            // At the start of the block, backspace should unstyle it.
            selection.update(ctx, |selection, _| {
                selection.set_single_cursor(1.into());
            });
            buffer.backspace(&mut None, selection.clone(), ctx);
            assert_eq!(buffer.debug(), "<text>fn ain() {  panic!();\\n}\\n");
        });
    });
}

#[test]
fn test_list_tab_behavior() {
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| {
            Buffer::new(Box::new(|block_style, _| match block_style {
                BufferBlockStyle::UnorderedList {
                    indent_level: ListIndentLevel::Three,
                } => IndentBehavior::Ignore,
                BufferBlockStyle::UnorderedList { indent_level } => {
                    IndentBehavior::Restyle(BufferBlockStyle::UnorderedList {
                        indent_level: indent_level.shift_right(),
                    })
                }
                _ => IndentBehavior::Ignore,
            }))
        });
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            buffer.edit_internal_first_selection(
                CharOffset::from(1)..CharOffset::from(1),
                "test\nline\nsecond",
                Default::default(),
                selection.clone(),
                ctx,
            );
            let _ = buffer.block_style_range(
                CharOffset::from(6)..CharOffset::from(10),
                BufferBlockStyle::UnorderedList {
                    indent_level: ListIndentLevel::One,
                },
                selection.clone(),
                ctx,
            );
            assert_eq!(buffer.content.debug(), "<text>test<ul0>line<text>second");

            let _ = buffer.edit_internal_first_selection(
                CharOffset::from(10)..CharOffset::from(10),
                "\n",
                TextStyles::default(),
                selection.clone(),
                ctx,
            );
            assert_eq!(
                buffer.content.debug(),
                "<text>test<ul0>line<ul0><text>second"
            );

            buffer.set_selection(
                CharOffset::from(11)..CharOffset::from(11),
                selection.clone(),
                ctx,
            );
            let prev_selection = buffer.to_rendered_selection_set(selection.clone(), ctx);
            let edit_result = buffer.indent(1, selection.clone(), ctx);
            assert_eq!(
                buffer.content.debug(),
                "<text>test<ul0>line<ul1><text>second"
            );

            let curr_selection = buffer.to_rendered_selection_set(selection.clone(), ctx);
            buffer.push_undo_item(
                prev_selection,
                curr_selection,
                edit_result.undo_item.expect("Should exist"),
                UndoActionType::Atomic,
            );

            let delta = edit_result.delta.expect("Should exist");
            assert_eq!(delta.old_offset, CharOffset::from(11)..CharOffset::from(12));
            assert_eq!(
                delta.new_lines,
                vec![StyledBufferBlock::Text(StyledTextBlock {
                    block: vec![StyledBufferRun {
                        run: "\n".to_string(),
                        text_styles: TextStylesWithMetadata::default(),
                        block_style: BufferBlockStyle::UnorderedList {
                            indent_level: ListIndentLevel::Two
                        }
                    },],
                    style: BufferBlockStyle::UnorderedList {
                        indent_level: ListIndentLevel::Two,
                    },
                    content_length: CharOffset::from(1)
                }),]
            );

            buffer.undo(selection.clone(), ctx);
            assert_eq!(
                buffer.content.debug(),
                "<text>test<ul0>line<ul0><text>second"
            );
            buffer.redo(selection.clone(), ctx);
            assert_eq!(
                buffer.content.debug(),
                "<text>test<ul0>line<ul1><text>second"
            );

            buffer.set_selection(
                CharOffset::from(11)..CharOffset::from(11),
                selection.clone(),
                ctx,
            );
            let prev_selection = buffer.to_rendered_selection_set(selection.clone(), ctx);
            let edit_result = buffer.indent(1, selection.clone(), ctx);
            assert_eq!(
                buffer.content.debug(),
                "<text>test<ul0>line<ul2><text>second"
            );
            let curr_selection = buffer.to_rendered_selection_set(selection.clone(), ctx);
            buffer.push_undo_item(
                prev_selection,
                curr_selection,
                edit_result.undo_item.expect("Should exist"),
                UndoActionType::Atomic,
            );

            let delta = edit_result.delta.expect("Should exist");
            assert_eq!(delta.old_offset, CharOffset::from(11)..CharOffset::from(12));
            assert_eq!(
                delta.new_lines,
                vec![StyledBufferBlock::Text(StyledTextBlock {
                    block: vec![StyledBufferRun {
                        run: "\n".to_string(),
                        text_styles: TextStylesWithMetadata::default(),
                        block_style: BufferBlockStyle::UnorderedList {
                            indent_level: ListIndentLevel::Three
                        }
                    },],
                    style: BufferBlockStyle::UnorderedList {
                        indent_level: ListIndentLevel::Three,
                    },
                    content_length: CharOffset::from(1)
                }),]
            );

            buffer.undo(selection.clone(), ctx);
            assert_eq!(
                buffer.content.debug(),
                "<text>test<ul0>line<ul1><text>second"
            );
            buffer.redo(selection.clone(), ctx);
            assert_eq!(
                buffer.content.debug(),
                "<text>test<ul0>line<ul2><text>second"
            );
        });
    });
}

#[test]
fn test_ordered_list_tab_behavior() {
    // Ordered list should have the same tab behavior as unordered list.
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| {
            Buffer::new(Box::new(|block_style, _| match block_style {
                BufferBlockStyle::OrderedList {
                    indent_level: ListIndentLevel::Three,
                    ..
                } => IndentBehavior::Ignore,
                BufferBlockStyle::OrderedList {
                    indent_level,
                    number,
                } => IndentBehavior::Restyle(BufferBlockStyle::OrderedList {
                    indent_level: indent_level.shift_right(),
                    number: *number,
                }),
                _ => IndentBehavior::Ignore,
            }))
        });
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            buffer.edit_internal_first_selection(
                CharOffset::from(1)..CharOffset::from(1),
                "test\nline\nsecond",
                Default::default(),
                selection.clone(),
                ctx,
            );
            let _ = buffer.block_style_range(
                CharOffset::from(6)..CharOffset::from(10),
                BufferBlockStyle::ordered_list(ListIndentLevel::One),
                selection.clone(),
                ctx,
            );
            assert_eq!(buffer.content.debug(), "<text>test<ol0>line<text>second");

            let _ = buffer.edit_internal_first_selection(
                CharOffset::from(10)..CharOffset::from(10),
                "\n",
                TextStyles::default(),
                selection.clone(),
                ctx,
            );
            assert_eq!(
                buffer.content.debug(),
                "<text>test<ol0>line<ol0><text>second"
            );

            buffer.set_selection(
                CharOffset::from(11)..CharOffset::from(11),
                selection.clone(),
                ctx,
            );
            let prev_selection = buffer.to_rendered_selection_set(selection.clone(), ctx);
            let edit_result = buffer.indent(1, selection.clone(), ctx);
            assert_eq!(
                buffer.content.debug(),
                "<text>test<ol0>line<ol1><text>second"
            );

            let curr_selection = buffer.to_rendered_selection_set(selection.clone(), ctx);
            buffer.push_undo_item(
                prev_selection,
                curr_selection,
                edit_result.undo_item.expect("Should exist"),
                UndoActionType::Atomic,
            );

            let delta = edit_result.delta.expect("Should exist");
            assert_eq!(delta.old_offset, CharOffset::from(11)..CharOffset::from(12));
            assert_eq!(
                delta.new_lines,
                vec![StyledBufferBlock::Text(StyledTextBlock {
                    block: vec![StyledBufferRun {
                        run: "\n".to_string(),
                        text_styles: TextStylesWithMetadata::default(),
                        block_style: BufferBlockStyle::ordered_list(ListIndentLevel::Two)
                    },],
                    style: BufferBlockStyle::ordered_list(ListIndentLevel::Two),
                    content_length: CharOffset::from(1)
                }),]
            );

            buffer.undo(selection.clone(), ctx);
            assert_eq!(
                buffer.content.debug(),
                "<text>test<ol0>line<ol0><text>second"
            );
            buffer.redo(selection.clone(), ctx);
            assert_eq!(
                buffer.content.debug(),
                "<text>test<ol0>line<ol1><text>second"
            );
        });
    });
}

#[test]
fn test_task_list_tab_behavior() {
    // Task list should have the same tab behavior as unordered/ordered list.
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| {
            Buffer::new(Box::new(|block_style, _| match block_style {
                BufferBlockStyle::TaskList {
                    indent_level,
                    complete,
                } => IndentBehavior::Restyle(BufferBlockStyle::TaskList {
                    indent_level: indent_level.shift_right(),
                    complete: *complete,
                }),
                _ => IndentBehavior::Ignore,
            }))
        });
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            buffer.edit_internal_first_selection(
                CharOffset::from(1)..CharOffset::from(1),
                "test\nline\nsecond",
                Default::default(),
                selection.clone(),
                ctx,
            );
            let _ = buffer.block_style_range(
                CharOffset::from(6)..CharOffset::from(10),
                BufferBlockStyle::TaskList {
                    indent_level: ListIndentLevel::One,
                    complete: true,
                },
                selection.clone(),
                ctx,
            );
            assert_eq!(
                buffer.content.debug(),
                "<text>test<cl0:true>line<text>second"
            );

            // Note that the new tasklist created from newline should not inherit its parent's completion status.
            let _ = buffer.edit_internal_first_selection(
                CharOffset::from(10)..CharOffset::from(10),
                "\n",
                TextStyles::default(),
                selection.clone(),
                ctx,
            );
            assert_eq!(
                buffer.content.debug(),
                "<text>test<cl0:true>line<cl0:false><text>second"
            );

            buffer.set_selection(
                CharOffset::from(11)..CharOffset::from(11),
                selection.clone(),
                ctx,
            );
            let prev_selection = buffer.to_rendered_selection_set(selection.clone(), ctx);
            let edit_result = buffer.indent(1, selection.clone(), ctx);
            assert_eq!(
                buffer.content.debug(),
                "<text>test<cl0:true>line<cl1:false><text>second"
            );

            let curr_selection = buffer.to_rendered_selection_set(selection.clone(), ctx);
            buffer.push_undo_item(
                prev_selection,
                curr_selection,
                edit_result.undo_item.expect("Should exist"),
                UndoActionType::Atomic,
            );

            let delta = edit_result.delta.expect("Should exist");
            assert_eq!(delta.old_offset, CharOffset::from(11)..CharOffset::from(12));
            assert_eq!(
                delta.new_lines,
                vec![StyledBufferBlock::Text(StyledTextBlock {
                    block: vec![StyledBufferRun {
                        run: "\n".to_string(),
                        text_styles: TextStylesWithMetadata::default(),
                        block_style: BufferBlockStyle::TaskList {
                            indent_level: ListIndentLevel::Two,
                            complete: false
                        }
                    },],
                    style: BufferBlockStyle::TaskList {
                        indent_level: ListIndentLevel::Two,
                        complete: false
                    },
                    content_length: CharOffset::from(1)
                }),]
            );

            buffer.undo(selection.clone(), ctx);
            assert_eq!(
                buffer.content.debug(),
                "<text>test<cl0:true>line<cl0:false><text>second"
            );
            buffer.redo(selection.clone(), ctx);
            assert_eq!(
                buffer.content.debug(),
                "<text>test<cl0:true>line<cl1:false><text>second"
            );
        });
    });
}

#[test]
fn test_ordered_list_numbering_export() {
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            buffer.edit_internal_first_selection(
                1.into()..1.into(),
                "First\nSecond\nA\nB\nThird",
                Default::default(),
                selection.clone(),
                ctx,
            );
            buffer.block_style_range(
                1.into()..6.into(),
                BufferBlockStyle::OrderedList {
                    number: Some(4),
                    indent_level: ListIndentLevel::One,
                },
                selection.clone(),
                ctx,
            );
            buffer.block_style_range(
                7.into()..13.into(),
                BufferBlockStyle::ordered_list(ListIndentLevel::One),
                selection.clone(),
                ctx,
            );
            // This tests starting a new, nested, list with different numbering.
            buffer.block_style_range(
                14.into()..15.into(),
                BufferBlockStyle::OrderedList {
                    number: Some(3),
                    indent_level: ListIndentLevel::Two,
                },
                selection.clone(),
                ctx,
            );
            buffer.block_style_range(
                16.into()..17.into(),
                BufferBlockStyle::ordered_list(ListIndentLevel::Two),
                selection.clone(),
                ctx,
            );
            // This tests going back to automatic numbering at a different indent level.
            buffer.block_style_range(
                18.into()..23.into(),
                BufferBlockStyle::ordered_list(ListIndentLevel::One),
                selection.clone(),
                ctx,
            );
            assert_eq!(
                buffer.debug(),
                "<ol0@4>First<ol0>Second<ol1@3>A<ol1>B<ol0>Third<text>"
            );
            assert_eq!(
                buffer.markdown(),
                "4. First\n5. Second\n    3. A\n    4. B\n6. Third\n"
            );
            assert_eq!(
                buffer.range_as_html(1.into()..buffer.max_charoffset(), ctx).unwrap(),
                "<ol start=\"4\"><li>First</li><li>Second<ol start=\"3\"><li>A</li><li>B</li></ol></li><li>Third</li></ol>"
            );
        });
    });
}

#[test]
fn test_code_block_text_styling() {
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            buffer.edit_internal_first_selection(
                CharOffset::from(1)..CharOffset::from(1),
                "start\nblock\nend",
                Default::default(),
                selection.clone(),
                ctx,
            );
            buffer.block_style_range(
                CharOffset::from(7)..CharOffset::from(12),
                BufferBlockStyle::CodeBlock {
                    code_block_type: Default::default(),
                },
                selection.clone(),
                ctx,
            );
            assert_eq!(
                buffer.content.debug(),
                "<text>start<code:Shell>block<text>end"
            );

            buffer.set_selection(
                CharOffset::from(7)..CharOffset::from(12),
                selection.clone(),
                ctx,
            );
            let edit_result =
                buffer.style_internal(TextStyles::default().bold(), selection.clone(), ctx);
            assert_eq!(
                buffer.content.debug(),
                "<text>start<code:Shell>block<text>end"
            );
            assert!(edit_result.delta.is_none());

            let prev_selection = buffer.to_rendered_selection_set(selection.clone(), ctx);
            buffer.set_selection(
                CharOffset::from(3)..CharOffset::from(10),
                selection.clone(),
                ctx,
            );
            let edit_result =
                buffer.style_internal(TextStyles::default().bold(), selection.clone(), ctx);
            let curr_selection = buffer.to_rendered_selection_set(selection.clone(), ctx);
            buffer.push_undo_item(
                prev_selection,
                curr_selection,
                edit_result.undo_item.expect("Should exist"),
                UndoActionType::Atomic,
            );
            assert_eq!(
                buffer.content.debug(),
                "<text>st<b_s>art<b_e><code:Shell>block<text>end"
            );

            let delta = edit_result.delta.expect("Should exist");
            assert_eq!(delta.old_offset, CharOffset::from(1)..CharOffset::from(7));
            assert_eq!(
                delta.new_lines,
                vec![StyledBufferBlock::Text(StyledTextBlock {
                    block: vec![
                        StyledBufferRun {
                            run: "st".to_string(),
                            text_styles: TextStylesWithMetadata::default(),
                            block_style: BufferBlockStyle::PlainText
                        },
                        StyledBufferRun {
                            run: "art".to_string(),
                            text_styles: TextStylesWithMetadata::default().bold(),
                            block_style: BufferBlockStyle::PlainText
                        },
                        StyledBufferRun {
                            run: "\n".to_string(),
                            text_styles: TextStylesWithMetadata::default(),
                            block_style: BufferBlockStyle::PlainText
                        },
                    ],
                    style: BufferBlockStyle::PlainText,
                    content_length: CharOffset::from(6)
                }),]
            );

            buffer.undo(selection.clone(), ctx);
            assert_eq!(
                buffer.content.debug(),
                "<text>start<code:Shell>block<text>end"
            );

            buffer.redo(selection.clone(), ctx);
            assert_eq!(
                buffer.content.debug(),
                "<text>st<b_s>art<b_e><code:Shell>block<text>end"
            );

            let prev_selection = buffer.to_rendered_selection_set(selection.clone(), ctx);
            buffer.set_selection(
                CharOffset::from(10)..CharOffset::from(16),
                selection.clone(),
                ctx,
            );
            let edit_result =
                buffer.style_internal(TextStyles::default().italic(), selection.clone(), ctx);
            let curr_selection = buffer.to_rendered_selection_set(selection.clone(), ctx);
            buffer.push_undo_item(
                prev_selection,
                curr_selection,
                edit_result.undo_item.expect("Should exist"),
                UndoActionType::Atomic,
            );
            assert_eq!(
                buffer.content.debug(),
                "<text>st<b_s>art<b_e><code:Shell>block<text><i_s>end<i_e>"
            );

            let delta = edit_result.delta.expect("Should exist");
            assert_eq!(delta.old_offset, CharOffset::from(13)..CharOffset::from(16));
            assert_eq!(
                delta.new_lines,
                vec![StyledBufferBlock::Text(StyledTextBlock {
                    block: vec![StyledBufferRun {
                        run: "end".to_string(),
                        text_styles: TextStylesWithMetadata::default().italic(),
                        block_style: BufferBlockStyle::PlainText
                    },],
                    style: BufferBlockStyle::PlainText,
                    content_length: CharOffset::from(3)
                }),]
            );

            buffer.undo(selection.clone(), ctx);
            assert_eq!(
                buffer.content.debug(),
                "<text>st<b_s>art<b_e><code:Shell>block<text>end"
            );

            buffer.redo(selection.clone(), ctx);
            assert_eq!(
                buffer.content.debug(),
                "<text>st<b_s>art<b_e><code:Shell>block<text><i_s>end<i_e>"
            );
        });
    });
}

#[test]
fn test_code_block_styling_over_styled_text() {
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            buffer.edit_internal_first_selection(
                CharOffset::from(1)..CharOffset::from(1),
                "block",
                Default::default(),
                selection.clone(),
                ctx,
            );

            buffer.set_selection(
                CharOffset::from(1)..CharOffset::from(4),
                selection.clone(),
                ctx,
            );
            let _ = buffer.style_internal(TextStyles::default().bold(), selection.clone(), ctx);

            buffer.set_selection(
                CharOffset::from(3)..CharOffset::from(6),
                selection.clone(),
                ctx,
            );
            let _ = buffer.style_internal(TextStyles::default().italic(), selection.clone(), ctx);

            assert_eq!(buffer.content.debug(), "<text><b_s>bl<i_s>o<b_e>ck<i_e>");

            let prev_selection = buffer.to_rendered_selection_set(selection.clone(), ctx);
            let edit_result = buffer.block_style_range(
                CharOffset::from(3)..CharOffset::from(4),
                BufferBlockStyle::CodeBlock {
                    code_block_type: Default::default(),
                },
                selection.clone(),
                ctx,
            );
            let curr_selection = buffer.to_rendered_selection_set(selection.clone(), ctx);
            buffer.push_undo_item(
                prev_selection,
                curr_selection,
                edit_result.undo_item.expect("Should exist"),
                UndoActionType::Atomic,
            );
            assert_eq!(
                buffer.content.debug(),
                "<text><b_s>bl<b_e><code:Shell>o<text><i_s>ck<i_e>"
            );

            let delta = edit_result.delta.expect("Should exist");
            assert_eq!(delta.old_offset, CharOffset::from(1)..CharOffset::from(6));
            assert_eq!(
                delta.new_lines,
                vec![
                    StyledBufferBlock::Text(StyledTextBlock {
                        block: vec![
                            StyledBufferRun {
                                run: "bl".to_string(),
                                text_styles: TextStylesWithMetadata::default().bold(),
                                block_style: BufferBlockStyle::PlainText
                            },
                            StyledBufferRun {
                                run: "\n".to_string(),
                                text_styles: TextStylesWithMetadata::default(),
                                block_style: BufferBlockStyle::PlainText
                            },
                        ],
                        style: BufferBlockStyle::PlainText,
                        content_length: CharOffset::from(3)
                    }),
                    StyledBufferBlock::Text(StyledTextBlock {
                        block: vec![StyledBufferRun {
                            run: "o\n".to_string(),
                            text_styles: TextStylesWithMetadata::default(),
                            block_style: BufferBlockStyle::CodeBlock {
                                code_block_type: Default::default()
                            }
                        },],
                        style: BufferBlockStyle::CodeBlock {
                            code_block_type: Default::default(),
                        },
                        content_length: CharOffset::from(2)
                    }),
                    StyledBufferBlock::Text(StyledTextBlock {
                        block: vec![StyledBufferRun {
                            run: "ck".to_string(),
                            text_styles: TextStylesWithMetadata::default().italic(),
                            block_style: BufferBlockStyle::PlainText
                        },],
                        style: BufferBlockStyle::PlainText,
                        content_length: CharOffset::from(2)
                    }),
                ]
            );

            buffer.undo(selection.clone(), ctx);
            assert_eq!(buffer.content.debug(), "<text><b_s>bl<i_s>o<b_e>ck<i_e>");

            buffer.redo(selection.clone(), ctx);
            assert_eq!(
                buffer.content.debug(),
                "<text><b_s>bl<b_e><code:Shell>o<text><i_s>ck<i_e>"
            );
        });
    });
}

// Regression test for CLD-751.
#[test]
fn test_insert_link_at_end_of_line() {
    App::test((), |mut app| async move {
        let (buffer, _selection) = Buffer::mock_from_markdown(
            "* [test](link)\n```\nabc\n```\n",
            None,
            Box::new(|_, _| IndentBehavior::Ignore),
            &mut app,
        );
        buffer.read(&app, |buffer, _| {
            assert_eq!(
                buffer.content.debug(),
                "<ul0><a_link>test<a><code:Shell>abc<text>"
            );
        });
    });
}

#[test]
fn test_copy_paste_behavior() {
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            buffer.edit_internal_first_selection(
                CharOffset::from(1)..CharOffset::from(1),
                "test\nline\n",
                Default::default(),
                selection.clone(),
                ctx,
            );
            assert_eq!(buffer.content.debug(), "<text>test\\nline\\n");

            let formatted_text = parse_html("<h2>New Header</h2>").expect("Should parse");
            buffer.replace_with_formatted_text(
                CharOffset::from(11)..CharOffset::from(11),
                formatted_text.clone(),
                EditOrigin::UserInitiated,
                selection.clone(),
                ctx,
            );

            // Scenario 1: When inserting a non-plain text block at an empty newline, we should replace that newline
            // with the pasted block style.
            assert_eq!(
                buffer.content.debug(),
                "<text>test\\nline<header2>New Header<text>"
            );

            // Scenario 2: When inserting a non-plain text block in the middle of a line, we should keep the existing styling
            // instead of replacing it.
            buffer.replace_with_formatted_text(
                CharOffset::from(10)..CharOffset::from(10),
                formatted_text.clone(),
                EditOrigin::UserInitiated,
                selection.clone(),
                ctx,
            );
            assert_eq!(
                buffer.content.debug(),
                "<text>test\\nlineNew Header<header2>New Header<text>"
            );

            // Scenario 3: When inserting a non-plain text block to replace an entire line, we should replace that newline
            // with the pasted block style.
            buffer.replace_with_formatted_text(CharOffset::from(6)..CharOffset::from(20), formatted_text, EditOrigin::UserInitiated, selection.clone(), ctx);
            assert_eq!(
                buffer.content.debug(),
                "<text>test<header2>New Header<header2>New Header<text>"
            );

            let formatted_text =
                parse_html("<ul><li>list<li><ul><li>sublist</li></ul></li></ul>").expect("Should parse");

            // Scenario 4: When inserting a multi-line non-plain text block in the middle of a line, we should keep the original
            // line's format for the first line. For the second line onwards, we will keep the pasted block style.
            buffer.replace_with_formatted_text(CharOffset::from(5)..CharOffset::from(5), formatted_text, EditOrigin::UserInitiated, selection.clone(), ctx);
            assert_eq!(
                buffer.content.debug(),
                "<text>testlist<ul1>sublist<header2>New Header<header2>New Header<text>"
            );

            let formatted_text = parse_html("<p>start</p>").expect("Should parse");

            // Scenario 5: When inserting a plain text block to replace a non-plain text line, we should keep the original
            // line's block style.
            buffer.replace_with_formatted_text(CharOffset::from(10)..CharOffset::from(17), formatted_text, EditOrigin::UserInitiated, selection.clone(), ctx);
            assert_eq!(
                buffer.content.debug(),
                "<text>testlist<ul1>start<header2>New Header<header2>New Header<text>"
            );

            let formatted_text = parse_html("<pre>first\nsecond</pre>").expect("Should parse");

            // Scenario 6: When inserting a multi-line code block in a non-plain text block, we should keep the original
            // line's format and treat it as inserting a multi-line plain text.
            buffer.replace_with_formatted_text(
                CharOffset::from(15)..CharOffset::from(15),
                formatted_text.clone(),
                EditOrigin::UserInitiated,
                selection.clone(),
                ctx,
            );
            assert_eq!(
                buffer.content.debug(),
                "<text>testlist<ul1>startfirst<ul1>second<header2>New Header<header2>New Header<text>"
            );

            // Scenario 7: When inserting a multi-line code block to replace a plain text line, we should keep the code block format.
            buffer.replace_with_formatted_text(
                CharOffset::from(1)..CharOffset::from(9),
                formatted_text.clone(),
                EditOrigin::UserInitiated,
                selection.clone(),
                ctx,
            );
            assert_eq!(
                buffer.content.debug(),
                "<code:Shell>first\\nsecond<ul1>startfirst<ul1>second<header2>New Header<header2>New Header<text>"
            );

            // Scenario 8: When inserting a multi-line code block into a multi-line code block, we should keep it as one code block.
            buffer.replace_with_formatted_text(
                CharOffset::from(7)..CharOffset::from(7),
                formatted_text.clone(),
                EditOrigin::UserInitiated,
                selection.clone(),
                ctx,
            );
            assert_eq!(
                buffer.content.debug(),
                "<code:Shell>first\\nfirst\\nsecondsecond<ul1>startfirst<ul1>second<header2>New Header<header2>New Header<text>"
            );
        });
    });
}

// Regression test for CLD-771.
#[test]
fn test_invalidate_content() {
    App::test((), |mut app| async move {
        let (buffer, _selection) = Buffer::mock_from_markdown(
            "* \nabc",
            None,
            Box::new(|_, _| IndentBehavior::Ignore),
            &mut app,
        );
        buffer.update(&mut app, |buffer, _ctx| {
            assert_eq!(buffer.content.debug(), "<ul0><text>abc");

            let edit_delta = buffer.invalidate_layout();
            assert_eq!(
                edit_delta.old_offset,
                CharOffset::from(1)..CharOffset::from(5)
            );
            assert_eq!(
                edit_delta.new_lines,
                vec![
                    StyledBufferBlock::Text(StyledTextBlock {
                        block: vec![StyledBufferRun {
                            run: "\n".to_string(),
                            text_styles: TextStylesWithMetadata::default(),
                            block_style: BufferBlockStyle::UnorderedList {
                                indent_level: ListIndentLevel::One
                            }
                        },],
                        style: BufferBlockStyle::UnorderedList {
                            indent_level: ListIndentLevel::One,
                        },
                        content_length: CharOffset::from(1)
                    }),
                    StyledBufferBlock::Text(StyledTextBlock {
                        block: vec![StyledBufferRun {
                            run: "abc".to_string(),
                            text_styles: TextStylesWithMetadata::default(),
                            block_style: BufferBlockStyle::PlainText
                        },],
                        style: BufferBlockStyle::PlainText,
                        content_length: CharOffset::from(3)
                    }),
                ]
            );
        });
    });
}

// Regression test for CLD-782.
#[test]
fn test_export_markdown_multiple_indentation_level() {
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            buffer.edit_internal_first_selection(
                CharOffset::from(1)..CharOffset::from(1),
                "test\nline\n",
                Default::default(),
                selection.clone(),
                ctx,
            );

            buffer.block_style_range(
                CharOffset::from(1)..CharOffset::from(5),
                BufferBlockStyle::ordered_list(ListIndentLevel::One),
                selection.clone(),
                ctx,
            );
            buffer.block_style_range(
                CharOffset::from(6)..CharOffset::from(10),
                BufferBlockStyle::ordered_list(ListIndentLevel::Three),
                selection.clone(),
                ctx,
            );

            assert_eq!(buffer.content.debug(), "<ol0>test<ol2>line<text>");

            assert_eq!(buffer.markdown(), "1. test\n        1. line\n");
        });
    });
}

#[test]
fn test_multi_linebreak_paste() {
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            buffer.edit_internal_first_selection(
                CharOffset::from(1)..CharOffset::from(1),
                "text",
                Default::default(),
                selection.clone(),
                ctx,
            );

            buffer.replace_with_formatted_text(
                CharOffset::from(3)..CharOffset::from(3),
                FormattedText::new([FormattedTextLine::LineBreak, FormattedTextLine::LineBreak]),
                EditOrigin::UserInitiated,
                selection.clone(),
                ctx,
            );

            assert_eq!(buffer.content.debug(), "<text>te\\n\\nxt");
        });
    });
}

#[test]
fn test_mixed_linebreak_paste() {
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            buffer.edit_internal_first_selection(
                CharOffset::from(1)..CharOffset::from(1),
                "te\n\nxt",
                Default::default(),
                selection.clone(),
                ctx,
            );

            buffer.replace_with_formatted_text(
                CharOffset::from(4)..CharOffset::from(4),
                FormattedText::new([
                    FormattedTextLine::UnorderedList(FormattedIndentTextInline {
                        indent_level: 0,
                        text: vec![FormattedTextFragment::plain_text("abc")],
                    }),
                    FormattedTextLine::LineBreak,
                    FormattedTextLine::Line(vec![FormattedTextFragment::plain_text("def")]),
                ]),
                EditOrigin::UserInitiated,
                selection.clone(),
                ctx,
            );

            assert_eq!(buffer.content.debug(), "<text>te<ul0>abc<text>\\ndef\\nxt");
        });
    });
}

// Regression test for CLD-862
#[test]
fn test_code_block_not_inherit_text_styling() {
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            buffer.edit_internal_first_selection(
                CharOffset::from(1)..CharOffset::from(1),
                "abc\n```",
                TextStyles::default().italic(),
                selection.clone(),
                ctx,
            );

            selection.update(ctx, |selection, _| {
                selection.set_single_cursor(8.into());
            });
            buffer.remove_prefix_and_style_blocks(
                BlockType::Text(BufferBlockStyle::CodeBlock {
                    code_block_type: Default::default(),
                }),
                selection.clone(),
                ctx,
            );

            assert_eq!(
                buffer.content.debug(),
                "<text><i_s>abc<i_e><code:Shell><text>"
            );
        });
    });
}

#[test]
fn test_selection_bias_with_mouse_movement() {
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            buffer.edit_internal_first_selection(
                CharOffset::from(1)..CharOffset::from(1),
                "textinlinetext",
                TextStyles::default(),
                selection.clone(),
                ctx,
            );
            buffer.set_selection(
                CharOffset::from(5)..CharOffset::from(11),
                selection.clone(),
                ctx,
            );
            buffer.style_internal(TextStyles::default().inline_code(), selection.clone(), ctx);
            assert_eq!(buffer.content.debug(), "<text>text<c_s>inline<c_e>text");

            selection.update(ctx, |selection, _| {
                selection.set_single_cursor(CharOffset::from(5));
            });
            assert_eq!(
                selection.as_ref(ctx).selection().bias(),
                TextStyleBias::OutOfStyle
            );

            // Moving the cursor right from out of style should change bias to in style.
            buffer.update_selection(
                selection.clone(),
                BufferSelectAction::MoveRight,
                AutoScrollBehavior::Selection,
                ctx,
            );

            assert_eq!(
                selection.as_ref(ctx).selection().bias(),
                TextStyleBias::InStyle
            );
            assert_eq!(
                selection.as_ref(ctx).first_selection_head(),
                CharOffset::from(5)
            );

            // Moving the cursor right again from in style should move the cursor right by 1.
            buffer.update_selection(
                selection.clone(),
                BufferSelectAction::MoveRight,
                AutoScrollBehavior::Selection,
                ctx,
            );
            assert_eq!(
                selection.as_ref(ctx).first_selection_head(),
                CharOffset::from(6)
            );

            selection.update(ctx, |selection, _| {
                selection.set_single_cursor(CharOffset::from(10));
            });
            buffer.update_selection(
                selection.clone(),
                BufferSelectAction::MoveRight,
                AutoScrollBehavior::Selection,
                ctx,
            );

            // Moving the cursor right from within inline code to border should change bias to in style.
            assert_eq!(
                selection.as_ref(ctx).selection().bias(),
                TextStyleBias::InStyle
            );
            assert_eq!(
                selection.as_ref(ctx).first_selection_head(),
                CharOffset::from(11)
            );

            // Moving the cursor right again from in style should change bias to out of style.
            buffer.update_selection(
                selection.clone(),
                BufferSelectAction::MoveRight,
                AutoScrollBehavior::Selection,
                ctx,
            );
            assert_eq!(
                selection.as_ref(ctx).selection().bias(),
                TextStyleBias::OutOfStyle
            );
            assert_eq!(
                selection.as_ref(ctx).first_selection_head(),
                CharOffset::from(11)
            );

            selection.update(ctx, |selection, _| {
                selection.set_single_cursor(CharOffset::from(6));
            });

            // Moving the cursor left from within inline code to border should change bias to in style.
            buffer.update_selection(
                selection.clone(),
                BufferSelectAction::MoveLeft,
                AutoScrollBehavior::Selection,
                ctx,
            );

            assert_eq!(
                selection.as_ref(ctx).selection().bias(),
                TextStyleBias::InStyle
            );
            assert_eq!(
                selection.as_ref(ctx).first_selection_head(),
                CharOffset::from(5)
            );

            // Moving the cursor left again from in style should change bias to out of style.
            buffer.update_selection(
                selection.clone(),
                BufferSelectAction::MoveLeft,
                AutoScrollBehavior::Selection,
                ctx,
            );
            assert_eq!(
                selection.as_ref(ctx).selection().bias(),
                TextStyleBias::OutOfStyle
            );
            assert_eq!(
                selection.as_ref(ctx).first_selection_head(),
                CharOffset::from(5)
            );

            // Moving the cursor left again should move the cursor left by 1.
            buffer.update_selection(
                selection.clone(),
                BufferSelectAction::MoveLeft,
                AutoScrollBehavior::Selection,
                ctx,
            );
            assert_eq!(
                selection.as_ref(ctx).selection().bias(),
                TextStyleBias::OutOfStyle
            );
            assert_eq!(
                selection.as_ref(ctx).first_selection_head(),
                CharOffset::from(4)
            );

            // Moving the cursor left from outside inline code to border should keep bias to out of style.
            selection.update(ctx, |selection, _| {
                selection.set_single_cursor(CharOffset::from(12));
            });
            buffer.update_selection(
                selection.clone(),
                BufferSelectAction::MoveLeft,
                AutoScrollBehavior::Selection,
                ctx,
            );
            assert_eq!(
                selection.as_ref(ctx).selection().bias(),
                TextStyleBias::OutOfStyle
            );
            assert_eq!(
                selection.as_ref(ctx).first_selection_head(),
                CharOffset::from(11)
            );

            // Moving the cursor left again from out of style should change bias to in style.
            buffer.update_selection(
                selection.clone(),
                BufferSelectAction::MoveLeft,
                AutoScrollBehavior::Selection,
                ctx,
            );
            assert_eq!(
                selection.as_ref(ctx).selection().bias(),
                TextStyleBias::InStyle
            );
            assert_eq!(
                selection.as_ref(ctx).first_selection_head(),
                CharOffset::from(11)
            );

            // Moving the cursor left again should move the cursor left by 1.
            buffer.update_selection(
                selection.clone(),
                BufferSelectAction::MoveLeft,
                AutoScrollBehavior::Selection,
                ctx,
            );
            assert_eq!(
                selection.as_ref(ctx).selection().bias(),
                TextStyleBias::OutOfStyle
            );
            assert_eq!(
                selection.as_ref(ctx).first_selection_head(),
                CharOffset::from(10)
            );
        });
    });
}

#[test]
fn test_insert_html_tasklist_on_plain_text() {
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            buffer.edit_internal_first_selection(
                CharOffset::from(1)..CharOffset::from(1),
                "Before\n\n",
                Default::default(),
                selection.clone(),
                ctx,
            );

            let formatted_text =
                parse_html("<ul><li><input type=\"checkbox\"></input>test text</li></ul>")
                    .expect("Should parse");
            buffer.replace_with_formatted_text(
                CharOffset::from(9)..CharOffset::from(9),
                formatted_text,
                EditOrigin::UserInitiated,
                selection.clone(),
                ctx,
            );
            assert_eq!(
                buffer.content.debug(),
                "<text>Before\\n<cl0:false>test text<text>"
            );
        });
    });
}

#[test]
fn test_insert_block_item_in_plain_text() {
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            buffer.edit_internal_first_selection(
                CharOffset::from(1)..CharOffset::from(1),
                "Before\n\n",
                Default::default(),
                selection.clone(),
                ctx,
            );

            let prev_selection = buffer.to_rendered_selection_set(selection.clone(), ctx);
            let edit_result = buffer.replace_with_formatted_text(
                CharOffset::from(4)..CharOffset::from(4),
                FormattedText::new(vec![FormattedTextLine::HorizontalRule]),
                EditOrigin::UserInitiated,
                selection.clone(),
                ctx,
            );
            let curr_selection = buffer.to_rendered_selection_set(selection.clone(), ctx);
            buffer.push_undo_item(
                prev_selection,
                curr_selection,
                edit_result.undo_item.expect("Should exist"),
                UndoActionType::Atomic,
            );
            assert_eq!(buffer.content.debug(), "<text>Bef<hr><text>ore\\n\\n");

            let delta = edit_result.delta.expect("Should exist");
            assert_eq!(delta.old_offset, CharOffset::from(1)..CharOffset::from(8));
            assert_eq!(
                delta.new_lines,
                vec![
                    StyledBufferBlock::Text(StyledTextBlock {
                        block: vec![StyledBufferRun {
                            run: "Bef\n".to_string(),
                            text_styles: TextStylesWithMetadata::default(),
                            block_style: BufferBlockStyle::PlainText
                        },],
                        style: BufferBlockStyle::PlainText,
                        content_length: CharOffset::from(4)
                    }),
                    StyledBufferBlock::Item(BufferBlockItem::HorizontalRule),
                    StyledBufferBlock::Text(StyledTextBlock {
                        block: vec![StyledBufferRun {
                            run: "ore\n".to_string(),
                            text_styles: TextStylesWithMetadata::default(),
                            block_style: BufferBlockStyle::PlainText
                        },],
                        style: BufferBlockStyle::PlainText,
                        content_length: CharOffset::from(4)
                    }),
                ]
            );

            buffer.undo(selection.clone(), ctx);
            assert_eq!(buffer.content.debug(), "<text>Before\\n\\n");

            buffer.redo(selection.clone(), ctx);
            assert_eq!(buffer.content.debug(), "<text>Bef<hr><text>ore\\n\\n");

            let prev_selection = buffer.to_rendered_selection_set(selection.clone(), ctx);
            let edit_result = buffer.replace_with_formatted_text(
                CharOffset::from(11)..CharOffset::from(11),
                FormattedText::new(vec![FormattedTextLine::HorizontalRule]),
                EditOrigin::UserInitiated,
                selection.clone(),
                ctx,
            );
            let curr_selection = buffer.to_rendered_selection_set(selection.clone(), ctx);
            buffer.push_undo_item(
                prev_selection,
                curr_selection,
                edit_result.undo_item.expect("Should exist"),
                UndoActionType::Atomic,
            );
            assert_eq!(
                buffer.content.debug(),
                "<text>Bef<hr><text>ore\\n\\n<hr><text>"
            );

            let delta = edit_result.delta.expect("Should exist");
            assert_eq!(delta.old_offset, CharOffset::from(11)..CharOffset::from(11));
            assert_eq!(
                delta.new_lines,
                vec![
                    StyledBufferBlock::Text(StyledTextBlock {
                        block: vec![StyledBufferRun {
                            run: "\n".to_string(),
                            text_styles: TextStylesWithMetadata::default(),
                            block_style: BufferBlockStyle::PlainText
                        },],
                        style: BufferBlockStyle::PlainText,
                        content_length: CharOffset::from(1)
                    }),
                    StyledBufferBlock::Item(BufferBlockItem::HorizontalRule),
                ]
            );

            buffer.undo(selection.clone(), ctx);
            assert_eq!(buffer.content.debug(), "<text>Bef<hr><text>ore\\n\\n");

            buffer.redo(selection.clone(), ctx);
            assert_eq!(
                buffer.content.debug(),
                "<text>Bef<hr><text>ore\\n\\n<hr><text>"
            );
        });
    });
}

#[test]
fn test_styling_over_block_item() {
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            buffer.edit_internal_first_selection(
                CharOffset::from(1)..CharOffset::from(1),
                "Test Line",
                Default::default(),
                selection.clone(),
                ctx,
            );
            buffer.set_selection(
                CharOffset::from(3)..CharOffset::from(8),
                selection.clone(),
                ctx,
            );
            buffer.style_internal(TextStyles::default().bold(), selection.clone(), ctx);

            let prev_selection = buffer.to_rendered_selection_set(selection.clone(), ctx);
            let edit_result = buffer.replace_with_formatted_text(
                CharOffset::from(5)..CharOffset::from(6),
                FormattedText::new(vec![FormattedTextLine::HorizontalRule]),
                EditOrigin::UserInitiated,
                selection.clone(),
                ctx,
            );
            let curr_selection = buffer.to_rendered_selection_set(selection.clone(), ctx);
            buffer.push_undo_item(
                prev_selection,
                curr_selection,
                edit_result.undo_item.expect("Should exist"),
                UndoActionType::Atomic,
            );
            assert_eq!(
                buffer.content.debug(),
                "<text>Te<b_s>st<b_e><hr><text><b_s>Li<b_e>ne"
            );

            buffer.undo(selection.clone(), ctx);
            assert_eq!(buffer.content.debug(), "<text>Te<b_s>st Li<b_e>ne");

            buffer.redo(selection.clone(), ctx);
            assert_eq!(
                buffer.content.debug(),
                "<text>Te<b_s>st<b_e><hr><text><b_s>Li<b_e>ne"
            );

            let prev_selection = buffer.to_rendered_selection_set(selection.clone(), ctx);
            buffer.set_selection(
                CharOffset::from(3)..CharOffset::from(9),
                selection.clone(),
                ctx,
            );
            let edit_result =
                buffer.style_internal(TextStyles::default().italic(), selection.clone(), ctx);
            let curr_selection = buffer.to_rendered_selection_set(selection.clone(), ctx);
            buffer.push_undo_item(
                prev_selection,
                curr_selection,
                edit_result.undo_item.expect("Should exist"),
                UndoActionType::Atomic,
            );

            assert_eq!(
                buffer.content.debug(),
                "<text>Te<b_s><i_s>st<b_e><i_e><hr><text><b_s><i_s>Li<b_e><i_e>ne"
            );

            let delta = edit_result.delta.expect("Should exist");
            assert_eq!(delta.old_offset, CharOffset::from(1)..CharOffset::from(11));
            assert_eq!(
                delta.new_lines,
                vec![
                    StyledBufferBlock::Text(StyledTextBlock {
                        block: vec![
                            StyledBufferRun {
                                run: "Te".to_string(),
                                text_styles: TextStylesWithMetadata::default(),
                                block_style: BufferBlockStyle::PlainText
                            },
                            StyledBufferRun {
                                run: "st".to_string(),
                                text_styles: TextStylesWithMetadata::default().bold().italic(),
                                block_style: BufferBlockStyle::PlainText
                            },
                            StyledBufferRun {
                                run: "\n".to_string(),
                                text_styles: TextStylesWithMetadata::default(),
                                block_style: BufferBlockStyle::PlainText
                            }
                        ],
                        style: BufferBlockStyle::PlainText,
                        content_length: CharOffset::from(5)
                    }),
                    StyledBufferBlock::Item(BufferBlockItem::HorizontalRule),
                    StyledBufferBlock::Text(StyledTextBlock {
                        block: vec![
                            StyledBufferRun {
                                run: "Li".to_string(),
                                text_styles: TextStylesWithMetadata::default().bold().italic(),
                                block_style: BufferBlockStyle::PlainText
                            },
                            StyledBufferRun {
                                run: "ne".to_string(),
                                text_styles: TextStylesWithMetadata::default(),
                                block_style: BufferBlockStyle::PlainText
                            }
                        ],
                        style: BufferBlockStyle::PlainText,
                        content_length: CharOffset::from(4)
                    }),
                ]
            );

            buffer.undo(selection.clone(), ctx);
            assert_eq!(
                buffer.content.debug(),
                "<text>Te<b_s>st<b_e><hr><text><b_s>Li<b_e>ne"
            );

            buffer.redo(selection.clone(), ctx);
            assert_eq!(
                buffer.content.debug(),
                "<text>Te<b_s><i_s>st<b_e><i_e><hr><text><b_s><i_s>Li<b_e><i_e>ne"
            );
        });
    });
}

#[test]
fn test_block_styling_around_block_item() {
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            buffer.edit_internal_first_selection(
                CharOffset::from(1)..CharOffset::from(1),
                "Test Line",
                Default::default(),
                selection.clone(),
                ctx,
            );

            buffer.replace_with_formatted_text(
                CharOffset::from(5)..CharOffset::from(6),
                FormattedText::new(vec![FormattedTextLine::HorizontalRule]),
                EditOrigin::UserInitiated,
                selection.clone(),
                ctx,
            );

            let prev_selection = buffer.to_rendered_selection_set(selection.clone(), ctx);
            let edit_result = buffer.block_style_range(
                CharOffset::from(1)..CharOffset::from(5),
                BufferBlockStyle::Header {
                    header_size: BlockHeaderSize::Header1,
                },
                selection.clone(),
                ctx,
            );
            let curr_selection = buffer.to_rendered_selection_set(selection.clone(), ctx);
            buffer.push_undo_item(
                prev_selection,
                curr_selection,
                edit_result.undo_item.expect("Should exist"),
                UndoActionType::Atomic,
            );
            assert_eq!(buffer.content.debug(), "<header1>Test<hr><text>Line");

            buffer.undo(selection.clone(), ctx);
            assert_eq!(buffer.content.debug(), "<text>Test<hr><text>Line");
            buffer.redo(selection.clone(), ctx);
            assert_eq!(buffer.content.debug(), "<header1>Test<hr><text>Line");

            let prev_selection = buffer.to_rendered_selection_set(selection.clone(), ctx);
            let edit_result = buffer.block_style_range(
                CharOffset::from(7)..CharOffset::from(11),
                BufferBlockStyle::Header {
                    header_size: BlockHeaderSize::Header2,
                },
                selection.clone(),
                ctx,
            );
            let curr_selection = buffer.to_rendered_selection_set(selection.clone(), ctx);
            buffer.push_undo_item(
                prev_selection,
                curr_selection,
                edit_result.undo_item.expect("Should exist"),
                UndoActionType::Atomic,
            );
            assert_eq!(
                buffer.content.debug(),
                "<header1>Test<hr><header2>Line<text>"
            );

            buffer.undo(selection.clone(), ctx);
            assert_eq!(buffer.content.debug(), "<header1>Test<hr><text>Line");
            buffer.redo(selection.clone(), ctx);
            assert_eq!(
                buffer.content.debug(),
                "<header1>Test<hr><header2>Line<text>"
            );
        });
    });
}

#[test]
fn test_block_styling_overlapping_block_item() {
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            buffer.edit_internal_first_selection(
                CharOffset::from(1)..CharOffset::from(1),
                "TestLineAfter",
                Default::default(),
                selection.clone(),
                ctx,
            );

            buffer.replace_with_formatted_text(
                CharOffset::from(5)..CharOffset::from(5),
                FormattedText::new(vec![FormattedTextLine::HorizontalRule]),
                EditOrigin::UserInitiated,
                selection.clone(),
                ctx,
            );

            buffer.replace_with_formatted_text(
                CharOffset::from(11)..CharOffset::from(11),
                FormattedText::new(vec![FormattedTextLine::HorizontalRule]),
                EditOrigin::UserInitiated,
                selection.clone(),
                ctx,
            );

            assert_eq!(
                buffer.content.debug(),
                "<text>Test<hr><text>Line<hr><text>After"
            );

            let prev_selection = buffer.to_rendered_selection_set(selection.clone(), ctx);
            let edit_result = buffer.block_style_range(
                CharOffset::from(3)..CharOffset::from(15),
                BufferBlockStyle::Header {
                    header_size: BlockHeaderSize::Header1,
                },
                selection.clone(),
                ctx,
            );
            let curr_selection = buffer.to_rendered_selection_set(selection.clone(), ctx);
            buffer.push_undo_item(
                prev_selection,
                curr_selection,
                edit_result.undo_item.expect("Should exist"),
                UndoActionType::Atomic,
            );

            assert_eq!(
                buffer.content.debug(),
                "<text>Te<header1>st<hr><header1>Line<hr><header1>Af<text>ter"
            );
            buffer.undo(selection.clone(), ctx);
            assert_eq!(
                buffer.content.debug(),
                "<text>Test<hr><text>Line<hr><text>After"
            );
            buffer.redo(selection.clone(), ctx);
            assert_eq!(
                buffer.content.debug(),
                "<text>Te<header1>st<hr><header1>Line<hr><header1>Af<text>ter"
            );
        });
    });
}

#[test]
fn test_insert_block_item_in_the_middle_of_consecutive_linebreak() {
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            buffer.edit_internal_first_selection(
                CharOffset::from(1)..CharOffset::from(1),
                "Test\n\n\n",
                Default::default(),
                selection.clone(),
                ctx,
            );

            buffer.replace_with_formatted_text(
                CharOffset::from(6)..CharOffset::from(6),
                FormattedText::new(vec![FormattedTextLine::HorizontalRule]),
                EditOrigin::UserInitiated,
                selection.clone(),
                ctx,
            );

            assert_eq!(buffer.content.debug(), "<text>Test\\n<hr><text>\\n");
        });
    });
}

#[test]
fn test_backspace_on_block_item() {
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            buffer.edit_internal_first_selection(
                CharOffset::from(1)..CharOffset::from(1),
                "TestLine",
                Default::default(),
                selection.clone(),
                ctx,
            );

            buffer.replace_with_formatted_text(
                CharOffset::from(5)..CharOffset::from(5),
                FormattedText::new(vec![FormattedTextLine::HorizontalRule]),
                EditOrigin::UserInitiated,
                selection.clone(),
                ctx,
            );

            assert_eq!(buffer.content.debug(), "<text>Test<hr><text>Line");

            selection.update(ctx, |selection, _| {
                selection.set_single_cursor(CharOffset::from(7));
            });
            let prev_selection = buffer.to_rendered_selection_set(selection.clone(), ctx);
            let edit_result = buffer.backspace(&mut None, selection.clone(), ctx);
            let curr_selection = buffer.to_rendered_selection_set(selection.clone(), ctx);
            buffer.push_undo_item(
                prev_selection,
                curr_selection,
                edit_result.undo_item.expect("Should exist"),
                UndoActionType::Atomic,
            );

            assert_eq!(buffer.content.debug(), "<text>Test\\nLine");
            let delta = edit_result.delta.expect("Should exist");
            assert_eq!(delta.old_offset, CharOffset::from(1)..CharOffset::from(7));
            assert_eq!(
                delta.new_lines,
                vec![StyledBufferBlock::Text(StyledTextBlock {
                    block: vec![StyledBufferRun {
                        run: "Test\n".to_string(),
                        text_styles: TextStylesWithMetadata::default(),
                        block_style: BufferBlockStyle::PlainText
                    },],
                    style: BufferBlockStyle::PlainText,
                    content_length: CharOffset::from(5)
                }),]
            );

            buffer.undo(selection.clone(), ctx);
            assert_eq!(buffer.content.debug(), "<text>Test<hr><text>Line");

            buffer.redo(selection.clone(), ctx);
            assert_eq!(buffer.content.debug(), "<text>Test\\nLine");
        });
    });
}

#[test]
fn test_insert_block_item_after_block() {
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            buffer.edit_internal_first_selection(
                CharOffset::from(1)..CharOffset::from(1),
                "Test\nAfter\nCode",
                Default::default(),
                selection.clone(),
                ctx,
            );

            buffer.block_style_range(
                CharOffset::from(12)..CharOffset::from(16),
                BufferBlockStyle::CodeBlock {
                    code_block_type: CodeBlockType::Shell,
                },
                selection.clone(),
                ctx,
            );
            assert_eq!(
                buffer.content.debug(),
                "<text>Test\\nAfter<code:Shell>Code<text>"
            );

            buffer.insert_block_after_block_with_offset(
                CharOffset::from(1),
                BlockType::Item(BufferBlockItem::HorizontalRule),
                selection.clone(),
                ctx,
            );

            assert_eq!(
                buffer.content.debug(),
                "<text>Test<hr><text>After<code:Shell>Code<text>"
            );

            buffer.insert_block_after_block_with_offset(
                CharOffset::from(13),
                BlockType::Item(BufferBlockItem::HorizontalRule),
                selection.clone(),
                ctx,
            );
            assert_eq!(
                buffer.content.debug(),
                "<text>Test<hr><text>After<code:Shell>Code<hr><text>"
            );
        });
    });
}

#[test]
fn test_insert_block_item_before_non_text_block() {
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            buffer.edit_internal_first_selection(
                CharOffset::from(1)..CharOffset::from(1),
                "Test\nCode",
                Default::default(),
                selection.clone(),
                ctx,
            );

            buffer.block_style_range(
                CharOffset::from(6)..CharOffset::from(10),
                BufferBlockStyle::CodeBlock {
                    code_block_type: CodeBlockType::Shell,
                },
                selection.clone(),
                ctx,
            );
            assert_eq!(buffer.content.debug(), "<text>Test<code:Shell>Code<text>");

            let prev_selection = buffer.to_rendered_selection_set(selection.clone(), ctx);
            let edit_result = buffer.insert_block_after_block_with_offset(
                CharOffset::from(1),
                BlockType::Item(BufferBlockItem::HorizontalRule),
                selection.clone(),
                ctx,
            );
            let curr_selection = buffer.to_rendered_selection_set(selection.clone(), ctx);
            buffer.push_undo_item(
                prev_selection,
                curr_selection,
                edit_result.undo_item.expect("Should exist"),
                UndoActionType::Atomic,
            );
            assert_eq!(
                buffer.content.debug(),
                "<text>Test<hr><code:Shell>Code<text>"
            );

            buffer.undo(selection.clone(), ctx);
            assert_eq!(buffer.content.debug(), "<text>Test<code:Shell>Code<text>");

            buffer.redo(selection.clone(), ctx);
            assert_eq!(
                buffer.content.debug(),
                "<text>Test<hr><code:Shell>Code<text>"
            );
        });
    });
}

#[test]
fn test_deleting_middle_block_item_between_line_break() {
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            buffer.edit_internal_first_selection(
                CharOffset::from(1)..CharOffset::from(1),
                "\n",
                Default::default(),
                selection.clone(),
                ctx,
            );

            buffer.insert_block_after_block_with_offset(
                CharOffset::from(1),
                BlockType::Item(BufferBlockItem::HorizontalRule),
                selection.clone(),
                ctx,
            );

            assert_eq!(buffer.content.debug(), "<text><hr><text>");

            selection.update(ctx, |selection, _| {
                selection.set_single_cursor(CharOffset::from(3));
            });

            let prev_selection = buffer.to_rendered_selection_set(selection.clone(), ctx);
            let edit_result = buffer.backspace(&mut None, selection.clone(), ctx);
            assert_eq!(buffer.content.debug(), "<text>\\n");
            let curr_selection = buffer.to_rendered_selection_set(selection.clone(), ctx);
            buffer.push_undo_item(
                prev_selection,
                curr_selection,
                edit_result.undo_item.expect("Should exist"),
                UndoActionType::Atomic,
            );

            buffer.undo(selection.clone(), ctx);
            assert_eq!(buffer.content.debug(), "<text><hr><text>");
        });
    });
}

#[test]
fn test_enter_on_text_before_block_item() {
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            buffer.edit_internal_first_selection(
                CharOffset::from(1)..CharOffset::from(1),
                "Test",
                Default::default(),
                selection.clone(),
                ctx,
            );

            buffer.insert_block_after_block_with_offset(
                CharOffset::from(1),
                BlockType::Item(BufferBlockItem::HorizontalRule),
                selection.clone(),
                ctx,
            );

            assert_eq!(buffer.content.debug(), "<text>Test<hr><text>");

            selection.update(ctx, |selection, _| {
                selection.set_single_cursor(CharOffset::from(5));
            });
            let prev_selection = buffer.to_rendered_selection_set(selection.clone(), ctx);
            let edit_result = buffer.enter(false, TextStyles::default(), selection.clone(), ctx);
            let curr_selection = buffer.to_rendered_selection_set(selection.clone(), ctx);
            buffer.push_undo_item(
                prev_selection,
                curr_selection,
                edit_result.undo_item.expect("Should exist"),
                UndoActionType::Atomic,
            );

            assert_eq!(buffer.content.debug(), "<text>Test\\n<hr><text>");
            let delta = edit_result.delta.expect("Should exist");
            assert_eq!(delta.old_offset, CharOffset::from(1)..CharOffset::from(6));
            assert_eq!(
                delta.new_lines,
                vec![
                    StyledBufferBlock::Text(StyledTextBlock {
                        block: vec![StyledBufferRun {
                            run: "Test\n".to_string(),
                            text_styles: TextStylesWithMetadata::default(),
                            block_style: BufferBlockStyle::PlainText
                        },],
                        style: BufferBlockStyle::PlainText,
                        content_length: CharOffset::from(5)
                    }),
                    StyledBufferBlock::Text(StyledTextBlock {
                        block: vec![StyledBufferRun {
                            run: "\n".to_string(),
                            text_styles: TextStylesWithMetadata::default(),
                            block_style: BufferBlockStyle::PlainText
                        },],
                        style: BufferBlockStyle::PlainText,
                        content_length: CharOffset::from(1)
                    }),
                ]
            );

            buffer.undo(selection.clone(), ctx);
            assert_eq!(buffer.content.debug(), "<text>Test<hr><text>");

            buffer.redo(selection.clone(), ctx);
            assert_eq!(buffer.content.debug(), "<text>Test\\n<hr><text>");
        });
    });
}

#[test]
fn test_enter_on_line_after_block_item() {
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            buffer.edit_internal_first_selection(
                CharOffset::from(1)..CharOffset::from(1),
                "Test\nLine",
                Default::default(),
                selection.clone(),
                ctx,
            );

            buffer.insert_block_after_block_with_offset(
                CharOffset::from(1),
                BlockType::Item(BufferBlockItem::HorizontalRule),
                selection.clone(),
                ctx,
            );

            assert_eq!(buffer.content.debug(), "<text>Test<hr><text>Line");

            selection.update(ctx, |selection, _| {
                selection.set_single_cursor(CharOffset::from(7));
            });
            buffer.enter(false, TextStyles::default(), selection.clone(), ctx);

            assert_eq!(buffer.content.debug(), "<text>Test<hr><text>\\nLine");
        });
    });
}

#[test]
fn test_inserting_consecutive_block_items() {
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            buffer.edit_internal_first_selection(
                CharOffset::from(1)..CharOffset::from(1),
                "Test\nLine",
                Default::default(),
                selection.clone(),
                ctx,
            );

            let prev_selection = buffer.to_rendered_selection_set(selection.clone(), ctx);
            let edit_result = buffer.insert_block_after_block_with_offset(
                CharOffset::from(1),
                BlockType::Item(BufferBlockItem::HorizontalRule),
                selection.clone(),
                ctx,
            );
            let curr_selection = buffer.to_rendered_selection_set(selection.clone(), ctx);
            buffer.push_undo_item(
                prev_selection,
                curr_selection,
                edit_result.undo_item.expect("Should exist"),
                UndoActionType::Atomic,
            );

            let prev_selection = buffer.to_rendered_selection_set(selection.clone(), ctx);
            let edit_result = buffer.insert_block_after_block_with_offset(
                CharOffset::from(1),
                BlockType::Item(BufferBlockItem::HorizontalRule),
                selection.clone(),
                ctx,
            );
            let curr_selection = buffer.to_rendered_selection_set(selection.clone(), ctx);
            buffer.push_undo_item(
                prev_selection,
                curr_selection,
                edit_result.undo_item.expect("Should exist"),
                UndoActionType::Atomic,
            );

            assert_eq!(buffer.content.debug(), "<text>Test<hr><hr><text>Line");
            buffer.undo(selection.clone(), ctx);
            assert_eq!(buffer.content.debug(), "<text>Test<hr><text>Line");
            buffer.undo(selection.clone(), ctx);
            assert_eq!(buffer.content.debug(), "<text>Test\\nLine");

            buffer.redo(selection.clone(), ctx);
            assert_eq!(buffer.content.debug(), "<text>Test<hr><text>Line");
            buffer.redo(selection.clone(), ctx);
            assert_eq!(buffer.content.debug(), "<text>Test<hr><hr><text>Line");
        });
    });
}

#[test]
fn test_style_on_consecutive_block_items() {
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            buffer.edit_internal_first_selection(
                CharOffset::from(1)..CharOffset::from(1),
                "Test\nLine",
                Default::default(),
                selection.clone(),
                ctx,
            );

            buffer.insert_block_after_block_with_offset(
                CharOffset::from(1),
                BlockType::Item(BufferBlockItem::HorizontalRule),
                selection.clone(),
                ctx,
            );
            buffer.insert_block_after_block_with_offset(
                CharOffset::from(1),
                BlockType::Item(BufferBlockItem::HorizontalRule),
                selection.clone(),
                ctx,
            );

            assert_eq!(buffer.content.debug(), "<text>Test<hr><hr><text>Line");

            let prev_selection = buffer.to_rendered_selection_set(selection.clone(), ctx);
            let edit_result = buffer.block_style_range(
                CharOffset::from(1)..CharOffset::from(12),
                BufferBlockStyle::CodeBlock {
                    code_block_type: CodeBlockType::Shell,
                },
                selection.clone(),
                ctx,
            );
            let curr_selection = buffer.to_rendered_selection_set(selection.clone(), ctx);
            buffer.push_undo_item(
                prev_selection,
                curr_selection,
                edit_result.undo_item.expect("Should exist"),
                UndoActionType::Atomic,
            );

            assert_eq!(
                buffer.content.debug(),
                "<code:Shell>Test<hr><hr><code:Shell>Line<text>"
            );

            buffer.undo(selection.clone(), ctx);
            assert_eq!(buffer.content.debug(), "<text>Test<hr><hr><text>Line");

            buffer.redo(selection.clone(), ctx);
            assert_eq!(
                buffer.content.debug(),
                "<code:Shell>Test<hr><hr><code:Shell>Line<text>"
            );
        });
    });
}

#[test]
fn test_color_code_block() {
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            buffer.edit_internal_first_selection(
                CharOffset::from(1)..CharOffset::from(1),
                "Test\nBlock\nLine\nBlock",
                Default::default(),
                selection.clone(),
                ctx,
            );

            buffer.block_style_range(
                CharOffset::from(6)..CharOffset::from(11),
                BufferBlockStyle::CodeBlock {
                    code_block_type: CodeBlockType::Shell,
                },
                selection.clone(),
                        ctx,
            );

            buffer.block_style_range(
                CharOffset::from(17)..CharOffset::from(22),
                BufferBlockStyle::CodeBlock {
                    code_block_type: CodeBlockType::Shell,
                },
                selection.clone(),
                        ctx,
            );

            assert_eq!(
                buffer.content.debug(),
                "<text>Test<code:Shell>Block<text>Line<code:Shell>Block<text>"
            );

            let edit_result = buffer.color_code_block_ranges_internal(
                CharOffset::from(6),
                &[
                    (ByteOffset::from(0)..ByteOffset::from(1), ColorU::white()),
                    (ByteOffset::from(1)..ByteOffset::from(4), ColorU::black()),
                ],
            );

            assert_eq!(
                buffer.content.debug(),
                "<text>Test<code:Shell><c_#ffffff>B<c><c_#000000>loc<c>k<text>Line<code:Shell>Block<text>"
            );

            let delta = edit_result.delta.expect("Should exist");
            assert_eq!(delta.old_offset, CharOffset::from(6)..CharOffset::from(12));
            assert_eq!(
                delta.new_lines,
                vec![StyledBufferBlock::Text(StyledTextBlock {
                    block: vec![
                        StyledBufferRun {
                            run: "B".to_string(),
                            text_styles: TextStylesWithMetadata::default().with_color(ColorU::white()),
                            block_style: BufferBlockStyle::CodeBlock {
                                code_block_type: CodeBlockType::Shell
                            }
                        },
                        StyledBufferRun {
                            run: "loc".to_string(),
                            text_styles: TextStylesWithMetadata::default().with_color(ColorU::black()),
                            block_style: BufferBlockStyle::CodeBlock {
                                code_block_type: CodeBlockType::Shell
                            }
                        },
                        StyledBufferRun {
                            run: "k\n".to_string(),
                            text_styles: TextStylesWithMetadata::default(),
                            block_style: BufferBlockStyle::CodeBlock {
                                code_block_type: CodeBlockType::Shell
                            }
                        }
                    ],
                    style: BufferBlockStyle::CodeBlock {
                        code_block_type: CodeBlockType::Shell
                    },
                    content_length: CharOffset::from(6)
                })]
            );

            let edit_result = buffer.color_code_block_ranges_internal(
                CharOffset::from(17),
                &[(ByteOffset::from(0)..ByteOffset::from(5), ColorU::white())],
            );

            assert_eq!(
                buffer.content.debug(),
                "<text>Test<code:Shell><c_#ffffff>B<c><c_#000000>loc<c>k<text>Line<code:Shell><c_#ffffff>Block<c><text>"
            );

            let delta = edit_result.delta.expect("Should exist");
            assert_eq!(delta.old_offset, CharOffset::from(17)..CharOffset::from(23));
            assert_eq!(
                delta.new_lines,
                vec![StyledBufferBlock::Text(StyledTextBlock {
                    block: vec![
                        StyledBufferRun {
                            run: "Block".to_string(),
                            text_styles: TextStylesWithMetadata::default().with_color(ColorU::white()),
                            block_style: BufferBlockStyle::CodeBlock {
                                code_block_type: CodeBlockType::Shell
                            }
                        },
                        StyledBufferRun {
                            run: "\n".to_string(),
                            text_styles: TextStylesWithMetadata::default(),
                            block_style: BufferBlockStyle::CodeBlock {
                                code_block_type: CodeBlockType::Shell
                            }
                        },
                    ],
                    style: BufferBlockStyle::CodeBlock {
                        code_block_type: CodeBlockType::Shell
                    },
                    content_length: CharOffset::from(6)
                })]
            );

            let edit_result = buffer.color_code_block_ranges_internal(CharOffset::from(6), &[]);

            assert_eq!(
                buffer.content.debug(),
                "<text>Test<code:Shell>Block<text>Line<code:Shell><c_#ffffff>Block<c><text>"
            );

            let delta = edit_result.delta.expect("Should exist");
            assert_eq!(delta.old_offset, CharOffset::from(6)..CharOffset::from(12));
            assert_eq!(
                delta.new_lines,
                vec![StyledBufferBlock::Text(StyledTextBlock {
                    block: vec![StyledBufferRun {
                        run: "Block\n".to_string(),
                        text_styles: TextStylesWithMetadata::default(),
                        block_style: BufferBlockStyle::CodeBlock {
                            code_block_type: CodeBlockType::Shell
                        }
                    },],
                    style: BufferBlockStyle::CodeBlock {
                        code_block_type: CodeBlockType::Shell
                    },
                    content_length: CharOffset::from(6)
                })]
            );
        });
    });
}

#[test]
fn test_remove_coloring_in_middle_of_block() {
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            buffer.edit_internal_first_selection(
                CharOffset::from(1)..CharOffset::from(1),
                "Test\nBlock\nLine",
                Default::default(),
                selection.clone(),
                ctx,
            );

            buffer.block_style_range(
                CharOffset::from(6)..CharOffset::from(11),
                BufferBlockStyle::CodeBlock {
                    code_block_type: CodeBlockType::Shell,
                },
                selection.clone(),
                ctx,
            );

            assert_eq!(
                buffer.content.debug(),
                "<text>Test<code:Shell>Block<text>Line"
            );

            buffer.color_code_block_ranges_internal(
                CharOffset::from(6),
                &[
                    (ByteOffset::from(0)..ByteOffset::from(1), ColorU::white()),
                    (ByteOffset::from(1)..ByteOffset::from(5), ColorU::black()),
                ],
            );

            assert_eq!(
                buffer.content.debug(),
                "<text>Test<code:Shell><c_#ffffff>B<c><c_#000000>lock<c><text>Line"
            );

            buffer.color_code_block_ranges_internal(CharOffset::from(8), &[]);

            assert_eq!(
                buffer.content.debug(),
                "<text>Test<code:Shell><c_#ffffff>B<c><c_#000000>l<c>ock<text>Line"
            );

            buffer.color_code_block_ranges_internal(CharOffset::from(7), &[]);
            assert_eq!(
                buffer.content.debug(),
                "<text>Test<code:Shell><c_#ffffff>B<c>lock<text>Line"
            );

            buffer.color_code_block_ranges_internal(CharOffset::from(6), &[]);
            assert_eq!(
                buffer.content.debug(),
                "<text>Test<code:Shell>Block<text>Line"
            );
        });
    });
}

#[test]
fn test_edit_colored_code_block() {
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            buffer.edit_internal_first_selection(
                CharOffset::from(1)..CharOffset::from(1),
                "Test\nBlock\nLine\nBlock",
                Default::default(),
                selection.clone(),
                ctx,
            );

            buffer.block_style_range(
                CharOffset::from(6)..CharOffset::from(16),
                BufferBlockStyle::CodeBlock {
                    code_block_type: CodeBlockType::Shell,
                },
                selection.clone(),
                ctx,
            );

            assert_eq!(
                buffer.content.debug(),
                "<text>Test<code:Shell>Block\\nLine<text>Block"
            );

            buffer.color_code_block_ranges_internal(
                CharOffset::from(6),
                &[
                    (ByteOffset::from(0)..ByteOffset::from(5), ColorU::white()),
                    (ByteOffset::from(6)..ByteOffset::from(10), ColorU::black()),
                ],
            );

            assert_eq!(
                buffer.content.debug(),
                "<text>Test<code:Shell><c_#ffffff>Block<c>\\n<c_#000000>Line<c><text>Block"
            );

            // Note that for editing, we don't clear syntax color to prevent flickering.
            buffer.edit_internal_first_selection(
                CharOffset::from(6)..CharOffset::from(8),
                "",
                TextStyles::default(),
                selection.clone(),
                ctx,
            );

            assert_eq!(
                buffer.content.debug(),
                "<text>Test<code:Shell><c_#ffffff>ock<c>\\n<c_#000000>Line<c><text>Block"
            );

            buffer.edit_internal_first_selection(
                CharOffset::from(9)..CharOffset::from(12),
                "",
                TextStyles::default(),
                selection.clone(),
                ctx,
            );

            assert_eq!(
                buffer.content.debug(),
                "<text>Test<code:Shell><c_#ffffff>ock<c><c_#000000>ne<c><text>Block"
            );
        });
    });
}

#[test]
fn test_unstyling_code_block_do_not_leak_syntax_color() {
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            buffer.edit_internal_first_selection(
                CharOffset::from(1)..CharOffset::from(1),
                "Test\nBlock\nLine\nBlock",
                Default::default(),
                selection.clone(),
                ctx,
            );

            buffer.block_style_range(
                CharOffset::from(6)..CharOffset::from(16),
                BufferBlockStyle::CodeBlock {
                    code_block_type: CodeBlockType::Shell,
                },
                selection.clone(),
                ctx,
            );

            assert_eq!(
                buffer.content.debug(),
                "<text>Test<code:Shell>Block\\nLine<text>Block"
            );

            buffer.color_code_block_ranges_internal(
                CharOffset::from(6),
                &[
                    (ByteOffset::from(0)..ByteOffset::from(5), ColorU::white()),
                    (ByteOffset::from(6)..ByteOffset::from(10), ColorU::black()),
                ],
            );

            assert_eq!(
                buffer.content.debug(),
                "<text>Test<code:Shell><c_#ffffff>Block<c>\\n<c_#000000>Line<c><text>Block"
            );

            buffer.block_style_range(
                CharOffset::from(6)..CharOffset::from(8),
                BufferBlockStyle::PlainText,
                selection.clone(),
                ctx,
            );

            // We should clear syntax color and wait for re-highlighting.
            assert_eq!(
                buffer.content.debug(),
                "<text>Test\\nBl<code:Shell>ock\\nLine<text>Block"
            );

            buffer.block_style_range(
                CharOffset::from(9)..CharOffset::from(17),
                BufferBlockStyle::PlainText,
                selection.clone(),
                ctx,
            );

            assert_eq!(
                buffer.content.debug(),
                "<text>Test\\nBl\\nock\\nLine\\nBlock"
            );
        });
    });
}

// Regression test for CLD-1218.
#[test]
fn test_undo_should_not_leak_syntax_color() {
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            buffer.edit_internal_first_selection(
                CharOffset::from(1)..CharOffset::from(1),
                "Test\nBlock\nLine\nBlock",
                Default::default(),
                selection.clone(),
                ctx,
            );

            let prev_selection = buffer.to_rendered_selection_set(selection.clone(), ctx);
            let edit_result = buffer.block_style_range(
                CharOffset::from(6)..CharOffset::from(16),
                BufferBlockStyle::CodeBlock {
                    code_block_type: CodeBlockType::Shell,
                },
                selection.clone(),
                ctx,
            );
            let curr_selection = buffer.to_rendered_selection_set(selection.clone(), ctx);
            buffer.push_undo_item(
                prev_selection,
                curr_selection,
                edit_result.undo_item.expect("Should exist"),
                UndoActionType::Atomic,
            );

            assert_eq!(
                buffer.content.debug(),
                "<text>Test<code:Shell>Block\\nLine<text>Block"
            );

            buffer.color_code_block_ranges_internal(
                CharOffset::from(6),
                &[
                    (ByteOffset::from(0)..ByteOffset::from(5), ColorU::white()),
                    (ByteOffset::from(6)..ByteOffset::from(10), ColorU::black()),
                ],
            );

            assert_eq!(
                buffer.content.debug(),
                "<text>Test<code:Shell><c_#ffffff>Block<c>\\n<c_#000000>Line<c><text>Block"
            );

            buffer.undo(selection.clone(), ctx);
            assert_eq!(buffer.content.debug(), "<text>Test\\nBlock\\nLine\\nBlock");

            buffer.redo(selection.clone(), ctx);
            assert_eq!(
                buffer.content.debug(),
                "<text>Test<code:Shell>Block\\nLine<text>Block"
            );
        });
    });
}

#[test]
fn test_insert_new_block_should_not_leak_style() {
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            buffer.edit_internal_first_selection(
                CharOffset::from(1)..CharOffset::from(1),
                "Test",
                Default::default(),
                selection.clone(),
                ctx,
            );
            buffer.set_selection(
                CharOffset::from(1)..CharOffset::from(5),
                selection.clone(),
                ctx,
            );
            buffer.style_internal(TextStyles::default().bold(), selection.clone(), ctx);
            assert_eq!(buffer.content.debug(), "<text><b_s>Test<b_e>");

            buffer.insert_block_after_block_with_offset(
                CharOffset::from(1),
                BlockType::Text(BufferBlockStyle::PlainText),
                selection.clone(),
                ctx,
            );
            assert_eq!(buffer.content.debug(), "<text><b_s>Test<b_e>\\n");
        });
    });
}

// Regression test for CLD-1106.
#[test]
fn test_deletion_range_include_block_marker_should_not_leak_style() {
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            buffer.edit_internal_first_selection(
                CharOffset::from(1)..CharOffset::from(1),
                "TestBlock",
                Default::default(),
                selection.clone(),
                ctx,
            );
            buffer.block_style_range(
                CharOffset::from(5)..CharOffset::from(10),
                BufferBlockStyle::CodeBlock {
                    code_block_type: CodeBlockType::Shell,
                },
                selection.clone(),
                ctx,
            );
            buffer.color_code_block_ranges_internal(
                CharOffset::from(6),
                &[(ByteOffset::from(0)..ByteOffset::from(5), ColorU::white())],
            );
            assert_eq!(
                buffer.content.debug(),
                "<text>Test<code:Shell><c_#ffffff>Block<c><text>"
            );

            buffer.edit_internal_first_selection(
                CharOffset::from(4)..CharOffset::from(7),
                "",
                TextStyles::default(),
                selection.clone(),
                ctx,
            );
            assert_eq!(buffer.content.debug(), "<text>Teslock\\n");
        });
    });
}

#[test]
fn test_insert_embedding() {
    App::test((), |mut app| async move {
        let (buffer, selection) = Buffer::mock_from_markdown(
            "",
            Some(
                |mut mapping| match mapping.remove(&Value::String("id".to_string())) {
                    Some(Value::String(hashed_id)) => {
                        Some(Arc::new(TestEmbeddedItem { id: hashed_id }))
                    }
                    _ => None,
                },
            ),
            Box::new(|_, _| IndentBehavior::Ignore),
            &mut app,
        );

        buffer.update(&mut app, |buffer, ctx| {
            buffer.edit_internal_first_selection(
                CharOffset::from(1)..CharOffset::from(1),
                "Before\nAfter",
                Default::default(),
                selection.clone(),
                ctx,
            );

            let prev_selection = buffer.to_rendered_selection_set(selection.clone(), ctx);
            let edit_result = buffer.replace_with_formatted_text(
                CharOffset::from(7)..CharOffset::from(7),
                FormattedText::new(vec![FormattedTextLine::Embedded(Mapping::from_iter([(
                    Value::String("id".to_string()),
                    Value::String("workflow-123".to_string()),
                )]))]),
                EditOrigin::UserInitiated,
                selection.clone(),
                ctx,
            );
            let curr_selection = buffer.to_rendered_selection_set(selection.clone(), ctx);

            buffer.push_undo_item(
                prev_selection,
                curr_selection,
                edit_result.undo_item.expect("Should exist"),
                UndoActionType::Atomic,
            );
            assert_eq!(
                buffer.content.debug(),
                "<text>Before<embed_workflow-123><text>After"
            );

            let delta = edit_result.delta.expect("Should exist");
            assert_eq!(delta.old_offset, CharOffset::from(1)..CharOffset::from(8));
            assert_eq!(
                delta.new_lines,
                vec![
                    StyledBufferBlock::Text(StyledTextBlock {
                        block: vec![StyledBufferRun {
                            run: "Before\n".to_string(),
                            text_styles: TextStylesWithMetadata::default(),
                            block_style: BufferBlockStyle::PlainText
                        },],
                        style: BufferBlockStyle::PlainText,
                        content_length: CharOffset::from(7)
                    }),
                    StyledBufferBlock::Item(BufferBlockItem::Embedded {
                        item: Arc::new(TestEmbeddedItem {
                            id: "workflow-123".to_string()
                        })
                    }),
                ]
            );

            buffer.undo(selection.clone(), ctx);
            assert_eq!(buffer.content.debug(), "<text>Before\\nAfter");

            buffer.redo(selection.clone(), ctx);
            assert_eq!(
                buffer.content.debug(),
                "<text>Before<embed_workflow-123><text>After"
            );
        });
    });
}

#[test]
#[should_panic(expected = "Trying to replace embedding at 1, but offset is not an embedding.")]
fn test_should_panic_on_replacing_embedding_on_wrong_block_type() {
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            buffer.edit_internal_first_selection(
                CharOffset::from(1)..CharOffset::from(1),
                "TestBlock",
                Default::default(),
                selection.clone(),
                ctx,
            );
            buffer.replace_embedding_at_offset_internal(
                CharOffset::from(1),
                Arc::new(TestEmbeddedItem {
                    id: "workflow-234".to_string(),
                }),
            );
        });
    });
}

#[test]
fn test_replace_or_remove_embedding() {
    App::test((), |mut app| async move {
        let (buffer, selection) = Buffer::mock_from_markdown(
            "",
            Some(|mut mapping: Mapping| -> Option<Arc<dyn EmbeddedItem>> {
                match mapping.remove(&Value::String("id".to_string())) {
                    Some(Value::String(hashed_id)) => {
                        Some(Arc::new(TestEmbeddedItem { id: hashed_id }))
                    }
                    _ => None,
                }
            }),
            Box::new(|_, _| IndentBehavior::Ignore),
            &mut app,
        );

        buffer.update(&mut app, |buffer, ctx| {
            buffer.edit_internal_first_selection(
                CharOffset::from(1)..CharOffset::from(1),
                "Before\nAfter",
                Default::default(),
                selection.clone(),
                ctx,
            );

            buffer.replace_with_formatted_text(
                CharOffset::from(7)..CharOffset::from(7),
                FormattedText::new(vec![FormattedTextLine::Embedded(Mapping::from_iter([(
                    Value::String("id".to_string()),
                    Value::String("workflow-123".to_string()),
                )]))]),
                EditOrigin::UserInitiated,
                selection.clone(),
                ctx,
            );

            assert_eq!(
                buffer.content.debug(),
                "<text>Before<embed_workflow-123><text>After"
            );

            buffer.replace_embedding_at_offset_internal(
                CharOffset::from(7),
                Arc::new(TestEmbeddedItem {
                    id: "workflow-234".to_string(),
                }),
            );
            assert_eq!(
                buffer.content.debug(),
                "<text>Before<embed_workflow-234><text>After"
            );

            buffer.remove_embedding_at_offset(CharOffset::from(7), selection.clone(), ctx);
            assert_eq!(buffer.content.debug(), "<text>Before\\nAfter");
        });
    });
}

#[test]
fn test_styled_blocks_from_buffer_start() {
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            buffer.edit_internal_first_selection(
                CharOffset::from(1)..CharOffset::from(1),
                "A\nB",
                Default::default(),
                selection.clone(),
                ctx,
            );
            buffer.block_style_range(
                CharOffset::from(1)..CharOffset::from(2),
                BufferBlockStyle::Header {
                    header_size: BlockHeaderSize::Header1,
                },
                selection.clone(),
                ctx,
            );
            assert_eq!(buffer.debug(), "<header1>A<text>B");
            assert_eq!(
                buffer.styled_blocks_in_range(
                    CharOffset::zero()..buffer.max_charoffset(),
                    StyledBlockBoundaryBehavior::Exclusive
                ),
                vec![
                    StyledBufferBlock::Text(StyledTextBlock {
                        block: vec![StyledBufferRun {
                            run: "A\n".to_string(),
                            text_styles: Default::default(),
                            block_style: BufferBlockStyle::Header {
                                header_size: BlockHeaderSize::Header1
                            }
                        }],
                        style: BufferBlockStyle::Header {
                            header_size: BlockHeaderSize::Header1,
                        },
                        content_length: CharOffset::from(2)
                    }),
                    StyledBufferBlock::Text(StyledTextBlock {
                        block: vec![StyledBufferRun {
                            run: "B".to_string(),
                            text_styles: Default::default(),
                            block_style: BufferBlockStyle::PlainText
                        }],
                        style: BufferBlockStyle::PlainText,
                        content_length: CharOffset::from(1)
                    })
                ]
            );
        });
    });
}

#[test]
fn test_styled_block_default_boundaries() {
    // As part of CLD-1178, this tests the StyledBufferBlocks iterator at block boundaries.
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            buffer.edit_internal_first_selection(
                CharOffset::from(1)..CharOffset::from(1),
                "code\ntext\ntext\nmore text",
                Default::default(),
                selection.clone(),
                ctx,
            );
            buffer.block_style_range(
                1.into()..5.into(),
                BufferBlockStyle::CodeBlock {
                    code_block_type: CodeBlockType::Shell,
                },
                selection.clone(),
                ctx,
            );
            buffer.insert_block_after_block_with_offset(
                6.into(),
                BlockType::Item(BufferBlockItem::HorizontalRule),
                selection.clone(),
                ctx,
            );
            assert_eq!(
                buffer.debug(),
                "<code:Shell>code<text>text<hr><text>text\\nmore text"
            );

            buffer.insert_block_after_block_with_offset(
                17.into(),
                BlockType::Item(BufferBlockItem::HorizontalRule),
                selection.clone(),
                ctx,
            );
            assert_eq!(
                buffer.debug(),
                "<code:Shell>code<text>text<hr><text>text\\nmore text<hr><text>"
            );

            buffer.insert_block_after_block_with_offset(
                26.into(),
                BlockType::Item(BufferBlockItem::HorizontalRule),
                selection.clone(),
                ctx,
            );

            assert_eq!(
                buffer.debug(),
                "<code:Shell>code<text>text<hr><text>text\\nmore text<hr><hr><text>"
            );

            assert_eq!(buffer.containing_block_start(2.into()), 1.into());
            assert_eq!(buffer.containing_block_end(2.into()), 6.into());

            assert_eq!(buffer.containing_block_start(7.into()), 6.into());
            assert_eq!(buffer.containing_block_end(7.into()), 11.into());

            assert_eq!(buffer.containing_block_start(11.into()), 11.into());
            assert_eq!(buffer.containing_block_end(11.into()), 12.into());

            assert_eq!(buffer.containing_block_start(13.into()), 12.into());
            assert_eq!(buffer.containing_block_end(13.into()), 27.into());
            assert_eq!(buffer.containing_line_end(13.into()), 17.into());

            // Edit at 7, this is the start to end.
            assert_eq!(
                buffer.styled_blocks_in_range(
                    6.into()..11.into(),
                    StyledBlockBoundaryBehavior::Exclusive
                ),
                vec![StyledBufferBlock::Text(StyledTextBlock {
                    block: vec![StyledBufferRun {
                        run: "text\n".to_string(),
                        text_styles: Default::default(),
                        block_style: BufferBlockStyle::PlainText
                    },],
                    style: BufferBlockStyle::PlainText,
                    // Horizontal rule shouldn't be included - it parses as a newline, but isn't complete.
                    content_length: CharOffset::from(5)
                })]
            );

            // This is the first <hr>.
            assert_eq!(
                buffer.styled_blocks_in_range(
                    11.into()..12.into(),
                    StyledBlockBoundaryBehavior::Exclusive
                ),
                vec![StyledBufferBlock::Item(BufferBlockItem::HorizontalRule)]
            );

            assert_eq!(buffer.containing_block_start(27.into()), 27.into());
            assert_eq!(buffer.containing_block_end(27.into()), 28.into());
            assert_eq!(
                buffer.styled_blocks_in_range(
                    27.into()..28.into(),
                    StyledBlockBoundaryBehavior::Exclusive
                ),
                vec![StyledBufferBlock::Item(BufferBlockItem::HorizontalRule)]
            );

            assert_eq!(buffer.containing_block_start(28.into()), 28.into());
            assert_eq!(buffer.containing_block_end(28.into()), 29.into());
            assert_eq!(
                buffer.styled_blocks_in_range(
                    28.into()..29.into(),
                    StyledBlockBoundaryBehavior::Exclusive
                ),
                vec![StyledBufferBlock::Item(BufferBlockItem::HorizontalRule)]
            );
        });
    });
}

#[test]
fn test_styled_block_alternate_boundaries() {
    // This implements the example in the `StyledBlockBoundaryBehavior` doc comment, to test
    // non-default boundary behavior.
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            buffer.edit_internal_first_selection(
                CharOffset::from(1)..CharOffset::from(1),
                "Hello\necho hello\nFirst\nSecond",
                Default::default(),
                selection.clone(),
                ctx,
            );

            buffer.block_style_range(
                7.into()..17.into(),
                BufferBlockStyle::CodeBlock {
                    code_block_type: CodeBlockType::Shell,
                },
                selection.clone(),
                ctx,
            );
            buffer.block_style_range(
                18.into()..30.into(),
                BufferBlockStyle::UnorderedList {
                    indent_level: ListIndentLevel::One,
                },
                selection.clone(),
                ctx,
            );
            buffer.insert_block_after_block_with_offset(
                7.into(),
                BlockType::Item(BufferBlockItem::HorizontalRule),
                selection.clone(),
                ctx,
            );
            assert_eq!(
                buffer.debug(),
                "<text>Hello<code:Shell>echo hello<hr><ul0>First<ul0>Second<text>"
            );

            let range = 3.into()..7.into();
            assert_eq!(
                buffer
                    .styled_blocks_in_range(range.clone(), StyledBlockBoundaryBehavior::Exclusive),
                vec![StyledBufferBlock::Text(StyledTextBlock {
                    block: vec![StyledBufferRun {
                        run: "llo\n".to_string(),
                        block_style: BufferBlockStyle::PlainText,
                        text_styles: Default::default()
                    }],
                    style: BufferBlockStyle::PlainText,
                    content_length: CharOffset::from(4)
                })]
            );
            assert_eq!(
                buffer.styled_blocks_in_range(
                    range.clone(),
                    StyledBlockBoundaryBehavior::InclusiveBlockItems
                ),
                vec![StyledBufferBlock::Text(StyledTextBlock {
                    block: vec![StyledBufferRun {
                        run: "llo\n".to_string(),
                        block_style: BufferBlockStyle::PlainText,
                        text_styles: Default::default()
                    }],
                    style: BufferBlockStyle::PlainText,
                    content_length: CharOffset::from(4)
                })]
            );
            assert_eq!(
                buffer
                    .styled_blocks_in_range(range.clone(), StyledBlockBoundaryBehavior::Inclusive),
                vec![
                    StyledBufferBlock::Text(StyledTextBlock {
                        block: vec![StyledBufferRun {
                            run: "llo\n".to_string(),
                            block_style: BufferBlockStyle::PlainText,
                            text_styles: Default::default()
                        }],
                        style: BufferBlockStyle::PlainText,
                        content_length: CharOffset::from(4)
                    }),
                    StyledBufferBlock::Text(StyledTextBlock {
                        block: vec![],
                        style: BufferBlockStyle::CodeBlock {
                            code_block_type: CodeBlockType::Shell,
                        },
                        content_length: CharOffset::from(0)
                    })
                ]
            );

            let range = 7.into()..18.into();
            assert_eq!(
                buffer
                    .styled_blocks_in_range(range.clone(), StyledBlockBoundaryBehavior::Exclusive),
                vec![StyledBufferBlock::Text(StyledTextBlock {
                    block: vec![StyledBufferRun {
                        run: "echo hello\n".to_string(),
                        block_style: BufferBlockStyle::CodeBlock {
                            code_block_type: CodeBlockType::Shell
                        },
                        text_styles: Default::default()
                    }],
                    style: BufferBlockStyle::CodeBlock {
                        code_block_type: CodeBlockType::Shell,
                    },
                    content_length: CharOffset::from(11)
                })]
            );
            assert_eq!(
                buffer
                    .styled_blocks_in_range(range.clone(), StyledBlockBoundaryBehavior::Inclusive),
                vec![
                    StyledBufferBlock::Text(StyledTextBlock {
                        block: vec![StyledBufferRun {
                            run: "echo hello\n".to_string(),
                            block_style: BufferBlockStyle::CodeBlock {
                                code_block_type: CodeBlockType::Shell
                            },
                            text_styles: Default::default()
                        }],
                        style: BufferBlockStyle::CodeBlock {
                            code_block_type: CodeBlockType::Shell,
                        },
                        content_length: CharOffset::from(11)
                    }),
                    StyledBufferBlock::Item(BufferBlockItem::HorizontalRule)
                ]
            );
            assert_eq!(
                buffer.styled_blocks_in_range(
                    range.clone(),
                    StyledBlockBoundaryBehavior::InclusiveBlockItems
                ),
                vec![
                    StyledBufferBlock::Text(StyledTextBlock {
                        block: vec![StyledBufferRun {
                            run: "echo hello\n".to_string(),
                            block_style: BufferBlockStyle::CodeBlock {
                                code_block_type: CodeBlockType::Shell
                            },
                            text_styles: Default::default()
                        }],
                        style: BufferBlockStyle::CodeBlock {
                            code_block_type: CodeBlockType::Shell,
                        },
                        content_length: CharOffset::from(11)
                    }),
                    StyledBufferBlock::Item(BufferBlockItem::HorizontalRule)
                ]
            );

            let range = 19.into()..24.into();
            assert_eq!(
                buffer
                    .styled_blocks_in_range(range.clone(), StyledBlockBoundaryBehavior::Exclusive),
                vec![StyledBufferBlock::Text(StyledTextBlock {
                    block: vec![StyledBufferRun {
                        run: "First".to_string(),
                        block_style: BufferBlockStyle::UnorderedList {
                            indent_level: ListIndentLevel::One
                        },
                        text_styles: Default::default()
                    }],
                    style: BufferBlockStyle::UnorderedList {
                        indent_level: ListIndentLevel::One,
                    },
                    content_length: CharOffset::from(5)
                })]
            );
            assert_eq!(
                buffer
                    .styled_blocks_in_range(range.clone(), StyledBlockBoundaryBehavior::Inclusive),
                vec![StyledBufferBlock::Text(StyledTextBlock {
                    block: vec![StyledBufferRun {
                        run: "First".to_string(),
                        block_style: BufferBlockStyle::UnorderedList {
                            indent_level: ListIndentLevel::One
                        },
                        text_styles: Default::default()
                    }],
                    style: BufferBlockStyle::UnorderedList {
                        indent_level: ListIndentLevel::One,
                    },
                    content_length: CharOffset::from(5)
                })]
            );
            assert_eq!(
                buffer.styled_blocks_in_range(
                    range.clone(),
                    StyledBlockBoundaryBehavior::InclusiveBlockItems
                ),
                vec![StyledBufferBlock::Text(StyledTextBlock {
                    block: vec![StyledBufferRun {
                        run: "First".to_string(),
                        block_style: BufferBlockStyle::UnorderedList {
                            indent_level: ListIndentLevel::One
                        },
                        text_styles: Default::default()
                    }],
                    style: BufferBlockStyle::UnorderedList {
                        indent_level: ListIndentLevel::One,
                    },
                    content_length: CharOffset::from(5)
                })]
            );
        });
    });
}

#[test]
fn test_deleting_range_with_trailing_block_marker() {
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            buffer.edit_internal_first_selection(
                CharOffset::from(1)..CharOffset::from(1),
                "A\nB\nC",
                Default::default(),
                selection.clone(),
                ctx,
            );

            buffer.block_style_range(
                CharOffset::from(3)..CharOffset::from(4),
                BufferBlockStyle::CodeBlock {
                    code_block_type: CodeBlockType::Shell,
                },
                selection.clone(),
                ctx,
            );
            assert_eq!(buffer.content.debug(), "<text>A<code:Shell>B<text>C");

            let prev_selection = buffer.to_rendered_selection_set(selection.clone(), ctx);
            let edit_result = buffer.edit_internal_first_selection(
                CharOffset::from(1)..CharOffset::from(5),
                "",
                Default::default(),
                selection.clone(),
                ctx,
            );
            let delta = edit_result.delta.unwrap();
            assert_eq!(delta.old_offset, 1.into()..6.into());
            assert_eq!(
                delta.new_lines,
                vec![StyledBufferBlock::Text(StyledTextBlock {
                    block: vec![StyledBufferRun {
                        run: "C".to_string(),
                        text_styles: Default::default(),
                        block_style: BufferBlockStyle::PlainText
                    }],
                    style: BufferBlockStyle::PlainText,
                    content_length: CharOffset::from(1)
                })]
            );

            let undo_item = edit_result.undo_item.expect("Should exist");
            let curr_selection = buffer.to_rendered_selection_set(selection.clone(), ctx);
            buffer.push_undo_item(
                prev_selection,
                curr_selection,
                undo_item,
                UndoActionType::Atomic,
            );
            assert_eq!(buffer.content.debug(), "<text>C");

            buffer.undo(selection.clone(), ctx);
            assert_eq!(buffer.content.debug(), "<text>A<code:Shell>B<text>C");

            buffer.redo(selection.clone(), ctx);
            assert_eq!(buffer.content.debug(), "<text>C");
        });
    });
}

#[test]
fn test_insert_block_at_buffer_start() {
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            buffer.edit_internal_first_selection(
                CharOffset::from(1)..CharOffset::from(1),
                "\n",
                Default::default(),
                selection.clone(),
                ctx,
            );
            assert_eq!(buffer.content.debug(), "<text>\\n");

            let prev_selection = buffer.to_rendered_selection_set(selection.clone(), ctx);
            let edit_result = buffer.insert_block_item(
                BufferBlockItem::HorizontalRule,
                CharOffset::from(1)..CharOffset::from(1),
            );
            assert_eq!(buffer.content.debug(), "<hr><text>");

            let delta = edit_result.delta.unwrap();
            assert_eq!(delta.old_offset, 0.into()..2.into());
            assert_eq!(
                delta.new_lines,
                vec![StyledBufferBlock::Item(BufferBlockItem::HorizontalRule)]
            );

            let undo_item = edit_result.undo_item.expect("Should exist");
            let curr_selection = buffer.to_rendered_selection_set(selection.clone(), ctx);
            buffer.push_undo_item(
                prev_selection,
                curr_selection,
                undo_item,
                UndoActionType::Atomic,
            );

            buffer.undo(selection.clone(), ctx);
            assert_eq!(buffer.content.debug(), "<text>\\n");

            buffer.redo(selection.clone(), ctx);
            assert_eq!(buffer.content.debug(), "<hr><text>");
        });
    });
}

#[test]
fn test_insert_block_in_middle() {
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            buffer.edit_internal_first_selection(
                CharOffset::from(1)..CharOffset::from(1),
                "text\nline",
                Default::default(),
                selection.clone(),
                ctx,
            );

            let prev_selection = buffer.to_rendered_selection_set(selection.clone(), ctx);
            let edit_result = buffer.insert_block_item(
                BufferBlockItem::HorizontalRule,
                CharOffset::from(5)..CharOffset::from(6),
            );
            assert_eq!(buffer.content.debug(), "<text>text<hr><text>line");

            let delta = edit_result.delta.unwrap();
            assert_eq!(delta.old_offset, 1.into()..10.into());
            assert_eq!(
                delta.new_lines,
                vec![
                    StyledBufferBlock::Text(StyledTextBlock {
                        block: vec![StyledBufferRun {
                            run: "text\n".to_string(),
                            text_styles: Default::default(),
                            block_style: BufferBlockStyle::PlainText
                        }],
                        style: BufferBlockStyle::PlainText,
                        content_length: CharOffset::from(5)
                    }),
                    StyledBufferBlock::Item(BufferBlockItem::HorizontalRule),
                    StyledBufferBlock::Text(StyledTextBlock {
                        block: vec![StyledBufferRun {
                            run: "line".to_string(),
                            text_styles: Default::default(),
                            block_style: BufferBlockStyle::PlainText
                        }],
                        style: BufferBlockStyle::PlainText,
                        content_length: CharOffset::from(4)
                    }),
                ]
            );

            let undo_item = edit_result.undo_item.expect("Should exist");
            let curr_selection = buffer.to_rendered_selection_set(selection.clone(), ctx);
            buffer.push_undo_item(
                prev_selection,
                curr_selection,
                undo_item,
                UndoActionType::Atomic,
            );

            buffer.undo(selection.clone(), ctx);
            assert_eq!(buffer.content.debug(), "<text>text\\nline");

            buffer.redo(selection.clone(), ctx);
            assert_eq!(buffer.content.debug(), "<text>text<hr><text>line");
        });
    });
}

#[test]
fn test_backspace_on_block_item_at_buffer_start() {
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            buffer.edit_internal_first_selection(
                CharOffset::from(1)..CharOffset::from(1),
                "\n",
                Default::default(),
                selection.clone(),
                ctx,
            );

            buffer.insert_block_item(
                BufferBlockItem::HorizontalRule,
                CharOffset::from(1)..CharOffset::from(1),
            );
            assert_eq!(buffer.content.debug(), "<hr><text>");

            let prev_selection = buffer.to_rendered_selection_set(selection.clone(), ctx);
            let edit_result = buffer.backspace(&mut None, selection.clone(), ctx);
            assert_eq!(buffer.content.debug(), "<text>");

            let delta = edit_result.delta.unwrap();
            assert_eq!(delta.old_offset, 0.into()..2.into());
            assert_eq!(delta.new_lines, vec![]);

            let undo_item = edit_result.undo_item.expect("Should exist");
            let curr_selection = buffer.to_rendered_selection_set(selection.clone(), ctx);
            buffer.push_undo_item(
                prev_selection,
                curr_selection,
                undo_item,
                UndoActionType::Atomic,
            );

            buffer.undo(selection.clone(), ctx);
            assert_eq!(buffer.content.debug(), "<hr><text>");

            buffer.redo(selection.clone(), ctx);
            assert_eq!(buffer.content.debug(), "<text>");
        });
    });
}

#[test]
fn test_multiple_selections() {
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            buffer.edit_internal_first_selection(
                CharOffset::from(1)..CharOffset::from(1),
                "This is some text!",
                Default::default(),
                selection.clone(),
                ctx,
            );

            assert_eq!(selection.as_ref(ctx).selection_offsets().len(), 1);

            buffer.add_cursor(4.into(), false, selection.clone(), ctx);

            assert_eq!(selection.as_ref(ctx).selection_offsets().len(), 2);
            assert_eq!(
                selection.as_ref(ctx).selections_to_offset_ranges()[0],
                19.into()..19.into()
            );
            assert_eq!(
                selection.as_ref(ctx).selections_to_offset_ranges()[1],
                4.into()..4.into()
            );
        });
    });
}

#[test]
fn test_multiselect_insert() {
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            buffer.edit_internal_first_selection(
                CharOffset::from(1)..CharOffset::from(1),
                "two three four five",
                Default::default(),
                selection.clone(),
                ctx,
            );

            // Two cursors
            buffer.add_cursor(1.into(), true, selection.clone(), ctx);
            buffer.add_cursor(5.into(), false, selection.clone(), ctx);

            // Insert "one"
            let prev_selection = buffer.to_rendered_selection_set(selection.clone(), ctx);
            let edit_result =
                buffer.edit_internal("one ", Default::default(), selection.clone(), ctx);

            buffer.push_undo_item_nonatomic(
                prev_selection,
                edit_result.undo_item.expect("Should exist"),
                NonAtomicType::Insert,
                selection.clone(),
                ctx,
            );

            assert_eq!(buffer.content.debug(), "<text>one two one three four five");

            // Undo
            buffer.undo(selection.clone(), ctx);
            assert_eq!(buffer.content.debug(), "<text>two three four five");

            // Insert "one " again and then "six "
            buffer.edit_internal("one ", Default::default(), selection.clone(), ctx);
            buffer.edit_internal("six ", Default::default(), selection.clone(), ctx);
            assert_eq!(
                buffer.content.debug(),
                "<text>one six two one six three four five"
            );

            // Two selections
            buffer.add_cursor(1.into(), true, selection.clone(), ctx);
            buffer.set_last_head(9.into(), selection.clone(), ctx);
            buffer.add_cursor(13.into(), false, selection.clone(), ctx);
            buffer.set_last_head(17.into(), selection.clone(), ctx);
            let prev_selection = buffer.to_rendered_selection_set(selection.clone(), ctx);

            // Replace two selections with "hi"
            let edit_result =
                buffer.edit_internal("hi ", Default::default(), selection.clone(), ctx);
            assert_eq!(
                buffer.content.debug(),
                "<text>hi two hi six three four five"
            );

            buffer.push_undo_item_nonatomic(
                prev_selection,
                edit_result.undo_item.expect("Should exist"),
                NonAtomicType::Insert,
                selection.clone(),
                ctx,
            );

            buffer.undo(selection.clone(), ctx);
            assert_eq!(
                buffer.content.debug(),
                "<text>one six two one six three four five"
            );
            assert_eq!(
                selection.as_ref(ctx).selections_to_offset_ranges(),
                vec1![1.into()..9.into(), 13.into()..17.into()]
            );

            buffer.redo(selection.clone(), ctx);
            assert_eq!(
                buffer.content.debug(),
                "<text>hi two hi six three four five"
            );
        });
    });
}

#[test]
fn test_multiselect_movement() {
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            let _ = buffer.edit_internal_first_selection(
                CharOffset::from(1)..CharOffset::from(1),
                "te\nst\nhey",
                TextStyles::default(),
                selection.clone(),
                ctx,
            );
            assert_eq!(buffer.content.debug(), "<text>te\\nst\\nhey");
            assert_eq!(
                selection.as_ref(ctx).selections_to_offset_ranges(),
                vec1![CharOffset::from(10)..CharOffset::from(10)]
            );

            // Add another cursor.
            buffer.add_cursor(3.into(), false, selection.clone(), ctx);

            // This should be no-op for one cursor.
            buffer.update_selection(
                selection.clone(),
                BufferSelectAction::MoveRight,
                AutoScrollBehavior::Selection,
                ctx,
            );
            assert_eq!(
                selection.as_ref(ctx).selections_to_offset_ranges(),
                vec1![
                    CharOffset::from(10)..CharOffset::from(10),
                    CharOffset::from(4)..CharOffset::from(4)
                ]
            );

            buffer.update_selection(
                selection.clone(),
                BufferSelectAction::MoveLeft,
                AutoScrollBehavior::Selection,
                ctx,
            );
            assert_eq!(
                selection.as_ref(ctx).selections_to_offset_ranges(),
                vec1![
                    CharOffset::from(9)..CharOffset::from(9),
                    CharOffset::from(3)..CharOffset::from(3)
                ]
            );

            buffer.extend_selection_left(2, selection.clone(), ctx);
            assert_eq!(
                selection.as_ref(ctx).selections_to_offset_ranges(),
                vec1![
                    CharOffset::from(7)..CharOffset::from(9),
                    CharOffset::from(1)..CharOffset::from(3)
                ]
            );

            buffer.extend_selection_right(3, selection.clone(), ctx);
            assert_eq!(
                selection.as_ref(ctx).selections_to_offset_ranges(),
                vec1![
                    CharOffset::from(9)..CharOffset::from(10),
                    CharOffset::from(3)..CharOffset::from(4)
                ]
            );

            buffer.update_selection(
                selection.clone(),
                BufferSelectAction::MoveRight,
                AutoScrollBehavior::Selection,
                ctx,
            );
            assert_eq!(
                selection.as_ref(ctx).selections_to_offset_ranges(),
                vec1![
                    CharOffset::from(10)..CharOffset::from(10),
                    CharOffset::from(4)..CharOffset::from(4)
                ]
            );

            buffer.add_cursor(1.into(), false, selection.clone(), ctx);
            // Should be no-op for the last cursor.
            buffer.update_selection(
                selection.clone(),
                BufferSelectAction::MoveLeft,
                AutoScrollBehavior::Selection,
                ctx,
            );
            assert_eq!(
                selection.as_ref(ctx).selections_to_offset_ranges(),
                vec1![
                    CharOffset::from(9)..CharOffset::from(9),
                    CharOffset::from(3)..CharOffset::from(3),
                    CharOffset::from(1)..CharOffset::from(1)
                ]
            );

            buffer.update_selection(
                selection.clone(),
                BufferSelectAction::MoveRight,
                AutoScrollBehavior::Selection,
                ctx,
            );
            assert_eq!(
                selection.as_ref(ctx).selections_to_offset_ranges(),
                vec1![
                    CharOffset::from(10)..CharOffset::from(10),
                    CharOffset::from(4)..CharOffset::from(4),
                    CharOffset::from(2)..CharOffset::from(2)
                ]
            );
        });
    });
}

#[test]
fn test_multiselect_enter_at_block_start() {
    // This tests that Enter at the start of a list or heading block preserves its styling and
    // inserts a new line above the block.
    App::test((), |mut app| async move {
        let (buffer, selection) = Buffer::mock_from_markdown(
            "Text\n3. List\n4. Next\n",
            None,
            Box::new(|_, _| IndentBehavior::Ignore),
            &mut app,
        );

        buffer.update(&mut app, |buffer, ctx| {
            assert_eq!(buffer.debug(), "<text>Text<ol0@3>List<ol0>Next<text>");

            buffer.add_cursor(6.into(), true, selection.clone(), ctx);
            buffer.add_cursor(11.into(), false, selection.clone(), ctx);

            buffer.enter(false, TextStyles::default(), selection.clone(), ctx);

            assert_eq!(
                buffer.debug(),
                "<text>Text<ol0@3><ol0>List<ol0><ol0>Next<text>"
            );
        });
    });
}

#[test]
fn test_multiselect_enter_at_code_block_start() {
    App::test((), |mut app| async move {
        let (buffer, selection) = Buffer::mock_from_markdown(
            "```\nThis is code\n```\n```\nMore code\n```",
            None,
            Box::new(|_, _| IndentBehavior::Ignore),
            &mut app,
        );

        buffer.update(&mut app, |buffer, ctx| {
            assert_eq!(
                buffer.debug(),
                "<code:Shell>This is code<code:Shell>More code<text>"
            );

            buffer.add_cursor(1.into(), true, selection.clone(), ctx);
            buffer.add_cursor(14.into(), false, selection.clone(), ctx);

            buffer.enter(false, TextStyles::default(), selection.clone(), ctx);

            assert_eq!(
                buffer.debug(),
                "<text><code:Shell>This is code<text><code:Shell>More code<text>"
            );
        });
    });
}

#[test]
fn test_multiselect_enter_at_code_block() {
    // This was a weird interaction between multiselecting an empty list line and the start of a code block.
    App::test((), |mut app| async move {
        let (buffer, selection) = Buffer::mock_from_markdown(
            "* Hey\n```\nThis is code\n```\n",
            None,
            Box::new(|_, _| IndentBehavior::Ignore),
            &mut app,
        );

        buffer.update(&mut app, |buffer, ctx| {
            // Add another empty bullet point.
            buffer.add_cursor(4.into(), true, selection.clone(), ctx);
            buffer.enter(false, TextStyles::default(), selection.clone(), ctx);

            assert_eq!(
                buffer.debug(),
                "<ul0>Hey<ul0><code:Shell>This is code<text>"
            );

            buffer.add_cursor(6.into(), false, selection.clone(), ctx);
            buffer.enter(false, TextStyles::default(), selection.clone(), ctx);

            assert_eq!(
                buffer.debug(),
                "<ul0>Hey<text>\\n<code:Shell>This is code<text>"
            );
        });
    });
}

#[test]
fn test_multiselect_enter_in_code_block_and_list() {
    // This scenario was failing, and has since been fixed.
    App::test((), |mut app| async move {
        let (buffer, selection) = Buffer::mock_from_markdown(
            "* Hey\n```\nThis is code\n```\n",
            None,
            Box::new(|_, _| IndentBehavior::Ignore),
            &mut app,
        );

        buffer.update(&mut app, |buffer, ctx| {
            // Add another empty bullet point.
            buffer.add_cursor(4.into(), true, selection.clone(), ctx);
            buffer.enter(false, TextStyles::default(), selection.clone(), ctx);

            assert_eq!(
                buffer.debug(),
                "<ul0>Hey<ul0><code:Shell>This is code<text>"
            );

            buffer.enter(false, TextStyles::default(), selection.clone(), ctx);

            buffer.add_cursor(6.into(), true, selection.clone(), ctx);

            buffer.enter(false, TextStyles::default(), selection.clone(), ctx);

            assert_eq!(
                buffer.debug(),
                "<ul0>Hey<text>\\n<code:Shell>This is code<text>"
            );
        });
    });
}

#[test]
fn test_multiselect_backspace_on_block_marker() {
    // Make sure that if the 2nd selection is on a block marker that it works correctly.
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            buffer.edit_internal_first_selection(
                CharOffset::from(1)..CharOffset::from(1),
                "test\nline\nsecond",
                Default::default(),
                selection.clone(),
                ctx,
            );
            let _ = buffer.block_style_range(
                CharOffset::from(6)..CharOffset::from(10),
                BufferBlockStyle::UnorderedList {
                    indent_level: ListIndentLevel::One,
                },
                selection.clone(),
                ctx,
            );
            assert_eq!(buffer.content.debug(), "<text>test<ul0>line<text>second");

            // Set a cursor at the start of `third` and before "line"
            selection.update(ctx, |selection, _| {
                selection.set_single_cursor(CharOffset::from(14));
            });
            buffer.add_cursor(6.into(), false, selection.clone(), ctx);
            buffer.backspace(&mut None, selection.clone(), ctx);
            assert_eq!(buffer.content.debug(), "<text>test\\nline\\nseond");
        });
    });
}

#[test]
fn test_multiselect_copy() {
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            buffer.edit_internal(
                "test\nline\nsecond",
                Default::default(),
                selection.clone(),
                ctx,
            );

            selection.update(ctx, |selection, _| {
                selection.set_selection_offsets(vec1![
                    SelectionOffsets {
                        tail: 1.into(),
                        head: 7.into(),
                    },
                    SelectionOffsets {
                        tail: 11.into(),
                        head: 14.into(),
                    },
                ]);
            });

            assert_eq!(
                buffer
                    .selected_text_as_plain_text(selection.clone(), ctx)
                    .as_str(),
                "test\nl\nsec"
            );
            assert_eq!(
                buffer.selected_text_as_html(selection.clone(), ctx),
                Some("<p>test</p><p>l</p><p>sec</p>".to_string())
            );
        })
    });
}

#[test]
fn test_multiselect_copy_blocks() {
    App::test((), |mut app| async move {
        let (buffer, selection) = Buffer::mock_from_markdown(
            "* Hey\n* You\n* Guys\nText\n```\nThis is code\n```\n",
            None,
            Box::new(|_, _| IndentBehavior::Ignore),
            &mut app,
        );
        buffer.update(&mut app, |buffer, ctx| {
            assert_eq!(
                buffer.content.debug(),
                "<ul0>Hey<ul0>You<ul0>Guys<text>Text<code:Shell>This is code<text>"
            );

            selection.update(ctx, |selection, _| {
                selection.set_selection_offsets(vec1![
                SelectionOffsets {
                    tail: 1.into(),
                    head: 7.into(),
                },
                SelectionOffsets {
                    tail: 15.into(),
                    head: 23.into(),
                },
            ]);
            });

            assert_eq!(
                buffer.selected_text_as_plain_text(selection.clone(), ctx).as_str(),
                "Hey\nYo\next\nThis"
            );
            assert_eq!(buffer.selected_text_as_html(selection.clone(), ctx), Some("<ul><li>Hey</li><li>Yo</li></ul><p>ext</p><pre><code class=\"language-warp-runnable-command\">This</code></pre>".to_string()));
        })
    });
}

#[test]
fn test_selected_table_copy_uses_visible_plain_text() {
    App::test((), |mut app| async move {
        let table_source = "Header\t**Bold**";
        let markdown = format!("```{TABLE_BLOCK_MARKDOWN_LANG}\n{table_source}\n```\n");
        let (buffer, selection) = Buffer::mock_from_markdown(
            &markdown,
            None,
            Box::new(|_, _| IndentBehavior::Ignore),
            &mut app,
        );

        buffer.update(&mut app, |buffer, ctx| {
            let block_start = buffer.containing_block_start(CharOffset::from(1));
            let bold_start = CharOffset::from(
                table_source
                    .find("**Bold**")
                    .expect("table source should contain bold cell"),
            );
            let visible_bold_start = block_start + bold_start + CharOffset::from(2);
            let visible_bold_end = visible_bold_start + CharOffset::from("Bold".chars().count());
            selection.update(ctx, |selection, _| {
                set_selections(selection, vec1![visible_bold_start..visible_bold_end]);
            });

            assert_eq!(
                buffer
                    .selected_text_as_plain_text(selection.clone(), ctx)
                    .as_str(),
                "Bold"
            );
        });
    });
}

#[test]
fn test_partial_table_selection_does_not_export_html() {
    App::test((), |mut app| async move {
        let table_source = "Header\t**Bold**";
        let markdown = format!("```{TABLE_BLOCK_MARKDOWN_LANG}\n{table_source}\n```\n");
        let (buffer, selection) = Buffer::mock_from_markdown(
            &markdown,
            None,
            Box::new(|_, _| IndentBehavior::Ignore),
            &mut app,
        );

        buffer.update(&mut app, |buffer, ctx| {
            let block_start = buffer.containing_block_start(CharOffset::from(1));
            let bold_start = CharOffset::from(
                table_source
                    .find("**Bold**")
                    .expect("table source should contain bold cell"),
            );
            let visible_bold_start = block_start + bold_start + CharOffset::from(2);
            let visible_bold_end = visible_bold_start + CharOffset::from("Bold".chars().count());
            selection.update(ctx, |selection, _| {
                set_selections(selection, vec1![visible_bold_start..visible_bold_end]);
            });

            assert_eq!(buffer.selected_text_as_html(selection.clone(), ctx), None);
        });
    });
}

#[test]
fn test_partial_table_selection_still_exports_html_for_non_table_ranges() {
    App::test((), |mut app| async move {
        let table_source = "Header\t**Bold**";
        let markdown =
            format!("```{TABLE_BLOCK_MARKDOWN_LANG}\n{table_source}\n```\n\nafter text\n");
        let (buffer, selection) = Buffer::mock_from_markdown(
            &markdown,
            None,
            Box::new(|_, _| IndentBehavior::Ignore),
            &mut app,
        );

        buffer.update(&mut app, |buffer, ctx| {
            let block_start = buffer.containing_block_start(CharOffset::from(1));
            let bold_start = CharOffset::from(
                table_source
                    .find("**Bold**")
                    .expect("table source should contain bold cell"),
            );
            let visible_bold_start = block_start + bold_start + CharOffset::from(2);
            let visible_bold_end = visible_bold_start + CharOffset::from("Bold".chars().count());

            let max_offset = buffer.max_charoffset();
            let after_text_start = max_offset - CharOffset::from("after text".chars().count());
            let after_text_end = after_text_start + CharOffset::from("after".chars().count());

            selection.update(ctx, |selection, _| {
                set_selections(
                    selection,
                    vec1![
                        visible_bold_start..visible_bold_end,
                        after_text_start..after_text_end,
                    ],
                );
            });

            let html = buffer
                .selected_text_as_html(selection.clone(), ctx)
                .expect("non-table range should still export HTML");
            assert!(
                !html.contains("Bold"),
                "partial table range should be dropped from HTML, got {html}"
            );
            assert!(
                html.contains("after"),
                "clean plain-text range should still appear in HTML, got {html}"
            );
        });
    });
}

#[test]
fn test_clipboard_table_copy_uses_source_offsets_for_later_formatted_cells() {
    App::test((), |mut app| async move {
        let table_source = "Header\t**Bold**\nNext\t*Italic*";
        let markdown = format!("```{TABLE_BLOCK_MARKDOWN_LANG}\n{table_source}\n```\n");
        let (buffer, _selection) = Buffer::mock_from_markdown(
            &markdown,
            None,
            Box::new(|_, _| IndentBehavior::Ignore),
            &mut app,
        );

        buffer.update(&mut app, |buffer, _ctx| {
            let block_start = buffer.containing_block_start(CharOffset::from(1));
            let italic_start = CharOffset::from(
                table_source
                    .find("*Italic*")
                    .expect("table source should contain italic cell"),
            );
            let italic_end = italic_start + CharOffset::from("*Italic*".chars().count());

            assert_eq!(
                buffer.clipboard_table_text_in_range(
                    block_start,
                    (block_start + italic_start)..(block_start + italic_end),
                    LineEnding::LF,
                ),
                "Italic"
            );
        });
    });
}

#[test]
fn test_multiselect_text_styling() {
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            let _ = buffer.edit_internal(
                "hello there you",
                Default::default(),
                selection.clone(),
                ctx,
            );
            assert_eq!(buffer.content.debug(), "<text>hello there you");

            selection.update(ctx, |selection, _| {
                selection.set_selection_offsets(vec1![
                    SelectionOffsets {
                        tail: 2.into(),
                        head: 4.into(),
                    },
                    SelectionOffsets {
                        tail: 8.into(),
                        head: 11.into(),
                    },
                ]);
            });

            // Correct buffer state: h<b>el<b>lo.
            let _ = buffer.style_internal(TextStyles::default().bold(), selection.clone(), ctx);
            assert_eq!(
                buffer.content.debug(),
                "<text>h<b_s>el<b_e>lo t<b_s>her<b_e>e you"
            );

            // Correct buffer state: h<b>e<i>ll<i><b>o.
            selection.update(ctx, |selection, _| {
                set_selections(selection, vec1![3..5, 10..13]);
            });

            let _ = buffer.style_internal(
                TextStyles::default().bold().italic(),
                selection.clone(),
                ctx,
            );
            assert_eq!(
                buffer.content.debug(),
                "<text>h<b_s>e<i_s>ll<b_e><i_e>o t<b_s>he<i_s>re <b_e><i_e>you"
            );

            // Correct buffer state: <b>he<i>ll<i><b>o.
            selection.update(ctx, |selection, _| {
                set_selections(selection, vec1![1..3, 7..9]);
            });
            let _ = buffer.style_internal(TextStyles::default().bold(), selection.clone(), ctx);
            assert_eq!(
                buffer.content.debug(),
                "<text><b_s>he<i_s>ll<b_e><i_e>o <b_s>the<i_s>re <b_e><i_e>you"
            );
        });
    });
}

#[test]
fn test_style_selection_unchanged() {
    // Make sure that after styling a selection the selection range isn't changed.
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            let _ = buffer.edit_internal_first_selection(
                CharOffset::from(1)..CharOffset::from(1),
                "hello",
                Default::default(),
                selection.clone(),
                ctx,
            );

            selection.update(ctx, |selection, _| {
                set_selections(selection, vec1![1..3]);
            });
            let _ = buffer.style_internal(TextStyles::default().bold(), selection.clone(), ctx);

            assert_eq!(
                selection.as_ref(ctx).selections_to_offset_ranges(),
                vec1![1.into()..3.into()]
            );
        });
    });
}

#[test]
fn test_multiselect_range_fully_styled() {
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            let _ = buffer.edit_internal_first_selection(
                CharOffset::from(1)..CharOffset::from(1),
                "hello there!",
                TextStyles::default(),
                selection.clone(),
                ctx,
            );
            assert_eq!(buffer.text().as_str(), "hello there!");

            // Correct buffer state: h<b>ell<b>o.
            selection.update(ctx, |selection, _| {
                set_selections(selection, vec1![2..5, 8..11]);
            });
            let _ = buffer.style_internal(TextStyles::default().bold(), selection.clone(), ctx);
            assert_eq!(
                buffer.content.debug(),
                "<text>h<b_s>ell<b_e>o t<b_s>her<b_e>e!"
            );

            // Correct buffer state: h<b>e<i>ll<i><b>o.
            selection.update(ctx, |selection, _| {
                set_selections(selection, vec1![3..5, 9..11]);
            });
            let _ = buffer.style_internal(TextStyles::default().italic(), selection.clone(), ctx);
            assert_eq!(
                buffer.content.debug(),
                "<text>h<b_s>e<i_s>ll<b_e><i_e>o t<b_s>h<i_s>er<b_e><i_e>e!"
            );

            assert!(!buffer.ranges_fully_styled(
                vec1![1.into()..3.into(), 8.into()..10.into()],
                TextStyles::default().bold()
            ));
            assert!(buffer.ranges_fully_styled(
                vec1![2.into()..4.into(), 9.into()..11.into()],
                TextStyles::default().bold()
            ));
            assert!(buffer.ranges_fully_styled(
                vec1![3.into()..5.into(), 9.into()..11.into()],
                TextStyles::default().bold().italic()
            ));
            assert!(!buffer.ranges_fully_styled(
                vec1![4.into()..6.into(), 11.into()..13.into()],
                TextStyles::default().italic()
            ));

            // First true, 2nd false.
            assert!(!buffer.ranges_fully_styled(
                vec1![3.into()..4.into(), 8.into()..11.into()],
                TextStyles::default().bold().italic()
            ));

            // First false, 2nd true.
            assert!(!buffer.ranges_fully_styled(
                vec1![2.into()..4.into(), 9.into()..11.into()],
                TextStyles::default().bold().italic()
            ));
        });
    });
}

#[test]
fn test_multiselect_text_unstyling() {
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            let _ = buffer.edit_internal_first_selection(
                CharOffset::from(1)..CharOffset::from(1),
                "hello there!",
                Default::default(),
                selection.clone(),
                ctx,
            );
            assert_eq!(buffer.content.debug(), "<text>hello there!");

            // Correct buffer state: h<b>ell<b>o t<b>her<b>e.
            selection.update(ctx, |selection, _| {
                set_selections(selection, vec1![2..5, 8..11]);
            });
            let _ = buffer.style_internal(TextStyles::default().bold(), selection.clone(), ctx);
            assert_eq!(
                buffer.content.debug(),
                "<text>h<b_s>ell<b_e>o t<b_s>her<b_e>e!"
            );

            // Correct buffer state: h<b>e<i>ll<i><b>o.
            selection.update(ctx, |selection, _| {
                set_selections(selection, vec1![3..5, 9..11]);
            });
            let _ = buffer.style_internal(TextStyles::default().italic(), selection.clone(), ctx);
            assert_eq!(
                buffer.content.debug(),
                "<text>h<b_s>e<i_s>ll<b_e><i_e>o t<b_s>h<i_s>er<b_e><i_e>e!"
            );

            // Correct buffer state: h<b>e<b>l<b><i>l<i><b>o.
            selection.update(ctx, |selection, _| {
                set_selections(selection, vec1![3..4, 9..10]);
            });
            let _ = buffer.unstyle_internal(
                TextStyles::default().bold().italic(),
                selection.clone(),
                ctx,
            );
            assert_eq!(
                buffer.content.debug(),
                "<text>h<b_s>e<b_e>l<b_s><i_s>l<b_e><i_e>o t<b_s>h<b_e>e<b_s><i_s>r<b_e><i_e>e!"
            );

            // Correct buffer state: h<b>e<b>l<i>l<i>o.
            selection.update(ctx, |selection, _| {
                set_selections(selection, vec1![4..5, 10..11]);
            });
            let _ = buffer.unstyle_internal(TextStyles::default().bold(), selection.clone(), ctx);
            assert_eq!(
                buffer.content.debug(),
                "<text>h<b_s>e<b_e>l<i_s>l<i_e>o t<b_s>h<b_e>e<i_s>r<i_e>e!"
            );

            selection.update(ctx, |selection, _| {
                set_selections(selection, vec1![2..4, 8..10]);
            });
            let _ = buffer.style_internal(
                TextStyles::default().strikethrough(),
                selection.clone(),
                ctx,
            );
            assert_eq!(
                buffer.content.debug(),
                "<text>h<b_s><s_s>e<b_e>l<i_s><s_e>l<i_e>o t<b_s><s_s>h<b_e>e<i_s><s_e>r<i_e>e!"
            );

            selection.update(ctx, |selection, _| {
                set_selections(selection, vec1![2..3, 8..9]);
            });
            let _ = buffer.unstyle_internal(
                TextStyles::default().strikethrough(),
                selection.clone(),
                ctx,
            );
            assert_eq!(
                buffer.content.debug(),
                "<text>h<b_s>e<b_e><s_s>l<i_s><s_e>l<i_e>o t<b_s>h<b_e><s_s>e<i_s><s_e>r<i_e>e!"
            );
        });
    });
}

#[test]
fn test_multiselect_indent_unindent() {
    App::test((), |mut app| async move {
        let (buffer, selection) = Buffer::mock_from_markdown(
            "hey\nthere\n- hey\n- there\nhey\nthere",
            None,
            Box::new(|block_style, shift| match block_style {
                BufferBlockStyle::UnorderedList { indent_level } if !shift => {
                    IndentBehavior::Restyle(BufferBlockStyle::UnorderedList {
                        indent_level: indent_level.shift_right(),
                    })
                }
                BufferBlockStyle::UnorderedList { indent_level } => {
                    IndentBehavior::Restyle(BufferBlockStyle::UnorderedList {
                        indent_level: indent_level.shift_left(),
                    })
                }
                BufferBlockStyle::CodeBlock { .. } => {
                    IndentBehavior::TabIndent(IndentUnit::Space(4))
                }
                _ => IndentBehavior::Ignore,
            }),
            &mut app,
        );

        buffer.update(&mut app, |buffer, ctx| {
            buffer.block_style_range(
                CharOffset::from(21)..CharOffset::from(30),
                BufferBlockStyle::CodeBlock {
                    code_block_type: CodeBlockType::Code {
                        lang: "Rust".to_string(),
                    },
                },
                selection.clone(),
                ctx,
            );
            assert_eq!(
                buffer.content.debug(),
                "<text>hey\\nthere<ul0>hey<ul0>there<code:Rust>hey\\nthere<text>"
            );

            // Tab before "hey" and before "- hey" and before "hey" in code block.
            selection.update(ctx, |selection, _| {
                set_selections(selection, vec1![1..1, 14..14, 21..21]);
            });
            let _ = buffer.indent(1, selection.clone(), ctx);
            assert_eq!(
                buffer.content.debug(),
                "<text>hey\\nthere<ul1>hey<ul0>there<code:Rust>    hey\\nthere<text>"
            );

            let _ = buffer.unindent(selection.clone(), ctx);
            assert_eq!(
                buffer.content.debug(),
                "<text>hey\\nthere<ul0>hey<ul0>there<code:Rust>hey\\nthere<text>"
            );
        });
    });
}

#[test]
fn test_selection_collapsing() {
    // To test
    // - Extend selection into each other.
    // - Backspace merges selections.
    // - Make sure it works with multiple selections overlapping at once.
    // - Make sure we call it again inside of selections.rs, like when extending selection by line.
    App::test((), |mut app| async move {
        let (buffer, selection) = Buffer::mock_from_markdown(
            "hey\nthere\nyou",
            None,
            Box::new(|_, _| IndentBehavior::Ignore),
            &mut app,
        );

        selection.update(&mut app, |selection, _| {
            selection.set_selection_offsets(vec1![
                SelectionOffsets {
                    tail: 4.into(),
                    head: 1.into(),
                },
                SelectionOffsets {
                    tail: 7.into(),
                    head: 5.into(),
                },
            ]);
        });

        buffer.update(&mut app, |buffer, ctx| {
            // Moving left once would make the selections touch.  They should not merge yet.
            assert_eq!(selection.as_ref(ctx).selection_offsets().len(), 2);
            buffer.update_selection(
                selection.clone(),
                BufferSelectAction::ExtendLeft,
                AutoScrollBehavior::Selection,
                ctx,
            );
            assert_eq!(selection.as_ref(ctx).selection_offsets().len(), 2);

            // Moving left another time makes them overlap.  They should merge.
            buffer.update_selection(
                selection.clone(),
                BufferSelectAction::ExtendLeft,
                AutoScrollBehavior::Selection,
                ctx,
            );
            assert_eq!(
                selection.as_ref(ctx).selections_to_offset_ranges(),
                vec1![1.into()..7.into()]
            );

            selection.update(ctx, |selection, _| {
                selection.set_single_cursor(4.into());
            });
            buffer.add_cursor(5.into(), false, selection.clone(), ctx);

            // Backspace will place the cursors at the same position, but they should not be merged
            // until the next backspace.
            assert_eq!(selection.as_ref(ctx).selection_offsets().len(), 2);
            buffer.update_content(
                BufferEditAction::Backspace,
                EditOrigin::UserTyped,
                selection.clone(),
                ctx,
            );
            assert_eq!(selection.as_ref(ctx).selection_offsets().len(), 2);

            // Backspace again and they should merge.
            buffer.update_content(
                BufferEditAction::Backspace,
                EditOrigin::UserTyped,
                selection.clone(),
                ctx,
            );
            assert_eq!(
                selection.as_ref(ctx).selection_offsets(),
                vec1![SelectionOffsets {
                    tail: 2.into(),
                    head: 2.into(),
                }]
            );

            // Place two selections near each other.
            selection.update(ctx, |selection, _| {
                selection.set_selection_offsets(vec1![
                    SelectionOffsets {
                        tail: 1.into(),
                        head: 4.into(),
                    },
                    SelectionOffsets {
                        tail: 5.into(),
                        head: 7.into(),
                    },
                ]);
            });

            // If we extend to the right, the head of the new selection should be to the right.
            buffer.update_selection(
                selection.clone(),
                BufferSelectAction::ExtendRight,
                AutoScrollBehavior::Selection,
                ctx,
            );
            buffer.update_selection(
                selection.clone(),
                BufferSelectAction::ExtendRight,
                AutoScrollBehavior::Selection,
                ctx,
            );
            assert_eq!(
                selection.as_ref(ctx).selection_offsets(),
                vec1![SelectionOffsets {
                    tail: 1.into(),
                    head: 9.into(),
                }]
            );

            // Opposite of the previous test.
            // Place two selections near each other.
            selection.update(ctx, |selection, _| {
                selection.set_selection_offsets(vec1![
                    SelectionOffsets {
                        tail: 4.into(),
                        head: 1.into(),
                    },
                    SelectionOffsets {
                        tail: 7.into(),
                        head: 5.into(),
                    },
                ]);
            });

            // If we extend to the right, the head of the new selection should be to the right.
            buffer.update_selection(
                selection.clone(),
                BufferSelectAction::ExtendLeft,
                AutoScrollBehavior::Selection,
                ctx,
            );
            buffer.update_selection(
                selection.clone(),
                BufferSelectAction::ExtendLeft,
                AutoScrollBehavior::Selection,
                ctx,
            );
            assert_eq!(
                selection.as_ref(ctx).selection_offsets(),
                vec1![SelectionOffsets {
                    tail: 7.into(),
                    head: 1.into(),
                }]
            );
        });
    });
}

#[test]
fn test_update_selection_same_range() {
    App::test((), |mut app| async move {
        let (buffer, selection) = Buffer::mock_from_markdown(
            "hello world",
            None,
            Box::new(|_, _| IndentBehavior::Ignore),
            &mut app,
        );

        selection.update(&mut app, |selection, _| {
            selection.set_selection_offsets(vec1![SelectionOffsets {
                tail: 1.into(),
                head: 5.into()
            }]);
        });

        let selection_changes = Arc::new(AtomicU8::new(0));
        app.update(|ctx| {
            let selection_changes = selection_changes.clone();
            ctx.subscribe_to_model(&buffer, move |_, event, _| {
                if let BufferEvent::SelectionChanged { .. } = event {
                    selection_changes.fetch_add(1, Ordering::Relaxed);
                }
            });
        });

        // Modify the selection.
        buffer.update(&mut app, |buffer, ctx| {
            buffer.update_selection(
                selection.clone(),
                BufferSelectAction::ExtendRight,
                AutoScrollBehavior::Selection,
                ctx,
            );
        });
        assert_eq!(selection_changes.load(Ordering::Relaxed), 1);

        // Make a no-op selection change.
        buffer.update(&mut app, |buffer, ctx| {
            buffer.update_selection(
                selection.clone(),
                BufferSelectAction::AddSelection {
                    head: 6.into(),
                    tail: 1.into(),
                    clear_selections: true,
                },
                AutoScrollBehavior::Selection,
                ctx,
            );
        });
        // No event should be emitted.
        assert_eq!(selection_changes.load(Ordering::Relaxed), 1);

        // Make another selection change.
        buffer.update(&mut app, |buffer, ctx| {
            buffer.update_selection(
                selection.clone(),
                BufferSelectAction::SelectAll,
                AutoScrollBehavior::Selection,
                ctx,
            );
        });
        assert_eq!(selection_changes.load(Ordering::Relaxed), 2);
    });
}

#[test]
#[allow(clippy::single_range_in_vec_init)]
#[allow(clippy::reversed_empty_ranges)]
fn test_overlapping_ranges() {
    assert_eq!(
        Buffer::overlapping_ranges(vec![1..5, 3..7, 8..10]),
        (vec![0, 1], vec![1..7])
    );
    assert_eq!(
        Buffer::overlapping_ranges(vec![1..5, 8..10, 3..7]),
        (vec![0, 1], vec![1..7])
    );
    assert_eq!(
        Buffer::overlapping_ranges(vec![1..5, 5..7]),
        (vec![0, 1], vec![1..7])
    );
    assert_eq!(
        Buffer::overlapping_ranges(vec![1..5, 5..7, 4..10, 11..15, 14..17, 20..27]),
        (vec![0, 1, 2, 3, 4], vec![1..10, 11..17])
    );
}

#[test]
fn test_multiselect_remove_prefix_and_style() {
    App::test((), |mut app| async move {
        let (buffer, selection) = Buffer::mock_from_markdown(
            "hey\nthere\nyou",
            None,
            Box::new(|_, _| IndentBehavior::Ignore),
            &mut app,
        );

        buffer.update(&mut app, |buffer, ctx| {
            // Make the first two lines into a list.
            assert_eq!(buffer.content.debug(), "<text>hey\\nthere\\nyou");

            selection.update(ctx, |selection, _| {
                set_selections(selection, vec1![1..1, 5..5]);
            });
            buffer.edit_internal("-", TextStyles::default(), selection.clone(), ctx);
            buffer.edit_internal(" ", TextStyles::default(), selection.clone(), ctx);

            assert_eq!(buffer.content.debug(), "<text>- hey\\n- there\\nyou");

            selection.update(ctx, |selection, _| {
                set_selections(selection, vec1![3..3, 9..9]);
            });
            let prev_selection = buffer.to_rendered_selection_set(selection.clone(), ctx);
            let edit_result = buffer.remove_prefix_and_style_blocks(
                BlockType::Text(BufferBlockStyle::UnorderedList {
                    indent_level: ListIndentLevel::One,
                }),
                selection.clone(),
                ctx,
            );
            let curr_selection = buffer.to_rendered_selection_set(selection.clone(), ctx);
            buffer.push_undo_item(
                prev_selection,
                curr_selection,
                edit_result.undo_item.expect("Should have undo item"),
                UndoActionType::Atomic,
            );

            assert_eq!(buffer.content.debug(), "<ul0>hey<ul0>there<text>you");
            assert_eq!(
                selection.as_ref(ctx).selections_to_offset_ranges(),
                vec1![1.into()..1.into(), 5.into()..5.into()]
            );
            // Should undo to the state before prefix was removed.
            let _ = buffer.undo(selection.clone(), ctx);
            assert_eq!(buffer.content.debug(), "<text>- hey\\n- there\\nyou");
        });
    });
}

#[test]
fn test_insert_for_each_selection() {
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            let _ = buffer.edit_internal_first_selection(
                CharOffset::from(1)..CharOffset::from(1),
                "start\nline\nend",
                Default::default(),
                selection.clone(),
                ctx,
            );

            selection.update(ctx, |selection, _| {
                set_selections(selection, vec1![6..6, 11..11, 15..15]);
            });
            let _ = buffer.edit_for_each_selection(
                &vec1![
                    ("ab".to_string(), 2),
                    ("def".to_string(), 2),
                    ("next".to_string(), 1)
                ],
                selection.clone(),
                ctx,
            );

            assert_eq!(buffer.content.debug(), "<text>startab\\nlinedef\\nendnext");
            // The cursor position should be (1) after "ab", (2) between "de" and "f", (3) between "n" and "ext".
            assert_eq!(
                selection.as_ref(ctx).selections_to_offset_ranges(),
                vec1![
                    CharOffset::from(8)..CharOffset::from(8),
                    CharOffset::from(15)..CharOffset::from(15),
                    CharOffset::from(21)..CharOffset::from(21)
                ]
            );
        });
    });
}

#[test]
fn test_dimension_conversions() {
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            let _ = buffer.edit_internal_first_selection(
                CharOffset::from(1)..CharOffset::from(1),
                "  start\n    line\n  end",
                Default::default(),
                selection.clone(),
                ctx,
            );

            assert_eq!(
                Point::new(1, 3).to_buffer_char_offset(buffer),
                CharOffset::from(4)
            );

            assert_eq!(
                CharOffset::from(4).to_buffer_point(buffer),
                Point::new(1, 3)
            );

            assert_eq!(
                Point::new(1, 7).to_buffer_char_offset(buffer),
                CharOffset::from(8)
            );

            assert_eq!(
                CharOffset::from(8).to_buffer_point(buffer),
                Point::new(1, 7)
            );

            assert_eq!(
                Point::new(2, 0).to_buffer_char_offset(buffer),
                CharOffset::from(9)
            );

            assert_eq!(
                CharOffset::from(9).to_buffer_point(buffer),
                Point::new(2, 0)
            );
        });

        let buffer2 = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection2 = app.add_model(|_| BufferSelectionModel::new(buffer2.clone()));

        buffer2.update(&mut app, |buffer, ctx| {
            let _ = buffer.edit_internal_first_selection(
                CharOffset::from(1)..CharOffset::from(1),
                "国际符号",
                Default::default(),
                selection2.clone(),
                ctx,
            );

            // Each CJK character should take 3 bytes + linebreak at start of buffer takes 1 byte.
            assert_eq!(
                CharOffset::from(2).to_buffer_byte_offset(buffer),
                ByteOffset::from(4)
            );

            assert_eq!(
                ByteOffset::from(4).to_buffer_char_offset(buffer),
                CharOffset::from(2)
            );

            assert_eq!(
                CharOffset::from(4).to_buffer_byte_offset(buffer),
                ByteOffset::from(10)
            );

            assert_eq!(
                ByteOffset::from(10).to_buffer_char_offset(buffer),
                CharOffset::from(4)
            );
        });
    });
}

#[test]
fn test_decorate_lines_with_prefix() {
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| {
            Buffer::new(Box::new(|_, _| {
                IndentBehavior::TabIndent(IndentUnit::Space(4))
            }))
        });
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            let _ = buffer.edit_internal_first_selection(
                CharOffset::from(1)..CharOffset::from(1),
                "  start\n    line\n  end",
                Default::default(),
                selection.clone(),
                ctx,
            );

            selection.update(ctx, |selection, _| {
                set_selections(selection, vec1![5..21]);
            });
            let prev_selection = buffer.to_rendered_selection_set(selection.clone(), ctx);
            let edit_result =
                buffer.decorate_lines_with_prefix(vec1![1, 2, 3], "// ", selection.clone(), ctx);
            let curr_selection = buffer.to_rendered_selection_set(selection.clone(), ctx);
            buffer.push_undo_item(
                prev_selection,
                curr_selection,
                edit_result.undo_item.expect("Should have undo item"),
                UndoActionType::Atomic,
            );

            assert_eq!(
                buffer.content.debug(),
                "<text>  // start\\n  //   line\\n  // end"
            );

            // Should not affect the selections.
            assert_eq!(
                selection.as_ref(ctx).selections_to_offset_ranges(),
                vec1![CharOffset::from(8)..CharOffset::from(30)]
            );

            let _ = buffer.undo(selection.clone(), ctx);
            assert_eq!(buffer.content.debug(), "<text>  start\\n    line\\n  end");
            assert_eq!(
                selection.as_ref(ctx).selections_to_offset_ranges(),
                vec1![CharOffset::from(5)..CharOffset::from(21)]
            );
        });
    });
}

#[test]
fn test_remove_prefix_from_lines() {
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| {
            Buffer::new(Box::new(|_, _| {
                IndentBehavior::TabIndent(IndentUnit::Space(4))
            }))
        });
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            let _ = buffer.edit_internal_first_selection(
                CharOffset::from(1)..CharOffset::from(1),
                "  // start\n    //line\n  //     end",
                Default::default(),
                selection.clone(),
                ctx,
            );

            selection.update(ctx, |selection, _| {
                set_selections(selection, vec1![5..28]);
            });
            let prev_selection = buffer.to_rendered_selection_set(selection.clone(), ctx);
            let edit_result =
                buffer.remove_prefix_from_lines(vec1![1, 2, 3], "//", selection.clone(), ctx);
            let curr_selection = buffer.to_rendered_selection_set(selection.clone(), ctx);
            buffer.push_undo_item(
                prev_selection,
                curr_selection,
                edit_result.undo_item.expect("Should have undo item"),
                UndoActionType::Atomic,
            );

            assert_eq!(
                buffer.content.debug(),
                "<text>  start\\n    line\\n      end",
            );

            // Should not affect the selections.
            assert_eq!(
                selection.as_ref(ctx).selections_to_offset_ranges(),
                vec1![CharOffset::from(3)..CharOffset::from(20)]
            );

            let _ = buffer.undo(selection.clone(), ctx);
            assert_eq!(
                buffer.content.debug(),
                "<text>  // start\\n    //line\\n  //     end"
            );
            assert_eq!(
                selection.as_ref(ctx).selections_to_offset_ranges(),
                vec1![CharOffset::from(5)..CharOffset::from(28)]
            );
        });
    });
}

#[test]
fn test_selected_lines() {
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            let _ = buffer.edit_internal_first_selection(
                CharOffset::from(1)..CharOffset::from(1),
                "abc\nd\nend\n\nnew",
                Default::default(),
                selection.clone(),
                ctx,
            );
        });

        selection.update(&mut app, |selection, ctx| {
            set_selections(selection, vec1![1..2, 5..5, 10..12]);
            assert_eq!(selection.selected_lines(ctx), vec1![1, 2, 3, 4]);

            // There should be no duplicates.
            set_selections(selection, vec1![1..2, 3..4]);
            assert_eq!(selection.selected_lines(ctx), vec1![1]);
        });
    });
}

#[test]
fn test_line_prefix() {
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| {
            Buffer::new(Box::new(|_, _| {
                IndentBehavior::TabIndent(IndentUnit::Space(4))
            }))
        });
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            let _ = buffer.edit_internal_first_selection(
                CharOffset::from(1)..CharOffset::from(1),
                "    // abc\ndef\n//line",
                Default::default(),
                selection.clone(),
                ctx,
            );

            assert!(buffer.line_decorated_with_prefix(1, "//"));
            assert!(!buffer.line_decorated_with_prefix(2, "//"));
            assert!(buffer.line_decorated_with_prefix(3, "//"));
        });
    });
}

#[test]
fn test_indent_unit() {
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| {
            Buffer::new(Box::new(|_, _| {
                IndentBehavior::TabIndent(IndentUnit::Space(4))
            }))
        });
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            let _ = buffer.edit_internal_first_selection(
                CharOffset::from(1)..CharOffset::from(1),
                "    line\n        test",
                Default::default(),
                selection.clone(),
                ctx,
            );

            assert_eq!(
                buffer.indented_units_at_offset(CharOffset::from(5)),
                Some(1)
            );

            assert_eq!(
                buffer.indented_units_at_offset(CharOffset::from(1)),
                Some(0)
            );
        });
    });
}

#[test]
fn test_indent_with_multiple_units() {
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| {
            Buffer::new(Box::new(|_, _| {
                IndentBehavior::TabIndent(IndentUnit::Space(4))
            }))
        });
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            let _ = buffer.edit_internal_first_selection(
                CharOffset::from(1)..CharOffset::from(1),
                "text\n line\n     test",
                Default::default(),
                selection.clone(),
                ctx,
            );

            selection.update(ctx, |selection, _| {
                set_selections(selection, vec1![1..1, 7..7, 17..17]);
            });
            let _ = buffer.indent(2, selection.clone(), ctx);
            assert_eq!(
                buffer.content.debug(),
                "<text>        text\\n        line\\n            test"
            );
        });
    });
}

#[test]
fn test_multiselect_insert_html() {
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            buffer.edit_internal_first_selection(
                CharOffset::from(1)..CharOffset::from(1),
                "First\nSecond\nAfter",
                Default::default(),
                selection.clone(),
                ctx,
            );

            let formatted_text =
                parse_html("<p><strong>test text</strong></p>").expect("Should parse");
            selection.update(ctx, |selection, _| {
                set_selections(selection, vec1![1..1, 7..7]);
            });
            buffer.insert_formatted_text_at_selections(
                formatted_text,
                EditOrigin::UserInitiated,
                selection.clone(),
                ctx,
            );
            assert_eq!(
                buffer.content.debug(),
                "<text><b_s>test text<b_e>First\\n<b_s>test text<b_e>Second\\nAfter"
            );
            assert_eq!(
                selection.as_ref(ctx).selections_to_offset_ranges(),
                vec1![10.into()..10.into(), 25.into()..25.into()]
            );
        });
    });
}

#[test]
fn test_get_multibyte_selected_text() {
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            buffer.edit_internal_first_selection(
                CharOffset::from(1)..CharOffset::from(1),
                "你好, my name is 文档",
                Default::default(),
                selection.clone(),
                ctx,
            );

            selection.update(ctx, |selection, _| {
                set_selections(selection, vec1![1..3]);
            });
            assert_eq!(
                buffer
                    .selected_text_as_plain_text(selection.clone(), ctx)
                    .as_str(),
                "你好"
            );

            selection.update(ctx, |selection, _| {
                set_selections(selection, vec1![2..7]);
            });
            assert_eq!(
                buffer
                    .selected_text_as_plain_text(selection.clone(), ctx)
                    .as_str(),
                "好, my"
            );

            selection.update(ctx, |selection, _| {
                set_selections(selection, vec1![15..18]);
            });
            assert_eq!(
                buffer
                    .selected_text_as_plain_text(selection.clone(), ctx)
                    .as_str(),
                " 文档"
            );
        });
    });
}

#[test]
fn test_buffer_version_changes() {
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_ctx| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            let version = buffer.content_version;
            buffer.update_content(
                BufferEditAction::Insert {
                    text: "hey there",
                    style: TextStyles::default(),
                    override_text_style: None,
                },
                EditOrigin::UserTyped,
                selection.clone(),
                ctx,
            );

            assert!(!buffer.version_match(&version));
        });
    });
}

#[test]
fn test_version_match() {
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_ctx| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            let version = buffer.content_version;
            assert!(buffer.version_match(&version));

            buffer.update_content(
                BufferEditAction::Insert {
                    text: "hey there",
                    style: TextStyles::default(),
                    override_text_style: None,
                },
                EditOrigin::UserTyped,
                selection.clone(),
                ctx,
            );

            assert!(!buffer.version_match(&version));
        });
    });
}

#[test]
fn test_undo_redo_versions() {
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_ctx| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            let version1 = buffer.content_version;

            assert!(buffer.version_match(&version1));

            buffer.update_content(
                BufferEditAction::Insert {
                    text: "hey there",
                    style: TextStyles::default(),
                    override_text_style: None,
                },
                EditOrigin::UserTyped,
                selection.clone(),
                ctx,
            );

            let version2 = buffer.content_version;

            assert!(!buffer.version_match(&version1));
            assert!(buffer.version_match(&version2));

            buffer.update_content(
                BufferEditAction::Undo,
                EditOrigin::UserTyped,
                selection.clone(),
                ctx,
            );

            assert!(buffer.version_match(&version1));
            assert!(!buffer.version_match(&version2));
            assert_ne!(buffer.content_version, version1);

            buffer.update_content(
                BufferEditAction::Redo,
                EditOrigin::UserTyped,
                selection.clone(),
                ctx,
            );

            assert!(!buffer.version_match(&version1));
            assert!(buffer.version_match(&version2));
        });
    });
}

/// Regression test: after a remote file save, the server's file-watcher sends a
/// `BufferUpdatedPush` with the same content the client just saved.
/// `insert_at_char_offset_ranges` is a no-op (content unchanged) and returns early
/// without calling `set_version(new_version)`. But the caller updates
/// `base_content_version` to `new_version` anyway, creating a mismatch that makes
/// `has_unsaved_changes` return true — causing a spurious "Save changes?" dialog.
#[test]
fn test_insert_at_char_offset_ranges_noop_skips_set_version() {
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            // Step 1: Simulate initial file load — populate buffer and set version.
            buffer.replace_all("hello world", ctx);
            let load_version = ContentVersion::new();
            buffer.set_version(load_version);
            assert!(buffer.version_match(&load_version));

            // Step 2: User edits the buffer.
            buffer.update_content(
                BufferEditAction::Insert {
                    text: "!",
                    style: TextStyles::default(),
                    override_text_style: None,
                },
                EditOrigin::UserTyped,
                selection.clone(),
                ctx,
            );
            assert!(!buffer.version_match(&load_version));

            // Step 3: Simulate save — capture the current buffer version as the base.
            let save_version = buffer.version();
            assert!(buffer.version_match(&save_version));

            // Step 4: Simulate post-save server push with NO edits.
            // The server file-watcher detected the save, computed a diff against
            // the buffer content, and found zero changes — so the push carries
            // an empty edit list.
            let push_version = ContentVersion::new();
            buffer.insert_at_char_offset_ranges(vec![], push_version, ctx);

            // Step 5: The caller (GlobalBufferModel) would set base_content_version = push_version.
            // version_match(push_version) should be true — the content hasn't changed.
            // BUG: insert_at_char_offset_ranges returned early without calling set_version,
            // so the buffer version is still save_version, not push_version.
            assert!(
                buffer.version_match(&push_version),
                "version_match should return true after a no-op insert_at_char_offset_ranges, \
                 but the buffer version was not updated because the no-op early return \
                 skipped set_version. Buffer version is {:?}, expected to match {:?}",
                buffer.version(),
                push_version,
            );
        });
    });
}

#[test]
fn test_insert_at_offsets() {
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            buffer.edit_internal_first_selection(
                CharOffset::from(1)..CharOffset::from(1),
                "Start\nLine\nSecond",
                Default::default(),
                selection.clone(),
                ctx,
            );

            let edits = Vec1::try_from_vec(vec![
                ("abc".to_string(), CharOffset::from(2)..CharOffset::from(5)),
                ("def".to_string(), CharOffset::from(7)..CharOffset::from(11)),
            ])
            .unwrap();
            buffer.insert_at_offsets(&edits, selection.clone(), ctx);
            assert_eq!(buffer.content.debug(), "<text>Sabct\\ndef\\nSecond");

            // Edits overlapping each other should be applied in sequence.
            let edits = Vec1::try_from_vec(vec![
                ("def".to_string(), CharOffset::from(2)..CharOffset::from(5)),
                ("abc".to_string(), CharOffset::from(1)..CharOffset::from(3)),
            ])
            .unwrap();
            buffer.insert_at_offsets(&edits, selection.clone(), ctx);
            assert_eq!(buffer.content.debug(), "<text>abceft\\ndef\\nSecond");

            // Edit should not affect the selection offsets.
            selection.update(ctx, |selection, _| {
                set_selections(selection, vec1![2..5]);
            });
            let edits = Vec1::try_from_vec(vec![(
                "def".to_string(),
                CharOffset::from(1)..CharOffset::from(4),
            )])
            .unwrap();
            buffer.insert_at_offsets(&edits, selection.clone(), ctx);
            assert_eq!(buffer.content.debug(), "<text>defeft\\ndef\\nSecond");
            assert_eq!(
                selection.as_ref(ctx).selections_to_offset_ranges(),
                vec1![CharOffset::from(2)..CharOffset::from(5)]
            );
        });
    });
}

/// Regression test for WARP-CLIENT-DEV-NYY: panic "Invalid edit range 4042..3982".
///
/// Root cause: `fuzzy_match_v4a_diffs` produces `DiffDelta`s with overlapping
/// `replacement_line_range` values when multiple V4A hunks target the same
/// region of a file (confirmed by the companion test
/// `test_v4a_maa_crash_d71bf84b_no_overlapping_deltas` in the `ai` crate).
///
/// `CodeEditorModel::apply_diffs` converts those line ranges to char offsets
/// and passes them to `insert_at_offsets`, which feeds them into
/// `apply_core_edit_actions` without validating.  The invalid range reaches
/// `Buffer::edit`, which panics on the `debug_assert!`.
///
/// This test passes the exact Sentry crash values (`4042..3982`) to
/// `insert_at_offsets` to confirm the editor does not defend against bad
/// input from the diff layer.
#[test]
fn test_insert_at_offsets_overlapping_ranges_skipped() {
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            // Populate the buffer with enough content.
            let content = (0..200)
                .map(|i| format!("line_{:03}_content_padding", i))
                .collect::<Vec<_>>()
                .join("\n");
            buffer.edit_internal_first_selection(
                CharOffset::from(1)..CharOffset::from(1),
                &content,
                Default::default(),
                selection.clone(),
                ctx,
            );

            let original_text = buffer.text().into_string();

            // Pass a range with start > end (the exact Sentry crash values).
            // After the fix in apply_core_edit_actions, the inverted range
            // should be skipped gracefully instead of panicking.
            let edits = Vec1::try_from_vec(vec![(
                "replacement\n".to_string(),
                CharOffset::from(4042)..CharOffset::from(3982),
            )])
            .unwrap();

            buffer.insert_at_offsets(&edits, selection.clone(), ctx);

            // Buffer should be unchanged — the invalid edit was skipped.
            assert_eq!(buffer.text().into_string(), original_text);
        });
    });
}

#[test]
fn test_from_plain_text() {
    App::test((), |mut app| async move {
        let (buffer, selection) = Buffer::mock_from_markdown(
            "Hello\nWorld",
            None,
            Box::new(|_, _| IndentBehavior::Ignore),
            &mut app,
        );

        buffer.update(&mut app, |buffer, _ctx| {
            // Ensure that there is a text marker at the start of the buffer.
            assert_eq!(buffer.content.debug(), "<text>Hello\\nWorld");
        });

        selection.read(&app, |selection, _| {
            // Ensure that the cursor is at the end of the buffer.
            assert_eq!(
                selection.selections_to_offset_ranges(),
                vec1![CharOffset::from(12)..CharOffset::from(12)]
            );
        });
    });
}

#[test]
fn test_unindent_panic_simpler_case() {
    // This verifies unindent does not invalidate selection anchors.
    App::test((), |mut app| async move {
        let buffer = app.add_model(|_| {
            Buffer::new(Box::new(|block_style, _shift| match block_style {
                BufferBlockStyle::PlainText => IndentBehavior::TabIndent(IndentUnit::Space(4)),
                _ => IndentBehavior::Ignore,
            }))
        });
        let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

        buffer.update(&mut app, |buffer, ctx| {
            // Insert simple indented text
            buffer.edit_internal_first_selection(
                CharOffset::from(1)..CharOffset::from(1),
                "    let x = 42;",
                TextStyles::default(),
                selection.clone(),
                ctx,
            );

            // Position cursor in middle of the indentation
            selection.update(ctx, |selection, _| {
                selection.set_single_cursor(CharOffset::from(3));
            });

            // Should not panic; anchors within the removed indentation are clamped
            buffer.unindent(selection.clone(), ctx);

            // Indentation should be removed
            assert_eq!(buffer.debug(), "<text>let x = 42;");

            // Ensure rendering selections does not panic
            let _ = buffer.to_rendered_selection_set(selection.clone(), ctx);
        });
    });
}
