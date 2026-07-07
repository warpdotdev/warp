use ordered_float::OrderedFloat;
use warpui::elements::{ConstrainedBox, Container, Highlight, Text};
use warpui::fonts::{Properties, Weight};
use warpui::{AppContext, Element, SingletonEntity};

use super::external_secret_fuzzy_match::FuzzyMatchExternalSecretResult;
use super::searcher::ExternalSecretSearchItemAction;
use crate::appearance::Appearance;
use crate::external_secrets::{ExternalSecret, ExternalSecretManager};
use crate::search::external_secrets::view::styles;
use crate::search::item::{IconLocation, SearchItem};
use crate::search::result_renderer::ItemHighlightState;

const ICON_SIZE: f32 = 16.;

#[derive(Clone, Debug)]
pub struct ExternalSecretSearchItem {
    pub external_secret: ExternalSecret,
    pub fuzzy_matched_secret: FuzzyMatchExternalSecretResult,
}

impl SearchItem for ExternalSecretSearchItem {
    type Action = ExternalSecretSearchItemAction;

    fn render_icon(
        &self,
        _highlight_state: ItemHighlightState,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        Container::new(
            ConstrainedBox::new(
                self.external_secret
                    .icon()
                    .to_warpui_icon(appearance.theme().active_ui_text_color())
                    .finish(),
            )
            .with_width(ICON_SIZE)
            .with_height(ICON_SIZE)
            .finish(),
        )
        .with_margin_right(12.)
        .finish()
    }

    fn icon_location(&self, appearance: &Appearance) -> IconLocation {
        let name_size = styles::name_font_size(appearance) * appearance.line_height_ratio();
        IconLocation::Top {
            margin_top: name_size - ICON_SIZE,
        }
    }

    fn render_item(
        &self,
        highlight_state: ItemHighlightState,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let secret = &self.external_secret;
        let appearance = Appearance::as_ref(app);

        let mut name_text = Text::new_inline(
            secret.get_display_name(),
            appearance.ui_font_family(),
            styles::name_font_size(appearance),
        )
        .with_color(highlight_state.main_text_fill(appearance).into_solid());

        if let Some(name_match_result) = &self.fuzzy_matched_secret.name_match_result {
            name_text = name_text.with_single_highlight(
                Highlight::new().with_properties(Properties::default().weight(Weight::Bold)),
                name_match_result.matched_indices.clone(),
            );
        }

        Container::new(name_text.finish())
            .with_padding_top(2.)
            .with_padding_bottom(2.)
            .finish()
    }

    fn render_details(&self, _ctx: &AppContext) -> Option<Box<dyn Element>> {
        None
    }

    fn score(&self) -> OrderedFloat<f64> {
        self.fuzzy_matched_secret.score()
    }

    fn accept_result(&self) -> ExternalSecretSearchItemAction {
        ExternalSecretSearchItemAction::AcceptSecret(self.external_secret.clone())
    }

    fn execute_result(&self) -> ExternalSecretSearchItemAction {
        self.accept_result()
    }

    fn accessibility_label(&self) -> String {
        format!("Secret: {}", &self.external_secret.get_display_name())
    }
}
