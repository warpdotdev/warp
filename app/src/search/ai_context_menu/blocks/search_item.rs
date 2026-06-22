use chrono::{DateTime, Local};
use fuzzy_match::FuzzyMatchResult;
use ordered_float::OrderedFloat;
use warp_core::command::ExitCode;
use warpui::elements::{
    ConstrainedBox, Container, CrossAxisAlignment, Flex, Highlight, Icon, ParentElement, Text,
};
use warpui::fonts::{Properties, Weight};
use warpui::{AppContext, Element, SingletonEntity};

use crate::appearance::Appearance;
use crate::search::ai_context_menu::mixer::AIContextMenuSearchableAction;
use crate::search::ai_context_menu::styles;
use crate::search::item::SearchItem;
use crate::search::result_renderer::ItemHighlightState;
use crate::terminal::model::block::BlockId;
use crate::util::truncation::truncate_from_end;

/// Calculate how long ago a timestamp was
fn time_ago_string(timestamp: Option<&DateTime<Local>>) -> String {
    let Some(timestamp) = timestamp else {
        return "Just now".to_string();
    };

    let now = Local::now();
    let duration = now.signed_duration_since(*timestamp);

    if duration.num_seconds() < 60 {
        "Just now".to_string()
    } else if duration.num_minutes() < 60 {
        format!("{} minutes ago", duration.num_minutes())
    } else if duration.num_hours() < 24 {
        format!("{} hours ago", duration.num_hours())
    } else {
        format!("{} days ago", duration.num_days())
    }
}

#[derive(Clone, Debug)]
pub struct BlockSearchItem {
    pub block_id: BlockId,
    pub command: String,
    pub directory: Option<String>,
    pub exit_code: ExitCode,
    pub output_lines: Vec<String>,
    pub completed_ts: Option<DateTime<Local>>,
    pub match_result: FuzzyMatchResult,
    /// Whether this block belongs to the currently active terminal session.
    /// Used to give active-session blocks higher priority in search results.
    pub is_active_session: bool,
}

impl SearchItem for BlockSearchItem {
    type Action = AIContextMenuSearchableAction;

    fn render_icon(
        &self,
        highlight_state: ItemHighlightState,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        // Show error icon if the block failed, otherwise show the regular block icon
        let (icon_path, icon_color) = if !self.exit_code.was_successful() {
            (
                "bundled/svg/alert-triangle.svg",
                appearance.theme().ui_error_color(),
            )
        } else {
            (
                "bundled/svg/terminal.svg",
                highlight_state.icon_fill(appearance).into_solid(),
            )
        };

        Container::new(
            ConstrainedBox::new(Icon::new(icon_path, icon_color).finish())
                .with_width(styles::ICON_SIZE)
                .with_height(styles::ICON_SIZE)
                .finish(),
        )
        .with_margin_right(styles::MARGIN_RIGHT)
        .finish()
    }

    fn render_item(
        &self,
        highlight_state: ItemHighlightState,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);

        // Create command text with highlighting
        let mut command_text = Text::new(
            self.command.clone(),
            appearance.ui_font_family(),
            appearance.monospace_font_size() - 1.0,
        )
        .with_color(highlight_state.main_text_fill(appearance).into_solid());

        if !self.match_result.matched_indices.is_empty() {
            command_text = command_text.with_single_highlight(
                Highlight::new().with_properties(Properties::default().weight(Weight::Bold)),
                self.match_result.matched_indices.clone(),
            );
        }

        // Create directory text with lighter color
        let directory_text = self.directory.as_ref().map(|directory| {
            Text::new(
                directory.clone(),
                appearance.ui_font_family(),
                appearance.monospace_font_size() - 2.0,
            )
            .with_color(highlight_state.sub_text_fill(appearance).into_solid())
            .finish()
        });

        // Create row with command name and directory on the same line
        let mut row = Flex::row()
            .with_child(command_text.finish())
            .with_cross_axis_alignment(CrossAxisAlignment::Center);

        if let Some(directory) = directory_text {
            row.add_child(Container::new(directory).with_padding_left(8.0).finish());
        }

        row.finish()
    }

    fn render_details(&self, ctx: &AppContext) -> Option<Box<dyn Element>> {
        let appearance = Appearance::as_ref(ctx);
        let theme = appearance.theme();

        // Create main text: command (truncate for hover card too)
        let main_text = truncate_from_end(&self.command, 100);

        // Create sub text: last 3 lines of output
        let sub_text = if self.output_lines.is_empty() {
            "No output".to_string()
        } else {
            let joined = self.output_lines.join("\n").trim().to_string();
            // Additional safety truncation for the hover card
            truncate_from_end(&joined, 400)
        };

        // Create time ago text
        let time_ago_text = time_ago_string(self.completed_ts.as_ref());

        // Create main text element - use monospace font for command
        let main_text_element = Text::new(
            main_text,
            appearance.monospace_font_family(),
            appearance.monospace_font_size() - 1.0,
        )
        .with_color(theme.active_ui_text_color().into())
        .with_style(Properties::default().weight(Weight::Medium))
        .finish();

        // Create sub text element - output lines
        let sub_text_element = Text::new(
            sub_text,
            appearance.monospace_font_family(),
            appearance.monospace_font_size() - 3.0,
        )
        .with_color(theme.nonactive_ui_text_color().into())
        .finish();

        // Create time ago element
        let time_ago_element = Text::new(
            time_ago_text,
            appearance.ui_font_family(),
            appearance.monospace_font_size() - 3.0,
        )
        .with_color(theme.nonactive_ui_text_color().into())
        .finish();

        // Create modal content with reduced spacing
        let content = Flex::column()
            .with_child(main_text_element)
            .with_child(
                Container::new(sub_text_element)
                    .with_padding_top(4.0)
                    .finish(),
            )
            .with_child(
                Container::new(time_ago_element)
                    .with_padding_top(4.0)
                    .finish(),
            )
            .finish();

        Some(content)
    }

    fn score(&self) -> OrderedFloat<f64> {
        OrderedFloat(self.match_result.score as f64)
    }

    fn accept_result(&self) -> AIContextMenuSearchableAction {
        AIContextMenuSearchableAction::InsertText {
            text: format!("<block:{}>", self.block_id),
        }
    }

    fn execute_result(&self) -> AIContextMenuSearchableAction {
        self.accept_result()
    }

    fn accessibility_label(&self) -> String {
        format!("Block: {}", self.command)
    }
}
