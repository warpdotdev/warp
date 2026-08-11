use ordered_float::OrderedFloat;
use warp_core::ui::icons::Icon;
use warpui::elements::{
    Container, CrossAxisAlignment, Expanded, Flex, Highlight, MainAxisSize, ParentElement, Text,
};
use warpui::fonts::{Properties, Weight};
use warpui::{AppContext, Element, SingletonEntity};

use crate::appearance::Appearance;
use crate::search::SearchItem;
use crate::search::command_palette::agent_sessions::candidate::CandidateOrigin;
use crate::search::command_palette::agent_sessions::search::MatchedAgentSession;
use crate::search::command_palette::agent_sessions::tiers::NAME_ROW_TIER;
use crate::search::command_palette::mixer::CommandPaletteItemAction;
use crate::search::command_palette::render_util::render_search_item_icon;
use crate::search::item::IconLocation;
use crate::search::result_renderer::ItemHighlightState;
use crate::util::time_format::format_approx_duration_from_now_utc;

/// The timestamp column's text for a session that is running right now. A live
/// session has no "last active" clock to format, and "now" would read as an
/// estimate of something Warp actually knows exactly.
const LIVE_TIMESTAMP_LABEL: &str = "open";

/// One session row in the session-search popup: the session's own name, the
/// project and directory it belongs to, and when it was last active.
#[derive(Debug)]
pub struct AgentSessionSearchItem {
    matched: MatchedAgentSession,
    /// Recency rank of this candidate within the popup's whole candidate list,
    /// in `[0, 1)`. See [`Self::score`].
    recency_bonus: f64,
}

impl AgentSessionSearchItem {
    pub fn new(matched: MatchedAgentSession, recency_bonus: f64) -> Self {
        Self {
            matched,
            recency_bonus,
        }
    }

    pub fn matched(&self) -> &MatchedAgentSession {
        &self.matched
    }

    /// The dim second line: which project the session belongs to and where it
    /// ran. Both are searchable, so both are shown — a user who found a row by
    /// typing part of a path needs to see that path to recognise it.
    fn subtitle(&self) -> String {
        let candidate = &self.matched.candidate;
        format!("{} · {}", candidate.project_name, candidate.cwd)
    }
}

impl SearchItem for AgentSessionSearchItem {
    type Action = CommandPaletteItemAction;

    fn is_multiline(&self) -> bool {
        true
    }

    fn render_icon(
        &self,
        highlight_state: ItemHighlightState,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        // The agent's own logo when it has one, so a mixed list is scannable by
        // which agent ran the session.
        let icon = self
            .matched
            .candidate
            .agent
            .icon()
            .unwrap_or(Icon::Terminal);
        render_search_item_icon(
            appearance,
            icon,
            appearance.theme().foreground().into_solid(),
            highlight_state,
        )
    }

    fn icon_location(&self, appearance: &Appearance) -> IconLocation {
        // The icon is one monospace-font tall while the text line is
        // `line_height_ratio` taller; offset by the difference so the icon
        // centres on the first line rather than on the whole two-line row.
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
        let candidate = &self.matched.candidate;
        let sub_text_font_size = appearance.monospace_font_size() - 2.;
        let highlight = Highlight::new()
            .with_properties(Properties::default().weight(Weight::Bold))
            .with_foreground_color(highlight_state.main_text_fill(appearance).into_solid());

        let mut task_name_element = Text::new_inline(
            candidate.task_name.clone(),
            appearance.ui_font_family(),
            appearance.monospace_font_size(),
        )
        .with_color(highlight_state.sub_text_fill(appearance).into_solid())
        .with_style(Properties::default().weight(Weight::Bold));

        let mut subtitle_element = Text::new_inline(
            self.subtitle(),
            appearance.ui_font_family(),
            sub_text_font_size,
        )
        .with_color(highlight_state.sub_text_fill(appearance).into_solid());

        let indices = self.matched.highlight_indices();
        if !indices.task_indices().is_empty() {
            task_name_element =
                task_name_element.with_single_highlight(highlight, indices.task_indices().clone());
        }
        // The subtitle is `project · cwd`, so the project's indices apply
        // as-is while the cwd's have to be shifted past the project and the
        // separator. Both are char indices, so the shift is a char count.
        let project_char_len = candidate.project_name.chars().count();
        let subtitle_indices: Vec<usize> = indices
            .project_indices()
            .iter()
            .copied()
            .chain(
                indices
                    .cwd_indices()
                    .iter()
                    .map(|index| index + project_char_len + SUBTITLE_SEPARATOR_CHAR_LEN),
            )
            .collect();
        if !subtitle_indices.is_empty() {
            subtitle_element = subtitle_element.with_single_highlight(highlight, subtitle_indices);
        }

        let last_active = match (candidate.origin, candidate.last_active) {
            (CandidateOrigin::Live, _) => LIVE_TIMESTAMP_LABEL.to_owned(),
            (CandidateOrigin::Handle | CandidateOrigin::Scanned, Some(last_active)) => {
                format_approx_duration_from_now_utc(last_active)
            }
            (CandidateOrigin::Handle | CandidateOrigin::Scanned, None) => String::new(),
        };
        let last_active_element = Container::new(
            Text::new_inline(last_active, appearance.ui_font_family(), sub_text_font_size)
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
                        .with_child(subtitle_element.finish())
                        .with_cross_axis_alignment(CrossAxisAlignment::Start)
                        .finish(),
                )
                .finish(),
            )
            .with_child(last_active_element)
            .with_main_axis_size(MainAxisSize::Max)
            .finish()
    }

    fn priority_tier(&self) -> u8 {
        NAME_ROW_TIER
    }

    /// The fuzzy score, with recency as a fractional tiebreaker.
    ///
    /// Fuzzy scores are integers, so a bonus in `[0, 1)` can only order rows
    /// that already scored the same — it can never promote a worse match. This
    /// is what keeps the empty query (every score `0`) ordered newest-first
    /// *explicitly*, instead of relying on insertion order surviving a stable
    /// sort and a reversal.
    fn score(&self) -> OrderedFloat<f64> {
        OrderedFloat(self.matched.score() as f64 + self.recency_bonus)
    }

    fn accept_result(&self) -> Self::Action {
        CommandPaletteItemAction::ResumeAgentSession {
            agent: self.matched.candidate.agent,
            session_id: self.matched.candidate.session_id.clone(),
        }
    }

    fn execute_result(&self) -> Self::Action {
        self.accept_result()
    }

    fn accessibility_label(&self) -> String {
        format!(
            "{} session: {}",
            self.matched.candidate.agent.display_name(),
            self.matched.candidate.task_name
        )
    }

    fn accessibility_help_message(&self) -> Option<String> {
        Some(format!(
            "Press enter to resume \"{}\".",
            self.matched.candidate.task_name
        ))
    }
}

/// Char length of the ` · ` that joins the project and the cwd in the subtitle.
const SUBTITLE_SEPARATOR_CHAR_LEN: usize = 3;

#[cfg(test)]
#[path = "search_item_tests.rs"]
mod tests;
