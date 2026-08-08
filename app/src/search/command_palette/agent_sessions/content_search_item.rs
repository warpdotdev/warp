use ordered_float::OrderedFloat;
use warp_core::ui::icons::Icon;
use warpui::elements::{
    Container, CrossAxisAlignment, Expanded, Flex, Highlight, MainAxisSize, ParentElement, Text,
};
use warpui::fonts::{Properties, Weight};
use warpui::{AppContext, Element, SingletonEntity};

use crate::appearance::Appearance;
use crate::search::SearchItem;
use crate::search::command_palette::agent_sessions::tiers::CONTENT_ROW_TIER;
use crate::search::command_palette::mixer::CommandPaletteItemAction;
use crate::search::command_palette::render_util::render_search_item_icon;
use crate::search::item::IconLocation;
use crate::search::result_renderer::ItemHighlightState;
use crate::terminal::cli_agent_sessions::transcript_digest::ContentHit;

/// Marks a hit found in a transcript that was too large to digest whole.
const PARTIAL_SCAN_LABEL: &str = "partial";

/// One session whose *transcript* contains the query.
///
/// Renders like a name row deliberately — same two-line shape, same icon — so
/// the two sections read as one list. The difference is the second line: a name
/// row shows where the session lives, this shows the sentence that matched,
/// because that sentence is the only reason this row is on screen.
#[derive(Debug)]
pub struct ContentSearchItem {
    hit: ContentHit,
    /// Rank within the published hits, in `(0, 1]`. See [`Self::score`].
    recency_bonus: f64,
}

impl ContentSearchItem {
    pub fn new(hit: ContentHit, recency_bonus: f64) -> Self {
        Self { hit, recency_bonus }
    }

    pub fn hit(&self) -> &ContentHit {
        &self.hit
    }
}

impl SearchItem for ContentSearchItem {
    type Action = CommandPaletteItemAction;

    fn is_multiline(&self) -> bool {
        true
    }

    fn render_icon(
        &self,
        highlight_state: ItemHighlightState,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let icon = self.hit.agent.icon().unwrap_or(Icon::Terminal);
        render_search_item_icon(
            appearance,
            icon,
            appearance.theme().foreground().into_solid(),
            highlight_state,
        )
    }

    fn icon_location(&self, appearance: &Appearance) -> IconLocation {
        // Centred on the first line rather than on the two-line row, exactly as
        // the name rows do it.
        let margin_top = (appearance.line_height_ratio() * appearance.monospace_font_size())
            - appearance.monospace_font_size();
        IconLocation::Top { margin_top }
    }

    fn render_item(
        &self,
        highlight_state: ItemHighlightState,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let sub_text_font_size = appearance.monospace_font_size() - 2.;

        let task_name_element = Text::new_inline(
            self.hit.task_name.clone(),
            appearance.ui_font_family(),
            appearance.monospace_font_size(),
        )
        .with_color(highlight_state.sub_text_fill(appearance).into_solid())
        .with_style(Properties::default().weight(Weight::Bold));

        let mut snippet_element = Text::new_inline(
            self.hit.snippet.clone(),
            appearance.ui_font_family(),
            sub_text_font_size,
        )
        .with_color(highlight_state.sub_text_fill(appearance).into_solid());
        if !self.hit.snippet_match_indices.is_empty() {
            // The matched term is bolded rather than recoloured: the snippet is
            // already dim, and a second colour here would compete with the row's
            // own selected/unselected state.
            snippet_element = snippet_element.with_single_highlight(
                Highlight::new()
                    .with_properties(Properties::default().weight(Weight::Bold))
                    .with_foreground_color(highlight_state.main_text_fill(appearance).into_solid()),
                self.hit.snippet_match_indices.clone(),
            );
        }

        // The project sits where a name row puts its timestamp: a content hit
        // has no clock worth showing, and without the project two identically
        // named tasks in different repos are indistinguishable.
        //
        // `partial` is appended rather than hidden because it changes what the
        // *absence* of a row means: this transcript was too large to read
        // whole, so a session that does not appear may still contain the query.
        // A search that silently half-answers is worse than one that says so.
        let project_label = if self.hit.partial {
            format!("{} · {PARTIAL_SCAN_LABEL}", self.hit.project_name)
        } else {
            self.hit.project_name.clone()
        };
        let project_element = Container::new(
            Text::new_inline(
                project_label,
                appearance.ui_font_family(),
                sub_text_font_size,
            )
            .with_color(highlight_state.sub_text_fill(appearance).into_solid())
            .finish(),
        )
        .with_padding_left(8.)
        .finish();

        Flex::row()
            .with_child(
                Expanded::new(
                    1.0,
                    Flex::column()
                        .with_spacing(4.)
                        .with_child(task_name_element.finish())
                        .with_child(snippet_element.finish())
                        .with_cross_axis_alignment(CrossAxisAlignment::Start)
                        .finish(),
                )
                .finish(),
            )
            .with_child(project_element)
            .with_main_axis_size(MainAxisSize::Max)
            .finish()
    }

    fn priority_tier(&self) -> u8 {
        CONTENT_ROW_TIER
    }

    /// Ordering within the content section.
    ///
    /// Content hits have no fuzzy score to rank by — a literal substring either
    /// occurs or it does not — so the only meaningful order is the corpus
    /// order, which is the popup's recency ranking. Expressed as an explicit
    /// descending score rather than left to insertion order, because the mixer
    /// sorts and the search bar then reverses.
    fn score(&self) -> OrderedFloat<f64> {
        OrderedFloat(self.recency_bonus)
    }

    fn accept_result(&self) -> Self::Action {
        // The same action a name row produces: the workspace owns the
        // activate-that-tab versus open-a-new-one decision, and a session found
        // by its text resumes exactly like one found by its name.
        CommandPaletteItemAction::ResumeAgentSession {
            agent: self.hit.agent,
            session_id: self.hit.session_id.clone(),
        }
    }

    fn execute_result(&self) -> Self::Action {
        self.accept_result()
    }

    fn accessibility_label(&self) -> String {
        format!(
            "{} session with matching text: {}",
            self.hit.agent.display_name(),
            self.hit.task_name
        )
    }

    fn accessibility_help_message(&self) -> Option<String> {
        let scope = if self.hit.partial {
            " Only part of this conversation was searched."
        } else {
            ""
        };
        Some(format!(
            "Matched \"{}\".{scope} Press enter to resume \"{}\".",
            self.hit.snippet, self.hit.task_name
        ))
    }
}

#[cfg(test)]
#[path = "content_search_item_tests.rs"]
mod tests;
