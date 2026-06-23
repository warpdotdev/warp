use clap::{Args, Subcommand};

/// Memory-store related subcommands.
#[derive(Debug, Clone, Subcommand)]
pub enum MemoryStoreCommand {
    /// List memory stores.
    List,
    /// List memories in a memory store.
    #[command(name = "list-memories", visible_alias = "memories")]
    ListMemories(ListMemoriesArgs),
    /// Create a manual memory in a memory store.
    #[command(name = "create-memory", visible_alias = "add-memory")]
    CreateMemory(CreateMemoryArgs),
    /// Update a memory in a memory store, creating a new version.
    #[command(name = "update-memory", visible_alias = "edit-memory")]
    UpdateMemory(UpdateMemoryArgs),
    /// Delete a memory from a memory store.
    #[command(name = "delete-memory", visible_alias = "remove-memory")]
    DeleteMemory(DeleteMemoryArgs),
    /// Get details of a single memory store.
    #[command(name = "get-store")]
    GetStore(GetStoreArgs),
    /// Update a memory store's description.
    #[command(name = "update-store", visible_alias = "edit-store")]
    UpdateStore(UpdateStoreArgs),
    /// List agents attached to a memory store.
    #[command(name = "list-store-agents", visible_alias = "store-agents")]
    ListStoreAgents(ListStoreAgentsArgs),
    /// List version history for a memory.
    #[command(name = "list-versions", visible_alias = "versions")]
    ListVersions(ListVersionsArgs),
}

#[derive(Debug, Clone, Args)]
pub struct ListMemoriesArgs {
    /// UID of the memory store.
    pub store_uid: String,
}

#[derive(Debug, Clone, Args)]
pub struct CreateMemoryArgs {
    /// UID of the memory store.
    pub store_uid: String,

    /// Memory content.
    #[arg(long = "content", short = 'c')]
    pub content: String,

    /// Reason for creating this memory.
    #[arg(long = "reason", short = 'r')]
    pub reason: String,

    /// Optional version string for this memory.
    #[arg(long = "version")]
    pub version: Option<String>,
}

#[derive(Debug, Clone, Args)]
pub struct UpdateMemoryArgs {
    /// UID of the memory to update.
    pub memory_uid: String,

    /// UID of the memory store that contains this memory.
    #[arg(long = "store", short = 's')]
    pub store_uid: String,

    /// Updated memory content.
    #[arg(long = "content", short = 'c')]
    pub content: String,

    /// Reason for updating this memory.
    #[arg(long = "reason", short = 'r')]
    pub reason: String,

    /// Optional version label for this update. Server picks a UUID when omitted.
    #[arg(long = "version")]
    pub version: Option<String>,
}

#[derive(Debug, Clone, Args)]
pub struct DeleteMemoryArgs {
    /// UID of the memory to delete.
    pub memory_uid: String,

    /// UID of the memory store that contains this memory.
    #[arg(long = "store", short = 's')]
    pub store_uid: String,
}

#[derive(Debug, Clone, Args)]
pub struct GetStoreArgs {
    /// UID of the memory store.
    pub store_uid: String,
}

#[derive(Debug, Clone, Args)]
pub struct UpdateStoreArgs {
    /// UID of the memory store.
    pub store_uid: String,

    /// Updated description for the memory store. Pass an empty string to clear.
    #[arg(long = "description", short = 'd')]
    pub description: Option<String>,
}

#[derive(Debug, Clone, Args)]
pub struct ListStoreAgentsArgs {
    /// UID of the memory store.
    pub store_uid: String,
}

#[derive(Debug, Clone, Args)]
pub struct ListVersionsArgs {
    /// UID of the memory to inspect.
    pub memory_uid: String,

    /// UID of the memory store that contains this memory.
    #[arg(long = "store", short = 's')]
    pub store_uid: String,
}

impl MemoryStoreCommand {
    pub(crate) fn as_str_for_tracing(&self) -> &'static str {
        match self {
            MemoryStoreCommand::List => "memory-store list",
            MemoryStoreCommand::ListMemories(_) => "memory-store list-memories",
            MemoryStoreCommand::CreateMemory(_) => "memory-store create-memory",
            MemoryStoreCommand::UpdateMemory(_) => "memory-store update-memory",
            MemoryStoreCommand::DeleteMemory(_) => "memory-store delete-memory",
            MemoryStoreCommand::GetStore(_) => "memory-store get-store",
            MemoryStoreCommand::UpdateStore(_) => "memory-store update-store",
            MemoryStoreCommand::ListStoreAgents(_) => "memory-store list-store-agents",
            MemoryStoreCommand::ListVersions(_) => "memory-store list-versions",
        }
    }
}
