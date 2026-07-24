use std::path::PathBuf;

use super::config_diagnostic_display;
use crate::ai::mcp::MCPProvider;
use crate::ai::mcp::file_mcp_watcher::{FileMCPConfigDiagnostic, FileMCPConfigDiagnosticKind};

fn diagnostic(kind: FileMCPConfigDiagnosticKind, message: &str) -> FileMCPConfigDiagnostic {
    FileMCPConfigDiagnostic {
        config_path: PathBuf::from("/tmp/project/.mcp.json"),
        provider: MCPProvider::Warp,
        kind,
        message: message.to_string(),
    }
}

#[test]
fn config_diagnostic_display_identifies_the_provider_and_file() {
    let display = config_diagnostic_display(&diagnostic(
        FileMCPConfigDiagnosticKind::Parse,
        "raw parser detail",
    ));

    assert_eq!(display.heading, "Warp MCP config · /tmp/project/.mcp.json");
}

#[test]
fn config_diagnostic_display_does_not_expose_raw_error_details() {
    let sensitive_details = "token = super-secret-value";

    for kind in [
        FileMCPConfigDiagnosticKind::Read,
        FileMCPConfigDiagnosticKind::Parse,
        FileMCPConfigDiagnosticKind::MissingEnvironmentVariable,
    ] {
        let display = config_diagnostic_display(&diagnostic(kind, sensitive_details));
        assert!(!display.message.contains(sensitive_details));
        assert!(!display.message.contains("super-secret-value"));
    }
}
