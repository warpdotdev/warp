//! Structured identity for plugin packages and the components they provide.
//!
//! Identity is structured rather than a display string so routing stays stable while UI and
//! model adapters render the qualified `<plugin>:<component>` label at their boundaries. Source
//! identity is deliberately opaque: adding a remote source kind later must not change the
//! identity of a component that a conversation already referenced.
use std::fmt;
use std::fmt::Write as _;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Separator between a plugin name and a component name in a qualified component name.
pub const QUALIFIED_NAME_SEPARATOR: char = ':';

/// Where a plugin package was sourced from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum PluginSourceKind {
    /// A `.agents/plugins` directory, in the user's home directory or a repository.
    AgentsDirectory,
    /// A Warp config `plugins` directory, in the Warp home config directory or a repository's
    /// `.warp` directory.
    WarpDirectory,
    /// A plugin collection inside a checked-out Factory source repository.
    FactoryRepository,
}

impl PluginSourceKind {
    /// Rank used to break ties between two sources at the same scope. Lower wins.
    ///
    /// This mirrors the `.agents`-before-`.warp` order in the flat skill provider list.
    pub fn provider_rank(self) -> u8 {
        match self {
            PluginSourceKind::AgentsDirectory => 0,
            PluginSourceKind::WarpDirectory => 1,
            PluginSourceKind::FactoryRepository => 2,
        }
    }
}

/// Identifies the provider directory a package came from, independently of its version.
///
/// `stable_identity` names the user root, repository, or Factory source. It is opaque at
/// component boundaries.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct PluginSourceId {
    pub kind: PluginSourceKind,
    pub stable_identity: String,
}

impl PluginSourceId {
    pub fn new(kind: PluginSourceKind, stable_identity: impl Into<String>) -> Self {
        Self {
            kind,
            stable_identity: stable_identity.into(),
        }
    }
}

/// The scope a plugin instance belongs to.
///
/// Scope separates otherwise identically named packages so their runtime state — most
/// importantly their persistent data directory — never collides.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum PluginScopeId {
    /// A plugin discovered under one of the user's home search roots.
    User,
    /// A plugin discovered under a repository search root.
    Repository,
    /// A factory-scoped Factory plugin.
    Factory,
    /// An agent-scoped Factory plugin.
    Agent { name: String },
    /// An automation-scoped Factory plugin.
    Automation { name: String },
}

impl PluginScopeId {
    /// Rank used to order two candidates for the same plugin name. Lower wins.
    ///
    /// Repository scope outranks user scope for interactive sessions, and the Factory scopes
    /// order automation over agent over factory.
    pub fn scope_rank(&self) -> u8 {
        match self {
            PluginScopeId::Automation { .. } => 0,
            PluginScopeId::Agent { .. } => 1,
            PluginScopeId::Factory => 2,
            PluginScopeId::Repository => 3,
            PluginScopeId::User => 4,
        }
    }

    /// A short stable token for this scope, used when deriving persistent data keys.
    pub fn key_token(&self) -> String {
        match self {
            PluginScopeId::User => "user".to_owned(),
            PluginScopeId::Repository => "repository".to_owned(),
            PluginScopeId::Factory => "factory".to_owned(),
            PluginScopeId::Agent { name } => format!("agent/{name}"),
            PluginScopeId::Automation { name } => format!("automation/{name}"),
        }
    }

    /// The single directory segment this scope contributes to a Factory's plugin data path.
    ///
    /// Flat, never nested, per the cross-repo contract's `scope_segment` mapping: an agent
    /// contributes `agent-<name>` rather than `agent/<name>`, so the composed path always has
    /// exactly two segments below the durable root. An agent or automation name comes from a
    /// repository, so it is sanitized before it becomes part of a directory name; a conformant
    /// name sanitizes to itself, which keeps real paths legible.
    pub fn path_segment(&self) -> String {
        match self {
            PluginScopeId::User => "user".to_owned(),
            PluginScopeId::Repository => "repository".to_owned(),
            PluginScopeId::Factory => "factory".to_owned(),
            PluginScopeId::Agent { name } => format!("agent-{}", filesystem_safe_segment(name)),
            PluginScopeId::Automation { name } => {
                format!("automation-{}", filesystem_safe_segment(name))
            }
        }
    }
}

/// Reduces an arbitrary name to one safe path segment.
///
/// Anything outside `[a-z0-9._-]` becomes `-`. A name that had to be changed, or that reduces to
/// a reserved or empty segment, gets a short digest suffix so two different names cannot collapse
/// onto one directory. A conformant plugin name (Agent Plugins §5.5 already restricts the
/// character set) passes through untouched, which keeps real paths readable.
pub fn filesystem_safe_segment(name: &str) -> String {
    let sanitized: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '-' | '_') {
                c
            } else if c.is_ascii_uppercase() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();

    let is_reserved = sanitized.is_empty() || sanitized == "." || sanitized == "..";
    if sanitized == name && !is_reserved {
        return sanitized;
    }

    let mut hasher = Sha256::new();
    hasher.update(name.as_bytes());
    let digest = hasher.finalize();
    let suffix = digest
        .iter()
        .take(4)
        .fold(String::with_capacity(8), |mut out, byte| {
            let _ = write!(out, "{byte:02x}");
            out
        });
    if is_reserved {
        suffix
    } else {
        format!("{sanitized}-{suffix}")
    }
}

impl fmt::Display for PluginScopeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PluginScopeId::User => write!(f, "user"),
            PluginScopeId::Repository => write!(f, "repository"),
            PluginScopeId::Factory => write!(f, "factory"),
            PluginScopeId::Agent { name } => write!(f, "agent {name}"),
            PluginScopeId::Automation { name } => write!(f, "automation {name}"),
        }
    }
}

/// Identifies one loaded plugin instance.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct PluginInstanceId {
    pub scope: PluginScopeId,
    pub source: PluginSourceId,
    pub manifest_name: String,
}

impl PluginInstanceId {
    pub fn new(
        scope: PluginScopeId,
        source: PluginSourceId,
        manifest_name: impl Into<String>,
    ) -> Self {
        Self {
            scope,
            source,
            manifest_name: manifest_name.into(),
        }
    }

    /// Precedence tuple for shadowing: `(scope rank, provider rank)`. Lower wins.
    pub fn precedence(&self) -> (u8, u8) {
        (self.scope.scope_rank(), self.source.kind.provider_rank())
    }
}

/// The standard component types Agent Plugins 1.0.0 defines.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum PluginComponentKind {
    Skill,
    McpServer,
}

impl fmt::Display for PluginComponentKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PluginComponentKind::Skill => write!(f, "skill"),
            PluginComponentKind::McpServer => write!(f, "MCP server"),
        }
    }
}

/// Identifies one component provided by one plugin instance.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct PluginComponentId {
    pub plugin: PluginInstanceId,
    pub kind: PluginComponentKind,
    pub local_name: String,
}

impl PluginComponentId {
    pub fn new(
        plugin: PluginInstanceId,
        kind: PluginComponentKind,
        local_name: impl Into<String>,
    ) -> Self {
        Self {
            plugin,
            kind,
            local_name: local_name.into(),
        }
    }

    /// The `<plugin>:<component>` name shown to users and sent to the model.
    ///
    /// This is runtime identity metadata. The component's own portable metadata — a skill's
    /// frontmatter `name`, an MCP server's native tool names — is never rewritten.
    pub fn qualified_name(&self) -> String {
        format!(
            "{}{QUALIFIED_NAME_SEPARATOR}{}",
            self.plugin.manifest_name, self.local_name
        )
    }
}

impl fmt::Display for PluginComponentId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.qualified_name())
    }
}

/// Splits a possibly qualified component name into its plugin and component parts.
///
/// Returns `None` when `name` has no separator, which means the caller must resolve it as an
/// unqualified name. A plugin name can itself contain periods but never a colon, so the first
/// separator is the boundary.
pub fn split_qualified_name(name: &str) -> Option<(&str, &str)> {
    let (plugin, component) = name.split_once(QUALIFIED_NAME_SEPARATOR)?;
    if plugin.is_empty() || component.is_empty() {
        return None;
    }
    Some((plugin, component))
}

#[cfg(test)]
#[path = "identity_tests.rs"]
mod tests;
