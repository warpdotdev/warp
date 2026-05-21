use fuzzy_match::FuzzyMatchResult;
use ordered_float::OrderedFloat;
use std::fmt::Debug;
use std::path::PathBuf;
use warp_util::path::LineAndColumnArg;

use crate::appearance::Appearance;
use crate::search::command_palette::mixer::CommandPaletteItemAction;
use crate::search::command_palette::styles;
use crate::search::item::{IconLocation, SearchItem};
use crate::search::result_renderer::ItemHighlightState;
use warpui::elements::{Align, ConstrainedBox, Container, Flex, Icon, ParentElement, Text};
use warpui::fonts::{Properties, Weight};
use warpui::{AppContext, Element, SingletonEntity};

use crate::search::files::icon::icon_from_file_path;
use crate::ui_components::render_file_search_row::{render_file_search_row, FileSearchRowOptions};

#[derive(Debug)]
pub struct FileSearchItem {
    pub path: PathBuf,
    pub project_directory: String,
    pub match_result: FuzzyMatchResult,
    pub line_and_column_arg: Option<LineAndColumnArg>,
    pub is_directory: bool,
}

impl SearchItem for FileSearchItem {
    type Action = CommandPaletteItemAction;

    fn render_icon(
        &self,
        highlight_state: ItemHighlightState,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        Container::new(
            ConstrainedBox::new(if self.is_directory {
                Icon::new(
                    "bundled/svg/completion-folder.svg",
                    highlight_state.icon_fill(appearance).into_solid(),
                )
                .finish()
            } else {
                icon_from_file_path(&self.path.to_string_lossy(), appearance, highlight_state)
            })
            .with_width(styles::SEARCH_ITEM_TEXT_PADDING * 4.0)
            .with_height(styles::SEARCH_ITEM_TEXT_PADDING * 4.0)
            .finish(),
        )
        .with_margin_right(styles::SEARCH_ITEM_TEXT_PADDING)
        .finish()
    }

    fn icon_location(&self, _appearance: &Appearance) -> IconLocation {
        IconLocation::Centered
    }

    fn render_item(
        &self,
        highlight_state: ItemHighlightState,
        app: &AppContext,
    ) -> Box<dyn Element> {
        render_file_search_row(
            &self.path,
            FileSearchRowOptions {
                match_result: Some(&self.match_result),
                highlight_state,
                max_combined_length: None,
                ..Default::default()
            },
            app,
        )
    }

    fn score(&self) -> OrderedFloat<f64> {
        OrderedFloat(self.match_result.score as f64)
    }

    fn accept_result(&self) -> Self::Action {
        if self.is_directory {
            CommandPaletteItemAction::OpenDirectory {
                path: self.path.to_string_lossy().to_string(),
                project_directory: self.project_directory.clone(),
            }
        } else {
            CommandPaletteItemAction::OpenFile {
                path: self.path.to_string_lossy().to_string(),
                project_directory: self.project_directory.clone(),
                line_and_column_arg: self.line_and_column_arg,
            }
        }
    }

    fn execute_result(&self) -> Self::Action {
        self.accept_result()
    }

    fn accessibility_label(&self) -> String {
        if self.is_directory {
            format!("Directory: {}", self.path.display())
        } else {
            format!("File: {}", self.path.display())
        }
    }

    fn accessibility_help_message(&self) -> Option<String> {
        Some(if self.is_directory {
            "Press Enter to navigate to this directory".to_string()
        } else {
            "Press Enter to open this file".to_string()
        })
    }

    fn render_details(&self, _ctx: &AppContext) -> Option<Box<dyn Element>> {
        None
    }
}

/// A search item for creating a new file with the specified name
#[derive(Debug)]
pub struct CreateFileSearchItem {
    pub file_name: String,
    pub current_directory: String,
}

impl SearchItem for CreateFileSearchItem {
    type Action = CommandPaletteItemAction;

    fn render_icon(
        &self,
        highlight_state: ItemHighlightState,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        Container::new(
            ConstrainedBox::new(
                Icon::new(
                    "bundled/svg/plus-circle.svg",
                    highlight_state.icon_fill(appearance).into_solid(),
                )
                .finish(),
            )
            .with_width(styles::SEARCH_ITEM_TEXT_PADDING * 4.0)
            .with_height(styles::SEARCH_ITEM_TEXT_PADDING * 4.0)
            .finish(),
        )
        .with_margin_right(styles::SEARCH_ITEM_TEXT_PADDING)
        .finish()
    }

    fn icon_location(&self, _appearance: &Appearance) -> IconLocation {
        IconLocation::Centered
    }

    fn render_item(
        &self,
        highlight_state: ItemHighlightState,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);

        let text_color = highlight_state.sub_text_fill(appearance).into_solid();

        let label = Text::new_inline(
            format!("Create {}…", &self.file_name),
            appearance.ui_font_family(),
            appearance.monospace_font_size(),
        )
        .with_color(text_color)
        .with_style(Properties::default().weight(Weight::Normal))
        .finish();

        ConstrainedBox::new(
            Align::new(Flex::row().with_child(label).finish())
                .left()
                .finish(),
        )
        .with_height(40.0)
        .finish()
    }

    fn score(&self) -> OrderedFloat<f64> {
        // Give it a very low score so it appears at the bottom
        OrderedFloat(-100000.0)
    }

    fn accept_result(&self) -> Self::Action {
        CommandPaletteItemAction::CreateFile {
            file_name: self.file_name.clone(),
            current_directory: self.current_directory.clone(),
        }
    }

    fn execute_result(&self) -> Self::Action {
        self.accept_result()
    }

    fn accessibility_label(&self) -> String {
        format!("Create file: {}", self.file_name)
    }

    fn accessibility_help_message(&self) -> Option<String> {
        Some(format!(
            "Press Enter to create {} in the current directory",
            self.file_name
        ))
    }

    fn render_details(&self, _ctx: &AppContext) -> Option<Box<dyn Element>> {
        None
    }
}
