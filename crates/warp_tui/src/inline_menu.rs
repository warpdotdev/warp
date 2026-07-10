//! Reusable active-menu routing and character-cell presentation for TUI inline menus.

use warp::tui_export::AcceptSlashCommandOrSavedPrompt;
use warpui_core::elements::tui::{
    TuiBuffer, TuiConstraint, TuiContainer, TuiElement, TuiFlex, TuiLayoutContext, TuiPaintContext,
    TuiRect, TuiSize, TuiText,
};
use warpui_core::elements::CrossAxisAlignment;
use warpui_core::{AppContext, ModelAsRef, ModelHandle, UpdateModel};

use crate::slash_commands::TuiSlashCommandModel;
use crate::tui_builder::TuiUiBuilder;

/// A presentation-only row in a TUI inline menu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TuiInlineMenuRow {
    pub(crate) title: String,
    pub(crate) description: Option<String>,
    pub(crate) is_selectable: bool,
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
            Self::SlashCommands(model) => {
                let action = model.as_ref(ctx).selected_action();
                model.update(ctx, |model, ctx| model.dismiss(ctx));
                action.map(TuiInlineMenuAccepted::SlashCommand)
            }
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
        let mut content = build_inline_menu(&self.snapshot, &self.builder, constraint.max.height);
        let size = content.layout(constraint, ctx, app);
        self.content = Some(content);
        size
    }

    fn render(&self, area: TuiRect, buffer: &mut TuiBuffer, ctx: &mut TuiPaintContext) {
        if let Some(content) = &self.content {
            content.render(area, buffer, ctx);
        }
    }
}

fn build_inline_menu(
    snapshot: &TuiInlineMenuSnapshot,
    builder: &TuiUiBuilder,
    allocated_height: u16,
) -> Box<dyn TuiElement> {
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
                builder,
            ));
        }
    }

    TuiContainer::new(column.finish())
        .with_border_style(builder.accent_border_style())
        .finish()
}

fn visible_result_capacity(snapshot: &TuiInlineMenuSnapshot, allocated_height: u16) -> usize {
    const BORDER_ROWS: usize = 2;
    let header_rows = snapshot.header.as_ref().map_or(0, |header| {
        usize::from(header.title.is_some()) + usize::from(!header.tabs.is_empty())
    });
    usize::from(allocated_height)
        .saturating_sub(BORDER_ROWS + header_rows)
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
    builder: &TuiUiBuilder,
) -> Box<dyn TuiElement> {
    let title_style = if is_selected {
        builder.input_text_style()
    } else if row.is_selectable {
        builder.primary_text_style()
    } else {
        builder.dim_text_style()
    };
    let description_style = if is_selected {
        builder.input_text_style()
    } else {
        builder.muted_text_style()
    };

    let mut content = TuiFlex::row()
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .child(
            TuiText::new(row.title.clone())
                .with_style(title_style)
                .truncate()
                .finish(),
        );
    if let Some(description) = &row.description {
        content = content.child(
            TuiText::new(format!("  {description}"))
                .with_style(description_style)
                .truncate()
                .finish(),
        );
    }

    let mut container = TuiContainer::new(content.finish())
        .with_padding_left(1)
        .with_padding_right(1);
    if is_selected {
        container = container.with_background(builder.input_background());
    }
    container.finish()
}

#[cfg(test)]
#[path = "inline_menu_tests.rs"]
mod tests;
