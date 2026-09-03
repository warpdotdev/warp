use warp_core::ui::theme::color::internal_colors;
use warpui::elements::{
    Container, CrossAxisAlignment, Expanded, Flex, MainAxisSize, ParentElement, Text,
};
use warpui::fonts::{Properties, Weight};
use warpui::{AppContext, Element, Entity, SingletonEntity, View, ViewContext};

use super::rich_content::{RichContentInsertionPosition, RichContentMetadata};
use crate::appearance::Appearance;
use crate::terminal::TerminalView;
use crate::terminal::model::ansi::{
    PowerShellTableBeginValue, PowerShellTableColumn, PowerShellTableEndValue,
    PowerShellTableRowsValue,
};
use crate::terminal::model::rich_content::RichContentType;
use crate::terminal::model::terminal_model::BlockIndex;

const MAX_COLUMNS: usize = 64;
const MAX_ROWS_PER_TABLE: usize = 10_000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct PowerShellTableData {
    pub table_id: String,
    pub columns: Vec<PowerShellTableColumn>,
    pub rows: Vec<Vec<String>>,
}

impl PowerShellTableData {
    fn from_begin(value: PowerShellTableBeginValue) -> Option<Self> {
        if value.table_id.is_empty()
            || value.columns.is_empty()
            || value.columns.len() > MAX_COLUMNS
        {
            return None;
        }
        Some(Self {
            table_id: value.table_id,
            columns: value.columns,
            rows: Vec::new(),
        })
    }

    fn push_rows(&mut self, rows: &[Vec<String>]) {
        let remaining = MAX_ROWS_PER_TABLE.saturating_sub(self.rows.len());
        self.rows.extend(rows.iter().take(remaining).map(|row| {
            (0..self.columns.len())
                .map(|index| row.get(index).cloned().unwrap_or_default())
                .collect()
        }));
    }
}

#[derive(Default)]
pub(super) struct PowerShellTableStream {
    current: Option<PowerShellTableData>,
    pending: Vec<PowerShellTableData>,
}

impl PowerShellTableStream {
    pub fn begin(&mut self, value: PowerShellTableBeginValue) {
        if let Some(previous) = self.take_current() {
            self.pending.push(previous);
        }
        self.current = PowerShellTableData::from_begin(value);
    }

    pub fn rows(&mut self, value: &PowerShellTableRowsValue) {
        if let Some(table) = self.current.as_mut()
            && table.table_id == value.table_id
        {
            table.push_rows(&value.rows);
        }
    }

    pub fn end(&mut self, value: &PowerShellTableEndValue) -> Vec<PowerShellTableData> {
        if self
            .current
            .as_ref()
            .is_some_and(|table| table.table_id == value.table_id)
        {
            if let Some(current) = self.take_current() {
                self.pending.push(current);
            }
            return std::mem::take(&mut self.pending);
        }
        Vec::new()
    }

    pub fn finish_command(&mut self) -> Vec<PowerShellTableData> {
        if let Some(current) = self.take_current() {
            self.pending.push(current);
        }
        std::mem::take(&mut self.pending)
    }

    pub fn clear(&mut self) {
        self.current = None;
        self.pending.clear();
    }

    fn take_current(&mut self) -> Option<PowerShellTableData> {
        self.current.take().filter(|table| !table.rows.is_empty())
    }
}

pub(super) struct PowerShellRichTable {
    data: PowerShellTableData,
}

impl PowerShellRichTable {
    pub fn new(data: PowerShellTableData) -> Self {
        Self { data }
    }
}
impl Entity for PowerShellRichTable {
    type Event = ();
}

impl View for PowerShellRichTable {
    fn ui_name() -> &'static str {
        "PowerShellRichTable"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        let font_size = appearance.monospace_font_size();
        let font_family = appearance.monospace_font_family();
        let foreground = theme.foreground().into_solid();
        let muted = internal_colors::neutral_5(theme);

        let mut header = Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch);
        for column in &self.data.columns {
            let text = Flex::column()
                .with_main_axis_size(MainAxisSize::Min)
                .with_child(
                    Text::new(column.name.clone(), font_family, font_size)
                        .with_color(foreground)
                        .with_style(Properties::default().weight(Weight::Bold))
                        .finish(),
                )
                .with_child(
                    Text::new(column.type_name.clone(), font_family, font_size - 2.)
                        .soft_wrap(true)
                        .with_color(muted)
                        .finish(),
                )
                .finish();
            header = header.with_child(
                Expanded::new(
                    1.,
                    Container::new(text)
                        .with_horizontal_padding(8.)
                        .with_vertical_padding(6.)
                        .finish(),
                )
                .finish(),
            );
        }

        let mut table = Flex::column()
            .with_main_axis_size(MainAxisSize::Min)
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_child(
                Container::new(header.finish())
                    .with_background(internal_colors::fg_overlay_2(theme))
                    .finish(),
            );

        for (row_index, row) in self.data.rows.iter().enumerate() {
            let mut row_view = Flex::row()
                .with_main_axis_size(MainAxisSize::Max)
                .with_cross_axis_alignment(CrossAxisAlignment::Stretch);
            for value in row {
                row_view = row_view.with_child(
                    Expanded::new(
                        1.,
                        Container::new(
                            Text::new(value.clone(), font_family, font_size)
                                .soft_wrap(true)
                                .with_color(foreground)
                                .finish(),
                        )
                        .with_horizontal_padding(8.)
                        .with_vertical_padding(5.)
                        .finish(),
                    )
                    .finish(),
                );
            }
            let mut container = Container::new(row_view.finish());
            if row_index % 2 == 1 {
                container = container.with_background(internal_colors::fg_overlay_1(theme));
            }
            table = table.with_child(container.finish());
        }

        Container::new(table.finish())
            .with_uniform_margin(8.)
            .finish()
    }
}

impl TerminalView {
    pub(super) fn insert_powershell_table(
        &mut self,
        table: PowerShellTableData,
        insert_before_block_index: Option<BlockIndex>,
        ctx: &mut ViewContext<Self>,
    ) {
        let view = ctx.add_view(|_| PowerShellRichTable::new(table));
        let position = match insert_before_block_index {
            Some(block_index) => RichContentInsertionPosition::BeforeBlockIndex(block_index),
            None => RichContentInsertionPosition::Append {
                insert_below_long_running_block: false,
            },
        };
        self.insert_rich_content(
            Some(RichContentType::PowerShellTable),
            view,
            Some(RichContentMetadata::PowerShellTable),
            position,
            ctx,
        );
    }

    pub(super) fn flush_powershell_tables(&mut self, ctx: &mut ViewContext<Self>) {
        for table in self.powershell_table_stream.finish_command() {
            self.insert_powershell_table(table, None, ctx);
        }
    }
}

#[cfg(test)]
#[path = "powershell_rich_table_tests.rs"]
mod tests;
