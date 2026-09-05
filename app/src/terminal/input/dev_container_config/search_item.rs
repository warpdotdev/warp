//! SearchItem implementation for Dev Container config selector items.
use std::path::{Path, PathBuf};

use fuzzy_match::FuzzyMatchResult;
use ordered_float::OrderedFloat;
use warp_core::ui::Icon;
use warp_core::ui::theme::Fill;
use warpui::elements::{ConstrainedBox, Container, Highlight, Text};
use warpui::fonts::{Properties, Weight};
use warpui::text_layout::ClipConfig;
use warpui::{AppContext, Element, SingletonEntity as _};

use crate::appearance::Appearance;
use crate::search::{ItemHighlightState, SearchItem};
use crate::terminal::input::dev_container_config::SelectDevContainerConfig;
use crate::terminal::input::inline_menu::styles as inline_styles;

#[derive(Debug, Clone)]
pub(super) struct DevContainerConfigSearchItem {
    config_path: PathBuf,
    match_result: Option<FuzzyMatchResult>,
    score: OrderedFloat<f64>,
}

impl DevContainerConfigSearchItem {
    pub fn new(config_path: PathBuf) -> Self {
        Self {
            config_path,
            match_result: None,
            score: OrderedFloat(0.0),
        }
    }

    pub fn with_match_result(mut self, match_result: FuzzyMatchResult) -> Self {
        self.match_result = Some(match_result);
        self
    }

    pub fn with_score(mut self, score: OrderedFloat<f64>) -> Self {
        self.score = score;
        self
    }

    /// The label shown for a config path: the name of the folder that directly contains its
    /// `devcontainer.json` (e.g. `.devcontainer`, or `<folder>` from
    /// `.devcontainer/<folder>/devcontainer.json`).
    pub fn display_label(config_path: &Path) -> String {
        config_path
            .parent()
            .and_then(|parent| parent.file_name())
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| config_path.display().to_string())
    }
}

impl SearchItem for DevContainerConfigSearchItem {
    type Action = SelectDevContainerConfig;

    fn render_icon(
        &self,
        _highlight_state: ItemHighlightState,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let icon = Icon::Docker.to_warpui_icon(inline_styles::icon_color(appearance));
        Container::new(
            ConstrainedBox::new(icon.finish())
                .with_width(inline_styles::font_size(appearance))
                .with_height(inline_styles::font_size(appearance))
                .finish(),
        )
        .with_margin_right(inline_styles::ICON_MARGIN)
        .finish()
    }

    fn render_item(
        &self,
        _highlight_state: ItemHighlightState,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let background = inline_styles::menu_background_color(app);
        let font_size = inline_styles::font_size(appearance);

        let label = Self::display_label(&self.config_path);
        let mut text = Text::new_inline(label, appearance.ui_font_family(), font_size)
            .with_color(
                inline_styles::primary_text_color(appearance.theme(), background.into()).into(),
            )
            .with_clip(ClipConfig::ellipsis());

        if let Some(match_result) = &self.match_result
            && !match_result.matched_indices.is_empty()
        {
            text = text.with_single_highlight(
                Highlight::new().with_properties(Properties::default().weight(Weight::Bold)),
                match_result.matched_indices.clone(),
            );
        }

        text.finish()
    }

    fn item_background(
        &self,
        highlight_state: ItemHighlightState,
        appearance: &Appearance,
    ) -> Option<Fill> {
        inline_styles::item_background(highlight_state, appearance)
    }

    fn score(&self) -> OrderedFloat<f64> {
        self.score
    }

    fn accept_result(&self) -> Self::Action {
        SelectDevContainerConfig {
            config_path: self.config_path.clone(),
        }
    }

    fn execute_result(&self) -> Self::Action {
        self.accept_result()
    }

    fn accessibility_label(&self) -> String {
        format!(
            "Dev Container config: {}",
            Self::display_label(&self.config_path)
        )
    }
}
