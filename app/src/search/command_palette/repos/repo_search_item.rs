use std::path::Path;

use ai::workspace::WorkspaceMetadata;
use fuzzy_match::FuzzyMatchResult;
use ordered_float::OrderedFloat;
use warp_core::ui::theme::Fill;
use warpui::elements::{Align, ConstrainedBox, Flex, Highlight, ParentElement, Shrinkable, Text};
use warpui::fonts::{Properties, Weight};
use warpui::{AppContext, Element, SingletonEntity};

use crate::appearance::Appearance;
use crate::search::action::search_item::styles;
use crate::search::command_palette::mixer::CommandPaletteItemAction;
use crate::search::command_palette::render_util;
use crate::search::item::SearchItem;
use crate::search::result_renderer::ItemHighlightState;
use crate::ui_components::icons::Icon as UiIcon;

#[derive(Debug)]
pub struct RepoSearchItem {
    pub display_name: String,
    pub metadata: WorkspaceMetadata,
    pub match_result: FuzzyMatchResult,
}

fn repo_display_name(repo_path: &Path) -> String {
    // Try to create a relative path from the user's home directory
    dirs::home_dir()
        .and_then(|home| repo_path.strip_prefix(&home).ok())
        .map(|relative_path| format!("~/{}", relative_path.display()))
        .unwrap_or_else(|| repo_path.display().to_string())
}

impl RepoSearchItem {
    pub fn new(metadata: WorkspaceMetadata) -> Self {
        RepoSearchItem {
            display_name: repo_display_name(&metadata.path),
            metadata,
            match_result: FuzzyMatchResult::no_match(),
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

    fn render_label(
        &self,
        item_highlight_state: ItemHighlightState,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        Text::new_inline(
            repo_display_name(&self.metadata.path),
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

impl SearchItem for RepoSearchItem {
    type Action = CommandPaletteItemAction;

    fn render_icon(
        &self,
        highlight_state: ItemHighlightState,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let icon_color: Fill = appearance.theme().terminal_colors().normal.cyan.into();

        render_util::render_search_item_icon(
            appearance,
            UiIcon::Folder,
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
        None
    }

    fn score(&self) -> OrderedFloat<f64> {
        OrderedFloat(self.match_result.score as f64)
    }

    fn accept_result(&self) -> CommandPaletteItemAction {
        // Convert the absolute repo path into parent + basename for OpenDirectory
        let repo_path: &Path = &self.metadata.path;
        let parent = repo_path.parent().unwrap_or(Path::new("/"));
        let basename = repo_path
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| repo_path.to_string_lossy().to_string());

        CommandPaletteItemAction::OpenDirectory {
            path: basename,
            project_directory: parent.to_string_lossy().to_string(),
        }
    }

    fn execute_result(&self) -> CommandPaletteItemAction {
        // For projects, execute and accept have the same behavior
        self.accept_result()
    }

    fn accessibility_label(&self) -> String {
        format!("Repo: {}", self.metadata.path.display())
    }
}
