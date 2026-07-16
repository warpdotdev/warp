use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};
use std::str::FromStr;
use std::{fmt, fs, io};

use anyhow::bail;
use clap::{Args, Subcommand};

/// Factory-related subcommands.
#[derive(Debug, Clone, Subcommand)]
pub enum FactoryCommand {
    /// Link a factory to a config source repository, making it file-managed.
    ///
    /// Registers a repository directory as the factory's config source. Re-linking
    /// replaces the existing source.
    Link(LinkFactoryArgs),
    /// Unlink a factory from its config source, returning it to live-managed.
    Unlink(UnlinkFactoryArgs),
    /// Show a factory's management mode, config source, and latest sync state.
    Status(StatusFactoryArgs),
    /// Show the changes a sync would apply, without applying anything.
    Plan(PlanFactoryArgs),
    /// Sync a factory from its config source.
    Apply(ApplyFactoryArgs),
    /// Write a scaffold factory config directory, with no server interaction.
    Init(InitFactoryArgs),
    /// Download a factory's rendered config files into a directory.
    Export(ExportFactoryArgs),
}

impl FactoryCommand {
    pub(crate) fn as_str_for_tracing(&self) -> &'static str {
        match self {
            FactoryCommand::Link(args) if args.unlink => "factory unlink",
            FactoryCommand::Link(_) => "factory link",
            FactoryCommand::Unlink(_) => "factory unlink",
            FactoryCommand::Status(_) => "factory status",
            FactoryCommand::Plan(_) => "factory plan",
            FactoryCommand::Apply(_) => "factory apply",
            FactoryCommand::Init(_) => "factory init",
            FactoryCommand::Export(_) => "factory export",
        }
    }
}

/// A repository reference in `owner/name` form.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoArg {
    pub owner: String,
    pub repo: String,
}

impl FromStr for RepoArg {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let Some((owner, repo)) = value.split_once('/') else {
            return Err(format!("expected `owner/name`, got `{value}`"));
        };
        if owner.is_empty() || repo.is_empty() || repo.contains('/') {
            return Err(format!("expected `owner/name`, got `{value}`"));
        }
        Ok(RepoArg {
            owner: owner.to_string(),
            repo: repo.to_string(),
        })
    }
}

impl fmt::Display for RepoArg {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.owner, self.repo)
    }
}

#[derive(Debug, Clone, Args)]
pub struct LinkFactoryArgs {
    /// UID of the factory to link.
    pub factory_uid: String,

    /// Repository containing the factory config, in `owner/name` form.
    #[arg(
        long = "repo",
        value_name = "OWNER/NAME",
        required_unless_present = "unlink"
    )]
    pub repo: Option<RepoArg>,

    /// Branch whose pushes drive reconciliation. Defaults to the repository's default branch.
    #[arg(long = "branch", value_name = "BRANCH", requires = "repo")]
    pub branch: Option<String>,

    /// Directory within the repository containing the factory config. Defaults to the
    /// repository root.
    #[arg(long = "path", value_name = "DIR", requires = "repo")]
    pub path: Option<String>,

    /// Unlink the factory from its config source instead of linking it.
    #[arg(long = "unlink", conflicts_with_all = ["repo", "branch", "path"])]
    pub unlink: bool,
}

#[derive(Debug, Clone, Args)]
pub struct UnlinkFactoryArgs {
    /// UID of the factory to unlink.
    pub factory_uid: String,
}

#[derive(Debug, Clone, Args)]
pub struct StatusFactoryArgs {
    /// UID of the factory to inspect.
    pub factory_uid: String,
}

#[derive(Debug, Clone, Args)]
pub struct PlanFactoryArgs {
    /// UID of the factory to plan against.
    pub factory_uid: String,

    /// Commit to plan against. Must be an ancestor of (or equal to) the head of the
    /// source's production branch. Defaults to the branch head.
    #[arg(long = "sha", value_name = "COMMIT")]
    pub sha: Option<String>,
}

#[derive(Debug, Clone, Args)]
pub struct ApplyFactoryArgs {
    /// UID of the factory to sync.
    pub factory_uid: String,

    /// Commit to sync. Must be an ancestor of (or equal to) the head of the source's
    /// production branch. Defaults to the branch head.
    #[arg(long = "sha", value_name = "COMMIT")]
    pub sha: Option<String>,

    /// Wait for the sync to finish, exiting non-zero if it fails.
    #[arg(long = "wait")]
    pub wait: bool,
}

#[derive(Debug, Clone, Args)]
pub struct InitFactoryArgs {
    /// Directory to write the scaffold into, created if missing. Defaults to the
    /// current directory. The scaffolded factory name is the directory's name.
    #[arg(value_name = "DIR")]
    pub dir: Option<PathBuf>,

    /// Write the scaffold even if the directory is not empty, overwriting any
    /// existing scaffold files. Other files are left alone.
    #[arg(long = "force")]
    pub force: bool,
}

#[derive(Debug, Clone, Args)]
pub struct ExportFactoryArgs {
    /// UID of the factory to export.
    pub factory_uid: String,

    /// Directory to write the exported files into. Defaults to `./<factory-name>`.
    #[arg(long = "out", value_name = "DIR")]
    pub out: Option<PathBuf>,

    /// Write the export even if the directory is not empty, overwriting any
    /// existing files with the same paths. Other files are left alone.
    #[arg(long = "force")]
    pub force: bool,
}

/// Directories created empty by `oz factory init`, ready to hold resources of
/// the corresponding kinds.
pub const SCAFFOLD_DIRS: [&str; 4] = ["automations", "environments", "runners", "skills"];

/// Path of the example agent written by `oz factory init`.
pub const SCAFFOLD_AGENT_PATH: &str = "agents/example-agent/agent.md";

const SCAFFOLD_AGENT_TEMPLATE: &str = r#"---
kind: Agent
schema_version: 1
description: An example agent. Replace this with your own.
# harness: claude_code
# model: claude-sonnet
# environment: staging
# secrets:
#   - GITHUB_TOKEN
# skills:
#   - name: my-skill
#     path: skills/my-skill
---
Describe the agent here. The markdown body becomes the agent's instructions.
"#;

const SCAFFOLD_SECRETS: &str = r#"kind: SecretManifest
schema_version: 1
# Managed secret names this factory depends on. Never secret values.
secrets: []
# secrets:
#   - name: GITHUB_TOKEN
#     description: Token for GitHub API access
"#;

fn scaffold_factory_yaml(name: &str) -> String {
    let name = yaml_quote(name);
    format!(
        r#"kind: Factory
schema_version: 1
name: {name}
# description: Describe this factory
# repositories:
#   - your-org/your-repo
# default_environment: staging
# default_model: claude-sonnet
# agent_defaults:
#   harness: claude_code
#   model: claude-sonnet
# agent_attribution_strategy: agent_identity
"#
    )
}

fn scaffold_readme(name: &str) -> String {
    format!(
        r#"# {name}

A Warp factory configured as code.

- `factory.yaml` — factory-wide settings
- `agents/<name>/agent.md` — agent definitions (YAML frontmatter + markdown instructions)
- `automations/<name>.md` — automation definitions (YAML frontmatter + markdown prompt)
- `environments/<name>.yaml` — environment definitions
- `runners/<name>.yaml` — runner definitions
- `skills/` — skills shared by this factory's agents
- `secrets.yaml` — managed secret names this factory depends on, never secret values

Link this directory to a factory so pushes reconcile it:

    oz factory link <factory-uid> --repo <owner>/<repo> [--branch <branch>] [--path <dir>]

See https://docs.warp.dev for documentation.
"#
    )
}

fn yaml_quote(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

/// The scaffold files written by `oz factory init`, keyed by path relative to
/// the factory root.
pub fn scaffold_files(factory_name: &str) -> BTreeMap<&'static str, String> {
    BTreeMap::from([
        ("README.md", scaffold_readme(factory_name)),
        (SCAFFOLD_AGENT_PATH, SCAFFOLD_AGENT_TEMPLATE.to_string()),
        ("factory.yaml", scaffold_factory_yaml(factory_name)),
        ("secrets.yaml", SCAFFOLD_SECRETS.to_string()),
    ])
}

/// Write the factory scaffold into `dir`, creating the directory if needed.
///
/// Refuses to write into a non-empty directory unless `force` is set. Returns
/// the paths of the written files, relative to `dir`.
pub fn init_factory_dir(dir: &Path, force: bool) -> anyhow::Result<Vec<String>> {
    if !force && dir_is_non_empty(dir)? {
        bail!(
            "{} is not empty; pass --force to write the scaffold anyway",
            dir.display()
        );
    }
    let files = scaffold_files(&factory_name_for_dir(dir));
    for subdir in SCAFFOLD_DIRS {
        fs::create_dir_all(dir.join(subdir))?;
    }
    for (path, content) in &files {
        let target = dir.join(path);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&target, content)?;
    }
    Ok(files.keys().map(|path| path.to_string()).collect())
}

/// The factory name to scaffold for `dir`: its directory name, resolved
/// against the working directory for relative paths like `.`.
fn factory_name_for_dir(dir: &Path) -> String {
    let absolute = if dir.is_absolute() {
        dir.to_path_buf()
    } else {
        match std::env::current_dir() {
            Ok(cwd) => cwd.join(dir),
            Err(_) => dir.to_path_buf(),
        }
    };
    absolute
        .components()
        .filter_map(|component| match component {
            Component::Normal(name) => Some(name.to_string_lossy().into_owned()),
            _ => None,
        })
        .next_back()
        .unwrap_or_else(|| "my-factory".to_string())
}

/// Write server-exported factory files under `dir`, creating it if needed.
///
/// Rejects the whole export, before writing anything, if any path is absolute
/// or would escape `dir` (e.g. contains `..`). Refuses to write into a
/// non-empty directory unless `force` is set. Returns the written paths,
/// relative to `dir`.
pub fn write_export_files(
    dir: &Path,
    files: &BTreeMap<String, String>,
    force: bool,
) -> anyhow::Result<Vec<String>> {
    for path in files.keys() {
        validate_export_path(path)?;
    }
    if !force && dir_is_non_empty(dir)? {
        bail!(
            "{} is not empty; pass --force to write the export anyway",
            dir.display()
        );
    }
    fs::create_dir_all(dir)?;
    let mut written = Vec::with_capacity(files.len());
    for (path, content) in files {
        let target = dir.join(path);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&target, content)?;
        written.push(path.clone());
    }
    Ok(written)
}

fn validate_export_path(path: &str) -> anyhow::Result<()> {
    if path.is_empty()
        || !Path::new(path)
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
    {
        bail!("refusing to write export file with unsafe path `{path}`");
    }
    Ok(())
}

fn dir_is_non_empty(dir: &Path) -> anyhow::Result<bool> {
    match fs::read_dir(dir) {
        Ok(mut entries) => Ok(entries.next().is_some()),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(err) => Err(err.into()),
    }
}

#[cfg(test)]
#[path = "factory_tests.rs"]
mod tests;
