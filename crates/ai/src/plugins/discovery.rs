//! Plugin search roots, candidate scanning, and cross-root precedence.
//!
//! Only fixed locations are scanned, and only immediate children of a search root are treated as
//! candidates. A bare `<repo-root>/plugins/` is deliberately not a search root: giving a generic
//! repository folder execution semantics would surprise anyone who already has one.
//!
//! Precedence is resolved on the manifest `name`, after validation, and shadows a whole package.
//! Warp never merges the manifest of one package with the components of another, so a lower-
//! precedence package can never contribute a component to a name it does not own outright.
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use super::diagnostics::{PluginDiagnostic, PluginDiagnosticCode};
use super::identity::{PluginScopeId, PluginSourceId, PluginSourceKind};
use super::package::{PluginPackage, load_plugin_package};

/// The directory name that holds plugin packages under each provider directory.
const PLUGINS_DIR_NAME: &str = "plugins";

/// Repository-relative directories that must remain visible to plugin discovery even when ignored.
pub const REPOSITORY_PLUGIN_PATHS: [&str; 2] = [".agents/plugins", ".warp/plugins"];

/// A directory whose immediate children are plugin candidates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginSearchRoot {
    pub path: PathBuf,
    pub scope: PluginScopeId,
    pub source: PluginSourceId,
}

impl PluginSearchRoot {
    fn new(path: PathBuf, scope: PluginScopeId, kind: PluginSourceKind, identity: &Path) -> Self {
        Self {
            path,
            scope,
            source: PluginSourceId::new(kind, identity.to_string_lossy().into_owned()),
        }
    }
}

/// One immediate child of a search root, before it is known to be a valid package.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginCandidate {
    pub root: PathBuf,
    pub scope: PluginScopeId,
    pub source: PluginSourceId,
}

/// A package that lost to a higher-precedence package with the same manifest name.
#[derive(Debug, Clone)]
pub struct ShadowedPlugin {
    pub package: PluginPackage,
    /// The source that won, so diagnostics can name it.
    pub shadowed_by: PluginSourceId,
}

/// The result of applying precedence across every scanned candidate.
#[derive(Debug, Clone, Default)]
pub struct ActivePluginSet {
    /// The winning package for each manifest name.
    pub active: BTreeMap<String, PluginPackage>,
    /// Packages that were valid but superseded, kept so diagnostics can show what was ignored.
    pub shadowed: Vec<ShadowedPlugin>,
    /// Names claimed by two equally ranked sources. Neither is active.
    pub ambiguous: BTreeMap<String, Vec<PluginPackage>>,
    pub diagnostics: Vec<PluginDiagnostic>,
}

impl ActivePluginSet {
    /// Returns the active package for `name`, if the name resolved unambiguously.
    pub fn get(&self, name: &str) -> Option<&PluginPackage> {
        self.active.get(name)
    }

    /// Every diagnostic from the resolution itself plus every package-level diagnostic.
    pub fn all_diagnostics(&self) -> Vec<PluginDiagnostic> {
        self.diagnostics
            .iter()
            .cloned()
            .chain(
                self.active
                    .values()
                    .flat_map(|package| package.diagnostics.iter().cloned()),
            )
            .collect()
    }
}

/// The user-level search roots: `~/.agents/plugins` and the channel-aware Warp home
/// `<warp-config-dir>/plugins`.
///
/// The TUI's separate global MCP configuration file does not add a third user root; both
/// front-ends scan the same package roots.
pub fn user_search_roots() -> Vec<PluginSearchRoot> {
    let mut roots = Vec::new();
    if let Some(home) = dirs::home_dir() {
        let agents_dir = home.join(".agents");
        roots.push(PluginSearchRoot::new(
            agents_dir.join(PLUGINS_DIR_NAME),
            PluginScopeId::User,
            PluginSourceKind::AgentsDirectory,
            &agents_dir,
        ));
    }
    if let Some(warp_config_dir) = warp_core::paths::warp_home_config_dir() {
        roots.push(PluginSearchRoot::new(
            warp_config_dir.join(PLUGINS_DIR_NAME),
            PluginScopeId::User,
            PluginSourceKind::WarpDirectory,
            &warp_config_dir,
        ));
    }
    roots
}

/// The repository search roots for one repository: `.agents/plugins` and `.warp/plugins`.
pub fn repository_search_roots(repo_root: &Path) -> Vec<PluginSearchRoot> {
    vec![
        PluginSearchRoot::new(
            repo_root.join(REPOSITORY_PLUGIN_PATHS[0]),
            PluginScopeId::Repository,
            PluginSourceKind::AgentsDirectory,
            repo_root,
        ),
        PluginSearchRoot::new(
            repo_root.join(REPOSITORY_PLUGIN_PATHS[1]),
            PluginScopeId::Repository,
            PluginSourceKind::WarpDirectory,
            repo_root,
        ),
    ]
}

/// Lists the immediate child directories of `root` as candidates.
///
/// A missing or unreadable search root yields no candidates and is not an error: most users have
/// no plugins directory at all.
pub fn scan_search_root(root: &PluginSearchRoot) -> Vec<PluginCandidate> {
    let Ok(entries) = fs::read_dir(&root.path) else {
        return Vec::new();
    };
    let mut candidates: Vec<PluginCandidate> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .map(|path| PluginCandidate {
            root: path,
            scope: root.scope.clone(),
            source: root.source.clone(),
        })
        .collect();
    candidates.sort_by(|left, right| left.root.cmp(&right.root));
    candidates
}

/// The precedence tuple for a candidate. Lower wins.
pub fn precedence_rank(scope: &PluginScopeId, kind: PluginSourceKind) -> (u8, u8) {
    (scope.scope_rank(), kind.provider_rank())
}

/// Loads every candidate and applies whole-package shadowing by manifest name.
///
/// Two candidates at the same rank that claim the same name — the cross-repository case — are
/// reported as ambiguous rather than resolved by filesystem order, so which one wins never
/// depends on directory iteration.
pub fn resolve_active_packages(candidates: Vec<PluginCandidate>) -> ActivePluginSet {
    let mut set = ActivePluginSet::default();
    let mut loaded: Vec<PluginPackage> = Vec::new();

    for candidate in candidates {
        match load_plugin_package(&candidate.root, candidate.scope, candidate.source) {
            Ok(package) => loaded.push(package),
            Err(diagnostic) => set.diagnostics.push(diagnostic.with_path(&candidate.root)),
        }
    }

    let mut by_name: BTreeMap<String, Vec<PluginPackage>> = BTreeMap::new();
    for package in loaded {
        by_name
            .entry(package.manifest.name.clone())
            .or_default()
            .push(package);
    }

    for (name, mut packages) in by_name {
        packages.sort_by_key(|package| package.instance.precedence());
        let best_rank = packages[0].instance.precedence();
        let tied = packages
            .iter()
            .filter(|package| package.instance.precedence() == best_rank)
            .count();

        if tied > 1 {
            let sources = packages
                .iter()
                .filter(|package| package.instance.precedence() == best_rank)
                .map(|package| package.root.display().to_string())
                .collect::<Vec<_>>()
                .join(", ");
            set.diagnostics.push(
                PluginDiagnostic::new(
                    PluginDiagnosticCode::PluginAmbiguous,
                    format!(
                        "plugin '{name}' is provided by equally ranked sources and was not \
                         loaded; remove or rename one of: {sources}"
                    ),
                )
                .with_plugin(&name),
            );
            set.ambiguous.insert(name, packages);
            continue;
        }

        let mut packages = packages.into_iter();
        let winner = packages.next().expect("at least one package per name");
        for shadowed in packages {
            set.diagnostics.push(
                PluginDiagnostic::new(
                    PluginDiagnosticCode::PluginShadowed,
                    format!(
                        "plugin '{name}' at {} is shadowed as a complete package by the {} \
                         source at {}",
                        shadowed.root.display(),
                        winner.instance.scope,
                        winner.root.display()
                    ),
                )
                .with_plugin(&name)
                .with_path(&shadowed.root),
            );
            set.shadowed.push(ShadowedPlugin {
                package: shadowed,
                shadowed_by: winner.instance.source.clone(),
            });
        }
        set.active.insert(name, winner);
    }

    set
}

#[cfg(test)]
#[path = "discovery_tests.rs"]
mod tests;
