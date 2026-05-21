use super::{OffsetMap, SelectableTextRun};
use crate::render::model::FrameOffset;
use string_offset::CharOffset;

#[test]
fn test_offset_map_basic() {
    // Baseline test for a no-placeholder OffsetMap. The content_start is non-zero to mimic
    // paragraphs within a code block.
    let map = OffsetMap::new(vec![SelectableTextRun {
        content_start: CharOffset::from(12),
        frame_start: FrameOffset::zero(),
        length: 10,
    }]);

    // The returned offset should be adjusted by the content start.
    assert_eq!(map.to_content(FrameOffset::from(4)), 16.into());
    // Mapping should clamp to run bounds.
    assert_eq!(map.to_content(FrameOffset::from(12)), 22.into());
}

#[test]
fn test_offset_map_placeholders() {
    // Set up an offset map for the following structure:
    //           |placeholder|text|placeholder|text|placeholder...
    // Frame:    0           6    14          24   28
    // Content:  0           1     9          10   14
    let map = OffsetMap::new(vec![
        SelectableTextRun {
            // Even in the zero-state placeholder case, there's an empty content run just before it.
            content_start: CharOffset::zero(),
            frame_start: FrameOffset::zero(),
            length: 0,
        },
        SelectableTextRun {
            content_start: CharOffset::from(1),
            frame_start: FrameOffset::from(6),
            length: 8,
        },
        SelectableTextRun {
            content_start: CharOffset::from(10),
            frame_start: FrameOffset::from(24),
            length: 4,
        },
    ]);

    // Depending on what they're closer to, characters at the start of the frame map to either
    // the start of the line or the first content run.
    assert_eq!(map.to_content(FrameOffset::from(2)), CharOffset::zero());
    assert_eq!(map.to_content(FrameOffset::from(4)), CharOffset::from(1));
    assert_eq!(map.to_frame(CharOffset::zero()), FrameOffset::zero());
    assert_eq!(map.to_frame(CharOffset::from(1)), FrameOffset::from(6));

    // Characters within the first text range map within the range.
    assert_eq!(map.to_content(FrameOffset::from(7)), CharOffset::from(2));
    assert_eq!(map.to_frame(CharOffset::from(2)), FrameOffset::from(7));

    // Characters within the second placeholder map to the closer run.
    assert_eq!(map.to_content(FrameOffset::from(16)), CharOffset::from(9));
    assert_eq!(map.to_content(FrameOffset::from(20)), CharOffset::from(10));
    assert_eq!(map.to_frame(CharOffset::from(9)), FrameOffset::from(14));
    assert_eq!(map.to_frame(CharOffset::from(10)), FrameOffset::from(24));

    // Characters in the last placeholder map to the end of the last text run.
    assert_eq!(map.to_content(FrameOffset::from(28)), CharOffset::from(14));
    assert_eq!(map.to_content(FrameOffset::from(50)), CharOffset::from(14));
    assert_eq!(map.to_frame(CharOffset::from(14)), FrameOffset::from(28));
}

/// Walkthrough test to demonstrate how placeholders are represented in the [`OffsetMap`] and
/// [`TextFrame`].
///
/// This test only runs on macOS because it needs a text-layout implementation for [`EditDelta`]
/// that creates non-empty text frames.
#[test]
#[cfg(target_os = "macos")]
fn test_end_to_end() {
    // Group imports here so they don't cause "unused import" warnings on other targets.

    use warpui::{
        App, color::ColorU, elements::Fill, fonts::Cache as FontCache, text_layout::LayoutCache,
    };

    use crate::{
        content::{
            buffer::{Buffer, BufferEditAction, EditOrigin},
            selection_model::BufferSelectionModel,
            text::IndentBehavior,
        },
        render::{
            layout::TextLayout,
            model::{
                BlockItem, BrokenLinkStyle, CheckBoxStyle, HorizontalRuleStyle, InlineCodeStyle,
                PARAGRAPH_MIN_HEIGHT, ParagraphStyles, RenderLayoutOptions, RichTextStyles,
                TableStyle, test_utils::TEST_BASELINE_OFFSET,
            },
        },
    };

    App::test((), |mut app| async move {
        let mut font_cache = FontCache::new(Box::new(warpui::platform::current::FontDB::new()));
        let layout_cache = LayoutCache::new();
        let paragraph_styles = ParagraphStyles {
            font_family: font_cache
                .load_system_font("Arial")
                .expect("Arial must exist"),
            font_size: 12.,
            font_weight: Default::default(),
            line_height_ratio: 1.2,
            text_color: ColorU::white(),
            baseline_ratio: TEST_BASELINE_OFFSET,
            fixed_width_tab_size: None,
        };
        let inline_code = InlineCodeStyle {
            font_family: font_cache
                .load_system_font("Arial")
                .expect("Arial must exist"),
            background: ColorU::black(),
            font_color: ColorU::white(),
        };
        let checkbox = CheckBoxStyle {
            border_color: ColorU::white(),
            border_width: 2.,
            icon_path: "bundled/svg/check-thick.svg",
            background: ColorU::black(),
            hover_background: ColorU::black(),
        };
        let horizontal_rule = HorizontalRuleStyle {
            rule_height: 2.,
            color: ColorU::black(),
        };
        let broken_link = BrokenLinkStyle {
            icon_path: "bundled/svg/link-broken-02.svg",
            icon_color: ColorU::black(),
        };
        let styles = RichTextStyles {
            base_text: paragraph_styles,
            code_text: paragraph_styles,
            embedding_text: paragraph_styles,
            code_background: Default::default(),
            embedding_background: Default::default(),
            placeholder_color: ColorU::black(),
            code_border: Default::default(),
            selection_fill: Fill::None,
            cursor_fill: Fill::None,
            inline_code_style: inline_code,
            check_box_style: checkbox,
            horizontal_rule_style: horizontal_rule,
            broken_link_style: broken_link,
            block_spacings: Default::default(),
            show_placeholder_text_on_empty_block: false,
            minimum_paragraph_height: Some(PARAGRAPH_MIN_HEIGHT),
            cursor_width: 1.,
            highlight_urls: true,
            table_style: TableStyle {
                border_color: ColorU::black(),
                header_background: ColorU::black(),
                cell_background: ColorU::black(),
                alternate_row_background: None,
                text_color: ColorU::white(),
                header_text_color: ColorU::white(),
                scrollbar_nonactive_thumb_color: ColorU::white(),
                scrollbar_active_thumb_color: ColorU::white(),
                font_family: paragraph_styles.font_family,
                font_size: 12.,
                cell_padding: 8.0,
                outer_border: true,
                column_dividers: true,
                row_dividers: true,
            },
        };

        // Start by creating a buffer with a single line of text that includes a placeholder.
        let buffer_handle = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
        let selection_handle = app.add_model(|_| BufferSelectionModel::new(buffer_handle.clone()));

        buffer_handle.update(&mut app, |buffer, ctx| {
            buffer.update_content(
                BufferEditAction::Insert {
                    text: "HelloWorld",
                    style: Default::default(),
                    override_text_style: None,
                },
                EditOrigin::UserInitiated,
                selection_handle.clone(),
                ctx,
            );
            buffer.update_content(
                BufferEditAction::InsertPlaceholder {
                    text: "test",
                    location: CharOffset::from(6),
                },
                EditOrigin::SystemEdit,
                selection_handle.clone(),
                ctx,
            );

            assert_eq!(
                buffer.debug(),
                "<text>Hello<placeholder_s>test<placeholder_e>World"
            );
            // The placeholder only counts as 1 character, so there are 11 buffer characters.
            assert_eq!(buffer.max_charoffset(), 12.into());
        });

        // Now, lay out the buffer, which should produce a single `Paragraph` block.
        let layout = app.read(|ctx| {
            let delta = buffer_handle.as_ref(ctx).invalidate_layout();
            let text_layout = TextLayout::new(
                &layout_cache,
                font_cache.text_layout_system(),
                &styles,
                1000.,
            );
            delta.layout_delta(
                &text_layout,
                None,
                &RenderLayoutOptions::default(),
                None,
                ctx,
            )
        });
        let paragraph = match &layout.laid_out_line[..] {
            [BlockItem::Paragraph(paragraph)] => paragraph,
            other => panic!("Unexpected blocks: {other:?}"),
        };

        // The `TextFrame` includes each character we paint: "HellotestWorld".
        let line = &paragraph.frame.lines()[0];
        assert_eq!(
            line.runs.iter().map(|run| run.glyphs.len()).sum::<usize>(),
            14
        );
        assert_eq!(line.first_index(), 0); // The "H" glyph.
        assert_eq!(line.last_index(), 13); // The "d" glyph.

        // Because "test" is a placeholder, it creates a gap in the `OffsetMap`:
        // - Characters 0-4 in the buffer map to characters 0-5 in the text frame ("Hello")
        // - The character at buffer index 5 is the placeholder ("test"). It's not in the map, but
        //   is painted by characters 5-8 in the TextFrame
        // - Characters 6-10 in the buffer map to characters 9-13 in the text frame ("World").
        // Overall, it looks like this:
        // Character:       H  e  l  l  o | t  e  s  t | W  o  r  l  d
        // Buffer Index:    0  1  2  3  4 |     5      | 6  7  8  9 10
        // TextFrame Index: 0  1  2  3  4 | 5  6  7  8 | 9 10 11 12 13

        // In the OffsetMap representation, we only store the runs of non-placeholder characters,
        // while placeholder characters form un-selectable "holes".
        assert_eq!(
            paragraph.offsets.runs,
            vec![
                // The run for "Hello":
                SelectableTextRun {
                    content_start: 0.into(),
                    frame_start: 0.into(),
                    length: 5
                },
                // The run for "World":
                SelectableTextRun {
                    content_start: 6.into(),
                    frame_start: 9.into(),
                    length: 5
                }
            ]
        );
        // To go from a buffer character to a TextFrame character, we find the run that contains
        // it - for offset `i`, this is the run where `run.content_start <= i < run.content_start + run.length`.
        // Going from a TextFrame character to a buffer character is more complicated, because the
        // character might belong to a placeholder. In that case, we find the two adjacent runs and
        // pick the closest.
        // Some examples:

        // The "e" in "Hello":
        assert_eq!(paragraph.offsets.to_frame(1.into()), 1.into());
        assert_eq!(paragraph.offsets.to_content(1.into()), 1.into());

        // The "s" in "test":
        // Since it's in a placeholder, we can only use the placeholder's buffer char offset.
        assert_eq!(paragraph.offsets.to_frame(5.into()), 5.into());
        // When going the other direction, it's closer to World than Hello.
        assert_eq!(paragraph.offsets.to_content(7.into()), 6.into());

        // The "r" in "World": Note that the offsets don't map 1:1 because we have to account for
        // the placeholder gap.
        assert_eq!(paragraph.offsets.to_frame(8.into()), 11.into());
        assert_eq!(paragraph.offsets.to_content(11.into()), 8.into());
    });
}
