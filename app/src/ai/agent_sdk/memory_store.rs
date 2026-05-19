use anyhow::Result;
use comfy_table::Cell;
use serde::Serialize;
use warp_cli::{
    agent::OutputFormat,
    memory_store::{CreateMemoryArgs, ListMemoriesArgs, MemoryStoreCommand},
    GlobalOptions,
};
use warpui::{platform::TerminationMode, AppContext, ModelContext, SingletonEntity};

use crate::{
    ai::agent_sdk::output::{self, TableFormat},
    server::{
        server_api::ai::AIClient,
        server_api::{
            ai::{
                CreateMemoryRequest, CreateMemoryResponse, MemoryItem, MemorySource,
                MemoryStoreItem,
            },
            ServerApiProvider,
        },
    },
    util::time_format::format_approx_duration_from_now_utc,
};

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
        MemoryStoreCommand::ListMemories(args) => {
            runner.update(ctx, |runner, ctx| {
                runner.list_memories(global_options.output_format, args, ctx)
            });
            Ok(())
        }
        MemoryStoreCommand::CreateMemory(args) => {
            runner.update(ctx, |runner, ctx| {
                runner.create_memory(global_options.output_format, args, ctx)
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
