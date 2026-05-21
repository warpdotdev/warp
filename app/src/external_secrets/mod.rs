// Most of this module is dead code on web as it is not possible to retrieve
// external secrets from the browser.
#![cfg_attr(target_family = "wasm", allow(dead_code, unused_variables))]

use anyhow::anyhow;
use core::fmt;
use itertools::Itertools;
use lazy_static::lazy_static;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::PathBuf;
use warp_util::path::ShellFamily;

#[cfg(all(not(target_family = "wasm"), feature = "local_tty"))]
use crate::terminal::local_shell::execute_command;

use crate::{terminal::shell::ShellType, ui_components::icons::Icon};

lazy_static! {
    // Used as a delimeter to separate metadata (such as names and references)
    // in cases the cli tool doesn't display secrets in a common format (i.e. json)
    static ref WARP_SECRET_DELIMITER: &'static str = "/warp-secret-delimeter/";
    static ref LASTPASS_LIST_SECRETS_COMMAND: Vec<String> = {
        vec![
            "lpass".to_owned(),
            "ls".to_owned(),
            format!("--format=%an{}%ai", *WARP_SECRET_DELIMITER),
        ]
    };
}

const ONE_PASSWORD_INSTALLED_COMMAND: [&str; 2] = ["op", "-v"];
// 1Password has more categories (logins, servers, etc),
// but we're limiting our support to these as categories
// may differ in the way they extract sensitive information
// (i.e. api credentials use the --credential field seen
// in get_secret_extraction_command, whereas logins use
// the --password field). We're waiting to see how users
// use secrets to inform what to support.
const ONE_PASSWORD_LIST_SECRETS_COMMAND: [&str; 6] = [
    "op",
    "item",
    "list",
    "--categories",
    "Database,Api\\ Credential",
    "--format=json",
];
const LASTPASS_INSTALLED_COMMAND: [&str; 2] = ["lpass", "-v"];

const ONEPASSWORD_DOCS_LINK: &str = "https://developer.1password.com/docs/cli/get-started/";
const LASTPASS_DOCS_LINK: &str = "https://github.com/lastpass/lastpass-cli";

/// Represents a "completed" secret
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum ExternalSecret {
    OnePassword(OnePasswordSecret),
    LastPass(LastPassSecret),
}

impl ExternalSecret {
    pub fn get_secret_extraction_command(&self, shell_family: ShellFamily) -> String {
        let prefix = match shell_family {
            ShellFamily::Posix => "\\",
            ShellFamily::PowerShell => "",
        };
        match self {
            ExternalSecret::OnePassword(secret) => {
                format!(
                    "{}op item get --fields credential --reveal {}",
                    prefix, secret.reference
                )
            }
            ExternalSecret::LastPass(secret) => {
                format!("{}lpass show --password {}", prefix, secret.reference)
            }
        }
    }

    pub fn get_display_name(&self) -> String {
        match self {
            ExternalSecret::OnePassword(secret) => secret.name.clone(),
            ExternalSecret::LastPass(secret) => secret.name.clone(),
        }
    }
}

/// Used to check if a secret manager is installed/fetch list of secrets
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum SecretManager {
    OnePassword,
    LastPass,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum SecretErrorType {
    NotInstalled,
    FetchFailed,
    InvalidPlatform,
}

pub struct ErrorMessageAndCommand {
    pub message: String,
    pub link_message: Option<String>,
    pub link: Option<String>,
}

impl SecretManager {
    async fn is_installed(
        &self,
        shell_type: ShellType,
        shell_path: PathBuf,
        path_env_var: Option<String>,
    ) -> bool {
        #[cfg(all(not(target_family = "wasm"), feature = "local_tty"))]
        {
            match self {
                SecretManager::OnePassword => {
                    return execute_command(
                        shell_type,
                        shell_path,
                        path_env_var,
                        ONE_PASSWORD_INSTALLED_COMMAND.join(" ").as_str(),
                    )
                    .await
                    .is_ok()
                }
                SecretManager::LastPass => {
                    return execute_command(
                        shell_type,
                        shell_path,
                        path_env_var,
                        LASTPASS_INSTALLED_COMMAND.join(" ").as_str(),
                    )
                    .await
                    .is_ok()
                }
            }
        }
        #[allow(unreachable_code)]
        false
    }

    async fn fetch_secrets(
        &self,
        shell_type: ShellType,
        shell_path: PathBuf,
        path_env_var: Option<String>,
    ) -> Option<Vec<ExternalSecret>> {
        #[cfg(all(not(target_family = "wasm"), feature = "local_tty"))]
        {
            match self {
                SecretManager::OnePassword => {
                    return execute_command(
                        shell_type,
                        shell_path,
                        path_env_var,
                        ONE_PASSWORD_LIST_SECRETS_COMMAND.join(" ").as_str(),
                    )
                    .await
                    .ok()
                    .and_then(|output| parse_onepassword_secrets(&output).ok())
                }
                SecretManager::LastPass => {
                    let lastpass_command: Vec<&str> = LASTPASS_LIST_SECRETS_COMMAND
                        .iter()
                        .map(|s| s.as_str())
                        .collect();
                    return execute_command(
                        shell_type,
                        shell_path,
                        path_env_var,
                        lastpass_command.join(" ").as_str(),
                    )
                    .await
                    .ok()
                    .and_then(|output| parse_lastpass_secrets(&output).ok());
                }
            }
        }
        #[allow(unreachable_code)]
        None
    }

    pub async fn verify_installed_and_fetch_secrets(
        &self,
        shell_type: ShellType,
        shell_path: PathBuf,
        path_env_var: Option<String>,
    ) -> Result<Vec<ExternalSecret>, SecretErrorType> {
        #[cfg(not(target_family = "wasm"))]
        {
            let is_installed = self
                .is_installed(shell_type, shell_path.clone(), path_env_var.clone())
                .await;

            if !is_installed {
                return Err(SecretErrorType::NotInstalled);
            }

            let secrets = self
                .fetch_secrets(shell_type, shell_path, path_env_var)
                .await;

            if let Some(secrets) = secrets {
                return Ok(secrets);
            } else {
                return Err(SecretErrorType::FetchFailed);
            }
        }
        #[allow(unreachable_code)]
        Err(SecretErrorType::InvalidPlatform)
    }

    pub fn get_toast_message_and_link(
        &self,
        error_type: SecretErrorType,
    ) -> ErrorMessageAndCommand {
        match error_type {
            SecretErrorType::NotInstalled => {
                let message = format!("{} CLI is not installed", &self);

                let (link, link_message) = (
                    match self {
                        SecretManager::OnePassword => Some(ONEPASSWORD_DOCS_LINK.to_owned()),
                        SecretManager::LastPass => Some(LASTPASS_DOCS_LINK.to_owned()),
                    },
                    Some(format!("View {} CLI installation documentation", &self)),
                );

                ErrorMessageAndCommand {
                    message,
                    link,
                    link_message,
                }
            }
            SecretErrorType::FetchFailed => {
                let (link, link_message) = match self {
                    SecretManager::OnePassword => (
                        Some(ONEPASSWORD_DOCS_LINK.to_owned()),
                        Some("Integrate 1Password app with CLI".to_owned()),
                    ),
                    SecretManager::LastPass => (None, None),
                };
                ErrorMessageAndCommand {
                    message: format!(
                        "{} didn't return secrets (likely not configured or authenticated)",
                        &self
                    ),
                    link,
                    link_message,
                }
            }
            SecretErrorType::InvalidPlatform => ErrorMessageAndCommand {
                message: "Platform not supported".to_owned(),
                link: None,
                link_message: None,
            },
        }
    }
}

impl fmt::Display for SecretManager {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            SecretManager::OnePassword => write!(f, "1Password"),
            SecretManager::LastPass => write!(f, "LastPass"),
        }
    }
}

pub trait ExternalSecretManager {
    fn icon(&self) -> Icon;
}

impl ExternalSecretManager for ExternalSecret {
    fn icon(&self) -> Icon {
        match self {
            ExternalSecret::OnePassword(_) => Icon::OnePassword,
            ExternalSecret::LastPass(_) => Icon::LastPass,
        }
    }
}

impl ExternalSecretManager for SecretManager {
    fn icon(&self) -> Icon {
        match self {
            SecretManager::OnePassword => Icon::OnePassword,
            SecretManager::LastPass => Icon::LastPass,
        }
    }
}

fn parse_onepassword_secrets(output: &str) -> anyhow::Result<Vec<ExternalSecret>> {
    let parsed_output: Vec<ExternalSecret> = serde_json::from_str::<Value>(output)
        .map_err(|e| anyhow!(e))?
        .as_array()
        .ok_or(anyhow!("Expected array in JSON"))?
        .iter()
        .map(|secret| {
            let name = secret
                .get("title")
                .and_then(|v| v.as_str())
                .ok_or(anyhow!("Secret is missing title"))?;
            let reference = secret
                .get("id")
                .and_then(|v| v.as_str())
                .ok_or(anyhow!("Secret is missing id"))?;

            Ok(ExternalSecret::OnePassword(OnePasswordSecret {
                name: name.to_string(),
                reference: reference.to_string(),
            }))
        })
        .collect::<Result<Vec<_>, anyhow::Error>>()?;

    Ok(parsed_output)
}

fn parse_lastpass_secrets(output: &str) -> anyhow::Result<Vec<ExternalSecret>> {
    let parsed_output: Vec<ExternalSecret> = output
        .lines()
        .filter_map(|line| {
            let parts = line.split(*WARP_SECRET_DELIMITER).collect_vec();
            if parts.len() == 2 && !parts[0].is_empty() && !parts[1].is_empty() {
                Some(ExternalSecret::LastPass(LastPassSecret {
                    name: parts[0].to_owned(),
                    reference: parts[1].to_owned(),
                }))
            } else {
                None
            }
        })
        .collect();

    if !parsed_output.is_empty() {
        Ok(parsed_output)
    } else {
        Err(anyhow!("Failed to parse any secrets"))
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct OnePasswordSecret {
    name: String,
    reference: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct LastPassSecret {
    name: String,
    reference: String,
}
