use std::collections::BTreeSet;

use crate::ai::agent_sdk::output::{self, TableFormat};
use crate::ai::llms::LLMPreferences;
use comfy_table::Cell;
use serde::Serialize;
use warp_cli::{model::ModelCommand, GlobalOptions};
use warpui::{platform::TerminationMode, AppContext, ModelContext, SingletonEntity};

/// Handle model-related CLI commands.
pub fn run(
    ctx: &mut AppContext,
    global_options: GlobalOptions,
    command: ModelCommand,
) -> anyhow::Result<()> {
    let runner = ctx.add_singleton_model(|_ctx| ModelCommandRunner);
    match command {
        ModelCommand::List => {
            runner.update(ctx, |runner, ctx| runner.list(global_options, ctx));
            Ok(())
        }
    }
}

/// Singleton model for running async work as part of model CLI commands.
struct ModelCommandRunner;

impl ModelCommandRunner {
    fn list(&self, global_options: GlobalOptions, ctx: &mut ModelContext<Self>) {
        let output_format = global_options.output_format;

        // Ensure workspace metadata is refreshed so LLM preferences are up-to-date.
        let refresh_future = super::common::refresh_workspace_metadata(ctx);

        ctx.spawn(refresh_future, move |_, refresh_result, ctx| {
            if refresh_result.is_err() {
                super::report_fatal_error(
                    anyhow::anyhow!("Timed out refreshing workspace metadata"),
                    ctx,
                );
                return;
            }

            let llm_prefs = LLMPreferences::as_ref(ctx);
            let mut ids = BTreeSet::new();
            for info in llm_prefs.get_base_llm_choices_for_agent_mode(ctx) {
                ids.insert(info.id.to_string());
            }

            let items = ids
                .into_iter()
                .map(|id| ModelListItem { id })
                .collect::<Vec<_>>();

            output::print_list(items, output_format);

            ctx.terminate_app(TerminationMode::ForceTerminate, None);
        });
    }
}

impl warpui::Entity for ModelCommandRunner {
    type Event = ();
}

impl SingletonEntity for ModelCommandRunner {}

/// Model information that's shown in the `list` command.
#[derive(Serialize)]
struct ModelListItem {
    id: String,
}

impl TableFormat for ModelListItem {
    fn header() -> Vec<Cell> {
        vec![Cell::new("MODEL ID")]
    }

    fn row(&self) -> Vec<Cell> {
        vec![Cell::new(&self.id)]
    }
}
