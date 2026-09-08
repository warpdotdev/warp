use std::fs;
use std::path::{Path, PathBuf};

use super::{CandidateProducer, MAX_CHILD_CANDIDATES, MAX_VISITED_DIRECTORIES, stable_child_id};
use crate::{RepoIdentity, RepositoryCacheSource};

fn source(root: &Path) -> RepositoryCacheSource {
    RepositoryCacheSource {
        name: "warp/example".to_owned(),
        identity: RepoIdentity::new("github.com", "warp", "example"),
        cwd: root.to_path_buf(),
    }
}

fn child_paths(root: &Path) -> Vec<String> {
    let mut producer = CandidateProducer::new(vec![source(root)]);
    let root = producer.next_candidate().unwrap();
    assert_eq!(root.key.normalized_relative_path, None);
    std::iter::from_fn(|| producer.next_candidate())
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
    for (index, marker) in [
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
    ]
    .into_iter()
    .enumerate()
    {
        touch(temp.path(), &format!("project-{index}/{marker}"));
    }

    let paths = child_paths(temp.path());

    assert_eq!(paths.len(), 29);
    for index in 0..29 {
        assert!(paths.contains(&format!("project-{index}")));
    }
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

    assert!(paths.contains(&"a".to_owned()));
    assert!(paths.contains(&"b".to_owned()));
    assert!(paths.contains(&"c".to_owned()));
    assert!(paths.contains(&"c/.config".to_owned()));
    assert!(paths.contains(&"d".to_owned()));
    assert!(paths.contains(&"e".to_owned()));
    assert!(paths.contains(&"f".to_owned()));
    assert!(paths.contains(&"g".to_owned()));
}

#[test]
fn non_markers_deep_candidates_and_ignored_subtrees_are_skipped() {
    let temp = tempfile::tempdir().unwrap();
    touch(temp.path(), "frontend/package.json");
    touch(temp.path(), "backend/pyproject.toml");
    touch(temp.path(), "gradle/settings.gradle");
    touch(temp.path(), "kotlin/build.gradle.kts");
    touch(temp.path(), "a/b/c/d/e/Cargo.toml");
    touch(temp.path(), "node_modules/nested/Cargo.toml");
    touch(temp.path(), "target/nested/go.mod");
    touch(temp.path(), "valid/Cargo.toml");

    assert_eq!(child_paths(temp.path()), ["valid"]);
}

#[test]
fn multiple_markers_deduplicate_exact_roots_but_keep_nested_roots() {
    let temp = tempfile::tempdir().unwrap();
    touch(temp.path(), "project/Cargo.toml");
    touch(temp.path(), "project/package-lock.json");
    touch(temp.path(), "project/nested/go.mod");

    assert_eq!(child_paths(temp.path()), ["project", "project/nested"]);
}

#[test]
fn traversal_is_sorted_depth_first() {
    let temp = tempfile::tempdir().unwrap();
    touch(temp.path(), "z/Cargo.toml");
    touch(temp.path(), "a/nested/Cargo.toml");
    touch(temp.path(), "a/Cargo.toml");

    assert_eq!(child_paths(temp.path()), ["a", "a/nested", "z"]);
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
fn depth_four_relative_marker_is_found_without_selecting_depth_five() {
    let temp = tempfile::tempdir().unwrap();
    touch(temp.path(), "one/two/three/four/.config/mise/config.toml");
    touch(temp.path(), "one/two/three/four/five/Cargo.toml");

    assert_eq!(child_paths(temp.path()), ["one/two/three/four"]);
}

#[test]
fn child_limit_retains_root_plus_first_32_children() {
    let temp = tempfile::tempdir().unwrap();
    for index in 0..MAX_CHILD_CANDIDATES + 1 {
        touch(temp.path(), &format!("{index:02}/Cargo.toml"));
    }
    let mut producer = CandidateProducer::new(vec![source(temp.path())]);
    let candidates = std::iter::from_fn(|| producer.next_candidate()).collect::<Vec<_>>();

    assert_eq!(candidates.len(), MAX_CHILD_CANDIDATES + 1);
    assert_eq!(candidates[0].key.normalized_relative_path, None);
    assert_eq!(
        candidates
            .last()
            .unwrap()
            .key
            .normalized_relative_path
            .as_deref(),
        Some("31")
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
        stable_child_id("frontend/web"),
        "4984839f9fe7d9730ec3fd45d7ededa43b0b5bfaf82f21666f7d9c25d1cf234c"
    );
    assert_eq!(stable_child_id("frontend/web").len(), 64);
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
    let mut producer = CandidateProducer::new(sources);
    let candidates = std::iter::from_fn(|| producer.next_candidate()).collect::<Vec<_>>();
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
    let mut producer = CandidateProducer::new(vec![source(temp.path())]);
    let root = producer.next_candidate().unwrap();
    let child = producer.next_candidate().unwrap();

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
