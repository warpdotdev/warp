use std::fs;
use std::io::{self, IsTerminal as _, Read};

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use comfy_table::Cell;
use inquire::{Confirm, InquireError, Password};
use serde::Serialize;
use warp_cli::agent::OutputFormat;
use warp_cli::scope::ObjectScope;
use warp_cli::secret::{
    AnthropicMethod, CodexMethod, CreateProvider, CreateSecretArgs, DeleteSecretArgs,
    ListSecretsArgs, SecretCommand, SecretType, UpdateSecretArgs, ValueArgs,
};
use warp_cli::GlobalOptions;
use warp_core::features::FeatureFlag;
use warp_graphql::managed_secrets::{ManagedSecret, ManagedSecretType};
use warp_graphql::object::SpaceType;
use warp_managed_secrets::client::SecretOwner;
use warp_managed_secrets::{ManagedSecretManager, ManagedSecretValue};
use warpui::platform::TerminationMode;
use warpui::{AppContext, SingletonEntity as _};

use super::output::{self, TableFormat};
use crate::auth::UserUid;
use crate::cloud_object::Owner;
use crate::server::ids::ServerId;
use crate::util::time_format::format_approx_duration_from_now_utc;

#[derive(Serialize)]
struct SecretInfo {
    name: String,
    scope: String,
    #[serde(rename = "type")]
    secret_type: ManagedSecretType,
    #[serde(rename = "created")]
    created_at: DateTime<Utc>,
    #[serde(rename = "updated")]
    updated_at: DateTime<Utc>,
}

impl TableFormat for SecretInfo {
    fn header() -> Vec<Cell> {
        vec![
            Cell::new("Name"),
            Cell::new("Scope"),
            Cell::new("Type"),
            Cell::new("Created"),
            Cell::new("Updated"),
        ]
    }

    fn row(&self) -> Vec<Cell> {
        vec![
            Cell::new(&self.name),
            Cell::new(&self.scope),
            Cell::new(format_secret_type(&self.secret_type)),
            Cell::new(format_approx_duration_from_now_utc(self.created_at)),
            Cell::new(format_approx_duration_from_now_utc(self.updated_at)),
        ]
    }
}

/// Run secret-related commands.
pub fn run(
    ctx: &mut AppContext,
    global_options: GlobalOptions,
    command: SecretCommand,
) -> Result<()> {
    if !FeatureFlag::WarpManagedSecrets.is_enabled() {
        return Err(anyhow::anyhow!("This feature is not enabled"));
    }

    match command {
        SecretCommand::Create(args) => create_secret(ctx, args),
        SecretCommand::Delete(args) => delete_secret(ctx, args),
        SecretCommand::Update(args) => update_secret(ctx, args),
        SecretCommand::List(args) => list_secrets(ctx, global_options.output_format, args),
    }
}

/// Deferred secret value reader. Constructed during argument parsing but read after validation
/// (refresh metadata, resolve owner) so users are not prompted for secrets before we know the
/// request is valid.
enum SecretInput {
    /// Single-field secret types (raw value, Anthropic API key).
    Simple {
        secret_type: SecretType,
        value_args: ValueArgs,
    },
    /// Multi-field Bedrock API key secret with dedicated CLI flags.
    Bedrock {
        bedrock_api_key: Option<String>,
        region: Option<String>,
    },
    /// Multi-field Bedrock access key secret with dedicated CLI flags.
    BedrockAccessKey {
        access_key_id: Option<String>,
        secret_access_key: Option<String>,
        session_token: Option<String>,
        region: Option<String>,
    },
    /// OpenAI API key secret with optional base URL.
    OpenaiApiKey {
        value_args: ValueArgs,
        base_url: Option<String>,
    },
}

impl SecretInput {
    /// Read the secret value, prompting the user if necessary.
    /// Returns `Ok(None)` when the user cancels the prompt.
    fn read(self) -> Result<Option<ManagedSecretValue>> {
        match self {
            SecretInput::Simple {
                secret_type,
                value_args,
            } => {
                let raw = match read_simple_secret_value(&value_args)? {
                    Some(v) => v,
                    None => return Ok(None),
                };
                Ok(Some(make_simple_secret_value(secret_type, &raw)))
            }
            SecretInput::Bedrock {
                bedrock_api_key,
                region,
            } => read_bedrock_secret_value(bedrock_api_key, region),
            SecretInput::BedrockAccessKey {
                access_key_id,
                secret_access_key,
                session_token,
                region,
            } => read_bedrock_access_key_secret_value(
                access_key_id,
                secret_access_key,
                session_token,
                region,
            ),
            SecretInput::OpenaiApiKey {
                value_args,
                base_url,
            } => read_openai_api_key_secret_value(&value_args, base_url),
        }
    }
}

/// Create a new secret. Dispatches to the provider subcommand if present.
fn create_secret(ctx: &mut AppContext, args: CreateSecretArgs) -> Result<()> {
    // Resolve provider subcommand into common fields plus a deferred value reader.
    let (name, input, description, scope) = match args.provider {
        Some(CreateProvider::Anthropic(anthropic)) => match anthropic.method {
            AnthropicMethod::ApiKey(a) => (
                a.common.name,
                SecretInput::Simple {
                    secret_type: SecretType::AnthropicApiKey,
                    value_args: a.value,
                },
                a.common.description,
                a.common.scope,
            ),
            AnthropicMethod::BedrockApiKey(a) => (
                a.common.name,
                SecretInput::Bedrock {
                    bedrock_api_key: a.bedrock_api_key,
                    region: a.region,
                },
                a.common.description,
                a.common.scope,
            ),
            AnthropicMethod::BedrockAccessKey(a) => (
                a.common.name,
                SecretInput::BedrockAccessKey {
                    access_key_id: a.access_key_id,
                    secret_access_key: a.secret_access_key,
                    session_token: a.session_token,
                    region: a.region,
                },
                a.common.description,
                a.common.scope,
            ),
        },
        Some(CreateProvider::Codex(codex)) => match codex.method {
            CodexMethod::ApiKey(a) => (
                a.common.name,
                SecretInput::OpenaiApiKey {
                    value_args: a.value,
                    base_url: a.base_url,
                },
                a.common.description,
                a.common.scope,
            ),
        },
        None => {
            let name = args.name.ok_or_else(|| {
                anyhow::anyhow!("Secret name is required. Usage: oz secret create <NAME>")
            })?;
            (
                name,
                SecretInput::Simple {
                    secret_type: args.secret_type,
                    value_args: args.value,
                },
                args.description,
                args.scope,
            )
        }
    };

    create_secret_with_input(ctx, name, input, description, scope)
}

/// Shared creation logic: refreshes metadata, resolves the owner, reads the secret value, and
/// creates the secret.
fn create_secret_with_input(
    ctx: &mut AppContext,
    name: String,
    input: SecretInput,
    description: Option<String>,
    scope: ObjectScope,
) -> Result<()> {
    ManagedSecretManager::handle(ctx).update(ctx, move |_manager, ctx| {
        // Perform as much validation as possible up-front, before prompting the user for a secret.
        // It's a bad UX if we make them type in a secret and then fail on something we could have
        // checked beforehand.
        let refresh_future = super::common::refresh_workspace_metadata(ctx);
        ctx.spawn(refresh_future, move |manager, refresh_result, ctx| {
            if let Err(err) = refresh_result {
                super::report_fatal_error(err, ctx);
                return;
            }

            let owner = match super::common::resolve_owner(scope.team, scope.personal, ctx) {
                Ok(owner) => owner,
                Err(err) => {
                    super::report_fatal_error(err, ctx);
                    return;
                }
            };

            let managed_value = match input.read() {
                Ok(Some(value)) => value,
                Ok(None) => {
                    // Treat this as a cancellation.
                    ctx.terminate_app(TerminationMode::ForceTerminate, None);
                    return;
                }
                Err(err) => {
                    super::report_fatal_error(err, ctx);
                    return;
                }
            };

            let secret_owner = match owner {
                Owner::User { .. } => SecretOwner::CurrentUser,
                Owner::Team { team_uid } => SecretOwner::Team {
                    team_uid: team_uid.uid(),
                },
            };

            let create_future = manager.create_secret(
                secret_owner,
                name.clone(),
                managed_value,
                description.clone(),
            );
            ctx.spawn(create_future, move |_, result, ctx| match result {
                Ok(secret) => {
                    println!("Secret '{}' created", secret.name);
                    ctx.terminate_app(TerminationMode::ForceTerminate, None);
                }
                Err(err) => {
                    super::report_fatal_error(err, ctx);
                }
            });
        });
    });

    Ok(())
}

/// Delete a secret.
fn delete_secret(ctx: &mut AppContext, args: DeleteSecretArgs) -> Result<()> {
    let name = args.name;
    let force = args.force;
    let team = args.scope.team;
    let personal = args.scope.personal;

    ManagedSecretManager::handle(ctx).update(ctx, move |_manager, ctx| {
        let refresh_future = super::common::refresh_workspace_metadata(ctx);
        let name = name.clone();
        ctx.spawn(refresh_future, move |manager, refresh_result, ctx| {
            if let Err(err) = refresh_result {
                super::report_fatal_error(err, ctx);
                return;
            }

            let owner = match super::common::resolve_owner(team, personal, ctx) {
                Ok(owner) => owner,
                Err(err) => {
                    super::report_fatal_error(err, ctx);
                    return;
                }
            };

            let secret_owner = match owner {
                Owner::User { .. } => SecretOwner::CurrentUser,
                Owner::Team { team_uid } => SecretOwner::Team {
                    team_uid: team_uid.uid(),
                },
            };

            if !force {
                if !io::stdin().is_terminal() {
                    super::report_fatal_error(
                        anyhow::anyhow!(
                            "Refusing to delete secret without confirmation in non-interactive mode (use --force to bypass)"
                        ),
                        ctx,
                    );
                    return;
                }

                let scope = match owner {
                    Owner::User { .. } => "personal",
                    Owner::Team { .. } => "team",
                };

                let should_delete = match Confirm::new(&format!("Delete {scope} secret '{name}'?"))
                    .with_default(false)
                    .with_help_message("This action cannot be undone")
                    .prompt()
                {
                    Ok(should_delete) => should_delete,
                    Err(InquireError::OperationCanceled | InquireError::OperationInterrupted) => {
                        ctx
                            .terminate_app(TerminationMode::ForceTerminate, None);
                        return;
                    }
                    Err(err) => {
                        super::report_fatal_error(err.into(), ctx);
                        return;
                    }
                };

                if !should_delete {
                    println!("Deletion cancelled");
                    ctx
                        .terminate_app(TerminationMode::ForceTerminate, None);
                    return;
                }
            }

            let delete_future = manager.delete_secret(secret_owner, name.clone());
            ctx.spawn(delete_future, move |_, result, ctx| match result {
                Ok(()) => {
                    println!("Secret '{name}' deleted");
                    ctx
                        .terminate_app(TerminationMode::ForceTerminate, None);
                }
                Err(err) => {
                    super::report_fatal_error(err, ctx);
                }
            });
        });
    });

    Ok(())
}

/// Update a secret.
fn update_secret(ctx: &mut AppContext, args: UpdateSecretArgs) -> Result<()> {
    ManagedSecretManager::handle(ctx).update(ctx, move |_manager, ctx| {
        // Perform as much validation as possible up-front, before prompting the user for a secret.
        let refresh_future = super::common::refresh_workspace_metadata(ctx);
        ctx.spawn(refresh_future, move |manager, refresh_result, ctx| {
            if let Err(err) = refresh_result {
                super::report_fatal_error(err, ctx);
                return;
            }

            let owner =
                match super::common::resolve_owner(args.scope.team, args.scope.personal, ctx) {
                    Ok(owner) => owner,
                    Err(err) => {
                        super::report_fatal_error(err, ctx);
                        return;
                    }
                };

            // Read the secret value if either --value or --value-file is provided.
            let secret_value = if args.value || args.value_args.value_file.is_some() {
                // Create ValueArgs to handle reading from file or prompting
                match read_simple_secret_value(&args.value_args) {
                    Ok(Some(value)) => Some(value),
                    Ok(None) => {
                        // Treat this as a cancellation.
                        ctx.terminate_app(TerminationMode::ForceTerminate, None);
                        return;
                    }
                    Err(err) => {
                        super::report_fatal_error(err, ctx);
                        return;
                    }
                }
            } else {
                None
            };

            let secret_owner = match owner {
                Owner::User { .. } => SecretOwner::CurrentUser,
                Owner::Team { team_uid } => SecretOwner::Team {
                    team_uid: team_uid.uid(),
                },
            };

            if let Some(secret_value) = secret_value {
                // Look up the existing secret's type so we use the correct ManagedSecretValue variant.
                let list_future = manager.list_secrets();
                ctx.spawn(list_future, move |manager, list_result, ctx| {
                    let secrets = match list_result {
                        Ok(secrets) => secrets,
                        Err(err) => {
                            super::report_fatal_error(err, ctx);
                            return;
                        }
                    };

                    let secret_type = match find_secret_type(&secrets, &args.name, &secret_owner) {
                        Some(t) => t,
                        None => {
                            super::report_fatal_error(
                                anyhow::anyhow!("Secret '{}' not found", args.name),
                                ctx,
                            );
                            return;
                        }
                    };

                    let managed_secret_value =
                        match make_secret_value_from_gql_type(secret_type, &secret_value) {
                            Ok(v) => v,
                            Err(err) => {
                                super::report_fatal_error(err, ctx);
                                return;
                            }
                        };
                    let update_future = manager.update_secret(
                        secret_owner,
                        args.name.clone(),
                        Some(managed_secret_value),
                        args.description.clone(),
                    );
                    ctx.spawn(update_future, move |_, result, ctx| match result {
                        Ok(secret) => {
                            println!("Secret '{}' updated", secret.name);
                            ctx.terminate_app(TerminationMode::ForceTerminate, None);
                        }
                        Err(err) => {
                            super::report_fatal_error(err, ctx);
                        }
                    });
                });
            } else {
                // Description-only update; no encryption needed.
                let update_future = manager.update_secret(
                    secret_owner,
                    args.name.clone(),
                    None,
                    args.description.clone(),
                );
                ctx.spawn(update_future, move |_, result, ctx| match result {
                    Ok(secret) => {
                        println!("Secret '{}' updated", secret.name);
                        ctx.terminate_app(TerminationMode::ForceTerminate, None);
                    }
                    Err(err) => {
                        super::report_fatal_error(err, ctx);
                    }
                });
            }
        });
    });

    Ok(())
}

/// List secrets.
fn list_secrets(
    ctx: &mut AppContext,
    output_format: OutputFormat,
    _args: ListSecretsArgs,
) -> Result<()> {
    ManagedSecretManager::handle(ctx).update(ctx, |manager, ctx| {
        ctx.spawn(manager.list_secrets(), move |_, result, ctx| match result {
            Ok(secrets) => {
                let secret_infos = secrets.into_iter().map(|secret| {
                    let owner = match secret.owner.type_ {
                        SpaceType::User => Owner::User {
                            user_uid: UserUid::new(secret.owner.uid.inner()),
                        },
                        SpaceType::Team => Owner::Team {
                            team_uid: ServerId::from_string_lossy(secret.owner.uid.inner()),
                        },
                    };

                    SecretInfo {
                        name: secret.name,
                        scope: super::common::format_owner(&owner).to_string(),
                        secret_type: secret.type_,
                        created_at: secret.created_at.utc(),
                        updated_at: secret.updated_at.utc(),
                    }
                });

                output::print_list(secret_infos, output_format);

                ctx.terminate_app(TerminationMode::ForceTerminate, None);
            }
            Err(err) => {
                super::report_fatal_error(err, ctx);
            }
        });
    });
    Ok(())
}
/// Read a raw secret string from either the provided file or stdin.
fn read_simple_secret_value(args: &ValueArgs) -> Result<Option<String>> {
    if let Some(value_file) = args.value_file.as_ref() {
        let value = fs::read_to_string(value_file).with_context(|| {
            format!("Failed to read secret value from: {}", value_file.display())
        })?;
        if value.is_empty() {
            Ok(None)
        } else {
            Ok(Some(value))
        }
    } else if io::stdin().is_terminal() {
        let result = Password::new("Secret value:")
            .with_display_toggle_enabled()
            .without_confirmation()
            .prompt();
        match result {
            Ok(value) => {
                if value.is_empty() {
                    Ok(None)
                } else {
                    Ok(Some(value))
                }
            }
            Err(InquireError::OperationCanceled | InquireError::OperationInterrupted) => Ok(None),
            Err(err) => Err(err.into()),
        }
    } else {
        let mut buf = String::new();
        io::stdin().read_to_string(&mut buf)?;
        if buf.is_empty() {
            Ok(None)
        } else {
            Ok(Some(buf))
        }
    }
}

/// Constructs the appropriate [`ManagedSecretValue`] for single-field CLI secret types.
fn make_simple_secret_value(secret_type: SecretType, raw: &str) -> ManagedSecretValue {
    match secret_type {
        SecretType::RawValue => ManagedSecretValue::raw_value(raw),
        SecretType::AnthropicApiKey => ManagedSecretValue::anthropic_api_key(raw),
        SecretType::AnthropicBedrockApiKey => {
            // Bedrock secrets are multi-field and handled via SecretInput::Bedrock.
            unreachable!("Bedrock secrets should not go through make_simple_secret_value")
        }
        SecretType::OpenaiApiKey => {
            // OpenAI API key secrets are handled via SecretInput::OpenaiApiKey so the optional
            // base URL can be plumbed through alongside the API key.
            unreachable!("OpenAI API key secrets should not go through make_simple_secret_value")
        }
    }
}

/// Constructs the appropriate [`ManagedSecretValue`] for the given GraphQL secret type.
/// Used when updating an existing secret whose type is fetched from the server.
fn make_secret_value_from_gql_type(
    gql_type: ManagedSecretType,
    raw: &str,
) -> Result<ManagedSecretValue> {
    match gql_type {
        ManagedSecretType::RawValue | ManagedSecretType::Dotenvx => {
            Ok(ManagedSecretValue::raw_value(raw))
        }
        ManagedSecretType::AnthropicApiKey => Ok(ManagedSecretValue::anthropic_api_key(raw)),
        ManagedSecretType::AnthropicBedrockAccessKey => {
            // Bedrock access key secrets cannot be updated through the generic raw-string path.
            Err(anyhow::anyhow!(
                "Bedrock access key secrets cannot be updated via `--value`; re-create the secret instead"
            ))
        }
        ManagedSecretType::AnthropicBedrockApiKey => {
            // Bedrock secrets cannot be updated through the generic raw-string path.
            // The caller should use the dedicated Bedrock creation flow instead.
            Err(anyhow::anyhow!(
                "Bedrock API key secrets cannot be updated via `--value`; re-create the secret instead"
            ))
        }
        ManagedSecretType::OpenaiApiKey => Ok(ManagedSecretValue::openai_api_key(raw, None)),
    }
}

/// Read an OpenAI API key secret from CLI flags or interactive prompts.
///
/// The API key value is read from `--value-file`, stdin, or an interactive password prompt (in
/// that order), matching the behavior of other simple secret types.
///
/// `base_url` is optional. When `--base-url` is provided we use it verbatim (an empty value is
/// treated as "no base URL"). When it is not provided and we are running interactively, we
/// prompt for it; pressing Enter at the prompt skips the base URL. In non-interactive mode we
/// silently default to no base URL, since the vast majority of users use the provider's default
/// endpoint.
fn read_openai_api_key_secret_value(
    value_args: &ValueArgs,
    base_url: Option<String>,
) -> Result<Option<ManagedSecretValue>> {
    let api_key = match read_simple_secret_value(value_args)? {
        Some(v) => v,
        None => return Ok(None),
    };

    // The base URL is optional. An empty `--base-url` flag, empty interactive submission, or
    // omission in non-interactive mode all map to `None` so we never persist a blank string.
    let base_url: Option<String> = match base_url {
        Some(s) => {
            let trimmed = s.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_owned())
            }
        }
        None => {
            if !io::stdin().is_terminal() {
                // Non-interactive: leave the base URL unset rather than prompting or failing.
                None
            } else {
                match inquire::Text::new("OpenAI base URL (optional, press Enter to skip):")
                    .with_help_message("e.g. https://us.api.openai.com/v1 for a regional endpoint")
                    .prompt()
                {
                    Ok(value) => {
                        let trimmed = value.trim();
                        if trimmed.is_empty() {
                            None
                        } else {
                            Some(trimmed.to_owned())
                        }
                    }
                    Err(InquireError::OperationCanceled | InquireError::OperationInterrupted) => {
                        return Ok(None);
                    }
                    Err(err) => return Err(err.into()),
                }
            }
        }
    };

    Ok(Some(ManagedSecretValue::openai_api_key(api_key, base_url)))
}

/// Read a Bedrock API key secret from dedicated CLI flags or interactive prompts.
fn read_bedrock_secret_value(
    bedrock_api_key: Option<String>,
    region: Option<String>,
) -> Result<Option<ManagedSecretValue>> {
    let api_key = match bedrock_api_key {
        Some(k) if !k.is_empty() => k,
        _ => {
            if !io::stdin().is_terminal() {
                return Err(anyhow::anyhow!(
                    "Bedrock secrets require --bedrock-api-key and --region in non-interactive mode"
                ));
            }
            let result = Password::new("Bedrock API key:")
                .with_display_toggle_enabled()
                .without_confirmation()
                .prompt();
            match result {
                Ok(value) if !value.is_empty() => value,
                Ok(_) => return Ok(None),
                Err(InquireError::OperationCanceled | InquireError::OperationInterrupted) => {
                    return Ok(None);
                }
                Err(err) => return Err(err.into()),
            }
        }
    };

    let region = match region {
        Some(r) if !r.is_empty() => r,
        _ => {
            if !io::stdin().is_terminal() {
                return Err(anyhow::anyhow!(
                    "Bedrock secrets require --bedrock-api-key and --region in non-interactive mode"
                ));
            }
            let result = inquire::Text::new("AWS Region:").prompt();
            match result {
                Ok(value) if !value.is_empty() => value,
                Ok(_) => return Ok(None),
                Err(InquireError::OperationCanceled | InquireError::OperationInterrupted) => {
                    return Ok(None);
                }
                Err(err) => return Err(err.into()),
            }
        }
    };

    Ok(Some(ManagedSecretValue::anthropic_bedrock_api_key(
        api_key, region,
    )))
}

/// Read a Bedrock access key secret from dedicated CLI flags or interactive prompts.
///
/// `session_token` is optional: if the user passes an empty `--session-token`
/// value or hits Enter at the interactive prompt, no session token is stored.
/// This supports persistent IAM credentials, which do not require a session token.
fn read_bedrock_access_key_secret_value(
    access_key_id: Option<String>,
    secret_access_key: Option<String>,
    session_token: Option<String>,
    region: Option<String>,
) -> Result<Option<ManagedSecretValue>> {
    // Error message used for all three required fields when running non-interactively.
    // --session-token is intentionally omitted because it is optional.
    const NON_INTERACTIVE_REQUIRED_MSG: &str = "Bedrock access key secrets require --access-key-id, --secret-access-key, and --region in non-interactive mode";

    let access_key_id = match access_key_id {
        Some(v) if !v.is_empty() => v,
        _ => {
            if !io::stdin().is_terminal() {
                return Err(anyhow::anyhow!(NON_INTERACTIVE_REQUIRED_MSG));
            }
            match inquire::Text::new("AWS Access Key ID:").prompt() {
                Ok(value) if !value.is_empty() => value,
                Ok(_) => return Ok(None),
                Err(InquireError::OperationCanceled | InquireError::OperationInterrupted) => {
                    return Ok(None);
                }
                Err(err) => return Err(err.into()),
            }
        }
    };

    let secret_access_key = match secret_access_key {
        Some(v) if !v.is_empty() => v,
        _ => {
            if !io::stdin().is_terminal() {
                return Err(anyhow::anyhow!(NON_INTERACTIVE_REQUIRED_MSG));
            }
            match Password::new("AWS Secret Access Key:")
                .with_display_toggle_enabled()
                .without_confirmation()
                .prompt()
            {
                Ok(value) if !value.is_empty() => value,
                Ok(_) => return Ok(None),
                Err(InquireError::OperationCanceled | InquireError::OperationInterrupted) => {
                    return Ok(None);
                }
                Err(err) => return Err(err.into()),
            }
        }
    };

    // The session token is optional. An empty --session-token flag or an empty
    // interactive submission is treated as "no token" rather than as a cancel.
    let session_token: Option<String> = match session_token {
        Some(v) if !v.is_empty() => Some(v),
        Some(_) => None,
        None => {
            if !io::stdin().is_terminal() {
                // In non-interactive mode, omitting --session-token is fine:
                // persistent IAM credentials do not need a session token.
                None
            } else {
                match Password::new("AWS Session Token (optional, press Enter to skip):")
                    .with_display_toggle_enabled()
                    .without_confirmation()
                    .prompt()
                {
                    Ok(value) if !value.is_empty() => Some(value),
                    // Empty input signals "no session token", not a cancel.
                    Ok(_) => None,
                    Err(InquireError::OperationCanceled | InquireError::OperationInterrupted) => {
                        return Ok(None);
                    }
                    Err(err) => return Err(err.into()),
                }
            }
        }
    };

    let region = match region {
        Some(r) if !r.is_empty() => r,
        _ => {
            if !io::stdin().is_terminal() {
                return Err(anyhow::anyhow!(NON_INTERACTIVE_REQUIRED_MSG));
            }
            match inquire::Text::new("AWS Region:").prompt() {
                Ok(value) if !value.is_empty() => value,
                Ok(_) => return Ok(None),
                Err(InquireError::OperationCanceled | InquireError::OperationInterrupted) => {
                    return Ok(None);
                }
                Err(err) => return Err(err.into()),
            }
        }
    };

    Ok(Some(ManagedSecretValue::anthropic_bedrock_access_key(
        access_key_id,
        secret_access_key,
        session_token,
        region,
    )))
}

/// Finds the type of an existing secret by name and owner scope.
fn find_secret_type(
    secrets: &[ManagedSecret],
    name: &str,
    owner: &SecretOwner,
) -> Option<ManagedSecretType> {
    secrets
        .iter()
        .find(|s| {
            s.name == name
                && match owner {
                    SecretOwner::CurrentUser => matches!(s.owner.type_, SpaceType::User),
                    SecretOwner::Team { team_uid } => {
                        matches!(s.owner.type_, SpaceType::Team) && s.owner.uid.inner() == team_uid
                    }
                }
        })
        .map(|s| s.type_)
}

fn format_secret_type(type_: &ManagedSecretType) -> String {
    match type_ {
        ManagedSecretType::RawValue => "Raw Value".to_string(),
        ManagedSecretType::Dotenvx => "dotenvx".to_string(),
        ManagedSecretType::AnthropicApiKey => "Anthropic API Key".to_string(),
        ManagedSecretType::AnthropicBedrockAccessKey => "Anthropic Bedrock Access Key".to_string(),
        ManagedSecretType::AnthropicBedrockApiKey => "Anthropic Bedrock API Key".to_string(),
        ManagedSecretType::OpenaiApiKey => "OpenAI API Key".to_string(),
    }
}
