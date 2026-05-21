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
