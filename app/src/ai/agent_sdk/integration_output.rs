use chrono::{DateTime, Utc};
use comfy_table::presets::UTF8_FULL;
use comfy_table::{Cell, Table};
use serde::Serialize;
use serde_json::{Map, Value};
use warp_cli::agent::OutputFormat;
use warp_graphql::queries::get_simple_integrations::{
    ListedSimpleIntegrationConfig, SimpleIntegration, SimpleIntegrationConnectionStatus,
    SimpleIntegrationsOutput,
};

use crate::ai::agent_sdk::output::{self, TableFormat};
use crate::util::time_format::format_approx_duration_from_now_utc;

const MAX_LINE_WIDTH: usize = 90;

/// Print simple integrations.
pub fn print_integrations(graphql_output: &SimpleIntegrationsOutput, output_format: OutputFormat) {
    if let Some(message) = &graphql_output.message {
        eprintln!("{message}");
        return;
    }

    let integrations = &graphql_output.integrations;

    if integrations.is_empty() {
        println!("No integrations found.");
        return;
    }

    match output_format {
        OutputFormat::Json | OutputFormat::Ndjson => {
            // Convert to serializable format and use common output utilities
            let integration_infos: Vec<IntegrationInfo> = integrations
                .iter()
                .map(IntegrationInfo::from_graphql)
                .collect();
            output::print_list(integration_infos, output_format);
        }
        OutputFormat::Pretty | OutputFormat::Text => {
            // Use the existing card-style layout for pretty/text output
            if integrations.len() == 1 {
                println!("\nIntegration:");
            } else {
                println!("\nIntegrations:");
            }

            for integration in integrations {
                print_integration_card(integration);
            }
        }
    }
}

fn render_labeled_wrapped_lines(label: &str, lines: &[String], width: usize) -> String {
    let indent = " ".repeat(label.len() + 2); // align under "{label}: "
    let mut out = String::new();

    for (idx, line) in lines.iter().enumerate() {
        let wrapped = crate::ai::agent_sdk::text_layout::word_wrap(line, width);
        for (widx, wline) in wrapped.iter().enumerate() {
            if !out.is_empty() {
                out.push('\n');
            }
            if idx == 0 && widx == 0 {
                out.push_str(&format!("{label}: {wline}"));
            } else {
                out.push_str(&indent);
                out.push_str(wline);
            }
        }
    }

    out
}

fn format_mcp_server_display(name: &str, config: &Value) -> String {
    let Some(obj) = config.as_object() else {
        return name.to_string();
    };

    if let Some(url) = obj
        .get("url")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        return format!("{name}: {url}");
    }

    if let Some(command) = obj
        .get("command")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        let args = obj
            .get("args")
            .and_then(Value::as_array)
            .map(|arr| {
                arr.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect::<Vec<String>>()
            })
            .unwrap_or_default();

        if args.is_empty() {
            return format!("{name}: {command}");
        }

        return format!("{name}: {command} {}", args.join(" "));
    }

    if let Some(warp_id) = obj
        .get("warp_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        return format!("{name}: warp_id={warp_id}");
    }

    name.to_string()
}

fn mcp_server_display_lines(config: &ListedSimpleIntegrationConfig) -> Vec<String> {
    let json = config.mcp_servers_json.trim();
    if json.is_empty() || json == "{}" {
        return Vec::new();
    }

    let Ok(map) = serde_json::from_str::<Map<String, Value>>(json) else {
        return Vec::new();
    };

    let mut entries: Vec<(String, Value)> = map.into_iter().collect();
    entries.sort_by(|a, b| a.0.cmp(&b.0));

    entries
        .into_iter()
        .map(|(name, cfg)| format_mcp_server_display(&name, &cfg))
        .collect()
}

fn print_integration_card(integration: &SimpleIntegration) {
    let mut table = Table::new();
    table.load_preset(UTF8_FULL);

    // Row 1: provider name (title-cased slug) and description, no label
    let provider_name =
        crate::ai::agent_sdk::text_layout::title_case_identifier(&integration.provider_slug);
    let title_row = crate::ai::agent_sdk::text_layout::render_labeled_wrapped_field(
        &provider_name,
        &integration.description,
        MAX_LINE_WIDTH,
    );
    table.add_row(vec![title_row]);

    // Row 2: Status: <emoji> Status description
    let emoji = status_emoji(integration.connection_status);
    let explanation = status_explanation(integration.connection_status);
    let status_text = format!("{emoji} {explanation}");
    let status_row = crate::ai::agent_sdk::text_layout::render_labeled_wrapped_field(
        "Status",
        &status_text,
        MAX_LINE_WIDTH,
    );
    table.add_row(vec![status_row]);

    // Environment row.
    let env_value = match &integration.integration_config {
        Some(ListedSimpleIntegrationConfig {
            environment_uid, ..
        }) if !environment_uid.is_empty() => environment_uid.clone(),
        _ => "(none)".to_string(),
    };
    let env_row = crate::ai::agent_sdk::text_layout::render_labeled_wrapped_field(
        "Environment",
        &env_value,
        MAX_LINE_WIDTH,
    );
    table.add_row(vec![env_row]);

    // Model row (only if present).
    if let Some(ListedSimpleIntegrationConfig { model_id, .. }) = &integration.integration_config {
        if !model_id.is_empty() {
            let model_row = crate::ai::agent_sdk::text_layout::render_labeled_wrapped_field(
                "Model",
                model_id,
                MAX_LINE_WIDTH,
            );
            table.add_row(vec![model_row]);
        }
    }

    // Base prompt row (only if present).
    if let Some(ListedSimpleIntegrationConfig { base_prompt, .. }) = &integration.integration_config
    {
        if !base_prompt.is_empty() {
            let base_prompt_row = crate::ai::agent_sdk::text_layout::render_labeled_wrapped_field(
                "Base prompt",
                base_prompt,
                MAX_LINE_WIDTH,
            );
            table.add_row(vec![base_prompt_row]);
        }
    }

    // MCP servers row (only if present).
    if let Some(config) = &integration.integration_config {
        let lines = mcp_server_display_lines(config);
        if !lines.is_empty() {
            let row = render_labeled_wrapped_lines("MCP servers", &lines, MAX_LINE_WIDTH);
            table.add_row(vec![row]);
        }
    }

    // Timestamps: keep created/updated in a single row, no label.
    let mut created_updated = String::new();
    if let Some(created) = integration.created_at {
        let dt = created.utc();
        let formatted = format_approx_duration_from_now_utc(dt);
        created_updated.push_str(&format!("Created: {formatted}"));
    }
    if let Some(updated) = integration.updated_at {
        let dt = updated.utc();
        let formatted = format_approx_duration_from_now_utc(dt);
        if !created_updated.is_empty() {
            created_updated.push_str(" | ");
        }
        created_updated.push_str(&format!("Updated: {formatted}"));
    }
    if !created_updated.is_empty() {
        let wrapped =
            crate::ai::agent_sdk::text_layout::word_wrap(&created_updated, MAX_LINE_WIDTH);
        let ts_cell = wrapped.join("\n");
        table.add_row(vec![ts_cell]);
    }

    println!("{table}");
}

fn status_emoji(status: SimpleIntegrationConnectionStatus) -> &'static str {
    match status {
        SimpleIntegrationConnectionStatus::NotConnected => "❌",
        // TODO(bens): these warning emojis render weirdly, maybe switch?
        SimpleIntegrationConnectionStatus::ConnectionError => "⚠️",
        SimpleIntegrationConnectionStatus::IntegrationNotConfigured => "⚠️",
        SimpleIntegrationConnectionStatus::NotEnabled => "⚠️",
        SimpleIntegrationConnectionStatus::Active => "✅",
    }
}

fn status_explanation(status: SimpleIntegrationConnectionStatus) -> &'static str {
    match status {
        SimpleIntegrationConnectionStatus::NotConnected => "This integration is not connected.",
        SimpleIntegrationConnectionStatus::ConnectionError => {
            "This provider is connected but there is an error."
        }
        SimpleIntegrationConnectionStatus::IntegrationNotConfigured => {
            "Connection is active, but the agent integration has not been configured yet."
        }
        SimpleIntegrationConnectionStatus::NotEnabled => {
            "Integration is configured but currently disabled."
        }
        SimpleIntegrationConnectionStatus::Active => "Integration is connected and enabled.",
    }
}

/// Serializable integration info for output.
#[derive(Serialize)]
struct IntegrationInfo {
    provider: String,
    description: String,
    status: String,
    environment_uid: Option<String>,
    base_prompt: Option<String>,
    created_at: Option<DateTime<Utc>>,
    updated_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing)]
    created_at_formatted: String,
    #[serde(skip_serializing)]
    updated_at_formatted: String,
}

impl IntegrationInfo {
    fn from_graphql(integration: &SimpleIntegration) -> Self {
        let provider =
            crate::ai::agent_sdk::text_layout::title_case_identifier(&integration.provider_slug);
        let status = status_explanation(integration.connection_status).to_string();

        let environment_uid = integration.integration_config.as_ref().and_then(|config| {
            if config.environment_uid.is_empty() {
                None
            } else {
                Some(config.environment_uid.clone())
            }
        });

        let base_prompt = integration.integration_config.as_ref().and_then(|config| {
            if config.base_prompt.is_empty() {
                None
            } else {
                Some(config.base_prompt.clone())
            }
        });

        let created_at = integration.created_at.map(|t| t.utc());
        let updated_at = integration.updated_at.map(|t| t.utc());

        let created_at_formatted = created_at
            .map(format_approx_duration_from_now_utc)
            .unwrap_or_else(|| "Unknown".to_string());

        let updated_at_formatted = updated_at
            .map(format_approx_duration_from_now_utc)
            .unwrap_or_else(|| "Unknown".to_string());

        Self {
            provider,
            description: integration.description.clone(),
            status,
            environment_uid,
            base_prompt,
            created_at,
            updated_at,
            created_at_formatted,
            updated_at_formatted,
        }
    }
}

impl TableFormat for IntegrationInfo {
    fn header() -> Vec<Cell> {
        vec![
            Cell::new("Provider"),
            Cell::new("Description"),
            Cell::new("Status"),
            Cell::new("Environment"),
            Cell::new("Created"),
            Cell::new("Updated"),
        ]
    }

    fn row(&self) -> Vec<Cell> {
        vec![
            Cell::new(&self.provider),
            Cell::new(&self.description),
            Cell::new(&self.status),
            Cell::new(self.environment_uid.as_deref().unwrap_or("(none)")),
            Cell::new(&self.created_at_formatted),
            Cell::new(&self.updated_at_formatted),
        ]
    }
}
