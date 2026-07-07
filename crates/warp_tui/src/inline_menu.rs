//! Reusable active-menu routing and character-cell presentation for TUI inline menus.
use std::ops::Range;

use string_offset::CharOffset;
use warp::tui_export::AcceptSlashCommandOrSavedPrompt;
use warpui_core::elements::tui::{
    TuiConstrainedBox, TuiConstraint, TuiContainer, TuiElement, TuiEvent, TuiEventContext, TuiFlex,
    TuiLayoutContext, TuiPaintContext, TuiPaintSurface, TuiPresentationContext, TuiScreenPoint,
    TuiScreenPosition, TuiSize, TuiText,
};
use warpui_core::elements::CrossAxisAlignment;
use warpui_core::{AppContext, ModelAsRef, ModelHandle, UpdateModel};

use crate::slash_commands::TuiSlashCommandModel;
use crate::tui_builder::TuiUiBuilder;
use crate::tui_column_layout::{
    format_tui_first_column, tui_two_column_layout, TuiTwoColumnConstraints, TuiTwoColumnLayout,
};

const SLASH_COMMAND_COLUMN_CONSTRAINTS: TuiTwoColumnConstraints = TuiTwoColumnConstraints {
    preferred_first_columns: 29,
    minimum_first_columns: 8,
    minimum_second_columns: 12,
    preferred_maximum_second_columns: 21,
    gap_columns: 1,
};

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TuiInlineMenuRowStyle {
    Default,
    SlashCommand,
}

/// A presentation-only row in a TUI inline menu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TuiInlineMenuRow {
    pub(crate) title: String,
    pub(crate) description: Option<String>,
    pub(crate) is_selectable: bool,
    pub(crate) style: TuiInlineMenuRowStyle,
}

/// A presentation-only tab in a TUI inline-menu header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TuiInlineMenuTab {
    pub(crate) label: String,
    pub(crate) is_selected: bool,
}

/// Optional header metadata rendered above menu rows.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct TuiInlineMenuHeader {
    pub(crate) title: Option<String>,
    pub(crate) tabs: Vec<TuiInlineMenuTab>,
}

/// Empty-list presentation for an open inline menu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TuiInlineMenuStatus {
    Loading(String),
    Empty(String),
}

/// Render-friendly, domain-neutral state for a TUI inline menu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TuiInlineMenuSnapshot {
    pub(crate) header: Option<TuiInlineMenuHeader>,
    pub(crate) rows: Vec<TuiInlineMenuRow>,
    pub(crate) selected_index: Option<usize>,
    pub(crate) scroll_offset: usize,
    pub(crate) max_visible_rows: usize,
    pub(crate) status: Option<TuiInlineMenuStatus>,
}

/// Domain action produced by accepting the selected item in an active menu.
#[derive(Debug, Clone)]
pub(crate) enum TuiInlineMenuAccepted {
    SlashCommand(AcceptSlashCommandOrSavedPrompt),
}

/// The active TUI inline menu.
///
/// This is the input and placement boundary shared by menu kinds. Each variant
/// retains its own query state and accepted action, while all variants expose
/// the same navigation, dismissal, snapshot, and rendering behavior.
#[derive(Clone)]
pub(crate) enum TuiInlineMenu {
    SlashCommands(ModelHandle<TuiSlashCommandModel>),
}

impl TuiInlineMenu {
    pub(crate) fn is_open(&self, ctx: &AppContext) -> bool {
        match self {
            Self::SlashCommands(model) => model.as_ref(ctx).is_open(),
        }
    }

    pub(crate) fn render(&self, ctx: &AppContext) -> Option<Box<dyn TuiElement>> {
        self.snapshot(ctx)
            .map(|snapshot| render_inline_menu(&snapshot, &TuiUiBuilder::from_app(ctx)))
    }
    pub(crate) fn input_highlight_range(&self, ctx: &AppContext) -> Option<Range<CharOffset>> {
        match self {
            Self::SlashCommands(model) => model.as_ref(ctx).highlighted_prefix_range(),
        }
    }

    pub(crate) fn input_argument_hint_text(&self, ctx: &AppContext) -> Option<&'static str> {
        match self {
            Self::SlashCommands(model) => model.as_ref(ctx).argument_hint_text(),
        }
    }

    pub(crate) fn select_previous(&self, ctx: &mut impl UpdateModel) {
        match self {
            Self::SlashCommands(model) => {
                model.update(ctx, |model, ctx| model.select_previous(ctx));
            }
        }
    }

    pub(crate) fn select_next(&self, ctx: &mut impl UpdateModel) {
        match self {
            Self::SlashCommands(model) => {
                model.update(ctx, |model, ctx| model.select_next(ctx));
            }
        }
    }

    pub(crate) fn accept(
        &self,
        ctx: &mut (impl ModelAsRef + UpdateModel),
    ) -> Option<TuiInlineMenuAccepted> {
        match self {
            Self::SlashCommands(model) => model
                .update(ctx, |model, ctx| model.accept_selected(ctx))
                .map(TuiInlineMenuAccepted::SlashCommand),
        }
    }

    pub(crate) fn dismiss(&self, ctx: &mut impl UpdateModel) {
        match self {
            Self::SlashCommands(model) => {
                model.update(ctx, |model, ctx| model.dismiss(ctx));
            }
        }
    }

    fn snapshot(&self, ctx: &AppContext) -> Option<TuiInlineMenuSnapshot> {
        match self {
            Self::SlashCommands(model) => model.as_ref(ctx).snapshot(),
        }
    }
}

pub(crate) fn render_inline_menu(
    snapshot: &TuiInlineMenuSnapshot,
    builder: &TuiUiBuilder,
) -> Box<dyn TuiElement> {
    Box::new(TuiInlineMenuElement {
        snapshot: snapshot.clone(),
        builder: builder.clone(),
        content: None,
    })
}

struct TuiInlineMenuElement {
    snapshot: TuiInlineMenuSnapshot,
    builder: TuiUiBuilder,
    content: Option<Box<dyn TuiElement>>,
}

impl TuiElement for TuiInlineMenuElement {
    fn layout(
        &mut self,
        constraint: TuiConstraint,
        ctx: &mut TuiLayoutContext,
        app: &AppContext,
    ) -> TuiSize {
        let mut content = build_inline_menu(
            &self.snapshot,
            &self.builder,
            constraint.max.width,
            constraint.max.height,
        );
        let size = content.layout(constraint, ctx, app);
        self.content = Some(content);
        size
    }

    fn render(
        &mut self,
        origin: TuiScreenPosition,
        surface: &mut TuiPaintSurface<'_>,
        ctx: &mut TuiPaintContext,
    ) {
        if let Some(content) = self.content.as_mut() {
            content.render(origin, surface, ctx);
        }
    }

    /// Returns the laid-out content size.
    fn size(&self) -> Option<TuiSize> {
        self.content.as_ref()?.size()
    }

    /// Returns the painted content origin.
    fn origin(&self) -> Option<TuiScreenPoint> {
        self.content.as_ref()?.origin()
    }

    /// Delegates child-view presentation to the laid-out content.
    fn present(&mut self, ctx: &mut TuiPresentationContext<'_>) {
        if let Some(content) = self.content.as_mut() {
            content.present(ctx);
        }
    }

    /// Delegates event dispatch to the laid-out content.
    fn dispatch_event(
        &mut self,
        event: &TuiEvent,
        event_ctx: &mut TuiEventContext<'_>,
        app: &AppContext,
    ) -> bool {
        self.content
            .as_mut()
            .is_some_and(|content| content.dispatch_event(event, event_ctx, app))
    }
}

fn build_inline_menu(
    snapshot: &TuiInlineMenuSnapshot,
    builder: &TuiUiBuilder,
    allocated_width: u16,
    allocated_height: u16,
) -> Box<dyn TuiElement> {
    let slash_command_columns = tui_two_column_layout(
        usize::from(allocated_width),
        snapshot.rows.iter().filter_map(|row| {
            if row.style != TuiInlineMenuRowStyle::SlashCommand {
                return None;
            }
            Some((row.title.as_str(), row.description.as_deref()?))
        }),
        SLASH_COMMAND_COLUMN_CONSTRAINTS,
    );
    let mut column = TuiFlex::column().with_cross_axis_alignment(CrossAxisAlignment::Stretch);
    if let Some(header) = &snapshot.header {
        if let Some(title) = &header.title {
            column = column.child(menu_status_row(title, builder));
        }
        if !header.tabs.is_empty() {
            let labels = header
                .tabs
                .iter()
                .map(|tab| {
                    if tab.is_selected {
                        format!("[{}]", tab.label)
                    } else {
                        tab.label.clone()
                    }
                })
                .collect::<Vec<_>>()
                .join("  ");
            column = column.child(menu_status_row(&labels, builder));
        }
    }

    if snapshot.rows.is_empty() {
        if let Some(status) = &snapshot.status {
            let label = match status {
                TuiInlineMenuStatus::Loading(label) | TuiInlineMenuStatus::Empty(label) => label,
            };
            column = column.child(menu_status_row(label, builder));
        }
    } else {
        let visible_rows = visible_result_capacity(snapshot, allocated_height);
        let mut scroll_offset = snapshot.scroll_offset;
        if let Some(selected_index) = snapshot.selected_index {
            keep_selected_visible(
                snapshot.rows.len(),
                selected_index,
                visible_rows,
                &mut scroll_offset,
            );
        } else {
            scroll_offset = scroll_offset.min(snapshot.rows.len().saturating_sub(visible_rows));
        }
        for (index, row) in snapshot
            .rows
            .iter()
            .enumerate()
            .skip(scroll_offset)
            .take(visible_rows)
        {
            column = column.child(menu_result_row(
                row,
                snapshot.selected_index == Some(index),
                slash_command_columns,
                builder,
            ));
        }
    }

    column.finish()
}

fn visible_result_capacity(snapshot: &TuiInlineMenuSnapshot, allocated_height: u16) -> usize {
    let header_rows = snapshot.header.as_ref().map_or(0, |header| {
        usize::from(header.title.is_some()) + usize::from(!header.tabs.is_empty())
    });
    usize::from(allocated_height)
        .saturating_sub(header_rows)
        .min(snapshot.max_visible_rows)
}

/// Clamps stale scroll offsets and moves the viewport only as far as needed to
/// keep the selected row within a window of `visible_rows`.
pub(crate) fn keep_selected_visible(
    rows_len: usize,
    selected_index: usize,
    visible_rows: usize,
    scroll_offset: &mut usize,
) {
    if rows_len == 0 || visible_rows == 0 {
        *scroll_offset = 0;
        return;
    }

    let max_scroll_offset = rows_len.saturating_sub(visible_rows);
    *scroll_offset = (*scroll_offset).min(max_scroll_offset);
    if selected_index < *scroll_offset {
        *scroll_offset = selected_index;
    } else if selected_index >= *scroll_offset + visible_rows {
        *scroll_offset = selected_index + 1 - visible_rows;
    }
}

fn menu_status_row(label: &str, builder: &TuiUiBuilder) -> Box<dyn TuiElement> {
    TuiContainer::new(
        TuiText::new(label.to_owned())
            .with_style(builder.dim_text_style())
            .truncate()
            .finish(),
    )
    .with_padding_left(1)
    .with_padding_right(1)
    .finish()
}

fn menu_result_row(
    row: &TuiInlineMenuRow,
    is_selected: bool,
    slash_command_columns: TuiTwoColumnLayout,
    builder: &TuiUiBuilder,
) -> Box<dyn TuiElement> {
    let title_style = if is_selected {
        builder.slash_command_selection_text_style()
    } else {
        match (row.is_selectable, row.style) {
            (true, TuiInlineMenuRowStyle::SlashCommand) => builder.slash_command_text_style(),
            (true, TuiInlineMenuRowStyle::Default) => builder.primary_text_style(),
            (false, TuiInlineMenuRowStyle::Default | TuiInlineMenuRowStyle::SlashCommand) => {
                builder.dim_text_style()
            }
        }
    };
    let show_description = match row.style {
        TuiInlineMenuRowStyle::Default => row.description.is_some(),
        TuiInlineMenuRowStyle::SlashCommand => {
            slash_command_columns.show_second && row.description.is_some()
        }
    };
    let title_columns = if show_description {
        slash_command_columns.first_columns
    } else {
        slash_command_columns.available_columns
    };
    let title = match row.style {
        TuiInlineMenuRowStyle::Default => row.title.clone(),
        TuiInlineMenuRowStyle::SlashCommand => format_tui_first_column(
            &row.title,
            slash_command_columns.with_second_visible(show_description),
        ),
    };
    let title = TuiText::new(title)
        .with_style(title_style)
        .truncate()
        .finish();
    let description_style = if is_selected {
        builder.slash_command_selection_text_style()
    } else {
        match row.style {
            TuiInlineMenuRowStyle::Default => builder.muted_text_style(),
            TuiInlineMenuRowStyle::SlashCommand => builder.primary_text_style(),
        }
    };

    let mut content = TuiFlex::row()
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .child(match row.style {
            TuiInlineMenuRowStyle::Default => title,
            TuiInlineMenuRowStyle::SlashCommand => TuiConstrainedBox::new(title)
                .with_max_cols(
                    u16::try_from(title_columns)
                        .expect("title columns come from the u16 width constraint"),
                )
                .finish(),
        });
    if let Some(description) = row.description.as_ref().filter(|_| show_description) {
        let description = match row.style {
            TuiInlineMenuRowStyle::Default => format!("  {description}"),
            TuiInlineMenuRowStyle::SlashCommand => description.clone(),
        };
        content = content.child(
            TuiText::new(description)
                .with_style(description_style)
                .truncate()
                .finish(),
        );
    }
    let mut container = TuiContainer::new(content.finish());
    if is_selected {
        container = container.with_background(builder.slash_command_selection_background());
    }
    container.finish()
}

#[cfg(test)]
#[path = "inline_menu_tests.rs"]
mod tests;
