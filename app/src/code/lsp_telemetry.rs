use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use strum_macros::{EnumDiscriminants, EnumIter};
use warp_core::telemetry::{EnablementState, TelemetryEvent, TelemetryEventDesc};

/// The source from which the user enabled an LSP server.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum LspEnablementSource {
    #[serde(rename = "init_flow")]
    InitFlow,
    #[serde(rename = "footer_button")]
    FooterButton,
    #[serde(rename = "settings")]
    Settings,
}

/// The control action the user performed on an LSP server.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum LspControlActionType {
    #[serde(rename = "open_logs")]
    OpenLogs,
    #[serde(rename = "restart")]
    Restart,
    #[serde(rename = "stop")]
    Stop,
    #[serde(rename = "start")]
    Start,
    #[serde(rename = "restart_all")]
    RestartAll,
    #[serde(rename = "stop_all")]
    StopAll,
}

#[derive(Debug, EnumDiscriminants)]
#[strum_discriminants(derive(EnumIter))]
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
pub enum LspTelemetryEvent {
    /// User enabled an LSP server for a workspace.
    ServerEnabled {
        server_type: String,
        source: LspEnablementSource,
        needed_install: bool,
    },
    /// User skipped LSP enablement during /init.
    ServerEnablementSkipped,
    /// An LSP server installation finished (success or failure).
    ServerInstallCompleted { server_type: String, success: bool },
    /// User removed an LSP server.
    ServerRemoved {
        server_type: String,
        source: LspEnablementSource,
    },
    /// Hover tooltip displayed with content.
    HoverShown {
        server_type: String,
        had_content: bool,
        had_diagnostics: bool,
    },
    /// User triggered goto definition.
    GotoDefinition {
        server_type: String,
        had_result: bool,
    },
    /// Find references card displayed.
    FindReferencesShown {
        server_type: String,
        num_references: usize,
    },
    /// User performed an LSP control action from the footer menu.
    ControlAction {
        action: LspControlActionType,
        server_type: Option<String>,
    },
    /// Server successfully started and is available.
    ServerStarted { server_type: String },
    /// Server failed to start.
    ServerFailed { server_type: String },
}

impl TelemetryEvent for LspTelemetryEvent {
    fn name(&self) -> &'static str {
        LspTelemetryEventDiscriminants::from(self).name()
    }

    fn payload(&self) -> Option<Value> {
        match self {
            LspTelemetryEvent::ServerEnabled {
                server_type,
                source,
                needed_install,
            } => Some(json!({
                "server_type": server_type,
                "source": source,
                "needed_install": needed_install,
            })),
            LspTelemetryEvent::ServerEnablementSkipped => None,
            LspTelemetryEvent::ServerInstallCompleted {
                server_type,
                success,
            } => Some(json!({
                "server_type": server_type,
                "success": success,
            })),
            LspTelemetryEvent::ServerRemoved {
                server_type,
                source,
            } => Some(json!({
                "server_type": server_type,
                "source": source,
            })),
            LspTelemetryEvent::HoverShown {
                server_type,
                had_content,
                had_diagnostics,
            } => Some(json!({
                "server_type": server_type,
                "had_content": had_content,
                "had_diagnostics": had_diagnostics,
            })),
            LspTelemetryEvent::GotoDefinition {
                server_type,
                had_result,
            } => Some(json!({
                "server_type": server_type,
                "had_result": had_result,
            })),
            LspTelemetryEvent::FindReferencesShown {
                server_type,
                num_references,
            } => Some(json!({
                "server_type": server_type,
                "num_references": num_references,
            })),
            LspTelemetryEvent::ControlAction {
                action,
                server_type,
            } => Some(json!({
                "action": action,
                "server_type": server_type,
            })),
            LspTelemetryEvent::ServerStarted { server_type } => Some(json!({
                "server_type": server_type,
            })),
            LspTelemetryEvent::ServerFailed { server_type } => Some(json!({
                "server_type": server_type,
            })),
        }
    }

    fn description(&self) -> &'static str {
        LspTelemetryEventDiscriminants::from(self).description()
    }

    fn enablement_state(&self) -> EnablementState {
        LspTelemetryEventDiscriminants::from(self).enablement_state()
    }

    fn contains_ugc(&self) -> bool {
        false
    }

    fn event_descs() -> impl Iterator<Item = Box<dyn TelemetryEventDesc>> {
        warp_core::telemetry::enum_events::<Self>()
    }
}

impl TelemetryEventDesc for LspTelemetryEventDiscriminants {
    fn name(&self) -> &'static str {
        match self {
            Self::ServerEnabled => "Lsp.ServerEnabled",
            Self::ServerEnablementSkipped => "Lsp.ServerEnablementSkipped",
            Self::ServerInstallCompleted => "Lsp.ServerInstallCompleted",
            Self::ServerRemoved => "Lsp.ServerRemoved",
            Self::HoverShown => "Lsp.HoverShown",
            Self::GotoDefinition => "Lsp.GotoDefinition",
            Self::FindReferencesShown => "Lsp.FindReferencesShown",
            Self::ControlAction => "Lsp.ControlAction",
            Self::ServerStarted => "Lsp.ServerStarted",
            Self::ServerFailed => "Lsp.ServerFailed",
        }
    }

    fn description(&self) -> &'static str {
        match self {
            Self::ServerEnabled => "User enabled an LSP server for a workspace",
            Self::ServerEnablementSkipped => "User skipped LSP enablement during /init",
            Self::ServerInstallCompleted => "An LSP server installation finished",
            Self::ServerRemoved => "User removed an LSP server",
            Self::HoverShown => "Hover tooltip displayed with LSP content or diagnostics",
            Self::GotoDefinition => "User triggered goto definition via LSP",
            Self::FindReferencesShown => "Find references card displayed via LSP",
            Self::ControlAction => "User performed an LSP control action from the footer menu",
            Self::ServerStarted => "LSP server successfully started and is available",
            Self::ServerFailed => "LSP server failed to start",
        }
    }

    fn enablement_state(&self) -> EnablementState {
        EnablementState::Always
    }
}

warp_core::register_telemetry_event!(LspTelemetryEvent);
