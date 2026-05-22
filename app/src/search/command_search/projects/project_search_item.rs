use std::cmp::Ordering;
use std::path::PathBuf;

use chrono::NaiveDateTime;
use fuzzy_match::FuzzyMatchResult;
use ordered_float::OrderedFloat;
use warp_core::ui::theme::Fill;
use warpui::elements::{Align, ConstrainedBox, Flex, Highlight, ParentElement, Shrinkable, Text};
use warpui::fonts::{Properties, Weight};
use warpui::{AppContext, Element, SingletonEntity};

use crate::appearance::Appearance;
use crate::search::action::search_item::styles;
use crate::search::command_palette::mixer::CommandPaletteItemAction;
use crate::search::command_palette::render_util::render_search_item_icon;
use crate::search::item::SearchItem;
use crate::search::result_renderer::ItemHighlightState;
use crate::ui_components::icons::Icon as UiIcon;

/// Stores data needed to display a project search result item in Command Search.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectSearchItem {
    pub path: String,
    pub name: String,
    pub match_result: FuzzyMatchResult,
    pub last_used_at: NaiveDateTime,
    pub popularity_score: i32,
}

/// Mac and windows are insensitive, Linux probably IS sensitive
/// WARNING: Don't use this function for use cases dependent on the session, e.g. a remote session or WSL on Windows. It only considers this specific host.
pub fn os_probably_case_sensitive() -> bool {
    !(cfg!(target_os = "macos") || cfg!(target_family = "windows"))
}

/// Extracts a display name from a project path (returns relative path from home directory).
fn project_display_name(project_path: &str) -> String {
    let path = PathBuf::from(project_path);

    // Try to create a relative path from the user's home directory
    dirs::home_dir()
        .and_then(|home| path.strip_prefix(&home).ok())
        .map(|relative_path| format!("~/{}", relative_path.display()))
        .unwrap_or_else(|| project_path.to_string())
}

impl ProjectSearchItem {
    pub fn new(path: String, match_result: FuzzyMatchResult, last_used_at: NaiveDateTime) -> Self {
        let name = project_display_name(&path);
        Self {
            path,
            name,
            match_result,
            last_used_at,
            popularity_score: 0,
        }
    }

    fn render(
        &self,
        item_highlight_state: ItemHighlightState,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let label = self.render_label(item_highlight_state, appearance);
        let mut binding = Flex::row();

        binding.add_child(Shrinkable::new(1., Align::new(label).left().finish()).finish());

        ConstrainedBox::new(binding.finish())
            .with_height(styles::SEARCH_ITEM_HEIGHT)
            .finish()
    }

    // TODO(jparker): Lift out this (and other rendienring below) into a helper function used here
    // and in search/action/search_item.rs
    fn render_label(
        &self,
        item_highlight_state: ItemHighlightState,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        Text::new_inline(
            self.name.clone(),
            appearance.ui_font_family(),
            appearance.monospace_font_size(),
        )
        .with_color(item_highlight_state.sub_text_fill(appearance).into_solid())
        .with_style(Properties::default().weight(Weight::Bold))
        .with_single_highlight(
            Highlight::new()
                .with_properties(Properties::default().weight(Weight::Bold))
                .with_foreground_color(
                    item_highlight_state.main_text_fill(appearance).into_solid(),
                ),
            self.match_result.matched_indices.clone(),
        )
        .finish()
    }
}

impl SearchItem for ProjectSearchItem {
    type Action = CommandPaletteItemAction;

    fn render_icon(
        &self,
        highlight_state: ItemHighlightState,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let icon_color: Fill = appearance.theme().terminal_colors().normal.cyan.into();

        render_search_item_icon(
            appearance,
            UiIcon::NewConversation,
            icon_color.into_solid(),
            highlight_state,
        )
    }

    fn render_item(
        &self,
        highlight_state: ItemHighlightState,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        self.render(highlight_state, appearance)
    }

    fn render_details(&self, _ctx: &AppContext) -> Option<Box<dyn Element>> {
        // TODO: Implement project details rendering
        None
    }

    fn score(&self) -> OrderedFloat<f64> {
        OrderedFloat(self.match_result.score as f64)
    }

    fn accept_result(&self) -> CommandPaletteItemAction {
        CommandPaletteItemAction::NewConversationInProject {
            path: self.path.clone(),
            project_name: self.name.clone(),
        }
    }

    fn execute_result(&self) -> CommandPaletteItemAction {
        // For projects, execute and accept have the same behavior
        self.accept_result()
    }

    fn accessibility_label(&self) -> String {
        format!("Project: {}", self.name)
    }

    fn dedup_key(&self) -> Option<String> {
        if os_probably_case_sensitive() {
            Some(self.path.clone())
        } else {
            Some(self.path.to_lowercase())
        }
    }
}

impl PartialOrd for ProjectSearchItem {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for ProjectSearchItem {
    // 1. Fuzzy match, 2. most recently used, 3. highest score
    // When sorting ProjectSearchItems, the "Best" is the "Largest" - so reverse the order of a typical sort
    fn cmp(&self, other: &Self) -> Ordering {
        self.match_result
            .score
            .cmp(&other.match_result.score)
            .then_with(|| self.last_used_at.cmp(&other.last_used_at))
            .then_with(|| self.popularity_score.cmp(&other.popularity_score))
    }
}
