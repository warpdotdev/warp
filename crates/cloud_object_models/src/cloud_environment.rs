use std::fmt;

use cloud_objects::cloud_object::{
    GenericCloudObject, GenericServerObject, GenericStringModel, JsonObjectType,
};
use cloud_objects::ids::GenericStringObjectId;
use serde::{Deserialize, Serialize};

use crate::{JsonModel, JsonSerializer};

/// Source-control provider hosting an environment's repositories.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum CodeForge {
    #[default]
    #[serde(rename = "GITHUB")]
    GitHub,
    #[serde(rename = "GITLAB")]
    GitLab,
}

impl CodeForge {
    pub const fn host(self) -> &'static str {
        match self {
            CodeForge::GitHub => "github.com",
            CodeForge::GitLab => "gitlab.com",
        }
    }
}

impl fmt::Display for CodeForge {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CodeForge::GitHub => write!(f, "GitHub"),
            CodeForge::GitLab => write!(f, "GitLab"),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct GithubRepo {
    /// Repository owner (e.g. "warpdotdev")
    pub owner: String,
    /// Repository name (e.g. "warp-internal")
    pub repo: String,
}

impl GithubRepo {
    pub fn new(owner: String, repo: String) -> Self {
        Self { owner, repo }
    }
}

impl fmt::Display for GithubRepo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.owner, self.repo)
    }
}

/// Identifies a repository and the source-control provider that hosts it.
///
/// For GitLab, `owner` contains the full, potentially nested namespace.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SourceRepo {
    /// The repository's explicit source-control provider.
    ///
    /// When absent, this inherits the associated environment's effective forge.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code_forge: Option<CodeForge>,
    pub owner: String,
    pub repo: String,
}

impl SourceRepo {
    pub fn new(code_forge: CodeForge, owner: String, repo: String) -> Self {
        Self {
            code_forge: Some(code_forge),
            owner,
            repo,
        }
    }
    pub fn with_default_code_forge(&self, code_forge: CodeForge) -> Self {
        Self::new(
            self.code_forge.unwrap_or(code_forge),
            self.owner.clone(),
            self.repo.clone(),
        )
    }

    pub fn https_clone_url(&self) -> String {
        format!(
            "https://{}/{}/{}.git",
            self.code_forge.unwrap_or_default().host(),
            self.owner,
            self.repo
        )
    }
}

/// Converts a legacy GitHub repository into the provider-neutral representation.
impl From<&GithubRepo> for SourceRepo {
    fn from(repo: &GithubRepo) -> Self {
        Self::new(CodeForge::GitHub, repo.owner.clone(), repo.repo.clone())
    }
}
/// Formats the forge-relative repository path.
impl fmt::Display for SourceRepo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.owner, self.repo)
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum BaseImage {
    DockerImage(String),
}

impl fmt::Display for BaseImage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BaseImage::DockerImage(s) => s.fmt(f),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct GcpProviderConfig {
    pub project_number: String,
    pub workload_identity_federation_pool_id: String,
    pub workload_identity_federation_provider_id: String,
    /// Service account email for impersonation. When set, the federated token
    /// is exchanged for a service account access token.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub service_account_email: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct AwsProviderConfig {
    pub role_arn: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Default)]
pub struct ProvidersConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gcp: Option<GcpProviderConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aws: Option<AwsProviderConfig>,
}

impl ProvidersConfig {
    pub fn is_empty(&self) -> bool {
        self.gcp.is_none() && self.aws.is_none()
    }
}

/// Identifies a managed secret configured on an environment.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EnvironmentSecretRef {
    pub name: String,
}

/// An AmbientAgentEnvironment represents an environment that we would run a Warp agent in.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct AmbientAgentEnvironment {
    /// Environment name
    #[serde(default)]
    pub name: String,
    /// Optional description of the environment (max 240 characters)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Source-control provider hosting this environment's repositories.
    ///
    /// Absent means GitHub for legacy environments.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code_forge: Option<CodeForge>,
    /// List of GitHub repositories
    #[serde(default)]
    pub github_repos: Vec<GithubRepo>,
    /// Provider-neutral repository list.
    ///
    /// When present, including when empty, this is authoritative over
    /// `github_repos`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_repos: Option<Vec<SourceRepo>>,
    /// Optional base image specification
    #[serde(flatten, default, skip_serializing_if = "Option::is_none")]
    pub base_image: Option<BaseImage>,
    /// List of setup commands to run after cloning
    #[serde(default)]
    pub setup_commands: Vec<String>,
    /// Optional cloud provider configurations for automatic auth.
    #[serde(default, skip_serializing_if = "ProvidersConfig::is_empty")]
    pub providers: ProvidersConfig,
    /// Default set of managed secrets for runs using this environment.
    ///   - `None`: no environment-level secret scoping (all secrets / defer to run config)
    ///   - `Some([])`: no secrets by default
    ///   - `Some([...])`: these specific secrets are the default
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secrets: Option<Vec<EnvironmentSecretRef>>,
}

impl AmbientAgentEnvironment {
    pub fn new(
        name: String,
        description: Option<String>,
        github_repos: Vec<GithubRepo>,
        docker_image: String,
        setup_commands: Vec<String>,
    ) -> Self {
        Self {
            name,
            description,
            code_forge: None,
            github_repos,
            source_repos: None,
            base_image: Some(BaseImage::DockerImage(docker_image)),
            setup_commands,
            providers: ProvidersConfig::default(),
            secrets: None,
        }
    }

    /// Returns the environment's source-control provider, defaulting to GitHub
    /// for legacy environments.
    pub fn effective_code_forge(&self) -> CodeForge {
        self.code_forge.unwrap_or_default()
    }

    /// Returns the authoritative provider-neutral repository list.
    pub fn effective_repos(&self) -> Vec<SourceRepo> {
        let code_forge = self.effective_code_forge();
        match &self.source_repos {
            Some(source_repos) => source_repos
                .iter()
                .map(|repo| repo.with_default_code_forge(code_forge))
                .collect(),
            None => self
                .github_repos
                .iter()
                .map(|repo| SourceRepo::new(code_forge, repo.owner.clone(), repo.repo.clone()))
                .collect(),
        }
    }
}

impl JsonModel for AmbientAgentEnvironment {
    fn json_object_type() -> JsonObjectType {
        JsonObjectType::CloudEnvironment
    }
}

pub type CloudAmbientAgentEnvironment =
    GenericCloudObject<GenericStringObjectId, CloudAmbientAgentEnvironmentModel>;
pub type CloudAmbientAgentEnvironmentModel =
    GenericStringModel<AmbientAgentEnvironment, JsonSerializer>;
pub type ServerAmbientAgentEnvironment =
    GenericServerObject<GenericStringObjectId, CloudAmbientAgentEnvironmentModel>;

#[cfg(test)]
#[path = "cloud_environment_tests.rs"]
mod tests;
