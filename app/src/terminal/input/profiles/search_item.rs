use fuzzy_match::FuzzyMatchResult;
use ordered_float::OrderedFloat;
use warp_core::ui::Icon;
use warp_core::ui::theme::Fill;
use warpui::elements::{ConstrainedBox, Container, Flex, Highlight, ParentElement as _, Text};
use warpui::fonts::{Properties, Style, Weight};
use warpui::prelude::CrossAxisAlignment;
use warpui::text_layout::ClipConfig;
use warpui::{AppContext, Element, SingletonEntity as _};

use crate::ai::execution_profiles::profiles::ClientProfileId;
use crate::appearance::Appearance;
use crate::search::{ItemHighlightState, SearchItem};
use crate::terminal::input::inline_menu::styles as inline_styles;
use crate::terminal::input::profiles::data_source::SelectProfileMenuItem;

const MANAGE_PROFILES_LABEL: &str = "Manage profiles";

#[derive(Debug, Clone)]
enum ProfileSearchItemKind {
    Profile {
        profile_id: ClientProfileId,
        profile_name: String,
        is_selected: bool,
    },
    ManageProfiles,
}

#[derive(Debug, Clone)]
pub(super) struct ProfileSearchItem {
    kind: ProfileSearchItemKind,
    match_result: Option<FuzzyMatchResult>,
    score: OrderedFloat<f64>,
}

impl ProfileSearchItem {
    pub fn new_profile_item(
        profile_id: ClientProfileId,
        profile_name: String,
        is_selected: bool,
    ) -> Self {
        Self {
            kind: ProfileSearchItemKind::Profile {
                profile_id,
                profile_name,
                is_selected,
            },
            match_result: None,
            score: OrderedFloat(0.0),
        }
    }

    pub fn new_manage_profiles_item() -> Self {
        Self {
            kind: ProfileSearchItemKind::ManageProfiles,
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
}

impl SearchItem for ProfileSearchItem {
    type Action = SelectProfileMenuItem;

    fn render_icon(
        &self,
        _highlight_state: ItemHighlightState,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let icon = match self.kind {
            ProfileSearchItemKind::Profile { .. } => Icon::Psychology,
            ProfileSearchItemKind::ManageProfiles => Icon::Gear,
        }
        .to_warpui_icon(inline_styles::icon_color(appearance));

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

        let (label_text, is_selected) = match &self.kind {
            ProfileSearchItemKind::Profile {
                profile_name,
                is_selected,
                ..
            } => (profile_name.clone(), *is_selected),
            ProfileSearchItemKind::ManageProfiles => (MANAGE_PROFILES_LABEL.to_owned(), false),
        };

        let mut label = Text::new_inline(label_text, appearance.ui_font_family(), font_size)
            .with_color(
                inline_styles::primary_text_color(appearance.theme(), background.into()).into(),
            )
            .with_clip(ClipConfig::ellipsis());

        // Apply search highlighting to the label.
        if let Some(match_result) = &self.match_result
            && !match_result.matched_indices.is_empty()
        {
            label = label.with_single_highlight(
                Highlight::new().with_properties(Properties::default().weight(Weight::Bold)),
                match_result.matched_indices.clone(),
            );
        }

        let mut row = Flex::row()
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(label.finish());

        if is_selected {
            let selected_label = "(selected)";
            let selected_text = Text::new_inline(
                selected_label.to_string(),
                appearance.ui_font_family(),
                font_size,
            )
            .with_color(
                inline_styles::secondary_text_color(appearance.theme(), background.into()).into(),
            )
            .with_single_highlight(
                Highlight::new().with_properties(Properties::default().style(Style::Italic)),
                (0..selected_label.len()).collect(),
            )
            .finish();

            row = row.with_child(Container::new(selected_text).with_margin_left(6.).finish());
        }

        row.finish()
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
        match self.kind {
            ProfileSearchItemKind::Profile { profile_id, .. } => {
                SelectProfileMenuItem::Profile { profile_id }
            }
            ProfileSearchItemKind::ManageProfiles => SelectProfileMenuItem::ManageProfiles,
        }
    }

    fn execute_result(&self) -> Self::Action {
        self.accept_result()
    }

    fn accessibility_label(&self) -> String {
        match &self.kind {
            ProfileSearchItemKind::Profile { profile_name, .. } => {
                format!("Profile: {profile_name}")
            }
            ProfileSearchItemKind::ManageProfiles => MANAGE_PROFILES_LABEL.to_string(),
        }
    }
}
