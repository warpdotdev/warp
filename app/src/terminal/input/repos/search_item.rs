//! SearchItem implementation for repo menu items.

use std::path::{Path, PathBuf};

use fuzzy_match::FuzzyMatchResult;
use ordered_float::OrderedFloat;
use warp_core::ui::Icon;
use warp_core::ui::theme::{AnsiColorIdentifier, Fill};
use warpui::elements::{ConstrainedBox, Container, Highlight, Text};
use warpui::fonts::{Properties, Weight};
use warpui::text_layout::ClipConfig;
use warpui::{AppContext, Element, SingletonEntity};

use crate::appearance::Appearance;
use crate::search::SearchItem;
use crate::search::result_renderer::ItemHighlightState;
use crate::terminal::input::inline_menu::styles as inline_styles;
use crate::terminal::input::repos::AcceptRepo;
use crate::util::git::RepoGitSummary;

/// Search item for rendering a repo in the inline repo menu.
#[derive(Debug, Clone)]
pub(super) struct RepoSearchItem {
    pub path: PathBuf,
    pub display_name: String,
    git_summary: Option<RepoGitSummary>,
    name_match_result: Option<FuzzyMatchResult>,
}

impl RepoSearchItem {
    pub fn new(path: PathBuf, git_summary: Option<RepoGitSummary>) -> Self {
        let display_name = repo_display_name(&path);
        Self {
            path,
            display_name,
            git_summary,
            name_match_result: None,
        }
    }

    pub fn with_name_match_result(mut self, result: Option<FuzzyMatchResult>) -> Self {
        self.name_match_result = result;
        self
    }
}

fn repo_display_name(repo_path: &Path) -> String {
    dirs::home_dir()
        .and_then(|home| repo_path.strip_prefix(&home).ok().map(|p| p.to_path_buf()))
        .map(|relative_path| format!("~/{}", relative_path.display()))
        .unwrap_or_else(|| repo_path.display().to_string())
}

impl SearchItem for RepoSearchItem {
    type Action = AcceptRepo;

    fn render_icon(
        &self,
        _highlight_state: ItemHighlightState,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let icon_size = inline_styles::font_size(appearance);
        let icon_color = inline_styles::icon_color(appearance);

        let icon = ConstrainedBox::new(Icon::Folder.to_warpui_icon(icon_color).finish())
            .with_width(icon_size)
            .with_height(icon_size)
            .finish();

        Container::new(icon)
            .with_margin_right(inline_styles::ICON_MARGIN)
            .finish()
    }

    fn render_item(
        &self,
        _highlight_state: ItemHighlightState,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();

        let font_size = inline_styles::font_size(appearance);
        let background_color = inline_styles::menu_background_color(app);

        let primary_text_color = inline_styles::primary_text_color(theme, background_color.into());
        let secondary_text_color =
            inline_styles::secondary_text_color(theme, background_color.into());

        // Build primary text: "~/path/to/repo" with optional " branch +N -N" suffix.
        let branch_suffix = self
            .git_summary
            .as_ref()
            .map(|s| format!("  {}", s.branch.to_lowercase()))
            .unwrap_or_default();

        let stats_suffix = self
            .git_summary
            .as_ref()
            .and_then(|s| {
                if s.lines_added > 0 || s.lines_removed > 0 {
                    Some(format!(" +{} -{}", s.lines_added, s.lines_removed))
                } else {
                    None
                }
            })
            .unwrap_or_default();

        let name_len = self.display_name.len();
        let mut full_text =
            String::with_capacity(name_len + branch_suffix.len() + stats_suffix.len());
        full_text.push_str(&self.display_name);
        full_text.push_str(&branch_suffix);
        full_text.push_str(&stats_suffix);

        let mut name_text = Text::new_inline(full_text, appearance.ui_font_family(), font_size)
            .with_color(primary_text_color.into_solid())
            .with_clip(ClipConfig::ellipsis());

        // Highlight fuzzy match indices on the name portion.
        if let Some(match_result) = &self.name_match_result
            && !match_result.matched_indices.is_empty()
        {
            name_text = name_text.with_single_highlight(
                Highlight::new().with_properties(Properties::default().weight(Weight::Bold)),
                match_result.matched_indices.clone(),
            );
        }

        // Style branch suffix in secondary color.
        let branch_end = name_len + branch_suffix.len();
        if !branch_suffix.is_empty() {
            let branch_range: Vec<usize> = (name_len..branch_end).collect();
            name_text = name_text.with_single_highlight(
                Highlight::new().with_foreground_color(secondary_text_color.into_solid()),
                branch_range,
            );
        }

        // Inline diff stats colored green/red right after branch.
        if !stats_suffix.is_empty() {
            let terminal_colors = &theme.terminal_colors().normal;
            let add_color = AnsiColorIdentifier::Green
                .to_ansi_color(terminal_colors)
                .into();
            let remove_color = AnsiColorIdentifier::Red
                .to_ansi_color(terminal_colors)
                .into();

            // stats_suffix is " +N -N"; find the boundary between +N and -N.
            let stats_start = branch_end;
            let add_part_len = stats_suffix.find(" -").unwrap_or(stats_suffix.len());
            let add_range: Vec<usize> = (stats_start..(stats_start + add_part_len)).collect();
            let remove_range: Vec<usize> =
                ((stats_start + add_part_len)..(stats_start + stats_suffix.len())).collect();

            name_text = name_text
                .with_single_highlight(Highlight::new().with_foreground_color(add_color), add_range)
                .with_single_highlight(
                    Highlight::new().with_foreground_color(remove_color),
                    remove_range,
                );
        }

        name_text.finish()
    }

    fn item_background(
        &self,
        highlight_state: ItemHighlightState,
        appearance: &Appearance,
    ) -> Option<Fill> {
        inline_styles::item_background(highlight_state, appearance)
    }

    fn score(&self) -> OrderedFloat<f64> {
        self.name_match_result
            .as_ref()
            .map(|m| OrderedFloat(m.score as f64))
            .unwrap_or(OrderedFloat(f64::MIN))
    }

    fn accept_result(&self) -> AcceptRepo {
        AcceptRepo {
            path: self.path.clone(),
        }
    }

    fn execute_result(&self) -> AcceptRepo {
        self.accept_result()
    }

    fn accessibility_label(&self) -> String {
        format!("Indexed repository: {}", self.display_name)
    }
}
