use std::collections::{BTreeSet, VecDeque};
use std::ffi::OsStr;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use sha2::{Digest, Sha256};
use walkdir::{DirEntry, WalkDir};

use crate::{RepoCacheKey, RepositoryCacheSource};

pub(super) const DETECTION_CONCURRENCY: usize = 8;

const MAX_CANDIDATE_DEPTH: usize = 4;
const MAX_WALK_DEPTH: usize = 7;
const MAX_VISITED_DIRECTORIES: usize = 10_000;
const MAX_CHILD_CANDIDATES: usize = 32;

const IGNORED_DIRECTORIES: &[&str] = &[
    ".git",
    "node_modules",
    "target",
    "Pods",
    "vendor",
    "dist",
    "build",
    ".venv",
    ".tox",
    "DerivedData",
];

const EXACT_MARKERS: &[&str] = &[
    "Brewfile",
    "bun.lock",
    "Podfile",
    "composer.json",
    "deno.lock",
    "go.mod",
    "go.work",
    ".golangci.yml",
    ".golangci.yaml",
    "gradlew",
    "build.gradle",
    "pom.xml",
    "mise.toml",
    ".mise.toml",
    ".tool-versions",
    "flake.nix",
    "shell.nix",
    "default.nix",
    "package-lock.json",
    "pnpm-lock.yaml",
    "poetry.lock",
    "requirements.txt",
    "Gemfile",
    "Cargo.toml",
    "Package.swift",
    "Tuist.swift",
    "tuist.toml",
    "uv.lock",
    "yarn.lock",
];

const RELATIVE_MARKERS: &[&[&str]] = &[
    &["mise", "config.toml"],
    &[".mise", "config.toml"],
    &[".config", "mise.toml"],
    &[".config", "mise", "config.toml"],
];

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) struct CandidateKey {
    pub repo_key: RepoCacheKey,
    pub normalized_relative_path: Option<String>,
}

#[derive(Clone, Debug)]
pub(super) struct CacheCandidate {
    pub key: CandidateKey,
    pub source: RepositoryCacheSource,
    pub relative_cache_dir: PathBuf,
    pub depth: usize,
    pub stable_child_id: Option<String>,
}

pub(super) struct CandidateProducer {
    repositories: VecDeque<(RepoCacheKey, RepositoryCacheSource)>,
    current: Option<RepositoryDiscovery>,
}

impl CandidateProducer {
    pub(super) fn new(repositories: Vec<RepositoryCacheSource>) -> Self {
        let mut repositories = repositories
            .into_iter()
            .map(|source| (RepoCacheKey::derive(&source.identity), source))
            .collect::<Vec<_>>();
        repositories.sort();
        Self {
            repositories: repositories.into(),
            current: None,
        }
    }

    pub(super) fn next_candidate(&mut self) -> Option<CacheCandidate> {
        loop {
            if let Some(discovery) = &mut self.current {
                if let Some(candidate) = discovery.next_candidate() {
                    return Some(candidate);
                }
                self.current = None;
            }

            let (key, source) = self.repositories.pop_front()?;
            self.current = Some(RepositoryDiscovery::new(key, source));
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum TruncationReason {
    DirectoryLimit,
    CandidateLimit,
}

impl TruncationReason {
    fn as_str(self) -> &'static str {
        match self {
            Self::DirectoryLimit => "directory_limit",
            Self::CandidateLimit => "candidate_limit",
        }
    }
}

struct RepositoryDiscovery {
    source: RepositoryCacheSource,
    key: RepoCacheKey,
    walker: Box<dyn Iterator<Item = Result<DirEntry, walkdir::Error>> + Send>,
    ignored_subtrees: Arc<AtomicUsize>,
    selected_paths: BTreeSet<String>,
    pending_paths: VecDeque<PathBuf>,
    root_pending: bool,
    visited_directories: usize,
    unreadable_entries: usize,
    truncation: Option<TruncationReason>,
    finished: bool,
    span: tracing::Span,
}

impl RepositoryDiscovery {
    fn new(key: RepoCacheKey, source: RepositoryCacheSource) -> Self {
        let ignored_subtrees = Arc::new(AtomicUsize::new(0));
        let filter_ignored_subtrees = Arc::clone(&ignored_subtrees);
        let walker = WalkDir::new(&source.cwd)
            .min_depth(1)
            .max_depth(MAX_WALK_DEPTH)
            .follow_links(false)
            .follow_root_links(false)
            .sort_by_file_name()
            .into_iter()
            .filter_entry(move |entry| {
                if entry.file_type().is_symlink() {
                    return false;
                }
                if entry.file_type().is_dir() && is_ignored_directory(entry.file_name()) {
                    filter_ignored_subtrees.fetch_add(1, Ordering::Relaxed);
                    return false;
                }
                true
            });
        let span = tracing::info_span!(
            target: "build_cache",
            "discover_cache_roots",
            tags.cloud_agent = true,
            repo_key = %key,
            visited_directory_count = tracing::field::Empty,
            selected_child_count = tracing::field::Empty,
            ignored_subtree_count = tracing::field::Empty,
            unreadable_entry_count = tracing::field::Empty,
            truncation_reason = tracing::field::Empty,
        );
        Self {
            source,
            key,
            walker: Box::new(walker),
            ignored_subtrees,
            selected_paths: BTreeSet::new(),
            pending_paths: VecDeque::new(),
            root_pending: true,
            visited_directories: 1,
            unreadable_entries: 0,
            truncation: None,
            finished: false,
            span,
        }
    }

    fn next_candidate(&mut self) -> Option<CacheCandidate> {
        let span = self.span.clone();
        let _guard = span.enter();
        if self.root_pending {
            self.root_pending = false;
            return Some(root_candidate(self.key.clone(), self.source.clone()));
        }

        loop {
            if let Some(path) = self.pending_paths.pop_front() {
                if let Some(candidate) = self.select_child(path) {
                    return Some(candidate);
                }
                if self.truncation.is_some() {
                    self.finish();
                    return None;
                }
            }

            let Some(entry) = self.walker.next() else {
                self.finish();
                return None;
            };
            let entry = match entry {
                Ok(entry) => entry,
                Err(_) => {
                    self.unreadable_entries += 1;
                    continue;
                }
            };
            if entry.file_type().is_dir() {
                if self.visited_directories == MAX_VISITED_DIRECTORIES {
                    self.truncation = Some(TruncationReason::DirectoryLimit);
                    self.finish();
                    return None;
                }
                self.visited_directories += 1;
            }
            self.pending_paths
                .extend(marker_candidate_paths(&entry, &self.source.cwd));
        }
    }

    fn select_child(&mut self, path: PathBuf) -> Option<CacheCandidate> {
        let normalized_relative_path = normalize_relative_path(&self.source.cwd, &path)?;
        let depth = normalized_relative_path.split('/').count();
        if !(1..=MAX_CANDIDATE_DEPTH).contains(&depth)
            || self.selected_paths.contains(&normalized_relative_path)
        {
            return None;
        }
        if self.selected_paths.len() == MAX_CHILD_CANDIDATES {
            self.truncation = Some(TruncationReason::CandidateLimit);
            return None;
        }

        self.selected_paths.insert(normalized_relative_path.clone());
        Some(child_candidate(
            self.key.clone(),
            self.source.clone(),
            path,
            normalized_relative_path,
            depth,
        ))
    }

    fn finish(&mut self) {
        if self.finished {
            return;
        }
        self.finished = true;
        let span = self.span.clone();
        let _guard = span.enter();
        span.record("visited_directory_count", self.visited_directories as u64);
        span.record("selected_child_count", self.selected_paths.len() as u64);
        span.record(
            "ignored_subtree_count",
            self.ignored_subtrees.load(Ordering::Relaxed) as u64,
        );
        span.record("unreadable_entry_count", self.unreadable_entries as u64);
        if let Some(reason) = self.truncation {
            span.record("truncation_reason", reason.as_str());
            tracing::warn!(
                target: "build_cache",
                truncation_reason = reason.as_str(),
                "build cache root discovery was truncated"
            );
        }
        if self.unreadable_entries > 0 {
            tracing::warn!(
                target: "build_cache",
                unreadable_entry_count = self.unreadable_entries,
                "build cache root discovery skipped unreadable entries"
            );
        }
    }
}

impl Drop for RepositoryDiscovery {
    fn drop(&mut self) {
        self.finish();
    }
}

fn root_candidate(key: RepoCacheKey, source: RepositoryCacheSource) -> CacheCandidate {
    CacheCandidate {
        relative_cache_dir: PathBuf::from("repos").join(key.as_str()),
        key: CandidateKey {
            repo_key: key,
            normalized_relative_path: None,
        },
        source,
        depth: 0,
        stable_child_id: None,
    }
}

fn child_candidate(
    key: RepoCacheKey,
    mut source: RepositoryCacheSource,
    cwd: PathBuf,
    normalized_relative_path: String,
    depth: usize,
) -> CacheCandidate {
    let stable_child_id = stable_child_id(&normalized_relative_path);
    source.cwd = cwd;
    CacheCandidate {
        relative_cache_dir: PathBuf::from("repos")
            .join(key.as_str())
            .join("nested")
            .join(&stable_child_id),
        key: CandidateKey {
            repo_key: key,
            normalized_relative_path: Some(normalized_relative_path),
        },
        source,
        depth,
        stable_child_id: Some(stable_child_id),
    }
}

fn stable_child_id(normalized_relative_path: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(normalized_relative_path.as_bytes());
    hex::encode(hasher.finalize())
}

fn normalize_relative_path(root: &Path, path: &Path) -> Option<String> {
    let relative = path.strip_prefix(root).ok()?;
    let mut normalized = Vec::new();
    for component in relative.components() {
        let Component::Normal(component) = component else {
            return None;
        };
        normalized.push(component.to_str()?);
    }
    if normalized.is_empty() {
        return None;
    }
    Some(normalized.join("/"))
}

fn marker_candidate_paths(entry: &DirEntry, root: &Path) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if entry.file_type().is_file()
        && is_exact_marker(entry.file_name())
        && let Some(parent) = entry.path().parent()
    {
        candidates.push(parent.to_path_buf());
    }
    if entry.file_type().is_dir()
        && (entry.file_name() == "Tuist"
            || entry
                .file_name()
                .to_str()
                .is_some_and(|name| name.ends_with(".xcodeproj") || name.ends_with(".xcworkspace")))
        && let Some(parent) = entry.path().parent()
    {
        candidates.push(parent.to_path_buf());
    }
    if entry.file_type().is_file()
        && let Ok(relative) = entry.path().strip_prefix(root)
    {
        let components = relative
            .components()
            .filter_map(|component| match component {
                Component::Normal(component) => Some(component),
                Component::Prefix(_)
                | Component::RootDir
                | Component::CurDir
                | Component::ParentDir => None,
            })
            .collect::<Vec<_>>();
        for marker in RELATIVE_MARKERS {
            if components.len() >= marker.len()
                && components[components.len() - marker.len()..]
                    .iter()
                    .zip(*marker)
                    .all(|(component, marker)| component == marker)
            {
                let mut candidate = root.to_path_buf();
                for component in &components[..components.len() - marker.len()] {
                    candidate.push(component);
                }
                candidates.push(candidate);
            }
        }
    }
    candidates
}

fn is_exact_marker(name: &OsStr) -> bool {
    EXACT_MARKERS.iter().any(|marker| name == *marker)
}

fn is_ignored_directory(name: &OsStr) -> bool {
    IGNORED_DIRECTORIES.iter().any(|ignored| name == *ignored)
}

#[cfg(test)]
#[path = "discovery_tests.rs"]
mod tests;
