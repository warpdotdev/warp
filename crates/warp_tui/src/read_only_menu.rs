//! Shared presentation and lifecycle types for stateless read-only menus.

use warpui_core::AppContext;
use warpui_core::elements::CrossAxisAlignment;
use warpui_core::elements::tui::{
    Color, Modifier, TuiConstrainedBox, TuiContainer, TuiElement, TuiEventContext, TuiFlex,
    TuiLayoutContext, TuiScrollable, TuiScrollableElement, TuiSelectable, TuiSelectionHandle,
    TuiStyle, TuiText, TuiViewportContent, TuiViewportWindow, TuiViewportedElement,
    TuiViewportedList, TuiViewportedListState, TuiVisibleViewportItem,
};

use crate::tui_builder::TuiUiBuilder;

/// The read-only menu currently projected above the input.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TuiReadOnlyMenuKind {
    Shortcuts,
    Status,
    Todos,
    Usage,
}
/// One styled text cell in a read-only menu row.
#[derive(Clone)]
pub(crate) struct TuiReadOnlyMenuText {
    spans: Vec<(String, TuiStyle)>,
}

impl TuiReadOnlyMenuText {
    pub(crate) fn new(spans: impl IntoIterator<Item = (String, TuiStyle)>) -> Self {
        Self {
            spans: spans.into_iter().collect(),
        }
    }

    pub(crate) fn empty() -> Self {
        Self { spans: Vec::new() }
    }

    fn render(&self) -> Box<dyn TuiElement> {
        TuiText::from_spans(self.spans.clone()).truncate().finish()
    }
}

/// One read-only menu row split into evenly sized text columns.
#[derive(Clone)]
pub(crate) struct TuiReadOnlyMenuRow {
    columns: Vec<TuiReadOnlyMenuText>,
}

impl TuiReadOnlyMenuRow {
    pub(crate) fn new(columns: impl IntoIterator<Item = TuiReadOnlyMenuText>) -> Self {
        Self {
            columns: columns.into_iter().collect(),
        }
    }

    fn render(&self) -> Box<dyn TuiElement> {
        let mut row = TuiFlex::row();
        for column in &self.columns {
            row = row.flex_child(column.render());
        }
        row.finish()
    }
}

/// One titled section in a [`TuiReadOnlyMenu`].
pub(crate) struct TuiReadOnlyMenuSection {
    title: String,
    rows: Vec<TuiReadOnlyMenuRow>,
}

impl TuiReadOnlyMenuSection {
    pub(crate) fn new(title: impl Into<String>, rows: Vec<TuiReadOnlyMenuRow>) -> Self {
        Self {
            title: title.into(),
            rows,
        }
    }
}
#[derive(Clone)]
enum TuiReadOnlyMenuVisualRow {
    SectionTitle { title: String, style: TuiStyle },
    Content(TuiReadOnlyMenuRow),
    Spacer,
}

impl TuiReadOnlyMenuVisualRow {
    fn render(&self, background: Color) -> Box<dyn TuiElement> {
        let content = match self {
            Self::SectionTitle { title, style } => TuiText::new(title.clone())
                .with_style(*style)
                .truncate()
                .finish(),
            Self::Content(row) => row.render(),
            Self::Spacer => TuiText::new(" ").finish(),
        };
        TuiFlex::row()
            .flex_child(
                TuiContainer::new(content)
                    .with_background(background)
                    .finish(),
            )
            .finish()
    }
}

#[derive(Clone)]
struct TuiReadOnlyMenuContent {
    rows: Vec<TuiReadOnlyMenuVisualRow>,
    background: Color,
}

impl TuiReadOnlyMenuContent {
    fn viewport_content(&self, window: TuiViewportWindow) -> TuiViewportContent {
        let viewport_bottom = window
            .scroll_top
            .saturating_add(usize::from(window.viewport_height));
        let items = self
            .rows
            .iter()
            .enumerate()
            .filter(|(row, _)| *row >= window.scroll_top && *row < viewport_bottom)
            .map(|(origin_y, row)| TuiVisibleViewportItem {
                origin_y,
                element: row.render(self.background),
            })
            .collect();
        TuiViewportContent {
            content_height: self.rows.len(),
            items,
        }
    }
}

impl TuiViewportedElement for TuiReadOnlyMenuContent {
    fn visible_items(
        &self,
        window: TuiViewportWindow,
        _available_width: u16,
        _ctx: &mut TuiLayoutContext,
        _app: &AppContext,
    ) -> TuiViewportContent {
        self.viewport_content(window)
    }

    fn selection_content(
        &self,
        window: TuiViewportWindow,
        _available_width: u16,
        _app: &AppContext,
    ) -> Option<TuiViewportContent> {
        Some(self.viewport_content(window))
    }
}

/// Shared stateless component used by the shortcuts and status menus.
pub(crate) struct TuiReadOnlyMenu {
    sections: Vec<TuiReadOnlyMenuSection>,
}

impl TuiReadOnlyMenu {
    pub(crate) fn new(sections: Vec<TuiReadOnlyMenuSection>) -> Self {
        Self { sections }
    }

    #[cfg(test)]
    pub(crate) fn render(
        self,
        selection: TuiSelectionHandle,
        builder: &TuiUiBuilder,
        on_selection_start: impl FnMut(&mut TuiEventContext, &AppContext) + 'static,
        on_copy: impl FnMut(String, &mut TuiEventContext, &AppContext) + 'static,
    ) -> Box<dyn TuiElement> {
        let viewport_state = TuiViewportedListState::new_at_end();
        viewport_state.scroll_to_rows_from_top(0);
        self.render_with_viewport(
            selection,
            viewport_state,
            builder,
            on_selection_start,
            on_copy,
        )
    }

    pub(crate) fn render_with_viewport(
        self,
        selection: TuiSelectionHandle,
        viewport_state: TuiViewportedListState,
        builder: &TuiUiBuilder,
        on_selection_start: impl FnMut(&mut TuiEventContext, &AppContext) + 'static,
        on_copy: impl FnMut(String, &mut TuiEventContext, &AppContext) + 'static,
    ) -> Box<dyn TuiElement> {
        let section_title_style = builder.primary_text_style().add_modifier(Modifier::BOLD);
        let background = builder.read_only_menu_background();
        let mut rows = Vec::new();

        for (index, section) in self.sections.into_iter().enumerate() {
            if index > 0 {
                rows.push(TuiReadOnlyMenuVisualRow::Spacer);
            }
            rows.push(TuiReadOnlyMenuVisualRow::SectionTitle {
                title: section.title,
                style: section_title_style,
            });
            rows.extend(
                section
                    .rows
                    .into_iter()
                    .map(TuiReadOnlyMenuVisualRow::Content),
            );
        }
        let content_height = u16::try_from(rows.len()).unwrap_or(u16::MAX);
        let viewport = TuiViewportedList::new(
            viewport_state,
            TuiReadOnlyMenuContent { rows, background },
            builder.selection_style(),
        )
        .with_trimmed_selection_line_ends();
        let selectable = TuiSelectable::new(selection, viewport)
            .with_semantic_selection_by_style()
            .on_selection_start(on_selection_start)
            .on_copy(on_copy);
        let scrollable = TuiScrollable::new(selectable.finish_scrollable()).finish();
        let content = TuiFlex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .child(
                TuiConstrainedBox::new(scrollable)
                    .with_max_rows(content_height)
                    .finish(),
            )
            .finish();
        TuiContainer::new(content)
            .with_padding_x(1)
            .with_background(background)
            .finish()
    }
}

#[cfg(test)]
#[path = "read_only_menu_tests.rs"]
mod tests;
