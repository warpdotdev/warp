use itertools::Itertools;
use ordered_float::OrderedFloat;
use warpui::elements::{Container, Flex, Highlight, ParentElement, Text};
use warpui::fonts::{Properties, Weight};
use warpui::{AppContext, Element, SingletonEntity};

use crate::appearance::Appearance;
use crate::cloud_object::CloudObject;
use crate::drive::cloud_object_styling::warp_drive_icon_color;
use crate::drive::{CloudObjectTypeAndId, DriveObjectType};
use crate::env_vars::CloudEnvVarCollection;
use crate::search::command_palette::mixer::CommandPaletteItemAction;
use crate::search::command_palette::render_util::render_search_item_icon;
use crate::search::command_palette::styles::SEARCH_ITEM_TEXT_PADDING;
use crate::search::env_var_collections::fuzzy_match::FuzzyMatchEnvVarCollectionResult;
use crate::search::item::{IconLocation, SearchItem};
use crate::search::result_renderer::ItemHighlightState;
use crate::ui_components::icons::Icon;

pub const ENV_VAR_NAME_SEPARATOR: &str = ", ";

/// Search item result for a cloud EnvVarCollection.
#[derive(Debug)]
pub struct EnvVarCollectionSearchItem {
    pub match_result: FuzzyMatchEnvVarCollectionResult,
    pub cloud_env_var_collection: CloudEnvVarCollection,
}

impl SearchItem for EnvVarCollectionSearchItem {
    type Action = CommandPaletteItemAction;

    fn is_multiline(&self) -> bool {
        true
    }

    fn render_icon(
        &self,
        highlight_state: ItemHighlightState,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        let color = warp_drive_icon_color(appearance, DriveObjectType::EnvVarCollection);
        render_search_item_icon(appearance, Icon::EnvVarCollection, color, highlight_state)
    }

    fn icon_location(&self, appearance: &Appearance) -> IconLocation {
        // The icon is has the size of the monospace font, whereas the text have a height of
        // `line_height_ratio * font_size`. Offset the icon by this difference so it is rendered
        // centered with the text.
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
        let mut title_text = Text::new_inline(
            self.cloud_env_var_collection
                .model()
                .string_model
                .title
                .clone()
                .unwrap_or("Untitled".to_owned())
                .to_owned(),
            appearance.ui_font_family(),
            appearance.monospace_font_size(),
        )
        .with_color(highlight_state.sub_text_fill(appearance).into_solid())
        .with_style(Properties::default().weight(Weight::Bold));

        if let Some(title_match_result) = &self.match_result.title_match_result {
            title_text = title_text.with_single_highlight(
                Highlight::new()
                    .with_properties(Properties::default().weight(Weight::Bold))
                    .with_foreground_color(highlight_state.main_text_fill(appearance).into_solid()),
                title_match_result.matched_indices.clone(),
            );
        }

        let vars_text = self
            .cloud_env_var_collection
            .model()
            .string_model
            .vars
            .iter()
            .map(|var| var.name.clone())
            .collect_vec()
            .join(ENV_VAR_NAME_SEPARATOR);

        let mut vars_element = Text::new_inline(
            vars_text.clone(),
            appearance.monospace_font_family(),
            appearance.monospace_font_size() - 2.,
        )
        .with_color(highlight_state.sub_text_fill(appearance).into_solid());

        if let Some(var_name_match_result) = &self.match_result.var_name_match_result {
            vars_element = vars_element.with_single_highlight(
                Highlight::new()
                    .with_foreground_color(highlight_state.main_text_fill(appearance).into_solid()),
                var_name_match_result.matched_indices.clone(),
            );
        }

        let mut breadcrumbs_text: Text = Text::new_inline(
            self.cloud_env_var_collection.breadcrumbs(app),
            appearance.ui_font_family(),
            appearance.monospace_font_size() - 2.,
        )
        .with_color(highlight_state.sub_text_fill(appearance).into_solid());

        if let Some(breadcrumbs_match_result) = &self.match_result.breadcrumbs_match_result {
            breadcrumbs_text = breadcrumbs_text.with_single_highlight(
                Highlight::new()
                    .with_foreground_color(highlight_state.main_text_fill(appearance).into_solid()),
                breadcrumbs_match_result.matched_indices.clone(),
            );
        }

        let mut item = Flex::column()
            .with_child(Container::new(title_text.finish()).finish())
            .with_child(
                Container::new(breadcrumbs_text.finish())
                    .with_padding_top(SEARCH_ITEM_TEXT_PADDING)
                    .finish(),
            );

        item.add_child(
            Container::new(vars_element.finish())
                .with_padding_top(SEARCH_ITEM_TEXT_PADDING)
                .finish(),
        );

        item.finish()
    }

    fn render_details(&self, _: &AppContext) -> Option<Box<dyn Element>> {
        None
    }

    fn score(&self) -> OrderedFloat<f64> {
        self.match_result.score()
    }

    fn accept_result(&self) -> Self::Action {
        CommandPaletteItemAction::InvokeEnvironmentVariables {
            id: self.cloud_env_var_collection.id,
        }
    }

    fn execute_result(&self) -> Self::Action {
        CommandPaletteItemAction::ViewInWarpDrive {
            id: CloudObjectTypeAndId::GenericStringObject {
                object_type: crate::cloud_object::GenericStringObjectFormat::Json(
                    crate::cloud_object::JsonObjectType::EnvVarCollection,
                ),
                id: self.cloud_env_var_collection.id,
            },
        }
    }

    fn accessibility_label(&self) -> String {
        format!(
            "Environment Variables: {}",
            self.cloud_env_var_collection
                .model()
                .string_model
                .title
                .clone()
                .unwrap_or("Untitled".to_owned())
        )
    }
}
