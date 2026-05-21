use crate::content::text::{FormattedTable, table_cell_offset_maps};
use crate::{
    content::text::{BufferBlockStyle, CodeBlockType},
    render::model::{
        BlockItem, COMMAND_SPACING, CellLayout, ImageBlockConfig, LaidOutTable, Location,
        ParagraphBlock, RenderState, TableBlockConfig, TableStyle,
        location::{HitTestBlockType, HitTestOptions, WrapDirection},
        table_offset_map,
        test_utils::{
            TEST_STYLES, laid_out_paragraph, laid_out_unordered_lists, layout_paragraphs,
        },
    },
};
use pathfinder_color::ColorU;
use std::{cell::Cell, sync::Arc};
use string_offset::CharOffset;

use sum_tree::SumTree;
use warpui::assets::asset_cache::AssetSource;
use warpui::fonts::FamilyId;
use warpui::text_layout::{CaretPosition, TextFrame};
use warpui::units::IntoPixels;

fn test_table_layout() -> LaidOutTable {
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
    let cell_layout = CellLayout {
        line_heights: vec![20.0],
        line_y_offsets: vec![0.0],
        line_char_ranges: vec![CharOffset::from(0)..CharOffset::from(3)],
        line_widths: vec![30.0],
        line_caret_positions: vec![vec![
            CaretPosition {
                position_in_line: 0.0,
                start_offset: 0,
                last_offset: 0,
            },
            CaretPosition {
                position_in_line: 10.0,
                start_offset: 1,
                last_offset: 1,
            },
            CaretPosition {
                position_in_line: 20.0,
                start_offset: 2,
                last_offset: 2,
            },
        ]],
    };
    let cell_frame = Arc::new(TextFrame::mock("aaa"));

    LaidOutTable {
        table,
        config: TableBlockConfig {
            width: 60.0.into_pixels(),
            spacing: Default::default(),
            style: TableStyle {
                border_color: ColorU::new(0, 0, 0, 255),
                header_background: ColorU::new(0, 0, 0, 255),
                cell_background: ColorU::new(0, 0, 0, 255),
                alternate_row_background: None,
                text_color: ColorU::new(0, 0, 0, 255),
                header_text_color: ColorU::new(0, 0, 0, 255),
                scrollbar_nonactive_thumb_color: ColorU::new(0, 0, 0, 255),
                scrollbar_active_thumb_color: ColorU::new(0, 0, 0, 255),
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
        scroll_left: Cell::new(30.0.into_pixels()),
        scrollbar_interaction_state: Default::default(),
        horizontal_scroll_allowed: true,
    }
}

#[test]
fn test_hit_within_line() {
    let mut model =
        RenderState::new_for_test(TEST_STYLES.clone(), 40.0.into_pixels(), 60.0.into_pixels());
    model.set_content(SumTree::from_item(laid_out_paragraph(
        "Hello, world!\n",
        &TEST_STYLES,
        40.,
    )));

    assert_eq!(
        model.render_coordinates_to_location(
            12.0.into_pixels(),
            4.3.into_pixels(),
            &Default::default()
        ),
        Location::Text {
            char_offset: 1.into(),
            clamped: false,
            wrap_direction: WrapDirection::Down,
            block_start: CharOffset::zero(),
            link: None,
        }
    );

    assert_eq!(
        model.render_coordinates_to_location(
            20.0.into_pixels(),
            4.3.into_pixels(),
            &Default::default()
        ),
        Location::Text {
            char_offset: 2.into(),
            clamped: false,
            wrap_direction: WrapDirection::Down,
            block_start: CharOffset::zero(),
            link: None,
        }
    );
}

#[test]
fn test_table_hit_testing_accounts_for_horizontal_scroll() {
    let mut model =
        RenderState::new_for_test(TEST_STYLES.clone(), 30.0.into_pixels(), 60.0.into_pixels());
    model.set_content(SumTree::from_item(BlockItem::Table(Box::new(
        test_table_layout(),
    ))));

    assert_eq!(
        model.render_coordinates_to_location(
            0.0.into_pixels(),
            0.0.into_pixels(),
            &Default::default()
        ),
        Location::Text {
            char_offset: 4.into(),
            clamped: false,
            wrap_direction: WrapDirection::Down,
            block_start: CharOffset::zero(),
            link: None,
        }
    );
}

#[test]
fn test_hit_mermaid_block_uses_block_locations_even_with_forced_text_selection() {
    let width = 200.;
    let mut model = RenderState::new_for_test(
        TEST_STYLES.clone(),
        width.into_pixels(),
        160.0.into_pixels(),
    );
    let mermaid = BlockItem::MermaidDiagram {
        content_length: 14.into(),
        asset_source: AssetSource::Bundled {
            path: "bundled/svg/test.svg",
        },
        config: ImageBlockConfig {
            width: 120.0.into_pixels(),
            height: 40.0.into_pixels(),
            spacing: COMMAND_SPACING,
        },
    };
    let mermaid_height = mermaid.height().as_f32();
    model.set_content(SumTree::from_item(mermaid));

    assert_eq!(
        model.render_coordinates_to_location(
            40.0.into_pixels(),
            (mermaid_height / 2.0).into_pixels(),
            &Default::default()
        ),
        Location::Block {
            start_offset: 0.into(),
            end_offset: 14.into(),
            block_type: HitTestBlockType::MermaidDiagram,
        }
    );

    assert_eq!(
        model.render_coordinates_to_location(
            40.0.into_pixels(),
            1.0.into_pixels(),
            &HitTestOptions {
                force_text_selection: true,
            }
        ),
        Location::Block {
            start_offset: 0.into(),
            end_offset: 14.into(),
            block_type: HitTestBlockType::MermaidDiagram,
        }
    );

    assert_eq!(
        model.render_coordinates_to_location(
            40.0.into_pixels(),
            (mermaid_height - 1.0).into_pixels(),
            &HitTestOptions {
                force_text_selection: true,
            }
        ),
        Location::Block {
            start_offset: 0.into(),
            end_offset: 14.into(),
            block_type: HitTestBlockType::MermaidDiagram,
        }
    );
}

#[test]
fn test_hit_within_list() {
    let mut model =
        RenderState::new_for_test(TEST_STYLES.clone(), 40.0.into_pixels(), 60.0.into_pixels());
    model.set_content(SumTree::from_item(laid_out_unordered_lists(
        "Hello, world!\n",
        &TEST_STYLES,
        40.,
    )));

    assert_eq!(
        model.render_coordinates_to_location(
            18.0.into_pixels(),
            4.3.into_pixels(),
            &Default::default()
        ),
        Location::Text {
            char_offset: 0.into(),
            clamped: false,
            wrap_direction: WrapDirection::Down,
            block_start: CharOffset::zero(),
            link: None,
        }
    );

    assert_eq!(
        model.render_coordinates_to_location(
            29.0.into_pixels(),
            4.3.into_pixels(),
            &Default::default()
        ),
        Location::Text {
            char_offset: 1.into(),
            clamped: false,
            wrap_direction: WrapDirection::Down,
            block_start: CharOffset::zero(),
            link: None,
        }
    );
}

#[test]
fn test_hit_empty_line() {
    // This is a regression test for CLD-591.
    let mut model =
        RenderState::new_for_test(TEST_STYLES.clone(), 40.0.into_pixels(), 42.0.into_pixels());
    let mut tree = SumTree::new();
    tree.extend([
        // Height: 0-32, chars: 0-4
        laid_out_paragraph("1st\n", &TEST_STYLES, 40.0),
        // Height: 32-64, chars: 4-5
        laid_out_paragraph("\n", &TEST_STYLES, 40.0),
        // Height: 64-96, chars: 5-9
        laid_out_paragraph("2nd\n", &TEST_STYLES, 40.0),
    ]);
    model.set_content(tree);

    // A hit on the empty line should clamp to within that line, not the start of the next one.
    assert_eq!(
        model.render_coordinates_to_location(
            20.0.into_pixels(),
            40.0.into_pixels(),
            &Default::default()
        ),
        Location::Text {
            char_offset: 4.into(),
            clamped: true,
            wrap_direction: WrapDirection::Up,
            block_start: CharOffset::from(4),
            link: None,
        }
    );
}

#[test]
fn test_hit_on_soft_wrapped_line() {
    let mut model =
        RenderState::new_for_test(TEST_STYLES.clone(), 60.0.into_pixels(), 30.0.into_pixels());
    model.set_content(SumTree::from_item(laid_out_paragraph(
        "Hello, world!\n",
        &TEST_STYLES,
        40., // This is less than the viewport width to account for the 20px of margin.
    )));

    // A hit just after the end of a soft-wrapped line should wrap up.
    assert_eq!(
        model.render_coordinates_to_location(
            46.0.into_pixels(),
            6.0.into_pixels(),
            &Default::default()
        ),
        Location::Text {
            char_offset: 4.into(),
            clamped: true,
            wrap_direction: WrapDirection::Up,
            block_start: CharOffset::zero(),
            link: None,
        }
    );

    // A hit at the start of the next line should have the same char offset, but wrap down.
    assert_eq!(
        model.render_coordinates_to_location(
            0.0.into_pixels(),
            15.0.into_pixels(),
            &Default::default()
        ),
        Location::Text {
            char_offset: 4.into(),
            clamped: false,
            wrap_direction: WrapDirection::Down,
            block_start: CharOffset::zero(),
            link: None,
        }
    );
}

#[test]
fn test_hit_after_end() {
    let mut model =
        RenderState::new_for_test(TEST_STYLES.clone(), 50.0.into_pixels(), 60.0.into_pixels());
    model.set_content(SumTree::from_item(laid_out_paragraph(
        "ABCD\n",
        &TEST_STYLES,
        50.,
    )));

    // A hit after the end, but on the same line, is like a soft-wrapped hit.
    assert_eq!(
        model.render_coordinates_to_location(
            45.0.into_pixels(),
            10.0.into_pixels(),
            &Default::default()
        ),
        Location::Text {
            char_offset: 4.into(),
            clamped: true,
            wrap_direction: WrapDirection::Up,
            block_start: CharOffset::zero(),
            link: None,
        }
    );

    // A hit on the line after the last clamps to the end of content, but wraps
    // to the placeholder next line.
    assert_eq!(
        model.render_coordinates_to_location(
            20.0.into_pixels(),
            33.0.into_pixels(),
            &Default::default()
        ),
        Location::Text {
            char_offset: 5.into(),
            clamped: true,
            wrap_direction: WrapDirection::Down,
            block_start: 5.into(),
            link: None,
        }
    );
}

#[test]
fn test_hit_before_start() {
    let mut model =
        RenderState::new_for_test(TEST_STYLES.clone(), 40.0.into_pixels(), 60.0.into_pixels());
    model.set_content(SumTree::from_item(laid_out_paragraph(
        "ABCDEFGH\n",
        &TEST_STYLES,
        model.viewport().width().as_f32(),
    )));

    // Hit before the start of the first soft-wrapped line, which should clamp to the first
    // character.
    assert_eq!(
        model.render_coordinates_to_location(
            (-4.).into_pixels(),
            10.0.into_pixels(),
            &Default::default()
        ),
        Location::Text {
            char_offset: 0.into(),
            // Not considered clamped, because BlockItem::coordinates_to_location pre-clamps when handling
            // padding.
            clamped: false,
            wrap_direction: WrapDirection::Down,
            block_start: 0.into(),
            link: None,
        }
    );

    // Hit before the start of the second soft-wrapped line, which should clamp to the first
    // character of that line.
    assert_eq!(
        model.render_coordinates_to_location(
            (-4.).into_pixels(),
            20.0.into_pixels(),
            &Default::default()
        ),
        Location::Text {
            char_offset: 4.into(),
            clamped: false,
            wrap_direction: WrapDirection::Down,
            block_start: 0.into(),
            link: None
        }
    );
}

#[test]
fn test_hit_scrolled() {
    let mut model =
        RenderState::new_for_test(TEST_STYLES.clone(), 40.0.into_pixels(), 42.0.into_pixels());
    let mut tree = SumTree::new();
    tree.extend([
        // Height: 0-32, chars: 0-3
        laid_out_paragraph("1st\n", &TEST_STYLES, 40.0),
        // Height: 32-64, chars: 4-7
        laid_out_paragraph("2nd\n", &TEST_STYLES, 40.0),
        // Height: 64-96, chars: 8-15
        laid_out_paragraph("wrapped\n", &TEST_STYLES, 40.0),
        // Height: 96-128, chars: 16-20
        laid_out_paragraph("last\n", &TEST_STYLES, 40.0),
    ]);
    model.set_content(tree);

    // Scroll the viewport directly since we don't have a ModelContext.
    model.viewport.scroll((-40.0).into_pixels(), model.height());

    // Because of scrolling, this hits the second line.
    assert_eq!(
        model.viewport_coordinates_to_location(
            22.0.into_pixels(),
            0.2.into_pixels(),
            &Default::default()
        ),
        Location::Text {
            char_offset: 6.into(),
            clamped: false,
            wrap_direction: WrapDirection::Down,
            block_start: 4.into(),
            link: None,
        }
    );

    // Now, scroll to the last viewport (containing the last paragraph, part of "ped", and the
    // trailing newline).
    model.viewport.scroll((-47.0).into_pixels(), model.height());

    // This line is soft-wrapped to be partially in-viewport.
    assert_eq!(
        model.viewport_coordinates_to_location(
            36.0.into_pixels(),
            0.5.into_pixels(),
            &Default::default()
        ),
        Location::Text {
            char_offset: 15.into(),
            clamped: true,
            wrap_direction: WrapDirection::Up,
            block_start: 8.into(),
            link: None,
        }
    );

    // We should still be able to hit-test at the last line - accounting for the scroll position,
    // this is the very end of it.
    assert_eq!(
        model.viewport_coordinates_to_location(
            0.5.into_pixels(),
            20.0.into_pixels(),
            &Default::default()
        ),
        Location::Text {
            char_offset: 16.into(),
            clamped: false,
            wrap_direction: WrapDirection::Down,
            block_start: 16.into(),
            link: None,
        }
    );
    // Hits past the last line should resolve to the last character in the buffer. In this case,
    // that's a newline, but in a real editor, it would be the last character of the last line,
    // with a TrailingNewline marker after it.
    assert_eq!(
        model.viewport_coordinates_to_location(
            2.0.into_pixels(),
            42.0.into_pixels(),
            &Default::default()
        ),
        Location::Text {
            char_offset: 21.into(),
            clamped: true,
            wrap_direction: WrapDirection::Down,
            block_start: 21.into(),
            link: None,
        }
    );
}

#[test]
fn test_hit_padding() {
    let mut model =
        RenderState::new_for_test(TEST_STYLES.clone(), 40.0.into_pixels(), 40.0.into_pixels());
    let mut tree = SumTree::new();
    tree.extend([laid_out_paragraph("line\n", &TEST_STYLES, 40.0)]);
    model.set_content(tree);

    // Hit in the padding after the paragraph ends. We should return the character that
    // matches the x-axis pixel position on the last line.
    assert_eq!(
        model.viewport_coordinates_to_location(
            38.0.into_pixels(),
            22.0.into_pixels(),
            &Default::default()
        ),
        Location::Text {
            char_offset: 3.into(),
            clamped: true,
            wrap_direction: WrapDirection::Down,
            block_start: CharOffset::zero(),
            link: None,
        }
    );
}

#[test]
fn test_hit_code_block() {
    let width = 200.;
    let mut model =
        RenderState::new_for_test(TEST_STYLES.clone(), width.into_pixels(), 20.0.into_pixels());
    let mut tree = SumTree::new();
    tree.extend([
        laid_out_paragraph("Text\n", &TEST_STYLES, width),
        BlockItem::RunnableCodeBlock {
            paragraph_block: ParagraphBlock::new(layout_paragraphs(
                "Code 1\nCode 2",
                &TEST_STYLES,
                &BufferBlockStyle::CodeBlock {
                    code_block_type: CodeBlockType::Shell,
                },
                width - COMMAND_SPACING.x_axis_offset().as_f32(),
            )),
            code_block_type: Default::default(),
            pending_mermaid_asset: None,
        },
    ]);
    model.set_content(tree);

    // Blocks by height:
    // * 0-32: First paragraph
    // * 32-56: Margin above code block
    // * 56-66: First line of code
    // * 66-76: Second line of code
    // * 76-92: Margin below code block
    // The code block is inset by 16px.

    // Hits within the code block should have the right start location.
    assert_eq!(
        model.render_coordinates_to_location(
            30.0.into_pixels(),
            70.0.into_pixels(),
            &Default::default()
        ),
        Location::Text {
            // The hit should be on the "o" on the second line of code.
            char_offset: 13.into(),
            clamped: false,
            wrap_direction: WrapDirection::Down,
            // The code block starts at offset 5.
            block_start: 5.into(),
            link: None,
        }
    );

    // Hits within the code block's margin are treated as block selections.
    assert_eq!(
        model.render_coordinates_to_location(
            10.0.into_pixels(),
            50.0.into_pixels(),
            &Default::default()
        ),
        Location::Block {
            start_offset: 5.into(),
            end_offset: 19.into(),
            block_type: HitTestBlockType::Code
        }
    );

    // Hits in the horizontal padding are considered part of the text.
    assert_eq!(
        model.render_coordinates_to_location(
            8.0.into_pixels(),
            60.0.into_pixels(),
            &Default::default()
        ),
        Location::Text {
            char_offset: 5.into(),
            clamped: false,
            wrap_direction: WrapDirection::Down,
            block_start: 5.into(),
            link: None
        }
    );
    assert_eq!(
        model.render_coordinates_to_location(
            90.0.into_pixels(),
            60.0.into_pixels(),
            &Default::default()
        ),
        Location::Text {
            char_offset: 11.into(),
            clamped: true,
            wrap_direction: WrapDirection::Up,
            block_start: 5.into(),
            link: None
        }
    );

    // The above rule holds even for out-of-bounds points.
    assert_eq!(
        model.render_coordinates_to_location(
            (-4.).into_pixels(),
            60.0.into_pixels(),
            &Default::default()
        ),
        Location::Text {
            char_offset: 5.into(),
            clamped: false,
            wrap_direction: WrapDirection::Down,
            block_start: 5.into(),
            link: None
        }
    );
    assert_eq!(
        model.render_coordinates_to_location(
            1000.0.into_pixels(),
            60.0.into_pixels(),
            &Default::default()
        ),
        Location::Text {
            char_offset: 11.into(),
            clamped: true,
            wrap_direction: WrapDirection::Up,
            block_start: 5.into(),
            link: None
        }
    );

    // If block selection is disabled (e.g. due to dragging), we still clamp to text.
    assert_eq!(
        model.render_coordinates_to_location(
            27.0.into_pixels(),
            50.0.into_pixels(),
            &HitTestOptions {
                force_text_selection: true
            }
        ),
        Location::Text {
            char_offset: 6.into(),
            clamped: false,
            wrap_direction: WrapDirection::Down,
            block_start: 5.into(),
            link: None
        }
    );
}
