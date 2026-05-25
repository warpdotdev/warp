use std::{
    cmp::Reverse,
    fmt,
    io::{self, IsTerminal as _},
};

use anyhow::{anyhow, Result};
use chrono::{DateTime, Utc};
use comfy_table::Cell;
use inquire::{Confirm, InquireError, Select};
use serde::Serialize;
use warp_cli::{
    agent::OutputFormat,
    api_key::{
        ApiKeyCommand, ApiKeyExpirationArgs, ApiKeySortByArg, ApiKeySortOrderArg, CreateApiKeyArgs,
        ExpireApiKeyArgs, ListApiKeysArgs,
    },
    GlobalOptions,
};
use warp_graphql::{
    mutations::{expire_api_key::ExpireApiKeyResult, generate_api_key::GenerateApiKeyResult},
    queries::api_keys::ApiKeyProperties,
    scalars::Time,
};
use warp_localization::LocaleId;
use warpui::{platform::TerminationMode, AppContext, ModelContext, SingletonEntity};

use crate::{
    localization,
    server::{ids::ApiKeyUid, server_api::auth::AuthClient},
    util::time_format::format_approx_duration_from_now_utc,
    ServerApiProvider,
};

use super::output::{self, TableFormat};

fn text(app: &AppContext, key: &str) -> String {
    localization::text_for_app(app, key)
}

fn text_for_locale(locale: LocaleId, key: &str) -> String {
    localization::text_for_locale(locale, key)
}

fn default_text(key: &str) -> String {
    text_for_locale(LocaleId::EnUs, key)
}

fn text_with_args(app: &AppContext, key: &str, args: &[(&str, &str)]) -> String {
    localization::text_for_app_with_args(app, key, args)
}

/// Run API key-related commands.
pub fn run(
    ctx: &mut AppContext,
    global_options: GlobalOptions,
    command: ApiKeyCommand,
) -> Result<()> {
    let runner = ctx.add_singleton_model(|_ctx| ApiKeyCommandRunner);
    match command {
        ApiKeyCommand::List(args) => {
            runner.update(ctx, |runner, ctx| {
                runner.list(global_options.output_format, args, ctx)
            });
            Ok(())
        }
        ApiKeyCommand::Create(args) => {
            runner.update(ctx, |runner, ctx| {
                runner.create(global_options.output_format, args, ctx)
            });
            Ok(())
        }
        ApiKeyCommand::Expire(args) => {
            runner.update(ctx, |runner, ctx| {
                runner.expire(global_options.output_format, args, ctx)
            });
            Ok(())
        }
    }
}

struct ApiKeyCommandRunner;

impl ApiKeyCommandRunner {
    fn list(
        &self,
        output_format: OutputFormat,
        args: ListApiKeysArgs,
        ctx: &mut ModelContext<Self>,
    ) {
        let server_api = ServerApiProvider::as_ref(ctx).get();

        ctx.spawn(
            async move {
                let mut keys: Vec<_> = server_api
                    .list_api_keys()
                    .await?
                    .into_iter()
                    .map(ApiKeyInfo::from)
                    .collect();
                sort_api_keys(&mut keys, args.sort_by, args.sort_order);
                Ok(keys)
            },
            move |_, result: Result<Vec<ApiKeyInfo>>, ctx| match result {
                Ok(keys) => {
                    let result = if args.json_output.force_json_output() {
                        serde_json::to_value(&keys)
                            .map_err(anyhow::Error::from)
                            .and_then(|value| output::print_raw_json(value, &args.json_output))
                    } else {
                        output::print_list_for_app(keys, output_format, ctx);
                        Ok(())
                    };
                    finish_command(result, ctx);
                }
                Err(err) => finish_command(Err(err), ctx),
            },
        );
    }

    fn create(
        &self,
        output_format: OutputFormat,
        args: CreateApiKeyArgs,
        ctx: &mut ModelContext<Self>,
    ) {
        let server_api = ServerApiProvider::as_ref(ctx).get();
        let unknown_error = text(ctx, "agent_sdk.api_key.error.create_failed");
        let name = args.name;
        let agent_uid = args.agent_uid;
        let expiration = args.expiration;
        let json_output = args.json_output;

        ctx.spawn(
            async move {
                let expires_at = expires_at_from_args(expiration)?;
                let agent_uid = agent_uid.map(cynic::Id::new);
                let result = server_api
                    .create_api_key(name, None, agent_uid, expires_at)
                    .await?;
                let result = match result {
                    GenerateApiKeyResult::GenerateApiKeyOutput(output) => CreatedApiKeyInfo {
                        raw_api_key: output.raw_api_key,
                        api_key: ApiKeyInfo::from(output.api_key),
                    },
                    GenerateApiKeyResult::UserFacingError(e) => {
                        return Err(anyhow!(
                            warp_graphql::client::get_user_facing_error_message(e)
                        ));
                    }
                    GenerateApiKeyResult::Unknown => return Err(anyhow!(unknown_error)),
                };
                Ok(result)
            },
            move |_, result: Result<CreatedApiKeyInfo>, ctx| match result {
                Ok(result) => {
                    match print_created_api_key(result, output_format, json_output, ctx) {
                        Ok(()) => ctx.terminate_app(TerminationMode::ForceTerminate, None),
                        Err(err) => super::report_fatal_error(err, ctx),
                    }
                }
                Err(err) => super::report_fatal_error(err, ctx),
            },
        );
    }

    fn expire(
        &self,
        output_format: OutputFormat,
        args: ExpireApiKeyArgs,
        ctx: &mut ModelContext<Self>,
    ) {
        let key_identifier = args.key_uid;
        let force = args.force;
        let json_output = args.json_output;
        let server_api = ServerApiProvider::as_ref(ctx).get();
        let unknown_error = text(ctx, "agent_sdk.api_key.error.expire_failed");

        ctx.spawn(
            async move {
                let keys = server_api
                    .list_api_keys()
                    .await?
                    .into_iter()
                    .map(ApiKeyInfo::from)
                    .collect();
                Ok(keys)
            },
            move |_, result: Result<Vec<ApiKeyInfo>>, ctx| {
                let keys = match result {
                    Ok(keys) => keys,
                    Err(err) => {
                        super::report_fatal_error(err, ctx);
                        return;
                    }
                };

                let key = match resolve_api_key_identifier(&keys, &key_identifier, ctx) {
                    Ok(Some(key)) => key,
                    Ok(None) => {
                        ctx.terminate_app(TerminationMode::ForceTerminate, None);
                        return;
                    }
                    Err(err) => {
                        super::report_fatal_error(err, ctx);
                        return;
                    }
                };

                if !force {
                    if !io::stdin().is_terminal() {
                        super::report_fatal_error(
                            anyhow!(text(
                                ctx,
                                "agent_sdk.api_key.error.expire_non_interactive_requires_force"
                            )),
                            ctx,
                        );
                        return;
                    }

                    let key_label = key.to_string();
                    let prompt = text_with_args(
                        ctx,
                        "agent_sdk.api_key.confirm.expire",
                        &[("key", &key_label)],
                    );
                    let help = text(ctx, "agent_sdk.api_key.confirm.expire_help");
                    let should_expire = match Confirm::new(&prompt)
                        .with_default(false)
                        .with_help_message(&help)
                        .prompt()
                    {
                        Ok(should_expire) => should_expire,
                        Err(
                            InquireError::OperationCanceled | InquireError::OperationInterrupted,
                        ) => {
                            ctx.terminate_app(TerminationMode::ForceTerminate, None);
                            return;
                        }
                        Err(err) => {
                            super::report_fatal_error(err.into(), ctx);
                            return;
                        }
                    };

                    if !should_expire {
                        println!(
                            "{}",
                            text(ctx, "agent_sdk.api_key.confirm.expire_cancelled")
                        );
                        ctx.terminate_app(TerminationMode::ForceTerminate, None);
                        return;
                    }
                }

                let uid = ApiKeyUid::from(key.uid);
                let server_api = ServerApiProvider::as_ref(ctx).get();
                ctx.spawn(
                    async move {
                        let result = server_api.expire_api_key(&uid).await?;
                        let expired = match result {
                            ExpireApiKeyResult::ExpireApiKeyOutput(output) => output.success,
                            ExpireApiKeyResult::UserFacingError(e) => {
                                return Err(anyhow!(
                                    warp_graphql::client::get_user_facing_error_message(e)
                                ));
                            }
                            ExpireApiKeyResult::Unknown => return Err(anyhow!(unknown_error)),
                        };
                        Ok(ExpiredApiKeyInfo {
                            key_uid: uid.to_string(),
                            expired,
                        })
                    },
                    move |_, result: Result<ExpiredApiKeyInfo>, ctx| match result {
                        Ok(result) => match print_expire_api_key_result(
                            result,
                            output_format,
                            json_output,
                            ctx,
                        ) {
                            Ok(()) => ctx.terminate_app(TerminationMode::ForceTerminate, None),
                            Err(err) => super::report_fatal_error(err, ctx),
                        },
                        Err(err) => super::report_fatal_error(err, ctx),
                    },
                );
            },
        );
    }
}

impl warpui::Entity for ApiKeyCommandRunner {
    type Event = ();
}

impl SingletonEntity for ApiKeyCommandRunner {}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
struct ApiKeyInfo {
    uid: String,
    name: String,
    key_suffix: String,
    scope: String,
    created_at: DateTime<Utc>,
    last_used_at: Option<DateTime<Utc>>,
    expires_at: Option<DateTime<Utc>>,
}

impl From<ApiKeyProperties> for ApiKeyInfo {
    fn from(key: ApiKeyProperties) -> Self {
        Self {
            uid: key.uid.into_inner(),
            name: key.name,
            key_suffix: key.key_suffix,
            scope: key.owner_type.to_string(),
            created_at: key.created_at.utc(),
            last_used_at: key.last_used_at.map(|t| t.utc()),
            expires_at: key.expires_at.map(|t| t.utc()),
        }
    }
}

impl fmt::Display for ApiKeyInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = &self.name;
        let uid = &self.uid;
        let created_at = self.created_at.format("%Y-%m-%d %H:%M:%S UTC");
        write!(f, "{name} ({uid}, created {created_at})")
    }
}

impl TableFormat for ApiKeyInfo {
    fn header() -> Vec<Cell> {
        vec![
            Cell::new(text_for_locale(
                LocaleId::EnUs,
                "agent_sdk.api_key.table.uid",
            )),
            Cell::new(text_for_locale(
                LocaleId::EnUs,
                "agent_sdk.api_key.table.name",
            )),
            Cell::new(text_for_locale(
                LocaleId::EnUs,
                "agent_sdk.api_key.table.key",
            )),
            Cell::new(text_for_locale(
                LocaleId::EnUs,
                "agent_sdk.api_key.table.scope",
            )),
            Cell::new(text_for_locale(
                LocaleId::EnUs,
                "agent_sdk.api_key.table.created",
            )),
            Cell::new(text_for_locale(
                LocaleId::EnUs,
                "agent_sdk.api_key.table.last_used",
            )),
            Cell::new(text_for_locale(
                LocaleId::EnUs,
                "agent_sdk.api_key.table.expires_at",
            )),
        ]
    }

    fn header_for_app(app: &AppContext) -> Vec<Cell> {
        vec![
            Cell::new(text(app, "agent_sdk.api_key.table.uid")),
            Cell::new(text(app, "agent_sdk.api_key.table.name")),
            Cell::new(text(app, "agent_sdk.api_key.table.key")),
            Cell::new(text(app, "agent_sdk.api_key.table.scope")),
            Cell::new(text(app, "agent_sdk.api_key.table.created")),
            Cell::new(text(app, "agent_sdk.api_key.table.last_used")),
            Cell::new(text(app, "agent_sdk.api_key.table.expires_at")),
        ]
    }

    fn row(&self) -> Vec<Cell> {
        vec![
            Cell::new(&self.uid),
            Cell::new(&self.name),
            Cell::new(format!("wk-**{}", self.key_suffix)),
            Cell::new(&self.scope),
            Cell::new(format_approx_duration_from_now_utc(self.created_at)),
            Cell::new(
                self.last_used_at
                    .map(format_approx_duration_from_now_utc)
                    .unwrap_or_else(|| "Never".to_string()),
            ),
            Cell::new(
                self.expires_at
                    .map(|dt| dt.format("%Y-%m-%d %H:%M:%S UTC").to_string())
                    .unwrap_or_else(|| "Never".to_string()),
            ),
        ]
    }

    fn row_for_app(&self, app: &AppContext) -> Vec<Cell> {
        let never = || text(app, "agent_sdk.api_key.table.never");

        vec![
            Cell::new(&self.uid),
            Cell::new(&self.name),
            Cell::new(format!("wk-**{}", self.key_suffix)),
            Cell::new(&self.scope),
            Cell::new(format_approx_duration_from_now_utc(self.created_at)),
            Cell::new(
                self.last_used_at
                    .map(format_approx_duration_from_now_utc)
                    .unwrap_or_else(never),
            ),
            Cell::new(
                self.expires_at
                    .map(|dt| dt.format("%Y-%m-%d %H:%M:%S UTC").to_string())
                    .unwrap_or_else(never),
            ),
        ]
    }
}

fn resolve_api_key_identifier(
    keys: &[ApiKeyInfo],
    key_identifier: &str,
    app: &AppContext,
) -> Result<Option<ApiKeyInfo>> {
    if let Some(key) = keys.iter().find(|key| key.uid == key_identifier) {
        return Ok(Some(key.clone()));
    }

    let mut matches = keys
        .iter()
        .filter(|key| key.name == key_identifier)
        .cloned()
        .collect::<Vec<_>>();
    matches.sort_by_key(|key| Reverse(key.created_at));

    if matches.is_empty() {
        return Err(anyhow!(text_with_args(
            app,
            "agent_sdk.api_key.error.not_found",
            &[("key_identifier", key_identifier)],
        )));
    } else if matches.len() == 1 {
        return Ok(Some(matches[0].clone()));
    }

    if io::stdin().is_terminal() {
        let prompt = text_with_args(
            app,
            "agent_sdk.api_key.prompt.select_key_to_expire",
            &[("key_identifier", key_identifier)],
        );
        return match Select::new(&prompt, matches).prompt() {
            Ok(key) => Ok(Some(key)),
            Err(InquireError::OperationCanceled | InquireError::OperationInterrupted) => Ok(None),
            Err(err) => Err(err.into()),
        };
    }
    println!(
        "{}",
        text_with_args(
            app,
            "agent_sdk.api_key.output.multiple_matches",
            &[("key_identifier", key_identifier)],
        )
    );
    for key in matches {
        println!("  {key}");
    }

    Err(anyhow!(text_with_args(
        app,
        "agent_sdk.api_key.error.multiple_matches_specify_uid",
        &[("key_identifier", key_identifier)],
    )))
}

#[derive(Debug, Clone, Serialize)]
struct CreatedApiKeyInfo {
    raw_api_key: String,
    api_key: ApiKeyInfo,
}

#[derive(Debug, Clone, Serialize)]
struct ExpiredApiKeyInfo {
    key_uid: String,
    expired: bool,
}

fn sort_api_keys(
    keys: &mut [ApiKeyInfo],
    sort_by: Option<ApiKeySortByArg>,
    sort_order: Option<ApiKeySortOrderArg>,
) {
    let Some(sort_by) = sort_by else {
        return;
    };
    let descending = matches!(sort_order, Some(ApiKeySortOrderArg::Desc));
    match sort_by {
        ApiKeySortByArg::Name => {
            if descending {
                keys.sort_by_key(|k| Reverse(k.name.to_lowercase()));
            } else {
                keys.sort_by_key(|k| k.name.to_lowercase());
            }
        }
        ApiKeySortByArg::CreatedAt => {
            if descending {
                keys.sort_by_key(|k| Reverse(k.created_at));
            } else {
                keys.sort_by_key(|k| k.created_at);
            }
        }
        ApiKeySortByArg::LastUsedAt => {
            if descending {
                keys.sort_by_key(|k| Reverse(k.last_used_at));
            } else {
                keys.sort_by_key(|k| k.last_used_at);
            }
        }
        ApiKeySortByArg::ExpiresAt => {
            if descending {
                keys.sort_by_key(|k| Reverse(k.expires_at));
            } else {
                keys.sort_by_key(|k| k.expires_at);
            }
        }
        ApiKeySortByArg::Scope => {
            if descending {
                keys.sort_by_key(|k| Reverse(k.scope.clone()));
            } else {
                keys.sort_by_key(|k| k.scope.clone());
            }
        }
    }
}

fn expires_at_from_args(args: ApiKeyExpirationArgs) -> Result<Option<Time>> {
    if args.no_expiration {
        return Ok(None);
    }

    if let Some(expires_at) = args.expires_at {
        return Ok(Some(Time::from(expires_at)));
    }

    if let Some(expires_in) = args.expires_in {
        let duration = chrono::Duration::from_std(expires_in.into())
            .map_err(|_| anyhow!(default_text("agent_sdk.api_key.error.expiration_too_large")))?;
        return Ok(Some(Time::from(Utc::now() + duration)));
    }

    Err(anyhow!(default_text(
        "agent_sdk.api_key.error.expiration_behavior_required"
    )))
}

fn print_created_api_key(
    result: CreatedApiKeyInfo,
    output_format: OutputFormat,
    json_output: warp_cli::json_filter::JsonOutput,
    app: &AppContext,
) -> Result<()> {
    if json_output.force_json_output() {
        output::print_raw_json(serde_json::to_value(&result)?, &json_output)?;
        return Ok(());
    }
    match output_format {
        OutputFormat::Json => output::write_json(&result, std::io::stdout())?,
        OutputFormat::Ndjson => output::write_json_line(&result, std::io::stdout())?,
        OutputFormat::Pretty | OutputFormat::Text => {
            println!(
                "{}",
                text_with_args(
                    app,
                    "agent_sdk.api_key.output.created",
                    &[("name", &result.api_key.name)],
                )
            );
            println!(
                "{} {}",
                text(app, "agent_sdk.api_key.output.uid"),
                result.api_key.uid
            );
            println!(
                "{} {}",
                text(app, "agent_sdk.api_key.output.raw_api_key"),
                result.raw_api_key
            );
            println!(
                "{}",
                text(app, "agent_sdk.api_key.output.secret_shown_once")
            );
        }
    }
    Ok(())
}

fn print_expire_api_key_result(
    result: ExpiredApiKeyInfo,
    output_format: OutputFormat,
    json_output: warp_cli::json_filter::JsonOutput,
    app: &AppContext,
) -> Result<()> {
    if json_output.force_json_output() {
        output::print_raw_json(serde_json::to_value(&result)?, &json_output)?;
        return Ok(());
    }

    match output_format {
        OutputFormat::Json => output::write_json(&result, std::io::stdout())?,
        OutputFormat::Ndjson => output::write_json_line(&result, std::io::stdout())?,
        OutputFormat::Pretty | OutputFormat::Text => {
            if result.expired {
                println!(
                    "{}",
                    text_with_args(
                        app,
                        "agent_sdk.api_key.output.expired",
                        &[("key_uid", &result.key_uid)],
                    )
                );
            } else {
                println!(
                    "{}",
                    text_with_args(
                        app,
                        "agent_sdk.api_key.output.not_expired",
                        &[("key_uid", &result.key_uid)],
                    )
                );
            }
        }
    }
    Ok(())
}

fn finish_command(result: Result<()>, ctx: &mut ModelContext<ApiKeyCommandRunner>) {
    match result {
        Ok(()) => ctx.terminate_app(TerminationMode::ForceTerminate, None),
        Err(err) => super::report_fatal_error(err, ctx),
    }
}

#[cfg(test)]
#[path = "api_key_tests.rs"]
mod tests;
