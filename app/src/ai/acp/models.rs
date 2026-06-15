use std::path::PathBuf;

use warp_cli::agent::Harness;

use acpx::RuntimeContext;
use agent_client_protocol as acp;
use anyhow::{anyhow, Context, Result};
use async_process::Command;

use super::{connection::Connection, path_search, registry};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LocalAcpModelInfo {
    pub(crate) id: String,
    pub(crate) name: String,
}

pub(crate) fn default_models_for_harness(harness: Harness) -> Vec<LocalAcpModelInfo> {
    registry::spec_for_harness(harness)
        .into_iter()
        .flat_map(|spec| spec.default_models.iter())
        .map(|model| LocalAcpModelInfo {
            id: (*model).to_string(),
            name: (*model).to_string(),
        })
        .collect()
}

pub(crate) async fn discover_models_for_harness(
    harness: Harness,
) -> Result<Vec<LocalAcpModelInfo>> {
    tokio::task::spawn_blocking(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .context("failed to build local ACP model discovery runtime")?;
        let local_set = tokio::task::LocalSet::new();
        runtime.block_on(local_set.run_until(discover_models_on_local_runtime(harness)))
    })
    .await
    .context("local ACP model discovery runtime task panicked")?
}

async fn discover_models_on_local_runtime(harness: Harness) -> Result<Vec<LocalAcpModelInfo>> {
    let spec = registry::spec_for_harness(harness)
        .ok_or_else(|| anyhow!("{harness} is not an ACP agent"))?;
    let program = path_search::resolve_command(spec.command)
        .with_context(|| format!("{harness} ACP command '{}' was not found", spec.command))?;
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/"));

    let mut command = Command::new(program);
    command.args(spec.args);
    command.current_dir(&cwd);
    command.env("PATH", path_search::augmented_path_env());
    for (key, value) in registry::process_env_for_harness(harness) {
        command.env(key, value);
    }

    let runtime = RuntimeContext::new(|task| {
        tokio::task::spawn_local(task);
    });
    let connection = Connection::spawn(&mut command, &runtime)?;
    let initialize_result = connection
        .initialize(
            acp::InitializeRequest::new(acp::ProtocolVersion::V1).client_info(
                acp::Implementation::new("warp", env!("CARGO_PKG_VERSION")).title("Warp"),
            ),
        )
        .await?;

    if registry::should_auto_authenticate(harness) {
        if let Some(auth_method) = initialize_result.auth_methods.first() {
            connection
                .authenticate(acp::AuthenticateRequest::new(auth_method.id().clone()))
                .await?;
        }
    }

    let session = connection
        .new_session(acp::NewSessionRequest::new(cwd))
        .await?;
    let models = models_from_config_options(session.config_options.as_deref());
    connection.close().await?;
    Ok(models)
}

pub(crate) fn models_from_config_options(
    config_options: Option<&[acp::SessionConfigOption]>,
) -> Vec<LocalAcpModelInfo> {
    let Some(model_config) = model_config_option(config_options) else {
        return Vec::new();
    };
    let acp::SessionConfigKind::Select(select) = &model_config.kind else {
        return Vec::new();
    };
    select_options(&select.options)
        .into_iter()
        .map(|option| LocalAcpModelInfo {
            id: option.value.to_string(),
            name: option.name,
        })
        .collect()
}

pub(crate) fn model_config_option(
    config_options: Option<&[acp::SessionConfigOption]>,
) -> Option<&acp::SessionConfigOption> {
    config_options?.iter().find(|option| {
        option.category == Some(acp::SessionConfigOptionCategory::Model)
            || option.id.to_string().to_ascii_lowercase().contains("model")
            || option.name.to_ascii_lowercase().contains("model")
    })
}

fn select_options(
    options: &acp::SessionConfigSelectOptions,
) -> Vec<acp::SessionConfigSelectOption> {
    match options {
        acp::SessionConfigSelectOptions::Ungrouped(options) => options.clone(),
        acp::SessionConfigSelectOptions::Grouped(groups) => groups
            .iter()
            .flat_map(|group| group.options.iter().cloned())
            .collect(),
        _ => Vec::new(),
    }
}

pub(crate) fn context_window_tokens_for_selection(
    harness: Harness,
    model_id: Option<&str>,
) -> Option<u32> {
    context_window_tokens_for_model(harness, model_id)
}

fn context_window_tokens_for_model(harness: Harness, model_id: Option<&str>) -> Option<u32> {
    match harness {
        Harness::Claude => Some(200_000),
        Harness::Codex => Some(match model_id.unwrap_or_default() {
            "gpt-5.4" | "gpt-5.3" | "gpt-5.2" | "gpt-5" => 400_000,
            _ => 400_000,
        }),
        Harness::Gemini => Some(match model_id.unwrap_or_default() {
            "gemini-2.5-pro" | "gemini-2.5-flash" => 1_000_000,
            _ => 1_000_000,
        }),
        Harness::Cursor => Some(200_000),
        Harness::Devin => Some(200_000),
        _ => None,
    }
}
