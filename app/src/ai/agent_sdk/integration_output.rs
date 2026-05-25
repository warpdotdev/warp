use chrono::{DateTime, Utc};
use comfy_table::{presets::UTF8_FULL, Cell, Table};
use serde::Serialize;
use serde_json::{Map, Value};
use warp_cli::agent::OutputFormat;
use warp_localization::LocaleId;
use warpui::AppContext;

use crate::ai::agent_sdk::output::{self, TableFormat};
use crate::localization;
use crate::util::time_format::format_approx_duration_from_now_utc;
use warp_graphql::queries::get_simple_integrations::{
    ListedSimpleIntegrationConfig, SimpleIntegration, SimpleIntegrationConnectionStatus,
    SimpleIntegrationsOutput,
};

const MAX_LINE_WIDTH: usize = 90;

fn text(app: &AppContext, key: &str) -> String {
    localization::text_for_app(app, key)
}

fn text_with_args(app: &AppContext, key: &str, args: &[(&str, &str)]) -> String {
    localization::text_for_app_with_args(app, key, args)
}

fn text_for_locale(locale: LocaleId, key: &str) -> String {
    localization::text_for_locale(locale, key)
}

/// Print simple integrations.
pub fn print_integrations(
    graphql_output: &SimpleIntegrationsOutput,
    output_format: OutputFormat,
    ctx: &AppContext,
) {
    if let Some(message) = &graphql_output.message {
        eprintln!("{message}");
        return;
    }

    let integrations = &graphql_output.integrations;

    if integrations.is_empty() {
        println!("{}", text(ctx, "agent_sdk.integration.output.none_found"));
        return;
    }

    match output_format {
        OutputFormat::Json | OutputFormat::Ndjson => {
            // Convert to serializable format and use common output utilities
            let integration_infos: Vec<IntegrationInfo> = integrations
                .iter()
                .map(IntegrationInfo::from_graphql)
                .collect();
            output::print_list_for_app(integration_infos, output_format, ctx);
        }
        OutputFormat::Pretty | OutputFormat::Text => {
            // Use the existing card-style layout for pretty/text output
            if integrations.len() == 1 {
                println!(
                    "\n{}",
                    text(ctx, "agent_sdk.integration.output.integration_header")
                );
            } else {
                println!(
                    "\n{}",
                    text(ctx, "agent_sdk.integration.output.integrations_header")
                );
            }

            for integration in integrations {
                print_integration_card(integration, ctx);
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

fn print_integration_card(integration: &SimpleIntegration, ctx: &AppContext) {
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

    // Row 2: Status.
    let status_text = status_explanation_for_app(integration.connection_status, ctx);
    let status_row = crate::ai::agent_sdk::text_layout::render_labeled_wrapped_field(
        &text(ctx, "agent_sdk.integration.field.status"),
        &status_text,
        MAX_LINE_WIDTH,
    );
    table.add_row(vec![status_row]);

    // Environment row.
    let env_value = match &integration.integration_config {
        Some(ListedSimpleIntegrationConfig {
            environment_uid, ..
        }) if !environment_uid.is_empty() => environment_uid.clone(),
        _ => text(ctx, "agent_sdk.common.value.none"),
    };
    let env_row = crate::ai::agent_sdk::text_layout::render_labeled_wrapped_field(
        &text(ctx, "agent_sdk.integration.field.environment"),
        &env_value,
        MAX_LINE_WIDTH,
    );
    table.add_row(vec![env_row]);

    // Model row (only if present).
    if let Some(ListedSimpleIntegrationConfig { model_id, .. }) = &integration.integration_config {
        if !model_id.is_empty() {
            let model_row = crate::ai::agent_sdk::text_layout::render_labeled_wrapped_field(
                &text(ctx, "agent_sdk.integration.field.model"),
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
                &text(ctx, "agent_sdk.integration.field.base_prompt"),
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
            let row = render_labeled_wrapped_lines(
                &text(ctx, "agent_sdk.integration.field.mcp_servers"),
                &lines,
                MAX_LINE_WIDTH,
            );
            table.add_row(vec![row]);
        }
    }

    // Timestamps: keep created/updated in a single row, no label.
    let mut created_updated = String::new();
    if let Some(created) = integration.created_at {
        let dt = created.utc();
        let formatted = format_approx_duration_from_now_utc(dt);
        created_updated.push_str(&text_with_args(
            ctx,
            "agent_sdk.integration.field.created_with_value",
            &[("created", &formatted)],
        ));
    }
    if let Some(updated) = integration.updated_at {
        let dt = updated.utc();
        let formatted = format_approx_duration_from_now_utc(dt);
        if !created_updated.is_empty() {
            created_updated.push_str(" | ");
        }
        created_updated.push_str(&text_with_args(
            ctx,
            "agent_sdk.integration.field.updated_with_value",
            &[("updated", &formatted)],
        ));
    }
    if !created_updated.is_empty() {
        let wrapped =
            crate::ai::agent_sdk::text_layout::word_wrap(&created_updated, MAX_LINE_WIDTH);
        let ts_cell = wrapped.join("\n");
        table.add_row(vec![ts_cell]);
    }

    println!("{table}");
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

fn status_explanation_for_app(
    status: SimpleIntegrationConnectionStatus,
    ctx: &AppContext,
) -> String {
    text(ctx, status_key(status))
}

fn status_key(status: SimpleIntegrationConnectionStatus) -> &'static str {
    match status {
        SimpleIntegrationConnectionStatus::NotConnected => {
            "agent_sdk.integration.status.not_connected"
        }
        SimpleIntegrationConnectionStatus::ConnectionError => {
            "agent_sdk.integration.status.connection_error"
        }
        SimpleIntegrationConnectionStatus::IntegrationNotConfigured => {
            "agent_sdk.integration.status.not_configured"
        }
        SimpleIntegrationConnectionStatus::NotEnabled => "agent_sdk.integration.status.not_enabled",
        SimpleIntegrationConnectionStatus::Active => "agent_sdk.integration.status.active",
    }
}

/// Serializable integration info for output.
#[derive(Serialize)]
struct IntegrationInfo {
    provider: String,
    description: String,
    status: String,
    #[serde(skip_serializing)]
    status_key: &'static str,
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
        let status_key = status_key(integration.connection_status);

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
            status_key,
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
        Self::header_for_locale(LocaleId::EnUs)
    }

    fn header_for_app(app: &AppContext) -> Vec<Cell> {
        vec![
            Cell::new(text(app, "agent_sdk.integration.table.provider")),
            Cell::new(text(app, "agent_sdk.integration.table.description")),
            Cell::new(text(app, "agent_sdk.integration.table.status")),
            Cell::new(text(app, "agent_sdk.integration.table.environment")),
            Cell::new(text(app, "agent_sdk.integration.table.created")),
            Cell::new(text(app, "agent_sdk.integration.table.updated")),
        ]
    }

    fn header_for_locale(locale: LocaleId) -> Vec<Cell> {
        vec![
            Cell::new(text_for_locale(
                locale,
                "agent_sdk.integration.table.provider",
            )),
            Cell::new(text_for_locale(
                locale,
                "agent_sdk.integration.table.description",
            )),
            Cell::new(text_for_locale(
                locale,
                "agent_sdk.integration.table.status",
            )),
            Cell::new(text_for_locale(
                locale,
                "agent_sdk.integration.table.environment",
            )),
            Cell::new(text_for_locale(
                locale,
                "agent_sdk.integration.table.created",
            )),
            Cell::new(text_for_locale(
                locale,
                "agent_sdk.integration.table.updated",
            )),
        ]
    }

    fn row(&self) -> Vec<Cell> {
        self.row_for_locale(LocaleId::EnUs)
    }

    fn row_for_app(&self, app: &AppContext) -> Vec<Cell> {
        let environment = self
            .environment_uid
            .clone()
            .unwrap_or_else(|| text(app, "agent_sdk.common.value.none"));
        let created_at = if self.created_at.is_some() {
            self.created_at_formatted.clone()
        } else {
            text(app, "agent_sdk.common.value.unknown")
        };
        let updated_at = if self.updated_at.is_some() {
            self.updated_at_formatted.clone()
        } else {
            text(app, "agent_sdk.common.value.unknown")
        };
        vec![
            Cell::new(&self.provider),
            Cell::new(&self.description),
            Cell::new(text(app, self.status_key)),
            Cell::new(environment),
            Cell::new(created_at),
            Cell::new(updated_at),
        ]
    }

    fn row_for_locale(&self, locale: LocaleId) -> Vec<Cell> {
        let environment = self
            .environment_uid
            .clone()
            .unwrap_or_else(|| text_for_locale(locale, "agent_sdk.common.value.none"));
        let created_at = if self.created_at.is_some() {
            self.created_at_formatted.clone()
        } else {
            text_for_locale(locale, "agent_sdk.common.value.unknown")
        };
        let updated_at = if self.updated_at.is_some() {
            self.updated_at_formatted.clone()
        } else {
            text_for_locale(locale, "agent_sdk.common.value.unknown")
        };
        vec![
            Cell::new(&self.provider),
            Cell::new(&self.description),
            Cell::new(text_for_locale(locale, self.status_key)),
            Cell::new(environment),
            Cell::new(created_at),
            Cell::new(updated_at),
        ]
    }
}
