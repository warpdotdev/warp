//! Provider command for linking third-party services.
use comfy_table::Cell;
use serde::Serialize;
use warp_cli::provider::{ProviderCommand, ProviderType};
use warp_cli::GlobalOptions;
use warp_core::channel::ChannelState;
use warpui::platform::TerminationMode;
use warpui::{AppContext, ModelContext, SingletonEntity};

use crate::ai::agent_sdk::output::{self, TableFormat};
use crate::workspaces::user_workspaces::UserWorkspaces;

/// Handle provider-related CLI commands.
pub fn run(
    ctx: &mut AppContext,
    global_options: GlobalOptions,
    command: ProviderCommand,
) -> anyhow::Result<()> {
    let runner = ctx.add_singleton_model(|_ctx| ProviderCommandRunner);
    match command {
        ProviderCommand::Setup(args) => runner.update(ctx, |runner, ctx| {
            runner.setup(args.provider_type, args.team, args.personal, ctx)
        }),
        ProviderCommand::List => runner.update(ctx, |runner, ctx| runner.list(global_options, ctx)),
    }
}

/// Singleton model for running provider CLI commands.
struct ProviderCommandRunner;

impl ProviderCommandRunner {
    // This shouldn't need to be done, it's usually done as part of create
    fn setup(
        &self,
        provider_type: ProviderType,
        team: bool,
        personal: bool,
        ctx: &mut ModelContext<Self>,
    ) -> anyhow::Result<()> {
        // Construct the OAuth connect URL
        let server_url = ChannelState::server_root_url();

        let mut use_team_auth = team;
        if !team && !personal {
            if provider_type.allowed_in_team_context()
                && provider_type.allowed_in_personal_context()
            {
                return Err(anyhow::anyhow!(
                    "Provider '{}' must be setup for either a team or personal account",
                    provider_type.slug()
                ));
            }
            use_team_auth = provider_type.allowed_in_team_context();
        } else if personal {
            use_team_auth = false;
        }

        // TODO(bens): initiate the OAuth flow and use the login-less auth URL
        let slug = provider_type.slug();
        let url = if use_team_auth {
            let team_uid = match UserWorkspaces::as_ref(ctx).current_team_uid() {
                Some(uid) => uid,
                None => {
                    return Err(anyhow::anyhow!("User is not on a team"));
                }
            };
            format!("{server_url}/oauth/connect/{slug}?principalType=team&principalId={team_uid}")
        } else {
            format!("{server_url}/oauth/connect/{slug}")
        };

        println!("To authenticate {slug}, open this URL in your browser: {url}");

        // Open the URL in the default browser
        ctx.open_url(&url);

        // TODO(bens): poll/subscribe until connection is created

        ctx.terminate_app(TerminationMode::ForceTerminate, None);

        Ok(())
    }

    fn list(
        &self,
        global_options: GlobalOptions,
        ctx: &mut ModelContext<Self>,
    ) -> anyhow::Result<()> {
        let providers = vec![ProviderType::Linear, ProviderType::Slack];

        let provider_infos: Vec<_> = providers
            .into_iter()
            .map(|provider| {
                let name = provider.name();
                let slug = provider.slug();
                let mut allowed_for = Vec::new();

                if provider.allowed_in_personal_context() {
                    allowed_for.push("personal");
                }
                if provider.allowed_in_team_context() {
                    allowed_for.push("team");
                }

                let allowed_str = allowed_for.join(", ");
                let status = "❌ Not Connected".to_string(); // TODO(bens): get this from gql

                ProviderInfo {
                    name,
                    slug,
                    allowed_for: allowed_str,
                    status,
                }
            })
            .collect();

        output::print_list(provider_infos, global_options.output_format);

        ctx.terminate_app(TerminationMode::ForceTerminate, None);

        Ok(())
    }
}

impl warpui::Entity for ProviderCommandRunner {
    type Event = ();
}
impl SingletonEntity for ProviderCommandRunner {}

/// Provider information that's shown in the `list` command.
#[derive(Serialize)]
struct ProviderInfo {
    name: String,
    slug: String,
    allowed_for: String,
    status: String,
}

impl TableFormat for ProviderInfo {
    fn header() -> Vec<Cell> {
        vec![
            Cell::new("NAME"),
            Cell::new("SLUG"),
            Cell::new("ALLOWED FOR"),
            Cell::new("STATUS"),
        ]
    }

    fn row(&self) -> Vec<Cell> {
        vec![
            Cell::new(&self.name),
            Cell::new(&self.slug),
            Cell::new(&self.allowed_for),
            Cell::new(&self.status),
        ]
    }
}
