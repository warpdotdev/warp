//! Semantic Markdown presentation for the headless TUI.
//!
//! Parsing and streaming ownership stay with the shared Markdown and agent
//! models. This module only turns an already-parsed [`FormattedText`] into a
//! composable TUI element tree, with an optional hook for stateful code-block
//! child views.

use markdown_parser::{
    CodeBlockText, FormattedImage, FormattedTable, FormattedText, FormattedTextFragment,
    FormattedTextInline, FormattedTextLine, Hyperlink, TableAlignment,
};
use unicode_width::UnicodeWidthStr;
use warpui_core::elements::tui::{
    Modifier, TuiConstraint, TuiContainer, TuiElement, TuiFlex, TuiLayoutContext, TuiPaintContext,
    TuiPaintSurface, TuiParentElement, TuiPresentationContext, TuiScreenPoint, TuiScreenPosition,
    TuiSize, TuiStyle, TuiText,
};
use warpui_core::elements::CrossAxisAlignment;
use warpui_core::AppContext;

use crate::tui_builder::TuiUiBuilder;

const LIST_INDENT_COLUMNS: u16 = 2;
const TABLE_COLUMN_GAP: u16 = 3;
const MIN_TABLE_COLUMN_WIDTH: u16 = 3;
const TARGET_TABLE_COLUMN_WIDTH: u16 = 8;

/// Semantic styles used by the presentation layer. Callers can derive the
/// default palette from the active theme and override individual roles.
#[derive(Clone, Copy, Debug)]
pub(crate) struct TuiMarkdownPalette {
    pub body: TuiStyle,
    pub muted: TuiStyle,
    pub heading: TuiStyle,
    pub marker: TuiStyle,
    pub link: TuiStyle,
    pub inline_code: TuiStyle,
    pub rule: TuiStyle,
    pub code: TuiStyle,
    pub table_header: TuiStyle,
    pub fallback: TuiStyle,
}

impl TuiMarkdownPalette {
    pub(crate) fn from_builder(builder: &TuiUiBuilder) -> Self {
        let body = builder.primary_text_style();
        let muted = builder.muted_text_style();
        Self {
            body,
            muted,
            heading: body.add_modifier(Modifier::BOLD),
            marker: muted,
            link: builder
                .accent_text_style()
                .add_modifier(Modifier::UNDERLINED),
            inline_code: builder.accent_text_style(),
            rule: muted,
            code: body,
            table_header: body.add_modifier(Modifier::BOLD),
            fallback: muted.add_modifier(Modifier::ITALIC),
        }
    }
}

/// Specialized block rendering supplied by an owning view. A code hook can
/// embed a persistent editor-backed child; returning `None` uses the safe
/// lightweight fallback.
type TuiMarkdownCodeRenderer<'a> =
    dyn Fn(usize, &CodeBlockText) -> Option<Box<dyn TuiElement>> + 'a;
#[derive(Default)]
pub(crate) struct TuiMarkdownBlockHooks<'a> {
    pub render_code: Option<&'a TuiMarkdownCodeRenderer<'a>>,
}

/// Renders parsed Markdown without reparsing or owning any streaming state.
pub(crate) fn render_formatted_text(
    formatted: &FormattedText,
    palette: TuiMarkdownPalette,
    hooks: &TuiMarkdownBlockHooks<'_>,
) -> Box<dyn TuiElement> {
    let mut column = TuiFlex::column();
    let mut code_index = 0;
    for line in &formatted.lines {
        let element = match line {
            FormattedTextLine::Heading(header) => {
                inline_text(&header.text, palette.heading, palette)
            }
            FormattedTextLine::Line(inline) => inline_text(inline, palette.body, palette),
            FormattedTextLine::OrderedList(item) => {
                let marker = format!("{}. ", item.number.unwrap_or(1));
                list_item(
                    item.indented_text.indent_level,
                    marker,
                    &item.indented_text.text,
                    palette.body,
                    palette,
                )
            }
            FormattedTextLine::UnorderedList(item) => list_item(
                item.indent_level,
                "• ".to_owned(),
                &item.text,
                palette.body,
                palette,
            ),
            FormattedTextLine::TaskList(item) => {
                let body = if item.complete {
                    palette.body.add_modifier(Modifier::CROSSED_OUT)
                } else {
                    palette.body
                };
                list_item(
                    item.indent_level,
                    if item.complete { "[x] " } else { "[ ] " }.to_owned(),
                    &item.text,
                    body,
                    palette,
                )
            }
            FormattedTextLine::CodeBlock(code) => {
                let rendered = hooks
                    .render_code
                    .and_then(|render| render(code_index, code))
                    .unwrap_or_else(|| code_fallback(code, palette));
                code_index += 1;
                rendered
            }
            FormattedTextLine::Table(table) => render_formatted_table(table, palette),
            FormattedTextLine::Image(image) => image_fallback(image, palette),
            FormattedTextLine::Embedded(_) => TuiText::new("[Unsupported embedded content]")
                .with_style(palette.fallback)
                .finish(),
            FormattedTextLine::LineBreak => blank_row(),
            FormattedTextLine::HorizontalRule => TuiMarkdownRule::new(palette.rule).finish(),
        };
        column.add_child(element);
    }
    column.finish()
}

/// Renders a structured table independently so agent semantic table sections
/// can share the same responsive presentation.
pub(crate) fn render_formatted_table(
    table: &FormattedTable,
    palette: TuiMarkdownPalette,
) -> Box<dyn TuiElement> {
    TuiMarkdownTable::new(table.clone(), palette).finish()
}

fn inline_text(
    inline: &FormattedTextInline,
    base: TuiStyle,
    palette: TuiMarkdownPalette,
) -> Box<dyn TuiElement> {
    TuiText::from_spans(inline_spans(inline, base, palette)).finish()
}

fn list_item(
    indent_level: usize,
    marker: String,
    inline: &FormattedTextInline,
    body_style: TuiStyle,
    palette: TuiMarkdownPalette,
) -> Box<dyn TuiElement> {
    let marker_width = UnicodeWidthStr::width(marker.as_str());
    let row = TuiFlex::row()
        .child(
            TuiFixedWidth::new(
                marker_width.try_into().unwrap_or(u16::MAX),
                TuiText::new(marker)
                    .with_style(palette.marker)
                    .truncate()
                    .finish(),
            )
            .finish(),
        )
        .flex_child(inline_text(inline, body_style, palette))
        .finish();
    let indent = u16::try_from(indent_level)
        .unwrap_or(u16::MAX)
        .saturating_mul(LIST_INDENT_COLUMNS);
    TuiContainer::new(row).with_padding_left(indent).finish()
}

fn inline_spans(
    inline: &FormattedTextInline,
    base: TuiStyle,
    palette: TuiMarkdownPalette,
) -> Vec<(String, TuiStyle)> {
    let mut spans = Vec::new();
    let mut active_url: Option<(String, String)> = None;

    for fragment in inline {
        let fragment_url = match &fragment.styles.hyperlink {
            Some(Hyperlink::Url(url)) => Some(url.as_str()),
            Some(Hyperlink::Action(_)) | None => None,
        };
        if active_url.as_ref().map(|(url, _)| url.as_str()) != fragment_url {
            finish_link(&mut spans, active_url.take(), palette.link);
            if let Some(url) = fragment_url {
                active_url = Some((url.to_owned(), String::new()));
            }
        }
        if let Some((_, display)) = &mut active_url {
            display.push_str(&fragment.text);
        }

        push_span(
            &mut spans,
            fragment.text.clone(),
            fragment_style(fragment, base, palette),
        );
    }
    finish_link(&mut spans, active_url, palette.link);
    spans
}

fn finish_link(
    spans: &mut Vec<(String, TuiStyle)>,
    link: Option<(String, String)>,
    style: TuiStyle,
) {
    if let Some((url, display)) = link {
        if url != display {
            push_span(spans, format!(" ({url})"), style);
        }
    }
}

fn fragment_style(
    fragment: &FormattedTextFragment,
    base: TuiStyle,
    palette: TuiMarkdownPalette,
) -> TuiStyle {
    let mut style = base;
    if fragment
        .styles
        .weight
        .is_some_and(|weight| weight.is_at_least_bold())
    {
        style = style.add_modifier(Modifier::BOLD);
    }
    if fragment.styles.italic {
        style = style.add_modifier(Modifier::ITALIC);
    }
    if fragment.styles.underline {
        style = style.add_modifier(Modifier::UNDERLINED);
    }
    if fragment.styles.strikethrough {
        style = style.add_modifier(Modifier::CROSSED_OUT);
    }
    if fragment.styles.inline_code {
        style = style.patch(palette.inline_code);
    }
    if fragment.styles.hyperlink.is_some() {
        style = style.patch(palette.link);
    }
    style
}

fn push_span(spans: &mut Vec<(String, TuiStyle)>, text: String, style: TuiStyle) {
    if text.is_empty() {
        return;
    }
    if let Some((previous, previous_style)) = spans.last_mut() {
        if *previous_style == style {
            previous.push_str(&text);
            return;
        }
    }
    spans.push((text, style));
}

fn code_fallback(code: &CodeBlockText, palette: TuiMarkdownPalette) -> Box<dyn TuiElement> {
    let mut column = TuiFlex::column().with_cross_axis_alignment(CrossAxisAlignment::Stretch);
    if !code.lang.is_empty() {
        column.add_child(
            TuiText::new(code.lang.clone())
                .with_style(palette.muted)
                .truncate()
                .finish(),
        );
    }
    column.add_child(
        TuiText::new(code.code.clone())
            .with_style(palette.code)
            .finish(),
    );
    TuiContainer::new(column.finish())
        .with_border_style(palette.rule)
        .with_padding_x(1)
        .finish()
}

fn image_fallback(image: &FormattedImage, palette: TuiMarkdownPalette) -> Box<dyn TuiElement> {
    let label = if image.alt_text.is_empty() {
        "Image".to_owned()
    } else {
        format!("Image: {}", image.alt_text)
    };
    TuiText::from_spans([
        (label, palette.fallback),
        (format!(" ({})", image.source), palette.link),
    ])
    .finish()
}

fn blank_row() -> Box<dyn TuiElement> {
    TuiText::new(" ").truncate().finish()
}

/// A full-width horizontal rule whose measurement and paint use the same
/// terminal-cell width.
struct TuiMarkdownRule {
    style: TuiStyle,
    inner: TuiText,
}

impl TuiMarkdownRule {
    fn new(style: TuiStyle) -> Self {
        Self {
            style,
            inner: TuiText::new(""),
        }
    }
}

impl TuiElement for TuiMarkdownRule {
    fn layout(
        &mut self,
        constraint: TuiConstraint,
        ctx: &mut TuiLayoutContext,
        app: &AppContext,
    ) -> TuiSize {
        let width = constraint.constrain_width(constraint.max.width);
        self.inner = TuiText::new("─".repeat(usize::from(width)))
            .with_style(self.style)
            .truncate();
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

/// Forces a child to one exact width while preserving its natural wrapped
/// height. This is the table grid's cell primitive.
struct TuiFixedWidth {
    width: u16,
    child: Box<dyn TuiElement>,
}

impl TuiFixedWidth {
    fn new(width: u16, child: Box<dyn TuiElement>) -> Self {
        Self { width, child }
    }
}

impl TuiElement for TuiFixedWidth {
    fn layout(
        &mut self,
        constraint: TuiConstraint,
        ctx: &mut TuiLayoutContext,
        app: &AppContext,
    ) -> TuiSize {
        let width = self.width.min(constraint.max.width);
        let child_constraint = TuiConstraint::new(
            TuiSize::new(width, 0),
            TuiSize::new(width, constraint.max.height),
        );
        let child_size = self.child.layout(child_constraint, ctx, app);
        TuiSize::new(width, child_size.height)
    }

    fn render(
        &mut self,
        origin: TuiScreenPosition,
        surface: &mut TuiPaintSurface<'_>,
        ctx: &mut TuiPaintContext,
    ) {
        self.child.render(origin, surface, ctx);
    }

    fn size(&self) -> Option<TuiSize> {
        self.child.size()
    }

    fn origin(&self) -> Option<TuiScreenPoint> {
        self.child.origin()
    }

    fn present(&mut self, ctx: &mut TuiPresentationContext<'_>) {
        self.child.present(ctx);
    }
}

#[cfg(test)]
#[path = "tui_markdown_tests.rs"]
mod tests;
