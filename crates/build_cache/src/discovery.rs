//! Deterministic discovery of repository roots that may need independent build caches.
//!
//! Repositories are scanned in cache-key order. Each repository root is emitted before marked
//! descendants selected by a sorted depth-first walk, so scan limits always retain the same roots.
//! The blocking filesystem walk feeds a bounded async channel: a full channel backpressures the
//! walk, and dropping the receiver stops it at the next cancellation check. [`CandidateKey`]
//! encodes root-first canonical order independently of delivery timing.
use std::collections::BTreeSet;
use std::path::{Component, Path, PathBuf};

use sha2::{Digest, Sha256};
use tokio::sync::mpsc;
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

const CODEBASE_MARKER_FILENAMES: &[&str] = &[
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

const CODEBASE_MARKER_PATHS: &[&[&str]] = &[
    &["mise", "config.toml"],
    &[".mise", "config.toml"],
    &[".config", "mise.toml"],
    &[".config", "mise", "config.toml"],
];

/// Canonical detection order: repository key, then root before normalized descendant paths.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) struct CandidateKey {
    pub repo_key: RepoCacheKey,
    pub normalized_relative_path: Option<PathBuf>,
}

#[derive(Clone, Debug)]
pub(super) struct CacheCandidate {
    pub key: CandidateKey,
    pub source: RepositoryCacheSource,
    pub relative_cache_dir: PathBuf,
    pub stable_child_id: Option<String>,
}

/// Emits one repository root followed by its distinct marked descendants.
///
/// The walk inspects entries below the candidate depth because multi-component markers can identify
/// shallower roots. Returning `false` means the receiver was dropped and all discovery must stop.
fn produce_repository_candidates(
    key: RepoCacheKey,
    source: RepositoryCacheSource,
    sender: &mpsc::Sender<CacheCandidate>,
) -> bool {
    let span = tracing::info_span!(
        target: "build_cache",
        "discover_cache_roots",
        tags.cloud_agent = true,
        repo_key = %key,
        visited_directory_count = tracing::field::Empty,
        selected_child_count = tracing::field::Empty,
        unreadable_entry_count = tracing::field::Empty,
        truncation_reason = tracing::field::Empty,
    );
    let _guard = span.enter();
    let mut selected_paths = BTreeSet::new();
    let mut visited_directories = 1;
    let mut unreadable_entries = 0;
    let mut truncation = None;
    let mut receiver_open = sender
        .blocking_send(root_candidate(key.clone(), source.clone()))
        .is_ok();

    if receiver_open {
        let walker = WalkDir::new(&source.cwd)
            .min_depth(1)
            .max_depth(MAX_WALK_DEPTH)
            .follow_links(false)
            .follow_root_links(false)
            .sort_by_file_name()
            .into_iter()
            .filter_entry(|entry| {
                !(entry.file_type().is_symlink()
                    || entry.file_type().is_dir()
                        && IGNORED_DIRECTORIES
                            .iter()
                            .any(|ignored| entry.file_name() == *ignored))
            });

        'walk: for entry in walker {
            if sender.is_closed() {
                receiver_open = false;
                break;
            }
            let entry = match entry {
                Ok(entry) => entry,
                Err(_) => {
                    unreadable_entries += 1;
                    continue;
                }
            };
            if entry.file_type().is_dir() {
                if visited_directories == MAX_VISITED_DIRECTORIES {
                    truncation = Some(TruncationReason::DirectoryLimit);
                    break;
                }
                visited_directories += 1;
            }
            for path in marker_candidate_paths(&entry, &source.cwd) {
                let Some(normalized_relative_path) = normalize_relative_path(&source.cwd, &path)
                else {
                    continue;
                };
                let depth = normalized_relative_path.components().count();
                if !(1..=MAX_CANDIDATE_DEPTH).contains(&depth)
                    || selected_paths.contains(&normalized_relative_path)
                {
                    continue;
                }
                if selected_paths.len() == MAX_CHILD_CANDIDATES {
                    truncation = Some(TruncationReason::CandidateLimit);
                    break 'walk;
                }

                selected_paths.insert(normalized_relative_path.clone());
                let candidate =
                    child_candidate(key.clone(), source.clone(), path, normalized_relative_path);
                if sender.blocking_send(candidate).is_err() {
                    receiver_open = false;
                    break 'walk;
                }
            }
        }
    }

    span.record("visited_directory_count", visited_directories as u64);
    span.record("selected_child_count", selected_paths.len() as u64);
    span.record("unreadable_entry_count", unreadable_entries as u64);
    if let Some(reason) = truncation {
        span.record("truncation_reason", reason.as_str());
        tracing::warn!(
            target: "build_cache",
            truncation_reason = reason.as_str(),
            "build cache root discovery was truncated"
        );
    }
    if unreadable_entries > 0 {
        tracing::warn!(
            target: "build_cache",
            unreadable_entry_count = unreadable_entries,
            "build cache root discovery skipped unreadable entries"
        );
    }
    receiver_open
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

/// Starts discovery on Tokio's blocking pool and returns its bounded candidate receiver.
///
/// The current span is entered on the blocking thread so per-repository diagnostics remain part of
/// the cache-setup trace. Dropping the receiver unblocks a pending send and cancels further scans.
pub(super) fn candidate_receiver(
    repositories: Vec<RepositoryCacheSource>,
) -> mpsc::Receiver<CacheCandidate> {
    let (sender, receiver) = mpsc::channel(DETECTION_CONCURRENCY);
    let parent_span = tracing::Span::current();
    tokio::task::spawn_blocking(move || {
        let _guard = parent_span.enter();
        produce_candidates(repositories, sender);
    });
    receiver
}

/// Emits candidates in canonical repository order until discovery completes or the receiver closes.
pub(super) fn produce_candidates(
    repositories: Vec<RepositoryCacheSource>,
    sender: mpsc::Sender<CacheCandidate>,
) {
    let mut repositories = repositories
        .into_iter()
        .map(|source| (RepoCacheKey::derive(&source.identity), source))
        .collect::<Vec<_>>();
    repositories.sort();
    for (key, source) in repositories {
        if !produce_repository_candidates(key, source, &sender) {
            return;
        }
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
        stable_child_id: None,
    }
}

fn child_candidate(
    key: RepoCacheKey,
    mut source: RepositoryCacheSource,
    cwd: PathBuf,
    normalized_relative_path: PathBuf,
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
        stable_child_id: Some(stable_child_id),
    }
}

/// Hashes normalized components with a fixed separator so child cache identities are host-agnostic.
fn stable_child_id(normalized_relative_path: &Path) -> String {
    let mut hasher = Sha256::new();
    for (index, component) in normalized_relative_path.components().enumerate() {
        let Component::Normal(component) = component else {
            continue;
        };
        if index > 0 {
            hasher.update(b"/");
        }
        hasher.update(
            component
                .to_str()
                .expect("normalized paths contain only UTF-8 components")
                .as_bytes(),
        );
    }
    hex::encode(hasher.finalize())
}

/// Returns a non-empty, relative UTF-8 path containing only normal components.
fn normalize_relative_path(root: &Path, path: &Path) -> Option<PathBuf> {
    let relative = path.strip_prefix(root).ok()?;
    let mut normalized = PathBuf::new();
    for component in relative.components() {
        let Component::Normal(component) = component else {
            return None;
        };
        component.to_str()?;
        normalized.push(component);
    }
    if normalized.as_os_str().is_empty() {
        return None;
    }
    Some(normalized)
}

/// Maps a marker entry to the directory where spacectl must run to observe that marker.
fn marker_candidate_paths(entry: &DirEntry, root: &Path) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if entry.file_type().is_file()
        && CODEBASE_MARKER_FILENAMES
            .iter()
            .any(|marker| entry.file_name() == *marker)
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
        for marker in CODEBASE_MARKER_PATHS {
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

#[cfg(test)]
#[path = "discovery_tests.rs"]
mod tests;
