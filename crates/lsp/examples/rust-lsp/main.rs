//! LSP crate demonstration showing proper usage of LspService and core LSP functionality.
//!
//! This demo showcases:
//! - LSP server initialization using rust-analyzer
//! - Document lifecycle management (open/close)
//! - Core LSP features: go-to-definition, hover, completion, symbols
//! - Proper shutdown and error handling

use std::env;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use log::LevelFilter;
use lsp::supported_servers::LSPServerType;
use lsp::{LspServerConfig, LspService, LspServiceInitializationResult, spawn_lsp_service};
use lsp_types::Position;
use warp_errors::report_error;
use warpui_core::r#async::Timer;
use warpui_core::r#async::executor::Background;

fn init_logging() {
    let mut base_logger = env_logger::builder();
    base_logger.filter_level(LevelFilter::Info);
    base_logger.parse_default_env();
    base_logger.init();
}

fn find_workspace_root() -> anyhow::Result<PathBuf> {
    let current_dir = env::current_dir()?;

    // Walk up directory tree to find Cargo.toml (workspace root)
    let mut path = current_dir.as_path();
    loop {
        if path.join("Cargo.toml").exists() {
            return Ok(path.to_path_buf());
        }
        match path.parent() {
            Some(parent) => path = parent,
            None => {
                return Err(anyhow::anyhow!(
                    "Could not find workspace root with Cargo.toml"
                ));
            }
        }
    }
}

async fn demo_goto_definition(
    service: &LspService,
    file_path: &Path,
    _content: &str,
) -> anyhow::Result<()> {
    println!("\n=== Testing Go-to-Definition ===");

    // This attempts to target find_workspace_root currently on line 105
    // Note that "lines" are 0-indexed
    let test_position = Position {
        line: 92,
        character: 7,
    };

    println!(
        "Testing go-to-definition at line {}, character {}",
        test_position.line, test_position.character
    );

    let start = Utc::now();

    // Use the text document service instead of direct send_request
    match service
        .text_document()
        .definition(file_path, test_position)
        .await
    {
        Ok(response) => {
            println!("Definition response: {response:?}");
        }
        Err(e) => {
            println!("Error requesting definition: {e}");
        }
    }

    let elapsed = Utc::now() - start;
    println!("Elapsed time for goto-definition: {elapsed:?}");

    Ok(())
}

/// Main demo function
fn main() -> anyhow::Result<()> {
    init_logging();

    println!("Starting LSP Crate Demonstration");
    println!("Using rust-analyzer as the LSP server");

    // === Setup Phase ===

    // Find workspace root for testing
    let workspace_root = find_workspace_root()?;

    log::info!("Workspace root: {}", workspace_root.display());

    let executor = Arc::new(Background::default());
    let executor_clone = executor.clone();

    let task = executor.spawn(async move {
        if let Err(e) = async_main(executor_clone, workspace_root).await {
            report_error!(&e);
            eprintln!("LSP demo failed: {e}");
        }
    });

    warpui_core::r#async::block_on(task)?;
    Ok(())
}

async fn async_main(executor: Arc<Background>, workspace_root: PathBuf) -> anyhow::Result<()> {
    println!("Initializing LSP Server (rust-analyzer)...");

    let config = LspServerConfig::new(
        LSPServerType::RustAnalyzer,
        workspace_root,
        None,
        "warp-dev-example".to_string(),
        Arc::new(http_client::Client::new()),
    );

    let LspServiceInitializationResult {
        service: lsp_service,
        channel: _rx,
    } = spawn_lsp_service(config, executor, None).await?;

    if let Some(capabilities) = lsp_service.server_capabilities() {
        println!("Server capabilities received");
        if capabilities.definition_provider.is_some() {
            println!("   - Go-to-definition supported");
        }
        if capabilities.hover_provider.is_some() {
            println!("   - Hover information supported");
        }
        if capabilities.completion_provider.is_some() {
            println!("   - Code completion supported");
        }
    }

    // Use this main.rs file as our test document
    let test_file = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples/rust-lsp/main.rs");
    let file_content = std::fs::read_to_string(&test_file)?;

    // This ensures rust-analyzer has finished its initial indexing and setup
    println!("Waiting 30 seconds for LSP service to be ready...");
    Timer::after(Duration::from_secs(30)).await;

    println!("Opening document: {}", test_file.display());

    lsp_service
        .text_document()
        .did_open(&test_file, file_content.clone(), 0)
        .await?;

    println!("Running first goto-definition call");
    demo_goto_definition(&lsp_service, &test_file, &file_content).await?;

    println!("Running second goto-definition call");
    demo_goto_definition(&lsp_service, &test_file, &file_content).await?;

    println!("Shutting down LSP service...");

    lsp_service.shutdown().await?;

    Ok(())
}
