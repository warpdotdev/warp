use itertools::Itertools;
use ordered_float::OrderedFloat;
use warpui::elements::{
    ConstrainedBox, Container, CrossAxisAlignment, Flex, Highlight, Icon, MainAxisAlignment,
    MainAxisSize, ParentElement, Text,
};
use warpui::fonts::{Properties, Weight};
use warpui::ui_components::components::{UiComponent, UiComponentStyles};
use warpui::{AppContext, Element, SingletonEntity};

use crate::appearance::Appearance;
use crate::env_vars::CloudEnvVarCollection;
use crate::search::command_search::searcher::CommandSearchItemAction;
use crate::search::env_var_collections::fuzzy_match::FuzzyMatchEnvVarCollectionResult;
use crate::search::item::SearchItem;
use crate::search::result_renderer::ItemHighlightState;

const ENV_VAR_COLLECTION_ICON_PATH: &str = "bundled/svg/env-var-collection.svg";

/// Struct designed to be the implementation of CommandSearchItem for EnvVarCollections.
#[derive(Clone, Debug)]
pub struct EnvVarCollectionSearchItem {
    pub env_var_collection: CloudEnvVarCollection,
    pub fuzzy_matched_env_var_collection: FuzzyMatchEnvVarCollectionResult,
}

impl EnvVarCollectionSearchItem {
    fn render_name(&self, appearance: &Appearance) -> Box<dyn Element> {
        let env_var_collection = self.env_var_collection.model().string_model.clone();

        appearance
            .ui_builder()
            .wrappable_text(
                env_var_collection
                    .title
                    .clone()
                    .unwrap_or("Untitled".to_owned()),
                true,
            )
            .with_style(UiComponentStyles {
                font_family_id: Some(appearance.ui_font_family()),
                font_color: Some(
                    appearance
                        .theme()
                        .main_text_color(appearance.theme().background())
                        .into(),
                ),
                font_size: Some(
                    appearance.monospace_font_size() * styles::TITLE_FONT_SIZE_SCALE_FACTOR,
                ),
                font_weight: Some(Weight::Bold),
                ..Default::default()
            })
            .build()
            .finish()
    }
}

impl SearchItem for EnvVarCollectionSearchItem {
    type Action = CommandSearchItemAction;

    fn render_icon(
        &self,
        highlight_state: ItemHighlightState,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        Container::new(
            ConstrainedBox::new(
                Icon::new(
                    ENV_VAR_COLLECTION_ICON_PATH,
                    highlight_state.icon_fill(appearance),
                )
                .finish(),
            )
            .with_width(appearance.monospace_font_size())
            .with_height(appearance.monospace_font_size())
            .finish(),
        )
        .with_margin_right(12.)
        .finish()
    }

    fn render_item(
        &self,
        highlight_state: ItemHighlightState,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let env_var_collection = self.env_var_collection.model().string_model.clone();
        let appearance = Appearance::as_ref(app);

        let mut title_text = Text::new_inline(
            env_var_collection
                .title
                .clone()
                .unwrap_or("Untitled".to_owned()),
            appearance.ui_font_family(),
            appearance.monospace_font_size(),
        )
        .with_color(highlight_state.main_text_fill(appearance).into_solid());

        if let Some(name_match_result) = &self.fuzzy_matched_env_var_collection.title_match_result {
            title_text = title_text.with_single_highlight(
                Highlight::new()
                    .with_properties(Properties::default().weight(Weight::Bold))
                    .with_foreground_color(highlight_state.main_text_fill(appearance).into_solid()),
                name_match_result.matched_indices.clone(),
            );
        }

        let vars_text = self
            .env_var_collection
            .model()
            .string_model
            .vars
            .iter()
            .map(|var| var.name.clone())
            .collect_vec()
            .join(", ");

        let mut vars_element = Text::new_inline(
            vars_text.clone(),
            appearance.monospace_font_family(),
            appearance.monospace_font_size() - 2.,
        )
        .with_color(highlight_state.sub_text_fill(appearance).into_solid());

        if let Some(var_name_match_result) =
            &self.fuzzy_matched_env_var_collection.var_name_match_result
        {
            vars_element = vars_element.with_single_highlight(
                Highlight::new()
                    .with_properties(Properties::default().weight(Weight::Bold))
                    .with_foreground_color(highlight_state.main_text_fill(appearance).into_solid()),
                var_name_match_result.matched_indices.clone(),
            );
        }

        Flex::column()
            .with_main_axis_alignment(MainAxisAlignment::Center)
            .with_main_axis_size(MainAxisSize::Max)
            .with_child(title_text.finish())
            .with_child(vars_element.finish())
            .finish()
    }

    fn render_details(&self, ctx: &AppContext) -> Option<Box<dyn Element>> {
        let appearance = Appearance::as_ref(ctx);
        let mut flex_column = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_child(
                Container::new(self.render_name(appearance))
                    .with_margin_bottom(16.)
                    .finish(),
            );

        let env_var_collection = self.env_var_collection.model().string_model.clone();

        if let Some(description) = env_var_collection.description.clone() {
            let mut description_text = appearance
                .ui_builder()
                .paragraph(description.clone())
                .with_style(UiComponentStyles {
                    font_family_id: Some(appearance.ui_font_family()),
                    font_color: Some(
                        appearance
                            .theme()
                            .sub_text_color(appearance.theme().surface_2())
                            .into(),
                    ),
                    font_size: Some(appearance.monospace_font_size()),
                    ..Default::default()
                });

            if let Some(description_match_result) = &self
                .fuzzy_matched_env_var_collection
                .description_match_result
            {
                description_text = description_text.with_highlights(
                    description_match_result.matched_indices.clone(),
                    Highlight::new()
                        .with_properties(Properties::default().weight(Weight::Bold))
                        .with_foreground_color(
                            appearance
                                .theme()
                                .main_text_color(appearance.theme().background())
                                .into(),
                        ),
                );
            }

            flex_column.add_child(
                Container::new(description_text.build().finish())
                    .with_margin_bottom(16.)
                    .finish(),
            )
        }

        Some(flex_column.finish())
    }

    /// The match score for a EnvVarCollection is an average of the match scores
    /// against the name and description of the EVC.
    fn score(&self) -> OrderedFloat<f64> {
        self.fuzzy_matched_env_var_collection.score()
    }

    fn accept_result(&self) -> CommandSearchItemAction {
        CommandSearchItemAction::AcceptEnvVarCollection(Box::new(self.env_var_collection.clone()))
    }

    fn execute_result(&self) -> CommandSearchItemAction {
        self.accept_result()
    }

    fn accessibility_label(&self) -> String {
        let env_var_collection = self.env_var_collection.model().string_model.clone();

        format!(
            "Environment Variables: {}",
            env_var_collection
                .title
                .clone()
                .unwrap_or("Untitled".to_owned())
        )
    }
}

mod styles {
    pub const TITLE_FONT_SIZE_SCALE_FACTOR: f32 = 1.12;
}
