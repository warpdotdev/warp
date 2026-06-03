use std::cmp::Reverse;
use std::fmt;
use std::io::{self, IsTerminal as _};

use anyhow::{anyhow, Result};
use chrono::{DateTime, Utc};
use comfy_table::Cell;
use inquire::{Confirm, InquireError, Select};
use serde::Serialize;
use warp_cli::agent::OutputFormat;
use warp_cli::api_key::{
    ApiKeyCommand, ApiKeyExpirationArgs, ApiKeySortByArg, CreateApiKeyArgs, ExpireApiKeyArgs,
    ListApiKeysArgs,
};
use warp_cli::{GlobalOptions, SortOrderArg};
use warp_graphql::mutations::expire_api_key::ExpireApiKeyResult;
use warp_graphql::mutations::generate_api_key::GenerateApiKeyResult;
use warp_graphql::queries::api_keys::ApiKeyProperties;
use warp_graphql::scalars::Time;
use warpui::platform::TerminationMode;
use warpui::{AppContext, ModelContext, SingletonEntity};

use super::output::{self, TableFormat};
use crate::server::ids::ApiKeyUid;
use crate::server::server_api::auth::AuthClient;
use crate::util::time_format::format_approx_duration_from_now_utc;
use crate::ServerApiProvider;

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
                if args.json_output.force_json_output() {
                    output::print_raw_json(serde_json::to_value(&keys)?, &args.json_output)?;
                } else {
                    output::print_list(keys, output_format);
                }
                Ok(())
            },
            |_, result: Result<()>, ctx| finish_command(result, ctx),
        );
    }

    fn create(
        &self,
        output_format: OutputFormat,
        args: CreateApiKeyArgs,
        ctx: &mut ModelContext<Self>,
    ) {
        let server_api = ServerApiProvider::as_ref(ctx).get();

        ctx.spawn(
            async move {
                let json_output = args.json_output;
                let expires_at = expires_at_from_args(args.expiration)?;
                let agent_uid = args.agent_uid.map(cynic::Id::new);
                let result = server_api
                    .create_api_key(args.name, None, agent_uid, expires_at)
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
                    GenerateApiKeyResult::Unknown => {
                        return Err(anyhow!(i18n::t("ai.agent_sdk.api_key.create_failed")))
                    }
                };
                print_created_api_key(result, output_format, json_output)?;
                Ok(())
            },
            |_, result: Result<()>, ctx| finish_command(result, ctx),
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

                let key = match resolve_api_key_identifier(&keys, &key_identifier) {
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
                            anyhow!(i18n::t(
                                "ai.agent_sdk.api_key.expire_refusing_noninteractive"
                            )),
                            ctx,
                        );
                        return;
                    }

                    let prompt = i18n::t("ai.agent_sdk.api_key.expire_confirm")
                        .replace("{key}", &key.to_string());
                    let should_expire = match Confirm::new(&prompt)
                        .with_default(false)
                        .with_help_message(&i18n::t("ai.agent_sdk.api_key.expire_confirm_help"))
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
                        println!("{}", i18n::t("ai.agent_sdk.api_key.expiration_cancelled"));
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
                            ExpireApiKeyResult::Unknown => {
                                return Err(anyhow!(i18n::t("ai.agent_sdk.api_key.expire_failed")))
                            }
                        };
                        print_expire_api_key_result(
                            uid.to_string(),
                            expired,
                            output_format,
                            json_output,
                        )?;
                        Ok(())
                    },
                    |_, result: Result<()>, ctx| finish_command(result, ctx),
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
        write!(
            f,
            "{}",
            i18n::t("ai.agent_sdk.api_key.display")
                .replace("{name}", name)
                .replace("{uid}", uid)
                .replace("{created_at}", &created_at.to_string())
        )
    }
}

impl TableFormat for ApiKeyInfo {
    fn header() -> Vec<Cell> {
        vec![
            Cell::new(i18n::t("ai.agent_sdk.api_key.table.uid")),
            Cell::new(i18n::t("ai.agent_sdk.api_key.table.name")),
            Cell::new(i18n::t("ai.agent_sdk.api_key.table.key")),
            Cell::new(i18n::t("ai.agent_sdk.api_key.table.scope")),
            Cell::new(i18n::t("ai.agent_sdk.api_key.table.created")),
            Cell::new(i18n::t("ai.agent_sdk.api_key.table.last_used")),
            Cell::new(i18n::t("ai.agent_sdk.api_key.table.expires_at")),
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
                    .unwrap_or_else(|| i18n::t("ai.agent_sdk.api_key.never")),
            ),
            Cell::new(
                self.expires_at
                    .map(|dt| dt.format("%Y-%m-%d %H:%M:%S UTC").to_string())
                    .unwrap_or_else(|| i18n::t("ai.agent_sdk.api_key.never")),
            ),
        ]
    }
}

fn resolve_api_key_identifier(
    keys: &[ApiKeyInfo],
    key_identifier: &str,
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
        return Err(anyhow!(
            i18n::t("ai.agent_sdk.api_key.not_found").replace("{key}", key_identifier)
        ));
    } else if matches.len() == 1 {
        return Ok(Some(matches[0].clone()));
    }

    if io::stdin().is_terminal() {
        return match Select::new(
            &i18n::t("ai.agent_sdk.api_key.multiple_select").replace("{key}", key_identifier),
            matches,
        )
        .prompt()
        {
            Ok(key) => Ok(Some(key)),
            Err(InquireError::OperationCanceled | InquireError::OperationInterrupted) => Ok(None),
            Err(err) => Err(err.into()),
        };
    }
    println!(
        "{}",
        i18n::t("ai.agent_sdk.api_key.multiple_matches").replace("{key}", key_identifier)
    );
    for key in matches {
        println!("  {key}");
    }

    Err(anyhow!(i18n::t(
        "ai.agent_sdk.api_key.multiple_specify_uid"
    )
    .replace("{key}", key_identifier)))
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
    sort_order: Option<SortOrderArg>,
) {
    let Some(sort_by) = sort_by else {
        return;
    };
    let descending = matches!(sort_order, Some(SortOrderArg::Desc));
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
            .map_err(|_| anyhow!(i18n::t("ai.agent_sdk.api_key.expiration_too_large")))?;
        return Ok(Some(Time::from(Utc::now() + duration)));
    }

    Err(anyhow!(i18n::t(
        "ai.agent_sdk.api_key.expiration_behavior_required"
    )))
}

fn print_created_api_key(
    result: CreatedApiKeyInfo,
    output_format: OutputFormat,
    json_output: warp_cli::json_filter::JsonOutput,
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
                i18n::t("ai.agent_sdk.api_key.created").replace("{name}", &result.api_key.name)
            );
            println!(
                "{}",
                i18n::t("ai.agent_sdk.api_key.uid").replace("{uid}", &result.api_key.uid)
            );
            println!(
                "{}",
                i18n::t("ai.agent_sdk.api_key.raw_api_key")
                    .replace("{api_key}", &result.raw_api_key)
            );
            println!("{}", i18n::t("ai.agent_sdk.api_key.store_securely"));
        }
    }
    Ok(())
}

fn print_expire_api_key_result(
    key_uid: String,
    expired: bool,
    output_format: OutputFormat,
    json_output: warp_cli::json_filter::JsonOutput,
) -> Result<()> {
    let result = ExpiredApiKeyInfo { key_uid, expired };
    if json_output.force_json_output() {
        output::print_raw_json(serde_json::to_value(&result)?, &json_output)?;
        return Ok(());
    }

    match output_format {
        OutputFormat::Json => output::write_json(&result, std::io::stdout())?,
        OutputFormat::Ndjson => output::write_json_line(&result, std::io::stdout())?,
        OutputFormat::Pretty | OutputFormat::Text => {
            if expired {
                println!(
                    "{}",
                    i18n::t("ai.agent_sdk.api_key.expired").replace("{uid}", &result.key_uid)
                );
            } else {
                println!(
                    "{}",
                    i18n::t("ai.agent_sdk.api_key.not_expired").replace("{uid}", &result.key_uid)
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
