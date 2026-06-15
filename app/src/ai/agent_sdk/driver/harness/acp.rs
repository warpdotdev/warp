#![allow(dead_code)]

use std::collections::HashMap;
use std::ffi::OsString;
use std::path::Path;
use std::sync::Arc;

use warp_cli::agent::Harness;
use warp_managed_secrets::ManagedSecretValue;
use warpui::ModelHandle;

use super::{HarnessRunner, ResumePayload, ThirdPartyHarness};
use crate::ai::acp::registry;
use crate::ai::agent_sdk::driver::harness::HarnessKind;
use crate::ai::agent_sdk::driver::terminal::TerminalDriver;
use crate::ai::agent_sdk::driver::AgentDriverError;
use crate::ai::ambient_agents::task::HarnessModelConfig;
use crate::ai::ambient_agents::AmbientAgentTaskId;
use crate::ai::mcp::JSONMCPServer;
use crate::server::server_api::ServerApi;
use crate::terminal::CLIAgent;

#[derive(Debug, Clone)]
pub(crate) struct AcpHarness {
    harness: Harness,
}

impl AcpHarness {
    pub(crate) fn new(harness: Harness) -> Option<Self> {
        registry::is_local_acp_harness(harness).then_some(Self { harness })
    }
}

pub(crate) fn local_acp_harness_kind(harness: Harness) -> Option<HarnessKind> {
    AcpHarness::new(harness).map(|harness| HarnessKind::ThirdParty(Box::new(harness)))
}

#[cfg_attr(not(target_family = "wasm"), async_trait::async_trait)]
#[cfg_attr(target_family = "wasm", async_trait::async_trait(?Send))]
impl ThirdPartyHarness for AcpHarness {
    fn harness(&self) -> Harness {
        self.harness
    }

    fn cli_agent(&self) -> CLIAgent {
        match self.harness {
            Harness::Claude => CLIAgent::Claude,
            Harness::Codex => CLIAgent::Codex,
            Harness::Gemini => CLIAgent::Gemini,
            Harness::Cursor | Harness::Devin => CLIAgent::Unknown,
            Harness::Oz | Harness::OpenCode | Harness::Unknown => CLIAgent::Unknown,
        }
    }

    fn install_docs_url(&self) -> Option<&'static str> {
        registry::spec_for_harness(self.harness).map(|spec| spec.install_url)
    }

    fn validate(&self) -> std::result::Result<(), AgentDriverError> {
        let Some((command, _)) = registry::command_for_harness(self.harness) else {
            return Err(AgentDriverError::InvalidRuntimeState);
        };

        let command = command.to_string_lossy();
        if crate::ai::acp::path_search::resolve_command(&command).is_some() {
            return Ok(());
        }

        let mut reason = format!("'{command}' ACP command not found on your machine.");
        if let Some(url) = self.install_docs_url() {
            reason.push_str(&format!(" Install it first: {url}"));
        }
        Err(AgentDriverError::HarnessSetupFailed {
            harness: self.harness.to_string(),
            reason,
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn build_runner(
        &self,
        _prompt: &str,
        _system_prompt: Option<&str>,
        _resumption_prompt: Option<&str>,
        _context: Option<&str>,
        _working_dir: &Path,
        _task_id: Option<AmbientAgentTaskId>,
        _server_api: Arc<ServerApi>,
        _terminal_driver: ModelHandle<TerminalDriver>,
        _resume: Option<ResumePayload>,
        _resolved_env_vars: &HashMap<OsString, OsString>,
        _resolved_secrets: &HashMap<String, ManagedSecretValue>,
        _resolved_mcp_servers: &HashMap<String, JSONMCPServer>,
        _third_party_harness_model_config: Option<&HarnessModelConfig>,
    ) -> std::result::Result<Box<dyn HarnessRunner>, AgentDriverError> {
        Err(AgentDriverError::InvalidRuntimeState)
    }
}
