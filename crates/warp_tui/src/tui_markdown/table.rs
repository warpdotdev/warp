use markdown_parser::{FormattedTable, FormattedTextInline, Hyperlink, TableAlignment};
use unicode_width::UnicodeWidthStr;
use warpui_core::elements::tui::{
    TuiConstraint, TuiElement, TuiFlex, TuiLayoutContext, TuiPaintContext, TuiPaintSurface,
    TuiParentElement, TuiPresentationContext, TuiScreenPoint, TuiScreenPosition, TuiSize, TuiStyle,
    TuiText,
};
use warpui_core::AppContext;

use super::{
    blank_row, inline_spans, push_span, TuiFixedWidth, TuiMarkdownPalette, TuiMarkdownRule,
};

const TABLE_COLUMN_GAP: u16 = 3;
const MIN_TABLE_COLUMN_WIDTH: u16 = 3;
const TARGET_TABLE_COLUMN_WIDTH: u16 = 8;

/// Renders a structured table independently so agent semantic table sections
/// can share the same responsive presentation.
pub(crate) fn render_formatted_table(
    table: &FormattedTable,
    palette: TuiMarkdownPalette,
) -> Box<dyn TuiElement> {
    TuiMarkdownTable::new(table.clone(), palette).finish()
}

/// A width-responsive table that rebuilds a composable inner element during
/// layout, once the actual terminal width is known.
struct TuiMarkdownTable {
    table: FormattedTable,
    palette: TuiMarkdownPalette,
    inner: TuiFlex,
}

impl TuiMarkdownTable {
    fn new(table: FormattedTable, palette: TuiMarkdownPalette) -> Self {
        Self {
            table,
            palette,
            inner: TuiFlex::column(),
        }
    }

    fn build(&self, width: u16) -> TuiFlex {
        let (headers, rows, alignments) = normalized_table(&self.table);
        if headers.is_empty() {
            return TuiFlex::column().child(
                TuiText::new("[Empty table]")
                    .with_style(self.palette.fallback)
                    .finish(),
            );
        }

        let preferred = preferred_column_widths(&headers, &rows);
        match allocate_column_widths(&preferred, width) {
            Some(widths) => self.aligned_table(&headers, &rows, &alignments, &widths),
            None => self.record_table(&headers, &rows),
        }
    }

    fn aligned_table(
        &self,
        headers: &[FormattedTextInline],
        rows: &[Vec<FormattedTextInline>],
        alignments: &[TableAlignment],
        widths: &[u16],
    ) -> TuiFlex {
        let mut table = TuiFlex::column();
        table.add_child(table_row(
            headers,
            alignments,
            widths,
            self.palette.table_header,
            self.palette,
        ));
        table.add_child(TuiMarkdownRule::new(self.palette.rule).finish());
        for row in rows {
            table.add_child(table_row(
                row,
                alignments,
                widths,
                self.palette.body,
                self.palette,
            ));
        }
        table
    }

    fn record_table(
        &self,
        headers: &[FormattedTextInline],
        rows: &[Vec<FormattedTextInline>],
    ) -> TuiFlex {
        let mut table = TuiFlex::column();
        if rows.is_empty() {
            table.add_child(
                TuiText::new("[Table has no rows]")
                    .with_style(self.palette.fallback)
                    .finish(),
            );
            return table;
        }
        for (row_index, row) in rows.iter().enumerate() {
            for (header, value) in headers.iter().zip(row) {
                let mut spans = inline_spans(header, self.palette.table_header, self.palette);
                push_span(&mut spans, ": ".to_owned(), self.palette.muted);
                spans.extend(inline_spans(value, self.palette.body, self.palette));
                table.add_child(TuiText::from_spans(spans).finish());
            }
            if row_index + 1 < rows.len() {
                table.add_child(blank_row());
            }
        }
        table
    }
}

impl TuiElement for TuiMarkdownTable {
    fn layout(
        &mut self,
        constraint: TuiConstraint,
        ctx: &mut TuiLayoutContext,
        app: &AppContext,
    ) -> TuiSize {
        let width = constraint.constrain_width(constraint.max.width);
        self.inner = self.build(width);
        self.inner.layout(constraint, ctx, app)
    }

    fn render(
        &mut self,
        origin: TuiScreenPosition,
        surface: &mut TuiPaintSurface<'_>,
        ctx: &mut TuiPaintContext,
    ) {
        self.inner.render(origin, surface, ctx);
    }

    fn size(&self) -> Option<TuiSize> {
        self.inner.size()
    }

    fn origin(&self) -> Option<TuiScreenPoint> {
        self.inner.origin()
    }

    fn present(&mut self, ctx: &mut TuiPresentationContext<'_>) {
        self.inner.present(ctx);
    }
}

fn normalized_table(
    table: &FormattedTable,
) -> (
    Vec<FormattedTextInline>,
    Vec<Vec<FormattedTextInline>>,
    Vec<TableAlignment>,
) {
    let column_count = table
        .rows
        .iter()
        .map(Vec::len)
        .chain([table.headers.len()])
        .max()
        .unwrap_or(0);
    let mut headers = table.headers.clone();
    headers.resize_with(column_count, Vec::new);
    let mut rows = table.rows.clone();
    for row in &mut rows {
        row.resize_with(column_count, Vec::new);
    }
    let mut alignments = table.alignments.clone();
    alignments.resize(column_count, TableAlignment::Left);
    alignments.truncate(column_count);
    (headers, rows, alignments)
}

fn preferred_column_widths(
    headers: &[FormattedTextInline],
    rows: &[Vec<FormattedTextInline>],
) -> Vec<u16> {
    (0..headers.len())
        .map(|column| {
            rows.iter()
                .filter_map(|row| row.get(column))
                .chain([&headers[column]])
                .map(inline_visible_width)
                .max()
                .unwrap_or(1)
                .try_into()
                .unwrap_or(u16::MAX)
        })
        .collect()
}

fn allocate_column_widths(preferred: &[u16], available: u16) -> Option<Vec<u16>> {
    if preferred.is_empty() {
        return Some(Vec::new());
    }
    let gaps = TABLE_COLUMN_GAP
        .saturating_mul(u16::try_from(preferred.len().saturating_sub(1)).unwrap_or(u16::MAX));
    let mut widths: Vec<u16> = preferred
        .iter()
        .map(|width| width.clamp(&MIN_TABLE_COLUMN_WIDTH, &TARGET_TABLE_COLUMN_WIDTH))
        .copied()
        .collect();
    let minimum = widths.iter().copied().fold(gaps, u16::saturating_add);
    if minimum > available {
        return None;
    }

    let mut remaining = available - minimum;
    while remaining > 0 {
        let mut grew = false;
        for (width, preferred) in widths.iter_mut().zip(preferred) {
            if *width < *preferred {
                *width += 1;
                remaining -= 1;
                grew = true;
                if remaining == 0 {
                    break;
                }
            }
        }
        if !grew {
            break;
        }
    }
    Some(widths)
}

fn table_row(
    cells: &[FormattedTextInline],
    alignments: &[TableAlignment],
    widths: &[u16],
    base: TuiStyle,
    palette: TuiMarkdownPalette,
) -> Box<dyn TuiElement> {
    let mut row = TuiFlex::row();
    for (index, ((cell, alignment), width)) in cells.iter().zip(alignments).zip(widths).enumerate()
    {
        if index > 0 {
            row.add_child(
                TuiFixedWidth::new(
                    TABLE_COLUMN_GAP,
                    TuiText::new(" │ ")
                        .with_style(palette.rule)
                        .truncate()
                        .finish(),
                )
                .finish(),
            );
        }
        let spans = aligned_cell_spans(cell, *alignment, *width, base, palette);
        row.add_child(TuiFixedWidth::new(*width, TuiText::from_spans(spans).finish()).finish());
    }
    row.finish()
}

fn aligned_cell_spans(
    cell: &FormattedTextInline,
    alignment: TableAlignment,
    width: u16,
    base: TuiStyle,
    palette: TuiMarkdownPalette,
) -> Vec<(String, TuiStyle)> {
    let content_width = inline_visible_width(cell);
    let available_padding = usize::from(width).saturating_sub(content_width);
    let leading_padding = match alignment {
        TableAlignment::Left => 0,
        TableAlignment::Center => available_padding / 2,
        TableAlignment::Right => available_padding,
    };
    let mut spans = Vec::new();
    if leading_padding > 0 {
        spans.push((" ".repeat(leading_padding), base));
    }
    spans.extend(inline_spans(cell, base, palette));
    spans
}

fn inline_visible_width(inline: &FormattedTextInline) -> usize {
    let mut width = inline
        .iter()
        .map(|fragment| UnicodeWidthStr::width(fragment.text.as_str()))
        .sum();
    let mut active_url: Option<(String, String)> = None;
    for fragment in inline {
        let fragment_url = match &fragment.styles.hyperlink {
            Some(Hyperlink::Url(url)) => Some(url.as_str()),
            Some(Hyperlink::Action(_)) | None => None,
        };
        if active_url.as_ref().map(|(url, _)| url.as_str()) != fragment_url {
            if let Some((url, display)) = active_url.take() {
                if url != display {
                    width += UnicodeWidthStr::width(format!(" ({url})").as_str());
                }
            }
            if let Some(url) = fragment_url {
                active_url = Some((url.to_owned(), String::new()));
            }
        }
        if let Some((_, display)) = &mut active_url {
            display.push_str(&fragment.text);
        }
    }
    if let Some((url, display)) = active_url {
        if url != display {
            width += UnicodeWidthStr::width(format!(" ({url})").as_str());
        }
    }
    width
}
