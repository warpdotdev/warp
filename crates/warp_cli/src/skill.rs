use std::fmt;
use std::path::Path;
use std::str::FromStr;

/// A skill specifier that can reference a skill in a specific repo or search the current directory.
///
/// The skill identifier (after the optional `repo:` or `org/repo:` prefix) can be either:
/// - A **simple skill name** - searched across skill directories with precedence (`.agents/skills/`, `.warp/skills/`, `.claude/skills/`, `.codex/skills/`)
/// - A **full path to SKILL.md** - resolved directly without precedence
///
/// # Formats
/// - `skill_name` - Simple name, search current directory
/// - `skill_path` - Full path (e.g., `.claude/skills/foo/SKILL.md`)
/// - `repo:skill_name` - Simple name in specific repo
/// - `repo:skill_path` - Full path in specific repo
/// - `org/repo:skill_name` - Simple name with org and repo
/// - `org/repo:skill_path` - Full path with org and repo
///
/// # Examples
///
/// Simple skill names (searched with directory precedence):
/// ```ignore
/// code-review                              // searches .agents/skills/, .warp/skills/, .claude/skills/, .codex/skills/
/// warp-internal:code-review                // searches in "warp-internal" repo
/// warpdotdev/warp-internal:code-review     // searches in specific org/repo
/// ```
///
/// Full paths (resolved directly, no precedence):
/// ```ignore
/// .agents/skills/my-skill/SKILL.md                              // directly resolves this path
/// warp-server:.claude/skills/deploy/SKILL.md                    // exact path in "warp-server" repo
/// warpdotdev/warp-internal:.claude/skills/code-review/SKILL.md  // exact path in org/repo
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillSpec {
    /// Optional GitHub organization (e.g., "warpdotdev" in "warpdotdev/warp-internal:code-review")
    pub org: Option<String>,
    /// Optional repository name (e.g., "warp-internal")
    pub repo: Option<String>,
    /// The skill identifier - either a simple name or a full path to SKILL.md.
    ///
    /// - **Simple name** (e.g., `"code-review"`): Searched across `.agents/skills/`, `.warp/skills/`, `.claude/skills/`, `.codex/skills/`
    ///   in precedence order. The name is used to construct paths like `.claude/skills/code-review/SKILL.md`.
    ///
    /// - **Full path** (e.g., `".claude/skills/code-review/SKILL.md"`): Resolved directly without precedence.
    ///   Detected by presence of path separators (e.g., `/` or `\`).
    ///
    /// Use [`is_full_path()`](Self::is_full_path) to distinguish between the two formats.
    pub skill_identifier: String,
}

impl SkillSpec {
    /// Create a new skill spec with org and repo qualifiers.
    pub fn with_org_and_repo(org: String, repo: String, skill_identifier: String) -> Self {
        Self {
            org: Some(org),
            repo: Some(repo),
            skill_identifier,
        }
    }

    /// Create a new skill spec with a repo qualifier.
    pub fn with_repo(repo: String, skill_identifier: String) -> Self {
        Self {
            org: None,
            repo: Some(repo),
            skill_identifier,
        }
    }

    /// Create a new skill spec without any qualifier.
    pub fn without_repo(skill_identifier: String) -> Self {
        Self {
            org: None,
            repo: None,
            skill_identifier,
        }
    }

    /// Returns true if `skill_identifier` is a full path, false if it's a simple skill name.
    ///
    /// A full path contains path separators (`/` or `\`), such as:
    /// - `.claude/skills/deploy/SKILL.md`
    /// - `.agents/skills/my-skill/SKILL.md`
    ///
    /// A simple skill name has no path separators, such as:
    /// - `code-review`
    /// - `deploy`
    ///
    /// Full paths are resolved directly, while simple names are searched across
    /// skill directories in precedence order (`.agents/skills/`, `.warp/skills/`, `.claude/skills/`, `.codex/skills/`).
    ///
    /// Uses cross-platform path semantics via [`std::path::Path`].
    pub fn is_full_path(&self) -> bool {
        let path = Path::new(&self.skill_identifier);
        // A path with multiple components (e.g., "foo/bar" or "foo\\bar") is a full path.
        // A single component (e.g., "code-review") is just a name.
        path.components().count() > 1
    }

    /// Extracts the displayable skill name from this spec.
    ///
    /// # Returns
    /// - For path-style identifiers (e.g., `.agents/skills/slack-triage/SKILL.md`): returns the parent directory name
    /// - For simple names: returns the name as-is
    /// - For invalid paths: falls back to file stem or the identifier itself
    pub fn skill_name(&self) -> String {
        let skill_identifier = self.skill_identifier.trim();
        let path = Path::new(skill_identifier);

        if path.components().count() > 1 {
            if let Some(skill_name) = path
                .parent()
                .and_then(Path::file_name)
                .and_then(|name| name.to_str())
            {
                return skill_name.to_string();
            }

            if let Some(file_stem) = path.file_stem().and_then(|stem| stem.to_str()) {
                return file_stem.to_string();
            }
        }

        skill_identifier.to_string()
    }
}

impl FromStr for SkillSpec {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.trim();
        if s.is_empty() {
            return Err("Skill specifier cannot be empty".to_string());
        }

        // Check for [qualifier:]skill_identifier format
        if let Some((qualifier, skill_identifier)) = s.split_once(':') {
            let qualifier = qualifier.trim();
            let skill_identifier = skill_identifier.trim();

            if qualifier.is_empty() {
                return Err(
                    "Qualifier cannot be empty in 'repo:skill_identifier' format".to_string(),
                );
            }
            if skill_identifier.is_empty() {
                return Err("Skill identifier cannot be empty".to_string());
            }

            // Check for org/repo format in qualifier
            if let Some((org, repo)) = qualifier.split_once('/') {
                let org = org.trim();
                let repo = repo.trim();

                if org.is_empty() {
                    return Err("Organization cannot be empty".to_string());
                }
                if repo.is_empty() {
                    return Err("Repository name cannot be empty".to_string());
                }

                Ok(Self::with_org_and_repo(
                    org.to_string(),
                    repo.to_string(),
                    skill_identifier.to_string(),
                ))
            } else {
                Ok(Self::with_repo(
                    qualifier.to_string(),
                    skill_identifier.to_string(),
                ))
            }
        } else {
            Ok(Self::without_repo(s.to_string()))
        }
    }
}

impl fmt::Display for SkillSpec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match (&self.org, &self.repo) {
            (Some(org), Some(repo)) => write!(f, "{}/{}:{}", org, repo, self.skill_identifier),
            (None, Some(repo)) => write!(f, "{}:{}", repo, self.skill_identifier),
            _ => write!(f, "{}", self.skill_identifier),
        }
    }
}

#[cfg(test)]
#[path = "skill_tests.rs"]
mod tests;
