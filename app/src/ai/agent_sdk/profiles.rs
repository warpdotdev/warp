use comfy_table::Cell;
use serde::Serialize;
use warp_cli::agent::AgentProfileCommand;
use warp_cli::GlobalOptions;
use warpui::{AppContext, ModelContext, SingletonEntity};

use crate::ai::agent_sdk::output::{self, TableFormat};
use crate::ai::execution_profiles::profiles::AIExecutionProfilesModel;
use crate::cloud_object::model::generic_string_model::StringModel;
use crate::server::cloud_objects::update_manager::UpdateManager;
use crate::server::ids::SyncId;

/// Handle Agent Profile-related CLI commands.
pub fn run(
    ctx: &mut AppContext,
    global_options: GlobalOptions,
    command: AgentProfileCommand,
) -> anyhow::Result<()> {
    let runner = ctx.add_singleton_model(|_ctx| ProfilesCommandRunner);
    match command {
        AgentProfileCommand::List => {
            runner.update(ctx, |runner, ctx| runner.list(global_options, ctx));
            Ok(())
        }
    }
}

/// Singleton model that runs async work for profile CLI commands.
struct ProfilesCommandRunner;

impl ProfilesCommandRunner {
    fn list(&self, global_options: GlobalOptions, ctx: &mut ModelContext<Self>) {
        // Ensure initial cloud sync completes so profiles from the server are available.
        let initial_sync = UpdateManager::as_ref(ctx).initial_load_complete();

        ctx.spawn(initial_sync, move |_, _, ctx| {
            let profiles_model = AIExecutionProfilesModel::as_ref(ctx);

            let profile_ids = profiles_model.get_all_profile_ids();

            let profiles: Vec<_> = profile_ids
                .iter()
                .flat_map(|id| profiles_model.get_profile_by_id(*id, ctx))
                .map(|profile| {
                    let name = profile.data().display_name().to_string();
                    let id = match profile.sync_id() {
                        Some(SyncId::ServerId(server_id)) => server_id.to_string(),
                        _ => "Unsynced".to_string(),
                    };
                    ProfileInfo { id, name }
                })
                .collect();

            output::print_list(profiles, global_options.output_format);

            ctx.terminate_app(warpui::platform::TerminationMode::ForceTerminate, None);
        });
    }
}

impl warpui::Entity for ProfilesCommandRunner {
    type Event = ();
}
impl SingletonEntity for ProfilesCommandRunner {}

/// Profile information that's shown in the `list` command.
#[derive(Serialize)]
struct ProfileInfo {
    id: String,
    name: String,
}

impl TableFormat for ProfileInfo {
    fn header() -> Vec<Cell> {
        vec![Cell::new("ID"), Cell::new("Name")]
    }

    fn row(&self) -> Vec<Cell> {
        vec![Cell::new(&self.id), Cell::new(&self.name)]
    }
}
