use warpui::elements::{Container, Flex, ParentElement};
use warpui::ui_components::components::{UiComponent, UiComponentStyles};
use warpui::ui_components::text::WrappableText;
use warpui::{AppContext, Element, Entity, ModelContext, SingletonEntity};

use crate::appearance::Appearance;
use crate::search::command_palette::mixer::CommandPaletteItemAction;
use crate::search::result_renderer::QueryResultRenderer;
use crate::search::search_bar::SelectionUpdate;

/// List of items shown within the zero state. "Recent" items are shown first followed by
/// "Suggested" items.
pub struct Items {
    recent: Vec<QueryResultRenderer<CommandPaletteItemAction>>,
    suggested: Vec<QueryResultRenderer<CommandPaletteItemAction>>,
    selected_index: Option<SelectedIndex>,
}

/// Current selected index within the list of zero state items.
#[derive(Copy, Clone, Debug, PartialEq)]
enum SelectedIndex {
    Recent(usize),
    Suggested(usize),
}

impl Items {
    pub fn new() -> Self {
        Self {
            recent: vec![],
            suggested: vec![],
            selected_index: None,
        }
    }

    /// Renders title text for a section of the zero state.
    fn render_section_text(
        header_text: impl Into<String>,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        Container::new(
            WrappableText::build(
                appearance
                    .ui_builder()
                    .wrappable_text(header_text.into(), false)
                    .with_style(UiComponentStyles {
                        font_size: Some(12.),
                        ..Default::default()
                    }),
            )
            .finish(),
        )
        .with_vertical_padding(styles::ZERO_STATE_SECTION_PADDING)
        .finish()
    }

    fn render_query_result(
        query_result: &QueryResultRenderer<CommandPaletteItemAction>,
        index: usize,
        is_selected: bool,
        app: &AppContext,
    ) -> Box<dyn Element> {
        Container::new(query_result.render(index, is_selected, app))
            .with_horizontal_padding(-super::styles::PADDING_HORIZONTAL)
            .finish()
    }

    /// Sets the recent items in the zero state to that of `recent`.
    pub fn set_recent_items(
        &mut self,
        recent: Vec<QueryResultRenderer<CommandPaletteItemAction>>,
        ctx: &mut ModelContext<Self>,
    ) {
        self.recent = recent;
        ctx.notify();
    }

    /// Sets the suggested items in the zero state to that of `suggested`.
    pub fn set_suggested_items(
        &mut self,
        suggested: Vec<QueryResultRenderer<CommandPaletteItemAction>>,
        ctx: &mut ModelContext<Self>,
    ) {
        self.suggested = suggested;
        ctx.notify();
    }

    /// Returns the current selected item. `None` if no item is selected.
    pub fn selected_item(&self) -> Option<&QueryResultRenderer<CommandPaletteItemAction>> {
        let selected_item = self.selected_index?;
        match selected_item {
            SelectedIndex::Recent(index) => self.recent.get(index),
            SelectedIndex::Suggested(index) => self.suggested.get(index),
        }
    }

    /// Returns an iterator of all of the [`SelectedIndex`]s in the order they would appear.
    fn all_indices(&self) -> impl Iterator<Item = SelectedIndex> + '_ {
        self.recent
            .iter()
            .enumerate()
            .map(|(idx, _)| SelectedIndex::Recent(idx))
            .chain(
                self.suggested
                    .iter()
                    .enumerate()
                    .map(|(idx, _)| SelectedIndex::Suggested(idx)),
            )
    }

    /// Returns the current [`SelectedIndex`] as a total index across both recent and suggested
    /// items.
    fn total_index(&self) -> Option<usize> {
        self.selected_index
            .map(|selected_index| match selected_index {
                SelectedIndex::Recent(index) => index,
                SelectedIndex::Suggested(index) => self.recent.len() + index,
            })
    }

    /// Returns the next [`SelectedIndex`]. `None` if the next selected index would exceed all of
    /// the items in the list.
    fn next_selected_index(&self) -> Option<SelectedIndex> {
        match self.total_index() {
            None => self.all_indices().next(),
            Some(index) => self.all_indices().nth(index + 1),
        }
    }

    /// Returns the previous [`SelectedIndex`]. `None` if the selected item would exceed the first
    /// item in the list.
    fn prev_selected_index(&self) -> Option<SelectedIndex> {
        match self.total_index() {
            None => None,
            Some(0) => None,
            // We don't use `saturating_sub` because you don't wanna be stuck on 0.
            Some(index) => self.all_indices().nth(index - 1),
        }
    }

    pub fn handle_selection_update(
        &mut self,
        selection_update: SelectionUpdate,
        ctx: &mut ModelContext<Self>,
    ) {
        match selection_update {
            SelectionUpdate::Up => {
                self.selected_index = self.prev_selected_index();
                ctx.notify();
            }
            SelectionUpdate::Down => {
                // Only update the selected item if not `None` to prevent unsetting the selected
                // item if the user presses down when the last item is selected.
                if let Some(next_index) = self.next_selected_index() {
                    self.selected_index = Some(next_index);
                    ctx.notify();
                }
            }
            SelectionUpdate::Clear => {
                self.selected_index = None;
                ctx.notify();
            }
            // We don't want an item selected by default in the zero state, so noop here.
            SelectionUpdate::Bottom | SelectionUpdate::Top => {}
        }
    }

    pub fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);

        let mut flex = Flex::column();

        if !self.recent.is_empty() {
            flex.add_child(Self::render_section_text("Recent", appearance));

            flex.add_children(self.recent.iter().enumerate().map(|(idx, result)| {
                Self::render_query_result(
                    result,
                    idx,
                    Some(SelectedIndex::Recent(idx)) == self.selected_index,
                    app,
                )
            }));
        }

        if !self.suggested.is_empty() {
            flex.add_child(Self::render_section_text("Suggested", appearance));

            flex.add_children(self.suggested.iter().enumerate().map(|(idx, result)| {
                Self::render_query_result(
                    result,
                    idx,
                    Some(SelectedIndex::Suggested(idx)) == self.selected_index,
                    app,
                )
            }));
        }
        flex.finish()
    }
}

impl Entity for Items {
    type Event = ();
}

mod styles {
    pub const ZERO_STATE_SECTION_PADDING: f32 = 8.;
}
