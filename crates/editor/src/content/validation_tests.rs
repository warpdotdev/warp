use sum_tree::SumTree;
use warpui::elements::ListIndentLevel;

use crate::content::cursor::BufferSumTree;
use crate::content::text::{
    BlockHeaderSize, BufferBlockItem, BufferBlockStyle, BufferText, BufferTextStyle, MarkerDir,
};

#[test]
#[should_panic(
    expected = "2: Found Weight(Bold) end marker, but style was not active\nBuffer: <text>x<b_e>"
)]
fn test_validate_unmatched_style_end() {
    let mut tree = SumTree::new();
    tree.push(BufferText::BlockMarker {
        marker_type: BufferBlockStyle::PlainText,
    });
    tree.append_str("x");
    tree.push(BufferText::Marker {
        marker_type: BufferTextStyle::bold(),
        dir: MarkerDir::End,
    });

    super::validate_content(&tree);
}

#[test]
#[should_panic(
    expected = "3: Found Italic start marker, but style was already active\nBuffer: <text>x<i_s>y<i_s><i_e>"
)]
fn test_validate_unmatched_style_start() {
    let mut tree = SumTree::new();
    tree.push(BufferText::BlockMarker {
        marker_type: BufferBlockStyle::PlainText,
    });
    tree.append_str("x");
    tree.push(BufferText::Marker {
        marker_type: BufferTextStyle::Italic,
        dir: MarkerDir::Start,
    });
    tree.append_str("y");
    tree.push(BufferText::Marker {
        marker_type: BufferTextStyle::Italic,
        dir: MarkerDir::Start,
    });
    tree.push(BufferText::Marker {
        marker_type: BufferTextStyle::Italic,
        dir: MarkerDir::End,
    });

    super::validate_content(&tree);
}

#[test]
#[should_panic(
    expected = "2: Found newline, but active block style only supports single line\nBuffer: <header1>x\\n"
)]
fn test_validate_single_line_header() {
    let mut tree = SumTree::new();
    tree.push(BufferText::BlockMarker {
        marker_type: BufferBlockStyle::Header {
            header_size: BlockHeaderSize::Header1,
        },
    });
    tree.append_str("x");
    tree.push(BufferText::Newline);

    super::validate_content(&tree);
}

#[test]
/*
 * TODO: this test should panic once we prevent styling in code blocks.
#[should_panic(
    expected = "3: Found Bold Start marker inside runnable command block\nBuffer: <text>a<code>x<b_s>y<text><b_e>"
)]
 */
fn test_validate_styled_code() {
    let mut tree = SumTree::new();
    tree.push(BufferText::BlockMarker {
        marker_type: BufferBlockStyle::PlainText,
    });
    tree.append_str("a");
    tree.push(BufferText::BlockMarker {
        marker_type: BufferBlockStyle::CodeBlock {
            code_block_type: Default::default(),
        },
    });
    tree.append_str("x");
    tree.push(BufferText::Marker {
        marker_type: BufferTextStyle::bold(),
        dir: MarkerDir::Start,
    });
    tree.append_str("y");
    tree.push(BufferText::BlockMarker {
        marker_type: BufferBlockStyle::PlainText,
    });
    tree.push(BufferText::Marker {
        marker_type: BufferTextStyle::bold(),
        dir: MarkerDir::End,
    });

    // The marker pairs are balanced, but there cannot be a bold marker inside
    // a runnable command.
    super::validate_content(&tree);
}

// #[test]
// #[should_panic(
//     expected = "2: Tried to start a RunnableCodeBlock block, but Bold was active\nBuffer: a<b_s>b<code_s>c<code_e>d<b_e>"
// )]
// fn test_validate_start_block_with_style() {
//     let mut tree = SumTree::new();
//     tree.push('a'.into());
//     tree.push(BufferText::Marker {
//         marker_type: BufferTextStyle::Bold,
//         dir: MarkerDir::Start,
//     });
//     tree.push('b'.into());
//     tree.push(BufferText::BlockMarker {
//         marker_type: BufferBlockStyle::CodeBlock,
//         dir: MarkerDir::Start,
//     });
//     tree.push('c'.into());
//     tree.push(BufferText::BlockMarker {
//         marker_type: BufferBlockStyle::CodeBlock,
//         dir: MarkerDir::End,
//     });
//     tree.push('d'.into());
//     tree.push(BufferText::Marker {
//         marker_type: BufferTextStyle::Bold,
//         dir: MarkerDir::End,
//     });

//     // The marker pairs are balanced, but there cannot be a style active when
//     // starting a runnable command block.
//     super::validate_content(&tree);
// }

#[test]
#[should_panic(expected = "0: Buffer doesn't have an active block style.\nBuffer: \\ny")]
fn test_validate_buffer_without_start_marker() {
    let mut tree = SumTree::new();
    tree.append_str("\ny");

    super::validate_content(&tree);
}

#[test]
#[should_panic(
    expected = "2: Found plain text marker when the active style is plain text\nBuffer: <text>y<text>x"
)]
fn test_validate_buffer_with_dup_text_marker() {
    let mut tree = SumTree::new();
    tree.push(BufferText::BlockMarker {
        marker_type: BufferBlockStyle::PlainText,
    });
    tree.append_str("y");
    tree.push(BufferText::BlockMarker {
        marker_type: BufferBlockStyle::PlainText,
    });
    tree.append_str("x");

    super::validate_content(&tree);
}

#[test]
#[should_panic(
    expected = "Buffer ends as Some(Text(UnorderedList { indent_level: One })), not plain text.\nBuffer: <text>t<ul0>l"
)]
fn test_validate_buffer_ends_with_plain_text() {
    let mut tree = SumTree::new();
    tree.push(BufferText::BlockMarker {
        marker_type: BufferBlockStyle::PlainText,
    });
    tree.append_str("t");
    tree.push(BufferText::BlockMarker {
        marker_type: BufferBlockStyle::UnorderedList {
            indent_level: ListIndentLevel::One,
        },
    });
    tree.append_str("l");

    super::validate_content(&tree);
}

#[test]
#[should_panic(
    expected = "1: Found character, but active block item does not decorate text\nBuffer: <hr>t"
)]
fn test_validate_block_item_not_decorating_text() {
    let mut tree = SumTree::new();
    tree.push(BufferText::BlockItem {
        item_type: BufferBlockItem::HorizontalRule,
    });
    tree.append_str("t");

    super::validate_content(&tree);
}

#[test]
fn test_validate_ok() {
    let mut tree = SumTree::new();
    tree.push(BufferText::BlockMarker {
        marker_type: BufferBlockStyle::PlainText,
    });
    tree.append_str("a");
    tree.push(BufferText::Marker {
        marker_type: BufferTextStyle::Italic,
        dir: MarkerDir::Start,
    });
    tree.append_str("i");
    tree.push(BufferText::Marker {
        marker_type: BufferTextStyle::Italic,
        dir: MarkerDir::End,
    });
    tree.push(BufferText::BlockItem {
        item_type: BufferBlockItem::HorizontalRule,
    });
    tree.push(BufferText::BlockMarker {
        marker_type: BufferBlockStyle::CodeBlock {
            code_block_type: Default::default(),
        },
    });
    tree.append_str("x");
    tree.push(BufferText::BlockMarker {
        marker_type: BufferBlockStyle::CodeBlock {
            code_block_type: Default::default(),
        },
    });
    tree.append_str("z");
    tree.push(BufferText::BlockMarker {
        marker_type: BufferBlockStyle::PlainText,
    });

    // This should not panic.
    super::validate_content(&tree);
}
