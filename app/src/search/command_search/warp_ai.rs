use std::any::Any;
use std::sync::Arc;

use async_trait::async_trait;
use itertools::Itertools;
use ordered_float::OrderedFloat;
use serde_json::json;
use warp_core::ui::builder;
use warpui::elements::{ConstrainedBox, Container, Text};
use warpui::{AppContext, Element, SingletonEntity};

use super::workflows::{WorkflowIdentity, WorkflowSearchItem};
use crate::ai::AIRequestUsageModel;
use crate::ai_assistant::execution_context::WarpAiExecutionContext;
use crate::ai_assistant::{AI_ASSISTANT_LOGO_COLOR, GenerateCommandsFromNaturalLanguageError};
use crate::appearance::Appearance;
use crate::features::FeatureFlag;
use crate::search::command_search::searcher::CommandSearchItemAction;
use crate::search::data_source::{Query, QueryResult};
use crate::search::item::SearchItem;
use crate::search::mixer::{
    AsyncDataSource, BoxFuture, DataSourceRunError, DataSourceRunErrorWrapper, SyncDataSource,
};
use crate::search::result_renderer::ItemHighlightState;
use crate::search::workflows::fuzzy_match::FuzzyMatchWorkflowResult;
use crate::server::server_api::ai::AIClient;
use crate::themes::theme::Blend;
use crate::ui_components::icons::Icon as UIIcon;
use crate::util::color::{ContrastingColor, MinimumAllowedContrast};
use crate::workflows::{AIWorkflowOrigin, WorkflowSource, WorkflowType};

const OPEN_WARP_AI_ITEM_BODY_TEXT: &str = "Ask Warp AI for command suggestions";
const TRANSLATE_WITH_WARP_AI_ITEM_BODY_TEXT: &str = "Translate into shell command using Warp AI";

#[derive(Clone, Debug)]
pub enum WarpAISearchItem {
    /// Translates the query within command search.
    Translate,

    /// Opens WarpAI with the query.
    Open,
}

impl WarpAISearchItem {
    fn item_body_text(&self) -> &'static str {
        match self {
            WarpAISearchItem::Translate => TRANSLATE_WITH_WARP_AI_ITEM_BODY_TEXT,
            WarpAISearchItem::Open => OPEN_WARP_AI_ITEM_BODY_TEXT,
        }
    }
}

impl SearchItem for WarpAISearchItem {
    type Action = CommandSearchItemAction;

    fn render_icon(
        &self,
        highlight_state: ItemHighlightState,
        appearance: &Appearance,
    ) -> Box<dyn Element> {
        // Since the Warp AI logo color is hardcoded, let's find the best
        // contrasting color depending on the user's theme and the item's selected state.
        let command_search_background = appearance.theme().surface_1();
        let item_background_color = match highlight_state.container_background_fill(appearance) {
            None => command_search_background,
            Some(highlight) => command_search_background.blend(&highlight),
        };

        let icon = if FeatureFlag::AgentMode.is_enabled() {
            UIIcon::Oz
                .to_warpui_icon(
                    appearance
                        .theme()
                        .main_text_color(appearance.theme().accent()),
                )
                .finish()
        } else {
            let color = (AI_ASSISTANT_LOGO_COLOR).on_background(
                item_background_color.into_solid(),
                MinimumAllowedContrast::NonText,
            );
            UIIcon::AiAssistant.to_warpui_icon(color.into()).finish()
        };

        Container::new(
            ConstrainedBox::new(icon)
                .with_width(styles::icon_size(appearance))
                .with_height(styles::icon_size(appearance))
                .finish(),
        )
        .with_margin_right(8.)
        .finish()
    }

    fn render_item(
        &self,
        highlight_state: ItemHighlightState,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        Text::new_inline(
            self.item_body_text(),
            appearance.monospace_font_family(),
            appearance.monospace_font_size(),
        )
        .autosize_text(builder::MIN_FONT_SIZE)
        .with_color(highlight_state.main_text_fill(appearance).into_solid())
        .finish()
    }

    fn render_details(&self, _: &AppContext) -> Option<Box<dyn Element>> {
        None
    }

    fn score(&self) -> OrderedFloat<f64> {
        // Decided to try using a score of 0 instead of a score of -f32::MAX.
        // This means it's not necessarily the lowest-ranked item, but often is.
        OrderedFloat(0.)
    }

    fn accept_result(&self) -> CommandSearchItemAction {
        match self {
            WarpAISearchItem::Translate => CommandSearchItemAction::TranslateUsingWarpAI,
            WarpAISearchItem::Open => CommandSearchItemAction::OpenWarpAI,
        }
    }

    fn execute_result(&self) -> CommandSearchItemAction {
        match self {
            WarpAISearchItem::Translate => CommandSearchItemAction::TranslateUsingWarpAI,
            WarpAISearchItem::Open => CommandSearchItemAction::OpenWarpAI,
        }
    }

    fn accessibility_label(&self) -> String {
        format!("Warp AI: {}", self.item_body_text())
    }
}

/// The Warp AI data source provides two different types of results:
/// - synchronous: the synchronous result provided by this data source is a
///   single item that opens/translates using Warp AI when selected.
/// - asynchronous: the asynchronous results are AI generated workflows
/// In most cases, the data source should be registered _twice_: once as a sync source
/// and once as an async source. That way, the mixer will treat these as two separate
/// data sources.
pub struct WarpAIDataSource {
    ai_client: Arc<dyn AIClient>,
    ai_execution_context: Option<WarpAiExecutionContext>,
}

impl WarpAIDataSource {
    pub fn new(
        ai_client: Arc<dyn AIClient>,
        ai_execution_context: Option<WarpAiExecutionContext>,
    ) -> Self {
        Self {
            ai_client,
            ai_execution_context,
        }
    }
}

impl SyncDataSource for WarpAIDataSource {
    type Action = CommandSearchItemAction;

    fn run_query(
        &self,
        query: &Query,
        _app: &AppContext,
    ) -> Result<Vec<QueryResult<Self::Action>>, DataSourceRunErrorWrapper> {
        if query.filters.is_empty() {
            Ok(vec![WarpAISearchItem::Translate.into()])
        } else {
            // Since the query matched, the `#` filter must be applied in this case.
            Ok(vec![WarpAISearchItem::Open.into()])
        }
    }
}

#[cfg_attr(not(target_family = "wasm"), async_trait)]
#[cfg_attr(target_family = "wasm", async_trait(?Send))]
impl AsyncDataSource for WarpAIDataSource {
    type Action = CommandSearchItemAction;

    fn run_query(
        &self,
        query: &Query,
        _app: &AppContext,
    ) -> BoxFuture<'static, Result<Vec<QueryResult<Self::Action>>, DataSourceRunErrorWrapper>> {
        let query_text = query.text.clone();
        let ai_execution_context = self.ai_execution_context.clone();
        let ai_client = self.ai_client.clone();

        Box::pin(async move {
            let res = ai_client
                .generate_commands_from_natural_language(query_text, ai_execution_context)
                .await;

            match res {
                Ok(ai_commands) => {
                    // The generated commands already have an inherent order so give
                    // them a dummy Match and reverse the list so that the most plausible
                    // commands are at the end.
                    Ok(ai_commands
                        .into_iter()
                        .map(|ai_command| {
                            WorkflowSearchItem {
                                identity: WorkflowIdentity::Local(Box::new(
                                    WorkflowType::AIGenerated {
                                        workflow: ai_command.into(),
                                        origin: AIWorkflowOrigin::CommandSearch,
                                    },
                                )),
                                source: WorkflowSource::WarpAI,
                                fuzzy_matched_workflow: FuzzyMatchWorkflowResult::no_match(),
                            }
                            .into()
                        })
                        .rev()
                        .collect_vec())
                }
                Err(e) => Err(Box::new(e) as Box<dyn DataSourceRunError>),
            }
        })
    }

    fn on_query_finished(&self, app: &mut AppContext) {
        AIRequestUsageModel::handle(app).update(app, |request_usage_model, ctx| {
            request_usage_model.refresh_request_usage_async(ctx);
        });
    }
}

impl DataSourceRunError for GenerateCommandsFromNaturalLanguageError {
    fn user_facing_error(&self) -> String {
        match self {
            Self::BadPrompt => "No results found. Please try again with a more specific query.",
            Self::AiProviderError => "Something went wrong. Please try again.",
            Self::RateLimited => "Looks like you're out of AI credits. Please try again later.",
            Self::Other => "Something went wrong. Please try again.",
        }
        .to_string()
    }

    fn telemetry_payload(&self) -> serde_json::Value {
        json!(self)
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

mod styles {
    use crate::appearance::Appearance;

    /// Returns the icon size to be used for the 'sparkle' icon in the AI command search result.
    /// The icon appeaars smaller than its size would indicate, so make a bit larger than icons
    /// used for other search result types.
    pub(super) fn icon_size(appearance: &Appearance) -> f32 {
        appearance.monospace_font_size() + 2.
    }
}
