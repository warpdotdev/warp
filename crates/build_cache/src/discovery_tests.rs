use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use super::{MAX_CHILD_CANDIDATES, MAX_VISITED_DIRECTORIES, produce_candidates, stable_child_id};
use crate::{RepoIdentity, RepositoryCacheSource};

fn source(root: &Path) -> RepositoryCacheSource {
    RepositoryCacheSource {
        name: "warp/example".to_owned(),
        identity: RepoIdentity::new("github.com", "warp", "example"),
        cwd: root.to_path_buf(),
    }
}

fn candidates(sources: Vec<RepositoryCacheSource>) -> Vec<super::CacheCandidate> {
    let capacity = sources.len().max(1) * (MAX_CHILD_CANDIDATES + 1);
    let (sender, mut receiver) = tokio::sync::mpsc::channel(capacity);
    produce_candidates(sources, sender);
    std::iter::from_fn(|| receiver.blocking_recv()).collect()
}

fn child_paths(root: &Path) -> Vec<PathBuf> {
    let mut candidates = candidates(vec![source(root)]).into_iter();
    let root = candidates.next().unwrap();
    assert_eq!(root.key.normalized_relative_path, None);
    candidates
        .map(|candidate| candidate.key.normalized_relative_path.unwrap())
        .collect()
}

fn touch(root: &Path, path: &str) {
    let path = root.join(path);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, "").unwrap();
}

#[test]
fn direct_markers_select_their_containing_directories() {
    let temp = tempfile::tempdir().unwrap();
    touch(temp.path(), "rust/Cargo.toml");
    touch(temp.path(), "javascript/package-lock.json");
    touch(temp.path(), "gradle/build.gradle");

    let paths = child_paths(temp.path());
    assert_eq!(
        paths,
        [
            PathBuf::from("gradle"),
            PathBuf::from("javascript"),
            PathBuf::from("rust"),
        ]
    );
}

#[test]
fn relative_and_directory_markers_select_the_expected_ancestors() {
    let temp = tempfile::tempdir().unwrap();
    touch(temp.path(), "a/mise/config.toml");
    touch(temp.path(), "b/.mise/config.toml");
    touch(temp.path(), "c/.config/mise.toml");
    touch(temp.path(), "d/.config/mise/config.toml");
    fs::create_dir_all(temp.path().join("e/Tuist")).unwrap();
    fs::create_dir_all(temp.path().join("f/App.xcodeproj")).unwrap();
    fs::create_dir_all(temp.path().join("g/App.xcworkspace")).unwrap();

    let paths = child_paths(temp.path());
    assert_eq!(
        paths,
        [
            PathBuf::from("a"),
            PathBuf::from("b"),
            PathBuf::from("c"),
            PathBuf::from("d"),
            PathBuf::from("e"),
            PathBuf::from("f"),
            PathBuf::from("g"),
        ]
    );
}

#[test]
fn non_markers_and_ignored_subtrees_are_skipped() {
    let temp = tempfile::tempdir().unwrap();
    touch(temp.path(), "frontend/package.json");
    touch(temp.path(), "backend/pyproject.toml");
    touch(temp.path(), "gradle/settings.gradle");
    touch(temp.path(), "kotlin/build.gradle.kts");
    touch(temp.path(), "node_modules/nested/Cargo.toml");
    touch(temp.path(), "target/nested/go.mod");
    touch(temp.path(), "valid/Cargo.toml");

    assert_eq!(child_paths(temp.path()), [PathBuf::from("valid")]);
}

#[test]
fn multiple_markers_deduplicate_exact_roots_but_keep_nested_roots() {
    let temp = tempfile::tempdir().unwrap();
    touch(temp.path(), "project/Cargo.toml");
    touch(temp.path(), "project/package-lock.json");
    touch(temp.path(), "project/nested/go.mod");

    assert_eq!(
        child_paths(temp.path()),
        [PathBuf::from("project"), PathBuf::from("project/nested")]
    );
}

#[test]
fn traversal_is_sorted_depth_first() {
    let temp = tempfile::tempdir().unwrap();
    touch(temp.path(), "z/Cargo.toml");
    touch(temp.path(), "a/nested/Cargo.toml");
    touch(temp.path(), "a/Cargo.toml");

    assert_eq!(
        child_paths(temp.path()),
        [
            PathBuf::from("a"),
            PathBuf::from("a/nested"),
            PathBuf::from("z"),
        ]
    );
}

#[cfg(unix)]
#[test]
fn symlinked_roots_and_entries_are_not_followed() {
    use std::os::unix::fs::symlink;

    let temp = tempfile::tempdir().unwrap();
    let external = tempfile::tempdir().unwrap();
    touch(external.path(), "project/Cargo.toml");
    symlink(external.path().join("project"), temp.path().join("linked")).unwrap();

    assert!(child_paths(temp.path()).is_empty());

    let linked_root = temp.path().join("root-link");
    symlink(external.path(), &linked_root).unwrap();
    assert!(child_paths(&linked_root).is_empty());
}

#[test]
fn walk_bound_includes_every_reached_candidate() {
    let temp = tempfile::tempdir().unwrap();
    touch(temp.path(), "one/two/three/four/.config/mise/config.toml");
    touch(temp.path(), "one/two/three/four/five/Cargo.toml");

    assert_eq!(
        child_paths(temp.path()),
        [
            PathBuf::from("one/two/three/four"),
            PathBuf::from("one/two/three/four/five"),
        ]
    );
}

#[test]
fn child_limit_retains_root_and_earliest_children() {
    let temp = tempfile::tempdir().unwrap();
    for index in 0..MAX_CHILD_CANDIDATES + 1 {
        touch(temp.path(), &format!("{index:02}/Cargo.toml"));
    }
    let candidates = candidates(vec![source(temp.path())]);

    assert_eq!(candidates.len(), MAX_CHILD_CANDIDATES + 1);
    assert_eq!(candidates[0].key.normalized_relative_path, None);
    assert_eq!(
        candidates
            .last()
            .unwrap()
            .key
            .normalized_relative_path
            .as_deref(),
        Some(Path::new("31"))
    );
}

#[test]
fn directory_limit_stops_before_later_marker() {
    let temp = tempfile::tempdir().unwrap();
    for index in 0..MAX_VISITED_DIRECTORIES {
        fs::create_dir(temp.path().join(format!("{index:05}"))).unwrap();
    }
    touch(temp.path(), "zzzzz/Cargo.toml");

    assert!(child_paths(temp.path()).is_empty());
}

#[test]
fn stable_child_ids_hash_normalized_relative_paths() {
    assert_eq!(
        stable_child_id(Path::new("frontend/web")),
        "4984839f9fe7d9730ec3fd45d7ededa43b0b5bfaf82f21666f7d9c25d1cf234c"
    );
    assert_eq!(stable_child_id(Path::new("frontend/web")).len(), 64);
}

#[test]
fn repositories_and_roots_are_produced_in_canonical_order() {
    let temp = tempfile::tempdir().unwrap();
    let first = temp.path().join("first");
    let second = temp.path().join("second");
    fs::create_dir_all(&first).unwrap();
    fs::create_dir_all(&second).unwrap();
    touch(&first, "nested/Cargo.toml");
    touch(&second, "nested/Cargo.toml");
    let sources = vec![
        RepositoryCacheSource {
            name: "z/repo".to_owned(),
            identity: RepoIdentity::new("github.com", "z", "repo"),
            cwd: first,
        },
        RepositoryCacheSource {
            name: "a/repo".to_owned(),
            identity: RepoIdentity::new("github.com", "a", "repo"),
            cwd: second,
        },
    ];
    let candidates = candidates(sources);
    let keys = candidates
        .iter()
        .map(|candidate| {
            (
                candidate.key.repo_key.clone(),
                candidate.key.normalized_relative_path.clone(),
            )
        })
        .collect::<Vec<_>>();

    assert!(keys.windows(2).all(|pair| pair[0] <= pair[1]));
}

#[test]
fn nested_cache_path_uses_stable_id_and_root_path_is_unchanged() {
    let temp = tempfile::tempdir().unwrap();
    touch(temp.path(), "frontend/Cargo.toml");
    let mut candidates = candidates(vec![source(temp.path())]).into_iter();
    let root = candidates.next().unwrap();
    let child = candidates.next().unwrap();

    assert_eq!(
        root.relative_cache_dir,
        PathBuf::from("repos").join(root.key.repo_key.as_str())
    );
    assert_eq!(
        child.relative_cache_dir,
        PathBuf::from("repos")
            .join(child.key.repo_key.as_str())
            .join("nested")
            .join(child.stable_child_id.unwrap())
    );
}

#[test]
fn dropping_bounded_receiver_stops_blocking_producer() {
    let temp = tempfile::tempdir().unwrap();
    for index in 0..MAX_CHILD_CANDIDATES {
        touch(temp.path(), &format!("{index:02}/Cargo.toml"));
    }
    let (sender, mut receiver) = tokio::sync::mpsc::channel(1);
    let sources = vec![source(temp.path())];
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_time()
        .build()
        .unwrap();

    runtime.block_on(async move {
        let producer = tokio::task::spawn_blocking(move || produce_candidates(sources, sender));
        assert!(receiver.recv().await.is_some());
        drop(receiver);

        tokio::time::timeout(Duration::from_secs(1), producer)
            .await
            .unwrap()
            .unwrap();
    });
}
