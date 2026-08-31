use std::path::PathBuf;

use anyhow::{Context, Result, anyhow};
use serde_json::Value;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AccountType {
    ChatGpt,
    ApiKey,
    AwsBedrock,
    Other(String),
}

impl AccountType {
    fn from_protocol(value: &str) -> Self {
        match value {
            "chatgpt" => Self::ChatGpt,
            "apiKey" => Self::ApiKey,
            "awsBedrock" | "bedrock" => Self::AwsBedrock,
            other => Self::Other(other.to_owned()),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Account {
    pub account_type: AccountType,
    pub email: Option<String>,
    pub plan_type: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AccountStatus {
    pub account: Option<Account>,
    pub requires_openai_auth: bool,
}

impl AccountStatus {
    pub(crate) fn from_value(value: Value) -> Result<Self> {
        let requires_openai_auth = value
            .get("requiresOpenaiAuth")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let account = value
            .get("account")
            .filter(|account| !account.is_null())
            .map(|account| -> Result<Account> {
                let account_type = account
                    .get("type")
                    .and_then(Value::as_str)
                    .context("Codex account response did not include an account type")?;
                Ok(Account {
                    account_type: AccountType::from_protocol(account_type),
                    email: account
                        .get("email")
                        .and_then(Value::as_str)
                        .map(str::to_owned),
                    plan_type: account
                        .get("planType")
                        .and_then(Value::as_str)
                        .map(str::to_owned),
                })
            })
            .transpose()?;

        Ok(Self {
            account,
            requires_openai_auth,
        })
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LoginMode {
    #[default]
    Browser,
    DeviceCode,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LoginChallenge {
    Browser {
        login_id: String,
        auth_url: String,
    },
    DeviceCode {
        login_id: String,
        verification_url: String,
        user_code: String,
    },
}

impl LoginChallenge {
    pub fn login_id(&self) -> &str {
        match self {
            Self::Browser { login_id, .. } | Self::DeviceCode { login_id, .. } => login_id,
        }
    }

    pub(crate) fn from_value(value: Value) -> Result<Self> {
        let kind = value
            .get("type")
            .and_then(Value::as_str)
            .context("Codex login response did not include a login type")?;
        let login_id = value
            .get("loginId")
            .and_then(Value::as_str)
            .context("Codex login response did not include a login id")?
            .to_owned();

        match kind {
            "chatgpt" => Ok(Self::Browser {
                login_id,
                auth_url: value
                    .get("authUrl")
                    .and_then(Value::as_str)
                    .context("Codex browser login response did not include an auth URL")?
                    .to_owned(),
            }),
            "chatgptDeviceCode" => Ok(Self::DeviceCode {
                login_id,
                verification_url: value
                    .get("verificationUrl")
                    .and_then(Value::as_str)
                    .context("Codex device login response did not include a verification URL")?
                    .to_owned(),
                user_code: value
                    .get("userCode")
                    .and_then(Value::as_str)
                    .context("Codex device login response did not include a user code")?
                    .to_owned(),
            }),
            other => Err(anyhow!("Unsupported Codex login response type: {other}")),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ApprovalPolicy {
    Untrusted,
    #[default]
    OnRequest,
    Never,
}

impl ApprovalPolicy {
    pub(crate) fn as_protocol(self) -> &'static str {
        match self {
            Self::Untrusted => "untrusted",
            Self::OnRequest => "on-request",
            Self::Never => "never",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SandboxMode {
    ReadOnly,
    #[default]
    WorkspaceWrite,
    DangerFullAccess,
}

impl SandboxMode {
    pub(crate) fn as_protocol(self) -> &'static str {
        match self {
            Self::ReadOnly => "read-only",
            Self::WorkspaceWrite => "workspace-write",
            Self::DangerFullAccess => "danger-full-access",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ThreadOptions {
    pub cwd: PathBuf,
    pub model: Option<String>,
    pub approval_policy: ApprovalPolicy,
    pub sandbox: SandboxMode,
    pub thread_source: String,
}

impl ThreadOptions {
    pub fn new(cwd: impl Into<PathBuf>) -> Self {
        Self {
            cwd: cwd.into(),
            model: None,
            approval_policy: ApprovalPolicy::OnRequest,
            sandbox: SandboxMode::WorkspaceWrite,
            thread_source: "warp".to_owned(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Notification {
    pub method: String,
    pub params: Value,
}

impl Notification {
    pub fn agent_message_delta(&self) -> Option<&str> {
        (self.method == "item/agentMessage/delta")
            .then(|| self.params.get("delta").and_then(Value::as_str))
            .flatten()
    }

    pub fn reasoning_delta(&self) -> Option<&str> {
        matches!(
            self.method.as_str(),
            "item/reasoning/textDelta" | "item/reasoning/summaryTextDelta"
        )
        .then(|| self.params.get("delta").and_then(Value::as_str))
        .flatten()
    }

    pub fn completed_agent_message(&self) -> Option<&str> {
        (self.method == "item/completed")
            .then(|| self.params.get("item"))
            .flatten()
            .filter(|item| item.get("type").and_then(Value::as_str) == Some("agentMessage"))
            .and_then(|item| item.get("text"))
            .and_then(Value::as_str)
    }

    pub fn command_output_delta(&self) -> Option<&str> {
        (self.method == "item/commandExecution/outputDelta")
            .then(|| self.params.get("delta").and_then(Value::as_str))
            .flatten()
    }

    pub fn file_change_output_delta(&self) -> Option<&str> {
        (self.method == "item/fileChange/outputDelta")
            .then(|| self.params.get("delta").and_then(Value::as_str))
            .flatten()
    }

    pub fn warning_message(&self) -> Option<&str> {
        (self.method == "warning")
            .then(|| self.params.get("message").and_then(Value::as_str))
            .flatten()
    }

    pub fn error_message(&self) -> Option<&str> {
        (self.method == "error")
            .then(|| self.params.get("error"))
            .flatten()
            .and_then(|error| {
                error
                    .get("message")
                    .and_then(Value::as_str)
                    .or_else(|| error.as_str())
            })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ServerRequest {
    pub id: Value,
    pub method: String,
    pub params: Value,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ServerRequestResponse {
    Result(Value),
    Error { code: i64, message: String },
}

impl ServerRequestResponse {
    pub fn result(value: Value) -> Self {
        Self::Result(value)
    }

    pub fn method_not_found(method: &str) -> Self {
        Self::Error {
            code: -32601,
            message: format!("Warp does not implement Codex server request {method}"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TurnResult {
    pub thread_id: String,
    pub turn_id: String,
    pub status: String,
    pub error: Option<String>,
}

/// A single event observed while an app-server turn is running.
///
/// The low-level client exposes this so rich clients can translate Codex's
/// streamed notifications into their own native event model without buffering
/// the whole response first.
#[derive(Clone, Debug, PartialEq)]
pub enum TurnEvent {
    Notification(Notification),
    Completed(TurnResult),
}
