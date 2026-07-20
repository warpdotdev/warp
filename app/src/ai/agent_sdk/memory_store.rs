use anyhow::Result;
use comfy_table::Cell;
use serde::Serialize;
use warp_cli::GlobalOptions;
use warp_cli::agent::OutputFormat;
use warp_cli::memory_store::{
    CreateMemoryArgs, DeleteMemoryArgs, GetStoreArgs, ListMemoriesArgs, ListStoreAgentsArgs,
    ListVersionsArgs, MemoryCommand, MemoryStoreCommand, UpdateMemoryArgs, UpdateStoreArgs,
};
use warpui::platform::TerminationMode;
use warpui::{AppContext, ModelContext, SingletonEntity};

use crate::ai::agent_sdk::output::{self, TableFormat};
use crate::server::server_api::ServerApiProvider;
use crate::server::server_api::ai::{
    AIClient, AgentAttachmentItem, CreateMemoryRequest, CreateMemoryResponse, MemoryItem,
    MemorySource, MemoryStoreItem, MemoryVersionItem, UpdateMemoryRequest, UpdateMemoryResponse,
    UpdateMemoryStoreRequest,
};
use crate::util::time_format::format_approx_duration_from_now_utc;

/// Run memory-store related commands.
pub fn run(
    ctx: &mut AppContext,
    global_options: GlobalOptions,
    command: MemoryStoreCommand,
) -> Result<()> {
    let runner = ctx.add_singleton_model(|_ctx| MemoryStoreCommandRunner);
    match command {
        MemoryStoreCommand::List => {
            runner.update(ctx, |runner, ctx| {
                runner.list_stores(global_options.output_format, ctx)
            });
            Ok(())
        }
        MemoryStoreCommand::Get(args) => {
            runner.update(ctx, |runner, ctx| {
                runner.get_store(global_options.output_format, args, ctx)
            });
            Ok(())
        }
        MemoryStoreCommand::Update(args) => {
            runner.update(ctx, |runner, ctx| {
                runner.update_store(global_options.output_format, args, ctx)
            });
            Ok(())
        }
        MemoryStoreCommand::ListStoreAgents(args) => {
            runner.update(ctx, |runner, ctx| {
                runner.list_store_agents(global_options.output_format, args, ctx)
            });
            Ok(())
        }
    }
}

/// Run memory related commands.
pub fn run_memory(
    ctx: &mut AppContext,
    global_options: GlobalOptions,
    command: MemoryCommand,
) -> Result<()> {
    let runner = ctx.add_singleton_model(|_ctx| MemoryStoreCommandRunner);
    match command {
        MemoryCommand::List(args) => {
            runner.update(ctx, |runner, ctx| {
                runner.list_memories(global_options.output_format, args, ctx)
            });
            Ok(())
        }
        MemoryCommand::Create(args) => {
            runner.update(ctx, |runner, ctx| {
                runner.create_memory(global_options.output_format, args, ctx)
            });
            Ok(())
        }
        MemoryCommand::Update(args) => {
            runner.update(ctx, |runner, ctx| {
                runner.update_memory(global_options.output_format, args, ctx)
            });
            Ok(())
        }
        MemoryCommand::Delete(args) => {
            runner.update(ctx, |runner, ctx| {
                runner.delete_memory(global_options.output_format, args, ctx)
            });
            Ok(())
        }
        MemoryCommand::Versions(args) => {
            runner.update(ctx, |runner, ctx| {
                runner.list_versions(global_options.output_format, args, ctx)
            });
            Ok(())
        }
    }
}

struct MemoryStoreCommandRunner;

impl MemoryStoreCommandRunner {
    fn list_stores(&self, output_format: OutputFormat, ctx: &mut ModelContext<Self>) {
        let server_api = ServerApiProvider::as_ref(ctx).get();

        ctx.spawn(
            async move {
                let stores = server_api.list_memory_stores().await?;
                print_memory_stores(stores, output_format);
                Ok(())
            },
            |_, result: Result<()>, ctx| finish_command(result, ctx),
        );
    }

    fn list_memories(
        &self,
        output_format: OutputFormat,
        args: ListMemoriesArgs,
        ctx: &mut ModelContext<Self>,
    ) {
        let server_api = ServerApiProvider::as_ref(ctx).get();

        ctx.spawn(
            async move {
                let memories = server_api
                    .list_memory_store_memories(&args.store_uid)
                    .await?;
                print_memories(memories, output_format);
                Ok(())
            },
            |_, result: Result<()>, ctx| finish_command(result, ctx),
        );
    }

    fn create_memory(
        &self,
        output_format: OutputFormat,
        args: CreateMemoryArgs,
        ctx: &mut ModelContext<Self>,
    ) {
        let server_api = ServerApiProvider::as_ref(ctx).get();

        ctx.spawn(
            async move {
                let request = CreateMemoryRequest {
                    content: args.content,
                    version: args.version,
                    source: MemorySource::Manual,
                    source_id: None,
                    reason: args.reason,
                };
                let response = server_api
                    .create_memory_store_memory(&args.store_uid, request)
                    .await?;
                print_create_memory_response(response, output_format)?;
                Ok(())
            },
            |_, result: Result<()>, ctx| finish_command(result, ctx),
        );
    }

    fn get_store(
        &self,
        output_format: OutputFormat,
        args: GetStoreArgs,
        ctx: &mut ModelContext<Self>,
    ) {
        let server_api = ServerApiProvider::as_ref(ctx).get();
        ctx.spawn(
            async move {
                let store = server_api.get_memory_store(&args.store_uid).await?;
                match output_format {
                    OutputFormat::Json => output::write_json(&store, std::io::stdout())?,
                    OutputFormat::Ndjson => output::write_json_line(&store, std::io::stdout())?,
                    OutputFormat::Pretty | OutputFormat::Text => {
                        output::print_list([store], output_format);
                    }
                }
                Ok(())
            },
            |_, result: Result<()>, ctx| finish_command(result, ctx),
        );
    }

    fn update_store(
        &self,
        output_format: OutputFormat,
        args: UpdateStoreArgs,
        ctx: &mut ModelContext<Self>,
    ) {
        let server_api = ServerApiProvider::as_ref(ctx).get();
        ctx.spawn(
            async move {
                let request = UpdateMemoryStoreRequest {
                    description: args.description,
                };
                let store = server_api
                    .update_memory_store(&args.store_uid, request)
                    .await?;
                match output_format {
                    OutputFormat::Json => output::write_json(&store, std::io::stdout())?,
                    OutputFormat::Ndjson => output::write_json_line(&store, std::io::stdout())?,
                    OutputFormat::Pretty | OutputFormat::Text => {
                        println!("Updated store {}.", store.uid);
                        output::print_list([store], output_format);
                    }
                }
                Ok(())
            },
            |_, result: Result<()>, ctx| finish_command(result, ctx),
        );
    }

    fn list_store_agents(
        &self,
        output_format: OutputFormat,
        args: ListStoreAgentsArgs,
        ctx: &mut ModelContext<Self>,
    ) {
        let server_api = ServerApiProvider::as_ref(ctx).get();
        ctx.spawn(
            async move {
                let agents = server_api.list_memory_store_agents(&args.store_uid).await?;
                if agents.is_empty()
                    && matches!(output_format, OutputFormat::Pretty | OutputFormat::Text)
                {
                    println!("No agents attached to this store.");
                    return Ok(());
                }
                output::print_list(agents, output_format);
                Ok(())
            },
            |_, result: Result<()>, ctx| finish_command(result, ctx),
        );
    }

    fn list_versions(
        &self,
        output_format: OutputFormat,
        args: ListVersionsArgs,
        ctx: &mut ModelContext<Self>,
    ) {
        let server_api = ServerApiProvider::as_ref(ctx).get();
        ctx.spawn(
            async move {
                let versions = server_api
                    .list_memory_versions(&args.store_uid, &args.memory_uid)
                    .await?;
                if versions.is_empty()
                    && matches!(output_format, OutputFormat::Pretty | OutputFormat::Text)
                {
                    println!("No versions found.");
                    return Ok(());
                }
                output::print_list(versions, output_format);
                Ok(())
            },
            |_, result: Result<()>, ctx| finish_command(result, ctx),
        );
    }

    fn update_memory(
        &self,
        output_format: OutputFormat,
        args: UpdateMemoryArgs,
        ctx: &mut ModelContext<Self>,
    ) {
        let server_api = ServerApiProvider::as_ref(ctx).get();

        ctx.spawn(
            async move {
                let request = UpdateMemoryRequest {
                    content: args.content,
                    version: args.version,
                    reason: args.reason,
                };
                let response = server_api
                    .update_memory_store_memory(&args.store_uid, &args.memory_uid, request)
                    .await?;
                print_update_memory_response(response, output_format)?;
                Ok(())
            },
            |_, result: Result<()>, ctx| finish_command(result, ctx),
        );
    }

    fn delete_memory(
        &self,
        output_format: OutputFormat,
        args: DeleteMemoryArgs,
        ctx: &mut ModelContext<Self>,
    ) {
        let server_api = ServerApiProvider::as_ref(ctx).get();

        ctx.spawn(
            async move {
                server_api
                    .delete_memory_store_memory(&args.store_uid, &args.memory_uid)
                    .await?;
                let output = DeleteMemoryOutput {
                    memory_uid: args.memory_uid.clone(),
                };
                match output_format {
                    OutputFormat::Json => output::write_json(&output, std::io::stdout())?,
                    OutputFormat::Ndjson => output::write_json_line(&output, std::io::stdout())?,
                    OutputFormat::Pretty | OutputFormat::Text => {
                        println!("Deleted memory {}.", args.memory_uid);
                    }
                }
                Ok(())
            },
            |_, result: Result<()>, ctx| finish_command(result, ctx),
        );
    }
}

impl warpui::Entity for MemoryStoreCommandRunner {
    type Event = ();
}

impl SingletonEntity for MemoryStoreCommandRunner {}

impl TableFormat for MemoryStoreItem {
    fn header() -> Vec<Cell> {
        vec![
            Cell::new("UID"),
            Cell::new("Owner Type"),
            Cell::new("Owner UID"),
            Cell::new("Description"),
            Cell::new("Created"),
            Cell::new("Updated"),
        ]
    }

    fn row(&self) -> Vec<Cell> {
        vec![
            Cell::new(&self.uid),
            Cell::new(&self.owner_type),
            Cell::new(&self.owner_uid),
            Cell::new(self.description.as_deref().unwrap_or("")),
            Cell::new(format_approx_duration_from_now_utc(self.created_at)),
            Cell::new(format_approx_duration_from_now_utc(self.updated_at)),
        ]
    }
}

impl TableFormat for MemoryVersionItem {
    fn header() -> Vec<Cell> {
        vec![
            Cell::new("UID"),
            Cell::new("Version"),
            Cell::new("Content"),
            Cell::new("Reason"),
            Cell::new("Created"),
        ]
    }

    fn row(&self) -> Vec<Cell> {
        vec![
            Cell::new(&self.uid),
            Cell::new(&self.version),
            Cell::new(&self.content),
            Cell::new(self.reason.as_deref().unwrap_or("")),
            Cell::new(format_approx_duration_from_now_utc(self.created_at)),
        ]
    }
}

impl TableFormat for AgentAttachmentItem {
    fn header() -> Vec<Cell> {
        vec![
            Cell::new("UID"),
            Cell::new("Name"),
            Cell::new("Access"),
            Cell::new("Instructions"),
        ]
    }

    fn row(&self) -> Vec<Cell> {
        vec![
            Cell::new(&self.uid),
            Cell::new(&self.name),
            Cell::new(&self.access),
            Cell::new(&self.instructions),
        ]
    }
}

impl TableFormat for MemoryItem {
    fn header() -> Vec<Cell> {
        vec![
            Cell::new("UID"),
            Cell::new("Version"),
            Cell::new("Source"),
            Cell::new("Content"),
            Cell::new("Created"),
            Cell::new("Updated"),
        ]
    }

    fn row(&self) -> Vec<Cell> {
        vec![
            Cell::new(&self.uid),
            Cell::new(&self.version_id),
            Cell::new(&self.source),
            Cell::new(&self.content),
            Cell::new(format_approx_duration_from_now_utc(self.created_at)),
            Cell::new(format_approx_duration_from_now_utc(self.updated_at)),
        ]
    }
}

#[derive(Serialize)]
struct DeleteMemoryOutput {
    memory_uid: String,
}

#[derive(Serialize)]
struct CreateMemoryOutput {
    memory_id: String,
    version_id: String,
}

impl From<CreateMemoryResponse> for CreateMemoryOutput {
    fn from(response: CreateMemoryResponse) -> Self {
        Self {
            memory_id: response.memory_id,
            version_id: response.version_id,
        }
    }
}

impl TableFormat for CreateMemoryOutput {
    fn header() -> Vec<Cell> {
        vec![Cell::new("Memory ID"), Cell::new("Version ID")]
    }

    fn row(&self) -> Vec<Cell> {
        vec![Cell::new(&self.memory_id), Cell::new(&self.version_id)]
    }
}

fn print_memory_stores(stores: Vec<MemoryStoreItem>, output_format: OutputFormat) {
    if stores.is_empty() && matches!(output_format, OutputFormat::Pretty | OutputFormat::Text) {
        println!("No memory stores found.");
        return;
    }
    output::print_list(stores, output_format);
}

fn print_memories(memories: Vec<MemoryItem>, output_format: OutputFormat) {
    if memories.is_empty() && matches!(output_format, OutputFormat::Pretty | OutputFormat::Text) {
        println!("No memories found.");
        return;
    }
    output::print_list(memories, output_format);
}

#[derive(Serialize)]
struct UpdateMemoryOutput {
    memory_id: String,
    version_id: String,
}

impl From<UpdateMemoryResponse> for UpdateMemoryOutput {
    fn from(response: UpdateMemoryResponse) -> Self {
        Self {
            memory_id: response.memory_id,
            version_id: response.version_id,
        }
    }
}

impl TableFormat for UpdateMemoryOutput {
    fn header() -> Vec<Cell> {
        vec![Cell::new("Memory ID"), Cell::new("Version ID")]
    }

    fn row(&self) -> Vec<Cell> {
        vec![Cell::new(&self.memory_id), Cell::new(&self.version_id)]
    }
}

fn print_update_memory_response(
    response: UpdateMemoryResponse,
    output_format: OutputFormat,
) -> Result<()> {
    let output = UpdateMemoryOutput::from(response);
    match output_format {
        OutputFormat::Json => output::write_json(&output, std::io::stdout())?,
        OutputFormat::Ndjson => output::write_json_line(&output, std::io::stdout())?,
        OutputFormat::Pretty | OutputFormat::Text => {
            println!("Updated memory {}.", output.memory_id);
            output::print_list([output], output_format);
        }
    }
    Ok(())
}

fn print_create_memory_response(
    response: CreateMemoryResponse,
    output_format: OutputFormat,
) -> Result<()> {
    let output = CreateMemoryOutput::from(response);
    match output_format {
        OutputFormat::Json => output::write_json(&output, std::io::stdout())?,
        OutputFormat::Ndjson => output::write_json_line(&output, std::io::stdout())?,
        OutputFormat::Pretty | OutputFormat::Text => {
            println!("Created memory {}.", output.memory_id);
            output::print_list([output], output_format);
        }
    }
    Ok(())
}

fn finish_command(result: Result<()>, ctx: &mut ModelContext<MemoryStoreCommandRunner>) {
    match result {
        Ok(()) => ctx.terminate_app(TerminationMode::ForceTerminate, None),
        Err(err) => super::report_fatal_error(err, ctx),
    }
}
