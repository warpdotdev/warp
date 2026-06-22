use std::time::SystemTime;

use chrono::{DateTime, Local};
use warp_core::ui::Icon;
use warp_multi_agent_api as api;

/// Temporary AWS credentials loaded from the AWS SDK.
/// These are not persisted and are only used at runtime.
#[derive(Clone, PartialEq, Eq)]
pub struct AwsCredentials {
    access_key: String,
    secret_key: String,
    session_token: Option<String>,
    expires_at: Option<SystemTime>,
}

impl std::fmt::Debug for AwsCredentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AwsCredentials")
            .field("access_key", &"<redacted>")
            .field("secret_key", &"<redacted>")
            .field(
                "session_token",
                &self.session_token.as_ref().map(|_| "<redacted>"),
            )
            .field("expires_at", &self.expires_at)
            .finish()
    }
}

impl AwsCredentials {
    pub fn new(
        access_key: String,
        secret_key: String,
        session_token: Option<String>,
        expires_at: Option<SystemTime>,
    ) -> Self {
        Self {
            access_key,
            secret_key,
            session_token,
            expires_at,
        }
    }

    pub fn expires_at(&self) -> Option<SystemTime> {
        self.expires_at
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AwsCredentialsState {
    Missing,
    Disabled,
    Refreshing,
    Loaded {
        credentials: AwsCredentials,
        loaded_at: SystemTime,
    },
    Failed {
        message: String,
    },
}

impl From<AwsCredentials> for api::request::settings::api_keys::AwsCredentials {
    fn from(creds: AwsCredentials) -> Self {
        Self {
            access_key: creds.access_key,
            secret_key: creds.secret_key,
            session_token: creds.session_token.unwrap_or_default(),
            region: String::new(),
        }
    }
}

fn format_status_timestamp(time: SystemTime) -> String {
    let datetime: DateTime<Local> = time.into();
    if datetime.date_naive() == Local::now().date_naive() {
        datetime.format("%-I:%M %p").to_string()
    } else {
        datetime.format("%b %-d at %-I:%M %p").to_string()
    }
}

impl AwsCredentialsState {
    pub fn user_facing_components(&self) -> (String, String, Icon) {
        match self {
            Self::Missing => (
                "AWS credentials not configured".to_string(),
                "Log in to the AWS CLI or configure AWS credentials for this profile, then refresh."
                    .to_string(),
                Icon::Key,
            ),
            Self::Disabled => (
                "AWS Bedrock Disabled".to_string(),
                "Warp will not load your AWS CLI credentials until AWS Bedrock is enabled by you or your workspace admin"
                    .to_string(),
                Icon::Key,
            ),
            Self::Refreshing => (
                "Refreshing credentials...".to_string(),
                "Loading your AWS CLI credentials into Warp".to_string(),
                Icon::RefreshCw04,
            ),
            Self::Loaded {
                credentials,
                loaded_at,
            } => (
                "Credentials loaded".to_string(),
                match credentials.expires_at() {
                    Some(expires_at) => format!(
                        "Loaded at {}, expires {}",
                        format_status_timestamp(*loaded_at),
                        format_status_timestamp(expires_at)
                    ),
                    None => format!("Loaded at {}", format_status_timestamp(*loaded_at)),
                },
                Icon::CheckCircleBroken,
            ),
            Self::Failed { message } => (
                "Unable to load credentials".to_string(),
                message.clone(),
                Icon::AlertTriangle,
            ),
        }
    }
}
