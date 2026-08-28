use std::collections::HashSet;
use std::fmt;

use cloud_objects::cloud_object::{
    GenericCloudObject, GenericServerObject, GenericStringModel, JsonObjectType,
};
use cloud_objects::ids::GenericStringObjectId;
use serde::{Deserialize, Serialize};

use crate::{JsonModel, JsonSerializer};

/// Source-control provider hosting an environment's repositories.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum CodeForge {
    #[default]
    #[serde(rename = "GITHUB")]
    GitHub,
    #[serde(rename = "GITLAB")]
    GitLab,
    /// Explicit "no code forge" container value: a repo-less environment
    /// that clones nothing and relies entirely on `setup_commands`.
    #[serde(rename = "NONE")]
    None,
    // Catches a forge value this client build doesn't recognize yet (e.g. the
    // server adds one before this client updates), so the rest of the
    // environment still deserializes instead of the whole object failing.
    #[serde(other)]
    Unknown,
}

impl CodeForge {
    /// The clonable host for this forge, empty for `None`/`Unknown` since
    /// neither identifies one; callers must not fall back to `github.com`
    /// for either, which would authenticate against the wrong host.
    pub const fn host(self) -> &'static str {
        match self {
            CodeForge::GitHub => "github.com",
            CodeForge::GitLab => "gitlab.com",
            CodeForge::None | CodeForge::Unknown => "",
        }
    }
}

impl fmt::Display for CodeForge {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CodeForge::GitHub => write!(f, "GitHub"),
            CodeForge::GitLab => write!(f, "GitLab"),
            CodeForge::None => write!(f, "None"),
            CodeForge::Unknown => write!(f, "Unknown"),
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
    /// When absent, a single-forge environment fills this from the container
    /// primary. A mixed environment leaves it unset so clone fails closed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code_forge: Option<CodeForge>,
    pub owner: String,
    pub repo: String,
    /// Ref to check out after cloning this repository (commit SHA, branch, or
    /// tag). Absent leaves the clone on the default branch. Benchmark trials
    /// use it to start from a pinned base commit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checkout_ref: Option<String>,
}

impl SourceRepo {
    pub fn new(code_forge: CodeForge, owner: String, repo: String) -> Self {
        Self {
            code_forge: Some(code_forge),
            owner,
            repo,
            checkout_ref: None,
        }
    }
    pub fn with_default_code_forge(&self, code_forge: CodeForge) -> Self {
        Self {
            code_forge: Some(self.code_forge.unwrap_or(code_forge)),
            owner: self.owner.clone(),
            repo: self.repo.clone(),
            checkout_ref: self.checkout_ref.clone(),
        }
    }
    /// Returns a copy of this repository pinned to `checkout_ref`.
    pub fn with_checkout_ref(mut self, checkout_ref: Option<String>) -> Self {
        self.checkout_ref = checkout_ref;
        self
    }

    pub fn https_clone_url(&self) -> String {
        format!(
            "https://{}/{}/{}.git",
            self.code_forge.map(CodeForge::host).unwrap_or(""),
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
    /// Absent means GitHub for legacy environments. This is the primary/legacy
    /// forge used to fill a repository that omits its own, never a mixed-host
    /// marker.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code_forge: Option<CodeForge>,
    /// Concrete forges enabled on this environment.
    ///
    /// Present (including empty) is authoritative. Absent is a legacy payload
    /// that predates the field; the set is then derived from `code_forge` and
    /// the repositories.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code_forges: Option<Vec<CodeForge>>,
    /// List of GitHub repositories
    #[serde(default)]
    pub github_repos: Vec<GithubRepo>,
    /// Provider-neutral repository list.
    ///
    /// When present, including when empty, this is authoritative over
    /// `github_repos`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_repos: Option<Vec<SourceRepo>>,
    /// Base image specification.
    ///
    /// Absent when the environment does not pin a base image. The server may
    /// omit the docker image, so the client must not fail to deserialize an
    /// environment that lacks one.
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
    /// Runner supplying compute for runs that do not name one themselves.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_runner_uid: Option<String>,
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
            code_forges: None,
            github_repos,
            source_repos: None,
            base_image: Some(BaseImage::DockerImage(docker_image)),
            setup_commands,
            providers: ProvidersConfig::default(),
            secrets: None,
            default_runner_uid: None,
        }
    }

    /// Returns the environment's primary source-control provider, defaulting to
    /// GitHub for legacy environments. This fills a repository that omits its
    /// own forge only when the environment is not mixed; see [`Self::effective_repos`].
    pub fn effective_code_forge(&self) -> CodeForge {
        self.code_forge.unwrap_or_default()
    }

    /// Returns the concrete forges enabled on this environment.
    ///
    /// Declared `code_forges` keep `Unknown` members so a future forge next to
    /// GitHub is not treated as a single-forge environment.
    pub fn effective_code_forges(&self) -> Vec<CodeForge> {
        if let Some(code_forges) = &self.code_forges {
            return unique_declared_forges(code_forges.iter().copied());
        }
        let mut forges = unique_clonable_forges(
            self.declared_repos()
                .into_iter()
                .filter_map(|repo| repo.code_forge),
        );
        match self.effective_code_forge() {
            CodeForge::None => {
                if forges.is_empty() {
                    return Vec::new();
                }
            }
            primary @ (CodeForge::GitHub | CodeForge::GitLab) => {
                if !forges.contains(&primary) {
                    forges.push(primary);
                    forges.sort_by_key(|forge| *forge as u8);
                }
            }
            CodeForge::Unknown => {}
        }
        forges
    }

    fn is_mixed_environment(&self) -> bool {
        let forges = self.effective_code_forges();
        forges.len() > 1 || forges.contains(&CodeForge::Unknown)
    }

    fn fill_forge_for_repo(&self, repo: &SourceRepo) -> Option<CodeForge> {
        if let Some(code_forge) = repo.code_forge {
            return Some(code_forge);
        }
        if self.is_mixed_environment() {
            return None;
        }
        Some(self.effective_code_forge())
    }

    fn declared_repos(&self) -> Vec<SourceRepo> {
        match &self.source_repos {
            Some(source_repos) => source_repos.clone(),
            None => self.github_repos.iter().map(SourceRepo::from).collect(),
        }
    }

    /// Display string for this environment's base image, empty when the
    /// environment does not pin a base image.
    pub fn base_image_display(&self) -> String {
        self.base_image
            .as_ref()
            .map(|image| image.to_string())
            .unwrap_or_default()
    }

    /// Returns the authoritative provider-neutral repository list.
    ///
    /// A repository that omits `code_forge` inherits the container primary in a
    /// single-forge environment. In a mixed environment it is left unset so the
    /// clone path can fail rather than silently choosing GitHub.
    pub fn effective_repos(&self) -> Vec<SourceRepo> {
        self.declared_repos()
            .into_iter()
            .map(|repo| SourceRepo {
                code_forge: self.fill_forge_for_repo(&repo),
                owner: repo.owner,
                repo: repo.repo,
                checkout_ref: repo.checkout_ref,
            })
            .collect()
    }
}

fn unique_declared_forges(forges: impl IntoIterator<Item = CodeForge>) -> Vec<CodeForge> {
    let mut seen = HashSet::new();
    let mut unique = Vec::new();
    for forge in forges {
        if matches!(forge, CodeForge::None) {
            continue;
        }
        if seen.insert(forge) {
            unique.push(forge);
        }
    }
    unique
}

fn unique_clonable_forges(forges: impl IntoIterator<Item = CodeForge>) -> Vec<CodeForge> {
    let mut seen = HashSet::new();
    let mut unique = Vec::new();
    for forge in forges {
        if matches!(forge, CodeForge::GitHub | CodeForge::GitLab) && seen.insert(forge) {
            unique.push(forge);
        }
    }
    unique
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
