use std::iter;
use std::ops::Range;

use arborium::tree_sitter::{Node, Parser, Query, QueryCursor, TextProvider, Tree};
use languages::language_by_name;
use rangemap::RangeMap;
use streaming_iterator::StreamingIterator;
use string_offset::{ByteOffset, CharOffset};
use warp_editor::content::buffer::{Buffer, ToBufferByteOffset, ToBufferCharOffset};
use warp_editor::content::text::Bytes;
use warpui_core::color::ColorU;

/// Color mapping from parsed syntax token name to its corresponding highlighting color.
#[derive(Clone, Copy)]
pub struct ColorMap {
    pub keyword_color: ColorU,
    pub function_color: ColorU,
    pub string_color: ColorU,
    pub type_color: ColorU,
    pub number_color: ColorU,
    pub comment_color: ColorU,
    pub property_color: ColorU,
    pub tag_color: ColorU,
}

/// Query for retrieving syntax highlighting information on the tokens.
pub struct HighlightQuery {
    highlight_map: Vec<Option<ColorU>>,
}

impl HighlightQuery {
    pub fn new(query: &Query, color_map: ColorMap) -> Self {
        let highlight_map = query
            .capture_names()
            .iter()
            .map(|name| convert_capture_name_to_color(name, &color_map))
            .collect();

        Self { highlight_map }
    }

    /// Given the a character range, return its corresponding highlight colors.
    pub fn get_highlighted_chunks(
        &self,
        range: Range<CharOffset>,
        query: &Query,
        buffer: &Buffer,
        tree: &Tree,
    ) -> RangeMap<CharOffset, ColorU> {
        let mut range_map = RangeMap::new();

        let mut cursor = QueryCursor::new();
        let byte_start = range.start.to_buffer_byte_offset(buffer).as_usize();
        let byte_end = range.end.to_buffer_byte_offset(buffer).as_usize();
        cursor.set_byte_range(byte_start..byte_end);
        let mut captures = cursor.captures(query, tree.root_node(), TextBuffer(buffer));

        while let Some(matches) = captures.next() {
            for cap in matches.0.captures {
                let insertion_range = cap.node.byte_range();
                let color = self
                    .highlight_map
                    .get(cap.index as usize)
                    .and_then(|inner| *inner);

                if let Some(color) = color {
                    let char_start =
                        ByteOffset::from(insertion_range.start).to_buffer_char_offset(buffer);
                    let char_end =
                        ByteOffset::from(insertion_range.end).to_buffer_char_offset(buffer);
                    if char_start < char_end {
                        range_map.insert(char_start..char_end, color);
                    }
                }
            }
        }

        range_map
    }
}

fn convert_capture_name_to_color(name: &str, color_map: &ColorMap) -> Option<ColorU> {
    match name.split('.').next() {
        Some("keyword") => Some(color_map.keyword_color),
        Some("function") => Some(color_map.function_color),
        Some("string") => Some(color_map.string_color),
        Some("type") => Some(color_map.type_color),
        Some("number") => Some(color_map.number_color),
        Some("comment") => Some(color_map.comment_color),
        Some("property") => Some(color_map.property_color),
        Some("tag") => Some(color_map.tag_color),
        _ => None,
    }
}

/// Highlight colors contributed by language injections (e.g. Markdown fenced code blocks).
///
/// For each injection in `tree` whose content overlaps `range`, the embedded language named by the
/// fence is resolved, its content is parsed and highlighted with its own grammar, and the resulting
/// colors are returned in the parent buffer's character coordinates. Injections that set a static
/// language without a captured language node (e.g. HTML blocks) are skipped here — those stay
/// colored by the host grammar's own highlight query.
pub fn injected_highlights(
    injections_query: &Query,
    color_map: &ColorMap,
    range: Range<CharOffset>,
    buffer: &Buffer,
    tree: &Tree,
) -> RangeMap<CharOffset, ColorU> {
    let mut range_map = RangeMap::new();

    let (Some(language_index), Some(content_index)) = (
        injections_query.capture_index_for_name("injection.language"),
        injections_query.capture_index_for_name("injection.content"),
    ) else {
        return range_map;
    };

    // Scope the query to the requested range so we only consider on-screen fences, using the same
    // byte-offset conversion the base highlighter uses for its cursor range.
    let view_start = range.start.to_buffer_byte_offset(buffer).as_usize();
    let view_end = range.end.to_buffer_byte_offset(buffer).as_usize();
    let mut cursor = QueryCursor::new();
    cursor.set_byte_range(view_start..view_end);
    let mut matches = cursor.matches(injections_query, tree.root_node(), TextBuffer(buffer));

    // The buffer's full source, read lazily (only once an on-screen fence is found). Embedded node
    // text is read via tree-sitter's 0-based byte offsets into this source; the buffer's
    // `ByteOffset`/`text_in_range` use a different base, which would shift the text by one byte.
    let mut source: Option<String> = None;
    while let Some(injection) = matches.next() {
        let mut language_node = None;
        let mut content_node = None;
        for capture in injection.captures {
            if capture.index == language_index {
                language_node = Some(capture.node);
            } else if capture.index == content_index {
                content_node = Some(capture.node);
            }
        }
        let (Some(language_node), Some(content_node)) = (language_node, content_node) else {
            continue;
        };
        let content_range = content_node.byte_range();

        let source = source.get_or_insert_with(|| buffer.text().into_string());
        let Ok(language_name) = language_node.utf8_text(source.as_bytes()) else {
            continue;
        };
        let Some(language) = language_by_name(&language_name.trim().to_ascii_lowercase()) else {
            continue;
        };

        let Ok(content) = content_node.utf8_text(source.as_bytes()) else {
            continue;
        };
        let mut parser = Parser::new();
        if parser.set_language(&language.grammar).is_err() {
            continue;
        }
        let Some(content_tree) = parser.parse(content.as_bytes(), None) else {
            continue;
        };

        let highlight_map: Vec<Option<ColorU>> = language
            .highlight_query
            .capture_names()
            .iter()
            .map(|name| convert_capture_name_to_color(name, color_map))
            .collect();

        let mut content_cursor = QueryCursor::new();
        let mut content_captures = content_cursor.captures(
            &language.highlight_query,
            content_tree.root_node(),
            TextSlice(content.as_bytes()),
        );
        while let Some(matches) = content_captures.next() {
            for capture in matches.0.captures {
                let Some(Some(color)) = highlight_map.get(capture.index as usize) else {
                    continue;
                };
                // Sub-tree byte offsets are relative to `content`, which is the parent buffer's
                // bytes starting at `content_range.start`, so shift back into parent coordinates.
                let local = capture.node.byte_range();
                let start = ByteOffset::from(content_range.start + local.start)
                    .to_buffer_char_offset(buffer);
                let end =
                    ByteOffset::from(content_range.start + local.end).to_buffer_char_offset(buffer);
                if start < end {
                    range_map.insert(start..end, *color);
                }
            }
        }
    }

    range_map
}

// The default tree-sitter implementation here is unsafe (since the cursor could query invalid ranges outside of content length).
// TODO(kevin): Once we migrate buffer to store ArrayStrings. We should implement the chunks API on buffer directly to avoid collecting
// into a String and then chunking them again for highlighting.
pub struct TextSlice<'a>(pub &'a [u8]);

impl TextSlice<'_> {
    fn get(&self, range: Range<usize>) -> Self {
        Self(self.0.get(range).unwrap_or_default())
    }
}

impl AsRef<[u8]> for TextSlice<'_> {
    fn as_ref(&self) -> &[u8] {
        self.0
    }
}

impl<'a> TextProvider<TextSlice<'a>> for TextSlice<'a> {
    type I = iter::Once<TextSlice<'a>>;

    fn text(&mut self, node: Node) -> Self::I {
        iter::once(self.get(node.byte_range()))
    }
}

pub struct TextBuffer<'a>(pub &'a Buffer);

impl<'a> TextProvider<&'a [u8]> for TextBuffer<'a> {
    type I = Bytes<'a>;

    fn text(&mut self, node: Node) -> Self::I {
        let range = node.range();
        self.0.bytes_in_range(
            ByteOffset::from(range.start_byte),
            ByteOffset::from(range.end_byte),
        )
    }
}

#[cfg(test)]
#[path = "highlight_query_tests.rs"]
mod tests;
