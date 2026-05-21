use rangemap::RangeSet;
use std::{cell::Cell, sync::Arc};
use sum_tree::SumTree;
use vec1::{Vec1, vec1};
use warpui::assets::asset_cache::AssetSource;
use warpui::{
    color::ColorU,
    fonts::FamilyId,
    geometry::{rect::RectF, vector::vec2f},
    text_layout::TextFrame,
    units::{IntoPixels, Pixels},
};

use super::{
    BlockItem, BlockLocation, COMMAND_SPACING, CellLayout, DEFAULT_BLOCK_SPACINGS,
    HiddenBlockConfig, ImageBlockConfig, LaidOutTable, ParagraphBlock, RenderState,
    TableBlockConfig, TableStyle,
    debug::Describe,
    table_offset_map,
    test_utils::{layout_paragraph, layout_paragraphs},
};
use crate::{
    content::{
        edit::ParsedUrl,
        text::{
            BufferBlockStyle, CodeBlockType, FormattedTable, FormattedTextFragment,
            table_cell_offset_maps,
        },
    },
    render::model::{
        Height, LayoutSummary, LineCount, RenderedSelection, SoftWrapPoint, TEXT_SPACING,
        test_utils::{TEST_STYLES, laid_out_paragraph, mock_paragraph},
    },
};
use markdown_parser::{FormattedTextStyles, Hyperlink};
use string_offset::CharOffset;
use warpui::elements::ListIndentLevel;

#[test]
fn test_height() {
    let mut render_state =
        RenderState::new_for_test(TEST_STYLES, 10.0.into_pixels(), 10.0.into_pixels());
    let mut content = SumTree::new();
    // Height: 32
    content.push(mock_paragraph(24., 1., 1));
    // Height: 56
    content.push(mock_paragraph(48., 1., 2));
    // Height: 32
    content.push(mock_paragraph(24., 1., 3));
    // Height: 32
    content.push(mock_paragraph(24., 1., 4));
    // Height: 40
    content.push(mock_paragraph(32., 1., 5));
    render_state.set_content(content);

    // This includes all content plus the trailing newline marker.
    assert_eq!(render_state.height(), 224.0.into_pixels());
    let content = render_state.content.borrow();
    let mut cursor = content.cursor::<Height, Height>();
    // Ensure we can seek in between items for scrolling.
    cursor.seek(&Height::from(64.), sum_tree::SeekBias::Left);
    assert_eq!(
        cursor.item().expect("Seek succeeded").height().as_f32(),
        56.
    );
    assert_eq!(cursor.start().into_pixels().as_f32(), 32.);
    assert_eq!(cursor.end().into_pixels().as_f32(), 88.);

    let end = cursor.slice(&Height::from(152.), sum_tree::SeekBias::Right);
    assert_eq!(
        end.summary(),
        LayoutSummary {
            content_length: 9.into(),
            height: 56. + 32. + 32.,
            width: (21.).into_pixels(),
            lines: LineCount(3),
            item_count: 3,
        }
    );
}

#[test]
fn test_is_entire_range_of_type_matches_exact_block_ranges() {
    let mut model = RenderState::new_for_test(
        TEST_STYLES.clone(),
        200.0.into_pixels(),
        160.0.into_pixels(),
    );
    let mut content = SumTree::new();
    content.push(laid_out_paragraph("Before\n", &TEST_STYLES, 200.0));
    let mermaid_start = content.extent::<CharOffset>();
    content.push(BlockItem::MermaidDiagram {
        content_length: 14.into(),
        asset_source: AssetSource::Bundled {
            path: "bundled/svg/test.svg",
        },
        config: ImageBlockConfig {
            width: 120.0.into_pixels(),
            height: 40.0.into_pixels(),
            spacing: COMMAND_SPACING,
        },
    });
    let mermaid_end = content.extent::<CharOffset>();
    content.push(laid_out_paragraph("After\n", &TEST_STYLES, 200.0));
    model.set_content(content);

    assert!(
        model.is_entire_range_of_type(&(mermaid_start..mermaid_end), |item| matches!(
            item,
            BlockItem::MermaidDiagram { .. }
        ),)
    );
    assert!(!model.is_entire_range_of_type(
        &(mermaid_start + CharOffset::from(1)..mermaid_end),
        |item| matches!(item, BlockItem::MermaidDiagram { .. }),
    ));
    assert!(!model.is_entire_range_of_type(
        &(mermaid_start..mermaid_end - CharOffset::from(1)),
        |item| matches!(item, BlockItem::MermaidDiagram { .. }),
    ));
    assert!(
        !model.is_entire_range_of_type(&(CharOffset::zero()..mermaid_end), |item| matches!(
            item,
            BlockItem::MermaidDiagram { .. }
        ),)
    );
}

#[test]
fn test_width() {
    let mut render_state =
        RenderState::new_for_test(TEST_STYLES, 10.0.into_pixels(), 10.0.into_pixels());
    let mut content = SumTree::new();
    // Width 25.
    content.push(mock_paragraph(24., 10., 1));
    // Width: 10.
    content.push(mock_paragraph(48., 25., 2));
    render_state.set_content(content);

    // This includes all content plus the trailing newline marker.
    assert_eq!(render_state.width(), (45.).into_pixels());
    let content = render_state.content.borrow();
    let mut cursor = content.cursor::<Height, Height>();
    let end = cursor.slice(&Height::from(40.), sum_tree::SeekBias::Right);
    assert_eq!(
        end.summary(),
        LayoutSummary {
            content_length: 1.into(),
            height: 32.,
            width: (30.).into_pixels(),
            lines: LineCount(1),
            item_count: 1,
        }
    );
}

#[test]
fn test_soft_wrap_point() {
    /// Helper to convert a character count to a pixel x-offset, accounting for plain-text spacing.
    fn char_x(chars: usize) -> Pixels {
        TEXT_SPACING.left_offset() + (chars as f32 * TEST_STYLES.base_text.font_size).into_pixels()
    }

    let mut model =
        RenderState::new_for_test(TEST_STYLES.clone(), 40.0.into_pixels(), 60.0.into_pixels());
    let mut content = SumTree::new();
    // This paragraph soft-wraps to 2 lines and includes chars 0-7.
    content.push(laid_out_paragraph("ABCDEFG\n", &TEST_STYLES, 40.));
    // This paragraph fits on a single line and includes chars 8-12.
    content.push(laid_out_paragraph("ABCD\n", &TEST_STYLES, 40.));
    // This paragraph soft-wraps to 2 lines and includes chars 13-20.
    content.push(laid_out_paragraph("ABCDEFG\n", &TEST_STYLES, 40.));
    // This line is empty and includes char 21.
    content.push(laid_out_paragraph("\n", &TEST_STYLES, 40.));
    // This paragraph fits on a single line and includes chars 22-25.
    content.push(laid_out_paragraph("ABC\n", &TEST_STYLES, 40.));
    assert_eq!(content.extent::<CharOffset>(), CharOffset::from(26));
    assert_eq!(content.extent::<LineCount>().as_usize(), 7);
    model.set_content(content);

    // Last point on the first softwrapped line.
    assert_eq!(
        model.offset_to_softwrap_point(CharOffset::from(3)),
        SoftWrapPoint::new(0, char_x(3))
    );

    // A point slightly closer to 2 than 3 should round to 2.
    assert_eq!(
        model.softwrap_point_to_offset(SoftWrapPoint::new(0, char_x(2) + 4.0.into_pixels())),
        CharOffset::from(2)
    );

    // A point slightly closer to 3 than 2 should round to 3.
    assert_eq!(
        model.softwrap_point_to_offset(SoftWrapPoint::new(0, char_x(3) - 4.0.into_pixels())),
        CharOffset::from(3)
    );

    assert_eq!(
        model.softwrap_point_to_offset(SoftWrapPoint::new(0, char_x(4))),
        CharOffset::from(4)
    );

    // Point on the second softwrapped line in the first paragraph.
    assert_eq!(
        model.offset_to_softwrap_point(CharOffset::from(7)),
        SoftWrapPoint::new(1, char_x(3))
    );
    assert_eq!(
        model.softwrap_point_to_offset(SoftWrapPoint::new(1, char_x(3))),
        CharOffset::from(7)
    );

    // Non-softwrapped line should work as well.
    assert_eq!(
        model.offset_to_softwrap_point(CharOffset::from(10)),
        SoftWrapPoint::new(2, char_x(2))
    );
    assert_eq!(
        model.softwrap_point_to_offset(SoftWrapPoint::new(2, char_x(2))),
        CharOffset::from(10)
    );

    assert_eq!(
        model.offset_to_softwrap_point(CharOffset::from(19)),
        SoftWrapPoint::new(4, char_x(2))
    );
    assert_eq!(
        model.softwrap_point_to_offset(SoftWrapPoint::new(4, char_x(2))),
        CharOffset::from(19)
    );

    // Softwrapping on an empty line should work.
    assert_eq!(
        model.offset_to_softwrap_point(CharOffset::from(21)),
        SoftWrapPoint::new(5, TEXT_SPACING.left_offset())
    );
    assert_eq!(
        model.softwrap_point_to_offset(SoftWrapPoint::new(5, Pixels::zero())),
        CharOffset::from(21)
    );

    // Out of bound points should be bounded to the trailing newline.
    assert_eq!(
        model.offset_to_softwrap_point(CharOffset::from(40)),
        SoftWrapPoint::new(8, Pixels::zero())
    );
    assert_eq!(
        model.softwrap_point_to_offset(SoftWrapPoint::new(7, Pixels::zero())),
        CharOffset::from(26)
    );

    // Points are bounded to their line's contents.
    assert_eq!(
        model.softwrap_point_to_offset(SoftWrapPoint::new(5, char_x(3))),
        CharOffset::from(21)
    );
    assert_eq!(
        model.softwrap_point_to_offset(SoftWrapPoint::new(5, char_x(2))),
        CharOffset::from(21)
    );
}

#[test]
fn test_character_bounds() {
    let mut model =
        RenderState::new_for_test(TEST_STYLES.clone(), 40.0.into_pixels(), 60.0.into_pixels());
    let mut content = SumTree::new();
    // This paragraph soft-wraps to 2 lines and includes chars 0-7.
    content.push(laid_out_paragraph(
        "ABCDEFG\n",
        &TEST_STYLES,
        model.viewport.width().as_f32(),
    ));
    // This paragraph soft-wraps to 2 lines and includes chars 8-14.
    content.push(laid_out_paragraph(
        "HIJKLMN\n",
        &TEST_STYLES,
        model.viewport.width().as_f32(),
    ));
    model.set_content(content);

    // Due to the minimum block height, there is 6px of top spacing. In addition, there's a 4px
    // left margin.

    let char_size = vec2f(10., 10.);

    // The middle of the first line.
    assert_eq!(
        model.character_bounds(2.into()),
        Some(RectF::new(vec2f(24., 6.), char_size))
    );

    // The first character of the second soft-wrapped line.
    assert_eq!(
        model.character_bounds(4.into()),
        Some(RectF::new(vec2f(4., 16.), char_size))
    );

    // The middle of the first line of the second paragraph.
    assert_eq!(
        model.character_bounds(9.into()),
        Some(RectF::new(vec2f(14., 38.), char_size))
    );

    // The end of the first line of the second paragraph.
    assert_eq!(
        model.character_bounds(11.into()),
        Some(RectF::new(vec2f(34., 38.), char_size))
    );

    // The middle of the second line of the second paragraph.
    assert_eq!(
        model.character_bounds(13.into()),
        Some(RectF::new(vec2f(14., 48.), char_size))
    );
}

#[test]
fn test_non_empty_content_can_hide_final_trailing_newline() {
    let mut model = RenderState::new_for_test(
        TEST_STYLES.clone(),
        100.0.into_pixels(),
        200.0.into_pixels(),
    );
    model.set_show_final_trailing_newline_when_non_empty(false);

    let mut content = SumTree::new();
    content.push(BlockItem::RunnableCodeBlock {
        paragraph_block: ParagraphBlock::new(layout_paragraphs(
            "First\nSecond\n",
            &TEST_STYLES,
            &BufferBlockStyle::CodeBlock {
                code_block_type: CodeBlockType::Shell,
            },
            model.viewport.width().as_f32(),
        )),
        code_block_type: Default::default(),
        pending_mermaid_asset: None,
    });
    model.set_content(content);

    assert_eq!(model.blocks(), 1);
    assert_eq!(model.height(), 104.0.into_pixels());
}

#[test]
fn test_empty_content_keeps_final_trailing_newline_when_suppressed() {
    let mut model = RenderState::new_for_test(
        TEST_STYLES.clone(),
        100.0.into_pixels(),
        200.0.into_pixels(),
    );
    model.set_show_final_trailing_newline_when_non_empty(false);

    assert_eq!(model.blocks(), 1);
    assert_eq!(model.height(), 32.0.into_pixels());
}

#[test]
fn test_ordered_list_counting() {
    let mut model =
        RenderState::new_for_test(TEST_STYLES.clone(), 40.0.into_pixels(), 30.0.into_pixels());
    let mut content = SumTree::new();
    content.push(laid_out_paragraph(
        "Text\n",
        &TEST_STYLES,
        model.viewport.width().as_f32(),
    ));
    content.push(BlockItem::OrderedList {
        indent_level: ListIndentLevel::One,
        number: None,
        paragraph: layout_paragraph(
            "One\n",
            &TEST_STYLES,
            &BufferBlockStyle::OrderedList {
                number: None,
                indent_level: ListIndentLevel::One,
            },
            model.viewport.width().as_f32(),
        ),
    });
    content.push(BlockItem::OrderedList {
        indent_level: ListIndentLevel::One,
        number: None,
        paragraph: layout_paragraph(
            "Two\n",
            &TEST_STYLES,
            &BufferBlockStyle::OrderedList {
                number: None,
                indent_level: ListIndentLevel::One,
            },
            model.viewport.width().as_f32(),
        ),
    });
    content.push(BlockItem::OrderedList {
        indent_level: ListIndentLevel::One,
        number: None,
        paragraph: layout_paragraph(
            "Three\n",
            &TEST_STYLES,
            &BufferBlockStyle::OrderedList {
                number: None,
                indent_level: ListIndentLevel::One,
            },
            model.viewport.width().as_f32(),
        ),
    });
    content.push(laid_out_paragraph(
        "Middle\n",
        &TEST_STYLES,
        model.viewport.width().as_f32(),
    ));
    content.push(BlockItem::OrderedList {
        indent_level: ListIndentLevel::One,
        number: Some(10),
        paragraph: layout_paragraph(
            "A\n",
            &TEST_STYLES,
            &BufferBlockStyle::OrderedList {
                number: None,
                indent_level: ListIndentLevel::One,
            },
            model.viewport.width().as_f32(),
        ),
    });
    content.push(BlockItem::OrderedList {
        indent_level: ListIndentLevel::One,
        number: None,
        paragraph: layout_paragraph(
            "B\n",
            &TEST_STYLES,
            &BufferBlockStyle::OrderedList {
                number: None,
                indent_level: ListIndentLevel::One,
            },
            model.viewport.width().as_f32(),
        ),
    });
    content.push(laid_out_paragraph(
        "Last\n",
        &TEST_STYLES,
        model.viewport.width().as_f32(),
    ));
    content.push(BlockItem::OrderedList {
        indent_level: ListIndentLevel::One,
        number: None,
        paragraph: layout_paragraph(
            "i\n",
            &TEST_STYLES,
            &BufferBlockStyle::OrderedList {
                number: None,
                indent_level: ListIndentLevel::One,
            },
            model.viewport.width().as_f32(),
        ),
    });
    content.push(BlockItem::OrderedList {
        indent_level: ListIndentLevel::Two,
        number: None,
        paragraph: layout_paragraph(
            "ii\n",
            &TEST_STYLES,
            &BufferBlockStyle::OrderedList {
                number: None,
                indent_level: ListIndentLevel::Two,
            },
            model.viewport.width().as_f32(),
        ),
    });
    content.push(BlockItem::OrderedList {
        indent_level: ListIndentLevel::Three,
        number: None,
        paragraph: layout_paragraph(
            "iii\n",
            &TEST_STYLES,
            &BufferBlockStyle::OrderedList {
                number: None,
                indent_level: ListIndentLevel::Three,
            },
            model.viewport.width().as_f32(),
        ),
    });
    content.push(BlockItem::OrderedList {
        indent_level: ListIndentLevel::Two,
        number: None,
        paragraph: layout_paragraph(
            "ii\n",
            &TEST_STYLES,
            &BufferBlockStyle::OrderedList {
                number: None,
                indent_level: ListIndentLevel::Two,
            },
            model.viewport.width().as_f32(),
        ),
    });
    content.push(BlockItem::OrderedList {
        indent_level: ListIndentLevel::Two,
        number: None,
        paragraph: layout_paragraph(
            "ii\n",
            &TEST_STYLES,
            &BufferBlockStyle::OrderedList {
                number: None,
                indent_level: ListIndentLevel::Two,
            },
            model.viewport.width().as_f32(),
        ),
    });
    model.set_content(content);

    // Map blocks to start offsets for test readability
    let block_starts = [0, 5, 9, 13, 19, 26, 28, 30, 35, 37, 40, 44, 47].map(CharOffset::from);

    // At the start of the buffer, there's no ordered list, so the numbering starts at 1.
    let mut numbering = model.viewport_list_numbering();
    assert_eq!(numbering.advance(0, None).label_index, 1);

    // If we scroll to just _above_ the first ordered list item, the numbering is still 1.
    model.scroll_near_block(block_starts[1], -2.);
    let mut numbering = model.viewport_list_numbering();
    assert_eq!(numbering.advance(0, None).label_index, 1);

    // If the first ordered list item is partially out of viewport, that still counts - numbering
    // should start at 1.
    model.viewport.scroll((-6.).into_pixels(), model.height());
    let mut numbering = model.viewport_list_numbering();
    assert_eq!(numbering.advance(0, None).label_index, 1);

    // Scroll to the second ordered list item, the numbering should now start at 2.
    model.scroll_near_block(block_starts[2], 1.);
    let mut numbering = model.viewport_list_numbering();
    assert_eq!(numbering.advance(0, None).label_index, 2);

    // Likewise for the third ordered list item.
    model.scroll_near_block(block_starts[3], 1.);
    let mut numbering = model.viewport_list_numbering();
    assert_eq!(numbering.advance(0, None).label_index, 3);

    // Because the plain-text paragraph in the middle isn't an ordered list, we won't bother
    // calculating an initial numbering for it.
    model.scroll_near_block(block_starts[4], 1.);
    let mut numbering = model.viewport_list_numbering();
    assert_eq!(numbering.advance(0, None).label_index, 1);

    // If we scroll to the second list, after the paragraph, numbering resets to its start number.
    model.scroll_near_block(block_starts[5], 1.);
    let mut numbering = model.viewport_list_numbering();
    assert_eq!(numbering.advance(0, Some(10)).label_index, 10);
    model.scroll_near_block(block_starts[6], 1.);
    let mut numbering = model.viewport_list_numbering();
    assert_eq!(numbering.advance(0, None).label_index, 11);

    // Test numbering across indent levels, with the last list.
    model.scroll_near_block(block_starts[11], 1.);
    let mut numbering = model.viewport_list_numbering();
    assert_eq!(numbering.advance(1, None).label_index, 2);
}

#[test]
fn test_first_line_bounds() {
    // Create a model with:
    // * Plain text
    // * A list
    // * A code block
    // * A trailing newline
    // We then test that the first line of each is correct.

    let mut model = RenderState::new_for_test(
        TEST_STYLES.clone(),
        100.0.into_pixels(),
        200.0.into_pixels(),
    );
    let mut content = SumTree::new();
    // This paragraph is 4 soft-wrapped lines.
    content.push(laid_out_paragraph(
        "This is a soft-wrapped paragraph\n",
        &TEST_STYLES,
        model.viewport.width().as_f32(),
    ));
    content.push(BlockItem::UnorderedList {
        indent_level: ListIndentLevel::One,
        paragraph: layout_paragraph(
            "List\n",
            &TEST_STYLES,
            &BufferBlockStyle::OrderedList {
                number: None,
                indent_level: ListIndentLevel::One,
            },
            model.viewport.width().as_f32(),
        ),
    });
    // This list item is 3 soft-wrapped lines.
    content.push(BlockItem::UnorderedList {
        indent_level: ListIndentLevel::Two,
        paragraph: layout_paragraph(
            "Nested and soft-wrapped\n",
            &TEST_STYLES,
            &BufferBlockStyle::OrderedList {
                number: None,
                indent_level: ListIndentLevel::Two,
            },
            model.viewport.width().as_f32(),
        ),
    });
    content.push(BlockItem::RunnableCodeBlock {
        paragraph_block: ParagraphBlock::new(layout_paragraphs(
            "First\nSecond\n",
            &TEST_STYLES,
            &BufferBlockStyle::CodeBlock {
                code_block_type: CodeBlockType::Shell,
            },
            model.viewport.width().as_f32(),
        )),
        code_block_type: Default::default(),
        pending_mermaid_asset: None,
    });
    model.set_content(content);

    let content = model.content();
    let text_block = content
        .block_at_offset(CharOffset::zero())
        .expect("Block should exist");
    // Because the paragraph is soft-wrapped, it doesn't need centering, so the top offset is 4px.
    assert_eq!(
        text_block.first_line_bounds().expect("Bounds should exist"),
        RectF::new(vec2f(0., 4.), vec2f(104., 10.))
    );
    assert_eq!(text_block.item.height().as_f32(), 48.);

    let list_block = content
        .block_at_offset(CharOffset::from(33))
        .expect("Block should exist");
    assert_eq!(
        list_block.first_line_bounds().expect("Bounds should exist"),
        RectF::new(
            vec2f(0., 52.),
            vec2f(
                64., /* 4px margin + 20px list padding + 40px of text */
                10.
            )
        )
    );
    assert_eq!(list_block.item.height().as_f32(), 18.);

    let list_block_2 = content
        .block_at_offset(CharOffset::from(38))
        .expect("Block should exist");
    assert_eq!(
        list_block_2
            .first_line_bounds()
            .expect("Bounds should exist"),
        RectF::new(
            vec2f(0., 70. /* 66px y-offset + 4px margin */),
            vec2f(
                144., /* 4px margin + 40px list padding + 10px of text - the test layout logic doesn't account for spacing */
                10.
            )
        )
    );
    assert_eq!(list_block_2.item.height(), 38.0.into_pixels());

    let code_block = content
        .block_at_offset(CharOffset::from(62))
        .expect("Block should exist");
    assert_eq!(
        code_block.first_line_bounds().expect("Bounds should exist"),
        RectF::new(
            vec2f(0., 112. /* 104px y-offset + 8px margin */),
            vec2f(
                70., /* 4px margin + 16px padding + 50px text */
                16.  /* 16px padding area */
            )
        )
    );
    assert_eq!(
        code_block.item.height(),
        104.0.into_pixels() /* 3 lines of text due to newlines + all the padding + footer*/
    );

    let trailing_block = content
        .block_at_offset(CharOffset::from(76))
        .expect("Block should exist");
    assert_eq!(
        trailing_block
            .first_line_bounds()
            .expect("Bounds should exist"),
        RectF::new(
            vec2f(
                0., 219., /* 198px y-offset + 14px margin + 7px centering */
            ),
            vec2f(5. /* 4px margin + 1px cursor */, 10.)
        )
    )
}

#[test]
fn test_scroll_snapshot() {
    // Lay out the content at the current viewport width.
    fn layout_content(model: &mut RenderState) {
        let mut content = SumTree::new();
        content.push(laid_out_paragraph(
            "AAAABBBBCCCC\n",
            &TEST_STYLES,
            model.viewport().width().as_f32(),
        ));
        content.push(laid_out_paragraph(
            "DDDDEEEEFFFFGGGG\n",
            &TEST_STYLES,
            model.viewport().width().as_f32(),
        ));
        model.set_content(content);
    }

    let mut model =
        RenderState::new_for_test(TEST_STYLES.clone(), 40.0.into_pixels(), 60.0.into_pixels());
    layout_content(&mut model);

    let content = model.content();
    // Verify the height of each block. Each text paragraph has 8px of vertical padding and 10px
    // per soft-wrapped line. The trailing newline block is 32px high.
    assert_eq!(
        content
            .block_at_offset(CharOffset::zero())
            .expect("Block should exist")
            .item
            .height()
            .as_f32(),
        38.
    );
    assert_eq!(
        content
            .block_at_offset(13.into())
            .expect("Block should exist")
            .item
            .height()
            .as_f32(),
        48.
    );
    assert_eq!(
        content
            .block_at_offset(30.into())
            .expect("Block should exist")
            .item
            .height()
            .as_f32(),
        32.
    );
    drop(content);

    // Scroll so that the EEEE line is at the top of the viewport.
    model.viewport.scroll((-52.).into_pixels(), model.height());
    let scroll_position = model.snapshot_scroll_position();
    assert_eq!(scroll_position.first_character_offset(), 17.into());

    // Now, double the viewport width, halving the number of soft-wrapped lines.
    model
        .viewport
        .set_size(vec2f(80., 60.), model.width(), model.height());

    // At first, the content will not have been laid out again, so the scroll position is
    // unaffected.
    assert_eq!(model.viewport.scroll_top(), 52.0.into_pixels());
    // After laying out again, each block is exactly 32px high (the two soft-wrapped blocks are
    // below the minimum height otherwise).
    layout_content(&mut model);
    assert_eq!(model.height().as_f32(), 32. * 3.);

    // Restore the scroll position at the new height. It should still start at the same content.
    assert!(
        model
            .viewport
            .scroll_to(scroll_position.to_scroll_top(&model), model.height())
    );
    // The new scroll position is 32px (the first block) plus 4px of padding on the second block.
    // The EEEE line is now part of that first line.
    assert_eq!(model.viewport.scroll_top().as_f32(), 36.);

    // Halve the original viewport width, leading to twice as many soft-wrapped lines.
    model
        .viewport
        .set_size(vec2f(20., 60.), model.width(), model.height());
    layout_content(&mut model);
    assert_eq!(model.height().as_f32(), 68. + 88. + 32.);

    // Restore the scroll position at the new height.
    assert!(
        model
            .viewport
            .scroll_to(scroll_position.to_scroll_top(&model), model.height())
    );
    // The new scroll position is on the third soft-wrapped line of the second paragraph.
    assert_eq!(model.viewport.scroll_top().as_f32(), 92.);
}

#[test]
fn test_offset_in_active_selection() {
    let render_state =
        RenderState::new_for_test(TEST_STYLES, 10.0.into_pixels(), 10.0.into_pixels());
    let selection_vec: Vec1<RenderedSelection> = vec1![
        RenderedSelection::new(2.into(), 4.into()),
        RenderedSelection::new(6.into(), 8.into()),
        RenderedSelection::new(12.into(), 10.into())
    ];
    let selections = selection_vec.into();
    *render_state.selections.borrow_mut() = selections;

    assert!(render_state.offset_in_active_selection(3.into()));
    assert!(!render_state.offset_in_active_selection(1.into()));
    assert!(render_state.offset_in_active_selection(7.into()));
    assert!(!render_state.offset_in_active_selection(9.into()));
    assert!(!render_state.offset_in_active_selection(2.into()));
    assert!(render_state.offset_in_active_selection(4.into()));
    assert!(!render_state.offset_in_active_selection(10.into()));
    assert!(render_state.offset_in_active_selection(12.into()));
    assert!(render_state.offset_in_active_selection(11.into()));
}

#[test]
fn test_is_selection_head() {
    let render_state =
        RenderState::new_for_test(TEST_STYLES, 10.0.into_pixels(), 10.0.into_pixels());
    let selection_vec: Vec1<RenderedSelection> = vec1![
        RenderedSelection::new(2.into(), 4.into()),
        RenderedSelection::new(6.into(), 8.into()),
        RenderedSelection::new(12.into(), 10.into())
    ];
    let selections = selection_vec.into();
    *render_state.selections.borrow_mut() = selections;

    assert!(render_state.is_selection_head(2.into()));
    assert!(!render_state.is_selection_head(1.into()));
    assert!(!render_state.is_selection_head(4.into()));
    assert!(render_state.is_selection_head(6.into()));
    assert!(render_state.is_selection_head(12.into()));
}

#[test]
fn test_multiselect_autoscroll_bounding_box() {
    // Test that the computation for the autoscroll bounding box work correctly.
    let view_height = 800.0.into_pixels();

    // One selection, on screen.
    assert_eq!(
        RenderState::multiselect_autoscroll_bounding_box(
            vec1![(vec2f(0., 0.), vec2f(0., 0.))],
            view_height,
            0.0.into_pixels(),
        ),
        (vec2f(0., 0.), vec2f(0., 0.))
    );

    // One selection, on screen.
    assert_eq!(
        RenderState::multiselect_autoscroll_bounding_box(
            vec1![(vec2f(100., 100.), vec2f(100., 100.))],
            view_height,
            0.0.into_pixels(),
        ),
        (vec2f(100., 100.), vec2f(100., 100.))
    );

    // Two selections, on screen.
    assert_eq!(
        RenderState::multiselect_autoscroll_bounding_box(
            vec1![
                (vec2f(100., 100.), vec2f(100.0, 100.0)),
                (vec2f(200., 200.), vec2f(200., 200.))
            ],
            view_height,
            0.0.into_pixels(),
        ),
        (vec2f(100., 100.), vec2f(200., 200.))
    );

    // Three selections, top two on screen, but the third one is too far to fit.
    // Pick a selection that isn't larger than the viewport
    assert_eq!(
        RenderState::multiselect_autoscroll_bounding_box(
            vec1![
                (vec2f(100., 100.), vec2f(100.0, 100.0)),
                (vec2f(200., 200.), vec2f(200., 200.)),
                (vec2f(300., 1000.), vec2f(300., 1000.))
            ],
            view_height,
            0.0.into_pixels(),
        ),
        (vec2f(100., 100.), vec2f(200., 200.))
    );

    // Three selections, one on screen, so the other two should not be scrolled to.
    // Pick a selection that isn't larger than the viewport
    assert_eq!(
        RenderState::multiselect_autoscroll_bounding_box(
            vec1![
                (vec2f(100., 700.), vec2f(100.0, 700.0)),
                (vec2f(200., 900.), vec2f(200., 900.)),
                (vec2f(300., 1000.), vec2f(300., 1000.))
            ],
            view_height,
            0.0.into_pixels(),
        ),
        (vec2f(100., 700.), vec2f(100., 700.))
    );

    // Three selections, all off screen to the bottom, so we should fit as many as we can.
    assert_eq!(
        RenderState::multiselect_autoscroll_bounding_box(
            vec1![
                (vec2f(100., 1000.), vec2f(100.0, 1000.0)),
                (vec2f(200., 1400.), vec2f(200., 1400.)),
                (vec2f(300., 1900.), vec2f(300., 1900.))
            ],
            view_height,
            0.0.into_pixels(),
        ),
        (vec2f(100., 1000.), vec2f(200., 1400.))
    );

    // Three selections, all off screen to the top, so we should fit as many as we can from the bottom up.
    assert_eq!(
        RenderState::multiselect_autoscroll_bounding_box(
            vec1![
                (vec2f(100., 0.), vec2f(100.0, 0.0)),
                (vec2f(200., 500.), vec2f(200., 500.)),
                (vec2f(300., 1200.), vec2f(300., 1200.))
            ],
            view_height,
            1500.0.into_pixels(),
        ),
        (vec2f(200., 500.), vec2f(300., 1200.))
    );
}

// 18:09:15 [INFO] [warp_editor::render::model] Initial tree:
// -------- 0.00px / 0 characters --------
// Hidden (3067 characters, 87 lines, 20.00px tall)
// -------- 20.00px / 3067 characters --------
// Paragraph (32 characters, 1 lines, 18.20px tall)
// -------- 38.20px / 3099 characters --------
// Paragraph (28 characters, 1 lines, 18.20px tall)
// -------- 56.40px / 3127 characters --------
// Paragraph (28 characters, 1 lines, 18.20px tall)
// -------- 74.60px / 3155 characters --------
// Paragraph (37 characters, 1 lines, 18.20px tall)
// -------- 92.80px / 3192 characters --------
// Paragraph (13 characters, 1 lines, 18.20px tall)
// -------- 111.00px / 3205 characters --------
// Paragraph (6 characters, 1 lines, 18.20px tall)
// -------- 129.20px / 3211 characters --------
// Paragraph (2 characters, 1 lines, 18.20px tall)
// -------- 147.40px / 3213 characters --------
// Hidden (406 characters, 15 lines, 20.00px tall)
// -------- 167.40px / 3619 characters --------
// Paragraph (41 characters, 1 lines, 18.20px tall)
// -------- 185.60px / 3660 characters --------
// Paragraph (73 characters, 1 lines, 18.20px tall)
// -------- 203.80px / 3733 characters --------
// Paragraph (57 characters, 1 lines, 18.20px tall)
// -------- 222.00px / 3790 characters --------
// Paragraph (17 characters, 1 lines, 18.20px tall)
// -------- 240.20px / 3807 characters --------
// Paragraph (36 characters, 1 lines, 18.20px tall)
// -------- 258.40px / 3843 characters --------
// Paragraph (29 characters, 1 lines, 18.20px tall)
// -------- 276.60px / 3872 characters --------
// Temporary Paragraph (0 characters, 0 lines, 18.20px tall)
// -------- 294.80px / 3872 characters --------
// Temporary Paragraph (0 characters, 0 lines, 18.20px tall)
// -------- 313.00px / 3872 characters --------
// Paragraph (10 characters, 1 lines, 18.20px tall)
// -------- 331.20px / 3882 characters --------
// Paragraph (6 characters, 1 lines, 18.20px tall)
// -------- 349.40px / 3888 characters --------
// Hidden (1 characters, 1 lines, 20.00px tall)
//
// Nothing needs to be changed here. There is no overlapping hidden ranges.
#[test]
fn test_dedupe_hidden_ranges_logged_tree_is_unchanged() {
    // This is a "golden" structure derived from the logs in the prompt.
    // The observed behavior was that `dedupe_hidden_ranges` is a no-op for this tree.

    let mut tree = SumTree::new();

    tree.push(BlockItem::Hidden(HiddenBlockConfig::new(
        LineCount(87),
        CharOffset::from(3066),
        BlockLocation::Start,
    )));

    for len in [32usize, 28, 28, 37, 13, 6, 2] {
        tree.push(mock_paragraph(18.2, 0., len));
    }

    tree.push(BlockItem::Hidden(HiddenBlockConfig::new(
        LineCount(15),
        CharOffset::from(406),
        BlockLocation::Middle,
    )));

    for len in [41usize, 73, 57, 17, 36, 29] {
        tree.push(mock_paragraph(18.2, 0., len));
    }

    let temporary_paragraph =
        layout_paragraph("\n", &TEST_STYLES, &BufferBlockStyle::PlainText, 80.);
    let temporary_block = BlockItem::TemporaryBlock {
        paragraph_block: ParagraphBlock::new(vec1![temporary_paragraph]),
        text_decoration: Vec::new(),
        decoration: None,
    };
    tree.push(temporary_block.clone());
    tree.push(temporary_block);

    for len in [10usize, 6] {
        tree.push(mock_paragraph(18.2, 0., len));
    }

    tree.push(BlockItem::Hidden(HiddenBlockConfig::new(
        LineCount(1),
        CharOffset::from(1),
        BlockLocation::End,
    )));

    let mut hidden_ranges = RangeSet::new();
    hidden_ranges.insert(CharOffset::from(1)..CharOffset::from(3067));
    hidden_ranges.insert(CharOffset::from(3213)..CharOffset::from(3619));
    hidden_ranges.insert(CharOffset::from(3888)..CharOffset::from(3889));

    let initial = tree.describe().to_string();
    let resulting = RenderState::dedupe_hidden_ranges(tree, hidden_ranges)
        .describe()
        .to_string();

    assert_eq!(initial, resulting);
}

// 18:09:14 [INFO] [warp_editor::render::model] Initial tree:
// -------- 0.00px / 0 characters --------
// Hidden (3066 characters, 87 lines, 20.00px tall)
// -------- 20.00px / 3067 characters --------
// Paragraph (32 characters, 1 lines, 18.20px tall)
// -------- 38.20px / 3099 characters --------
// Paragraph (28 characters, 1 lines, 18.20px tall)
// -------- 56.40px / 3127 characters --------
// Paragraph (28 characters, 1 lines, 18.20px tall)
// -------- 74.60px / 3155 characters --------
// Paragraph (37 characters, 1 lines, 18.20px tall)
// -------- 92.80px / 3192 characters --------
// Paragraph (13 characters, 1 lines, 18.20px tall)
// -------- 111.00px / 3205 characters --------
// Paragraph (6 characters, 1 lines, 18.20px tall)
// -------- 129.20px / 3211 characters --------
// Paragraph (2 characters, 1 lines, 18.20px tall)
// -------- 147.40px / 3213 characters --------
// Hidden (406 characters, 15 lines, 20.00px tall)
// -------- 167.40px / 3619 characters --------
// Paragraph (41 characters, 1 lines, 18.20px tall)
// -------- 185.60px / 3660 characters --------
// Paragraph (73 characters, 1 lines, 18.20px tall)
// -------- 203.80px / 3733 characters --------
// Paragraph (57 characters, 1 lines, 18.20px tall)
// -------- 222.00px / 3790 characters --------
// Paragraph (17 characters, 1 lines, 18.20px tall)
// -------- 240.20px / 3807 characters --------
// Paragraph (36 characters, 1 lines, 18.20px tall)
// -------- 258.40px / 3843 characters --------
// Paragraph (29 characters, 1 lines, 18.20px tall)
// -------- 276.60px / 3872 characters --------
// Hidden (1 characters, 1 lines, 20.00px tall)
// -------- 296.60px / 3873 characters --------
// Hidden (1944 characters, 45 lines, 20.00px tall)
//
// The last two hidden sections should be collapsed.
#[test]
fn test_dedupe_hidden_ranges_merges_adjacent_hidden_blocks() {
    let mut tree = SumTree::new();

    // Pushing a hidden range that actually exceed what is expected from the canonical range.
    tree.push(BlockItem::Hidden(HiddenBlockConfig::new(
        LineCount(87),
        CharOffset::from(3067),
        BlockLocation::Start,
    )));

    for len in [32usize, 28, 28, 37, 13, 6, 2] {
        tree.push(mock_paragraph(18.2, 0., len));
    }

    tree.push(BlockItem::Hidden(HiddenBlockConfig::new(
        LineCount(15),
        CharOffset::from(406),
        BlockLocation::Middle,
    )));

    for len in [41usize, 73, 57, 17, 36, 29] {
        tree.push(mock_paragraph(18.2, 0., len));
    }

    // Two adjacent hidden blocks.
    tree.push(BlockItem::Hidden(HiddenBlockConfig::new(
        LineCount(1),
        CharOffset::from(1),
        BlockLocation::Middle,
    )));
    tree.push(BlockItem::Hidden(HiddenBlockConfig::new(
        LineCount(45),
        CharOffset::from(1944),
        BlockLocation::End,
    )));

    let mut hidden_ranges = RangeSet::new();
    hidden_ranges.insert(CharOffset::from(1)..CharOffset::from(3067));
    hidden_ranges.insert(CharOffset::from(3213)..CharOffset::from(3619));

    // Covers both adjacent hidden blocks (3872 + 1 + 1944 = 5817 total content length).
    hidden_ranges.insert(CharOffset::from(3872)..CharOffset::from(5818));

    let resulting = RenderState::dedupe_hidden_ranges(tree, hidden_ranges);

    let mut expected = SumTree::new();

    expected.push(BlockItem::Hidden(HiddenBlockConfig::new(
        LineCount(87),
        CharOffset::from(3066),
        BlockLocation::Start,
    )));

    for len in [32usize, 28, 28, 37, 13, 6, 2] {
        expected.push(mock_paragraph(18.2, 0., len));
    }

    expected.push(BlockItem::Hidden(HiddenBlockConfig::new(
        LineCount(15),
        CharOffset::from(406),
        BlockLocation::Middle,
    )));

    for len in [41usize, 73, 57, 17, 36, 29] {
        expected.push(mock_paragraph(18.2, 0., len));
    }

    expected.push(BlockItem::Hidden(HiddenBlockConfig::new(
        LineCount(46),
        CharOffset::from(1946),
        BlockLocation::End,
    )));

    assert_eq!(
        expected.describe().to_string(),
        resulting.describe().to_string()
    );
}

#[allow(clippy::single_range_in_vec_init)]
fn make_test_cell_layout() -> CellLayout {
    CellLayout {
        line_heights: vec![20.0],
        line_y_offsets: vec![0.0],
        line_char_ranges: vec![CharOffset::from(0)..CharOffset::from(3)],
        line_widths: vec![30.0],
        line_caret_positions: vec![vec![
            warpui::text_layout::CaretPosition {
                position_in_line: 0.0,
                start_offset: 0,
                last_offset: 0,
            },
            warpui::text_layout::CaretPosition {
                position_in_line: 10.0,
                start_offset: 1,
                last_offset: 1,
            },
            warpui::text_layout::CaretPosition {
                position_in_line: 20.0,
                start_offset: 2,
                last_offset: 2,
            },
        ]],
    }
}

#[test]
fn test_line_at_char_offset() {
    let layout = make_test_cell_layout();
    assert_eq!(layout.line_at_char_offset(CharOffset::from(0)), Some(0));
    assert_eq!(layout.line_at_char_offset(CharOffset::from(1)), Some(0));
    assert_eq!(layout.line_at_char_offset(CharOffset::from(2)), Some(0));
    assert_eq!(layout.line_at_char_offset(CharOffset::from(5)), Some(0));
}

#[test]
fn test_x_for_char_in_line() {
    let layout = make_test_cell_layout();
    assert_eq!(layout.x_for_char_in_line(0, 0), 0.0);
    assert_eq!(layout.x_for_char_in_line(0, 1), 10.0);
    assert_eq!(layout.x_for_char_in_line(0, 2), 20.0);
    assert_eq!(layout.x_for_char_in_line(0, 3), 30.0);
}

#[test]
fn test_line_at_y_offset() {
    let layout = make_test_cell_layout();
    assert_eq!(layout.line_at_y_offset(0.0), 0);
    assert_eq!(layout.line_at_y_offset(10.0), 0);
    assert_eq!(layout.line_at_y_offset(19.9), 0);
    assert_eq!(layout.line_at_y_offset(20.0), 0);
}

#[test]
fn test_char_at_x_in_line_at_zero() {
    let layout = make_test_cell_layout();
    assert_eq!(layout.char_at_x_in_line(0, 0.0), CharOffset::from(0));
}

#[test]
fn test_char_at_x_in_line_at_small_x() {
    let layout = make_test_cell_layout();
    assert_eq!(layout.char_at_x_in_line(0, 1.0), CharOffset::from(0));
    assert_eq!(layout.char_at_x_in_line(0, 4.0), CharOffset::from(0));
}

#[test]
fn test_char_at_x_in_line_at_boundary() {
    let layout = make_test_cell_layout();
    assert_eq!(layout.char_at_x_in_line(0, 5.0), CharOffset::from(1));
    assert_eq!(layout.char_at_x_in_line(0, 10.0), CharOffset::from(1));
}

#[test]
fn test_char_at_x_in_line_near_line_end_maps_to_end_offset() {
    let layout = make_test_cell_layout();
    assert_eq!(layout.char_at_x_in_line(0, 25.0), CharOffset::from(3));
}

fn make_test_laid_out_table() -> LaidOutTable {
    let source = "aaa\tbbb\nccc\tddd\n";
    let table = FormattedTable::from_internal_format(source);
    let cell_offset_maps = table_cell_offset_maps(&table, source);
    let offset_map = table_offset_map::TableOffsetMap::new(
        cell_offset_maps
            .iter()
            .map(|row| {
                row.iter()
                    .map(|cell| cell.source_length().as_usize())
                    .collect()
            })
            .collect(),
    );
    let content_length = offset_map.total_length();
    let cell_layout = make_test_cell_layout();
    let cell_frame = Arc::new(TextFrame::mock("aaa"));
    LaidOutTable {
        table,
        config: TableBlockConfig {
            width: 60.0.into_pixels(),
            spacing: DEFAULT_BLOCK_SPACINGS.text,
            style: TableStyle {
                border_color: ColorU {
                    r: 0,
                    g: 0,
                    b: 0,
                    a: 255,
                },
                header_background: ColorU {
                    r: 0,
                    g: 0,
                    b: 0,
                    a: 255,
                },
                cell_background: ColorU {
                    r: 0,
                    g: 0,
                    b: 0,
                    a: 255,
                },
                alternate_row_background: None,
                text_color: ColorU {
                    r: 0,
                    g: 0,
                    b: 0,
                    a: 255,
                },
                header_text_color: ColorU {
                    r: 0,
                    g: 0,
                    b: 0,
                    a: 255,
                },
                scrollbar_nonactive_thumb_color: ColorU {
                    r: 0,
                    g: 0,
                    b: 0,
                    a: 255,
                },
                scrollbar_active_thumb_color: ColorU {
                    r: 0,
                    g: 0,
                    b: 0,
                    a: 255,
                },
                font_family: FamilyId(0),
                font_size: 10.0,
                cell_padding: 0.0,
                outer_border: true,
                column_dividers: true,
                row_dividers: true,
            },
        },
        row_heights: vec![20.0.into_pixels(), 20.0.into_pixels()],
        column_widths: vec![30.0.into_pixels(), 30.0.into_pixels()],
        total_height: 40.0.into_pixels(),
        offset_map,
        content_length,
        cell_offset_maps,
        row_y_offsets: vec![0.0, 20.0, 40.0],
        col_x_offsets: vec![0.0, 30.0, 60.0],
        cell_text_frames: vec![
            vec![cell_frame.clone(), cell_frame.clone()],
            vec![cell_frame.clone(), cell_frame],
        ],
        cell_layouts: vec![
            vec![cell_layout.clone(), cell_layout.clone()],
            vec![cell_layout.clone(), cell_layout],
        ],
        cell_links: vec![vec![vec![], vec![]], vec![vec![], vec![]]],
        scroll_left: Cell::new(Pixels::zero()),
        scrollbar_interaction_state: Default::default(),
        horizontal_scroll_allowed: true,
    }
}

#[test]
fn test_coordinate_to_offset() {
    let table = make_test_laid_out_table();
    assert_eq!(table.coordinate_to_offset(0.0, 0.0), CharOffset::from(0));
    assert_eq!(table.coordinate_to_offset(10.0, 0.0), CharOffset::from(1));
    assert_eq!(table.coordinate_to_offset(30.0, 0.0), CharOffset::from(4));
    assert_eq!(table.coordinate_to_offset(0.0, 20.0), CharOffset::from(8));
}

#[test]
fn test_coordinate_to_offset_near_cell_line_end_maps_to_cell_end() {
    let table = make_test_laid_out_table();
    assert_eq!(table.coordinate_to_offset(25.0, 0.0), CharOffset::from(3));
}

#[test]
fn test_reveal_offset_scrolls_table_character_into_view() {
    let table = make_test_laid_out_table();
    assert_eq!(table.scroll_left(), Pixels::zero());
    assert!(table.reveal_offset(CharOffset::from(5), 30.0.into_pixels()));
    assert_eq!(table.scroll_left(), 28.0.into_pixels());
}

#[test]
fn test_disabled_horizontal_scroll_returns_full_viewport_width() {
    let mut table = make_test_laid_out_table();
    table.horizontal_scroll_allowed = false;

    assert_eq!(table.viewport_width(30.0.into_pixels()), table.width());
    assert_eq!(table.max_scroll_left(30.0.into_pixels()), Pixels::zero());
}

#[test]
fn test_disabled_horizontal_scroll_reports_zero_scroll_left() {
    let mut table = make_test_laid_out_table();
    table.scroll_left.set(15.0.into_pixels());
    table.horizontal_scroll_allowed = false;

    assert_eq!(table.scroll_left(), Pixels::zero());
}

#[test]
fn test_disabled_horizontal_scroll_set_scroll_left_is_noop() {
    let mut table = make_test_laid_out_table();
    table.horizontal_scroll_allowed = false;

    assert!(!table.set_scroll_left(20.0.into_pixels(), 30.0.into_pixels()));
    assert!(!table.scroll_horizontally(10.0.into_pixels(), 30.0.into_pixels()));
    assert_eq!(table.scroll_left(), Pixels::zero());
}

#[test]
fn test_disabled_horizontal_scroll_reveal_offset_is_noop() {
    let mut table = make_test_laid_out_table();
    table.horizontal_scroll_allowed = false;

    assert!(!table.reveal_offset(CharOffset::from(5), 30.0.into_pixels()));
    assert_eq!(table.scroll_left(), Pixels::zero());
}

#[test]
fn test_link_at_offset_uses_cached_cell_links() {
    let mut table = make_test_laid_out_table();
    table.table = FormattedTable {
        headers: vec![
            vec![
                FormattedTextFragment::plain_text("a"),
                FormattedTextFragment {
                    text: "bc".into(),
                    styles: FormattedTextStyles {
                        hyperlink: Some(Hyperlink::Url("https://warp.dev".into())),
                        ..Default::default()
                    },
                },
            ],
            vec![FormattedTextFragment::plain_text("bbb")],
        ],
        alignments: vec![],
        rows: vec![vec![
            vec![FormattedTextFragment::plain_text("ccc")],
            vec![FormattedTextFragment::plain_text("ddd")],
        ]],
    };
    table.cell_links = vec![
        vec![
            vec![ParsedUrl::new(1..3, "https://warp.dev".into())],
            vec![],
        ],
        vec![vec![], vec![]],
    ];

    assert_eq!(
        table.link_at_offset(CharOffset::from(1)),
        Some("https://warp.dev".into())
    );
    assert_eq!(
        table.link_at_offset(CharOffset::from(2)),
        Some("https://warp.dev".into())
    );
    assert_eq!(table.link_at_offset(CharOffset::from(0)), None);
    assert_eq!(table.link_at_offset(CharOffset::from(3)), None);
}
