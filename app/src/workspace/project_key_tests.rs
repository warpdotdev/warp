use std::path::{Path, PathBuf};

use super::{GitResolution, ProjectKey, ProjectKeyInput, display_name, resolve};

fn key(path: &str) -> ProjectKey {
    ProjectKey(PathBuf::from(path))
}

/// A resolution for a directory inside a repository whose shared git directory
/// is already known.
fn in_repo(directory: &str, common_git_dir: &str, existing: &[ProjectKey]) -> Option<ProjectKey> {
    resolve(&ProjectKeyInput {
        directory: Some(Path::new(directory)),
        git: GitResolution::Resolved(PathBuf::from(common_git_dir)),
        existing_non_git_keys: existing,
        home_dir: Some(Path::new("/Users/me")),
    })
}

/// A resolution for a directory that detection has confirmed is not in a repo.
fn outside_repo(directory: &str, existing: &[ProjectKey]) -> Option<ProjectKey> {
    resolve(&ProjectKeyInput {
        directory: Some(Path::new(directory)),
        git: GitResolution::NotARepository,
        existing_non_git_keys: existing,
        home_dir: Some(Path::new("/Users/me")),
    })
}

// Git identity.

#[test]
fn worktrees_of_one_repository_resolve_to_one_key() {
    let main = in_repo("/work/api", "/work/api/.git", &[]);
    let feature = in_repo("/tmp/wt-feature", "/work/api/.git", &[]);

    assert_eq!(main, feature);
    assert_eq!(main, Some(key("/work/api/.git")));
}

#[test]
fn worktrees_of_one_bare_repository_resolve_to_one_key() {
    let a = in_repo("/tmp/wt-a", "/repos/api.git", &[]);
    let b = in_repo("/tmp/wt-b", "/repos/api.git", &[]);

    assert_eq!(a, b);
    assert_eq!(a, Some(key("/repos/api.git")));
}

#[test]
fn different_repositories_resolve_to_different_keys() {
    assert_ne!(
        in_repo("/work/api", "/work/api/.git", &[]),
        in_repo("/work/web", "/work/web/.git", &[]),
    );
}

// Non-git identity.

#[test]
fn a_directory_outside_any_repository_keys_on_itself() {
    assert_eq!(
        outside_repo("/Users/me/notes", &[]),
        Some(key("/Users/me/notes")),
    );
}

#[test]
fn a_subdirectory_joins_the_existing_group_for_its_prefix() {
    // Descending must not destroy one group and create another.
    assert_eq!(
        outside_repo("/Users/me/notes/daily", &[key("/Users/me/notes")]),
        Some(key("/Users/me/notes")),
    );
}

#[test]
fn the_longest_matching_prefix_wins() {
    let existing = [key("/Users/me/notes"), key("/Users/me/notes/daily")];

    assert_eq!(
        outside_repo("/Users/me/notes/daily/monday", &existing),
        Some(key("/Users/me/notes/daily")),
    );
}

#[test]
fn a_directory_above_every_existing_key_yields_no_key() {
    // Keying here would create a parent group that swallows the existing one.
    assert_eq!(outside_repo("/Users/me", &[key("/Users/me/notes")]), None);
}

#[test]
fn the_home_directory_and_the_filesystem_root_yield_no_key() {
    assert_eq!(outside_repo("/Users/me", &[]), None);
    assert_eq!(outside_repo("/", &[]), None);
}

#[test]
fn a_sideways_move_to_an_unrelated_directory_produces_a_new_key() {
    assert_eq!(
        outside_repo("/Users/me/scratch", &[key("/Users/me/notes")]),
        Some(key("/Users/me/scratch")),
    );
}

// Unresolvable identity.

#[test]
fn no_directory_yields_no_key() {
    assert_eq!(
        resolve(&ProjectKeyInput {
            directory: None,
            git: GitResolution::NotARepository,
            existing_non_git_keys: &[],
            home_dir: Some(Path::new("/Users/me")),
        }),
        None,
    );
}

#[test]
fn a_pending_lookup_yields_no_key_rather_than_a_directory_key() {
    // The asynchronous-detection window must not be read as "not a repo".
    assert_eq!(
        resolve(&ProjectKeyInput {
            directory: Some(Path::new("/work/api")),
            git: GitResolution::Pending,
            existing_non_git_keys: &[],
            home_dir: Some(Path::new("/Users/me")),
        }),
        None,
    );
}

// Display names. The rule has to be total, or the name-provenance comparison
// that protects a user's rename cannot work.

#[test]
fn a_normal_checkout_derives_the_repository_directory_name() {
    assert_eq!(display_name(&key("/work/api/.git"), &[]), "api");
}

#[test]
fn a_bare_repository_derives_its_name_without_the_git_suffix() {
    assert_eq!(display_name(&key("/repos/api.git"), &[]), "api");
}

#[test]
fn a_non_git_key_derives_its_basename() {
    assert_eq!(display_name(&key("/Users/me/notes"), &[]), "notes");
}

#[test]
fn two_repositories_sharing_a_basename_derive_distinct_qualified_names() {
    let a = key("/work/services/api/.git");
    let b = key("/work/vendor/api/.git");
    let all = [a.clone(), b.clone()];

    assert_eq!(display_name(&a, &all), "services/api");
    assert_eq!(display_name(&b, &all), "vendor/api");
}

#[test]
fn a_name_is_left_unqualified_when_nothing_collides() {
    let a = key("/work/services/api/.git");
    let b = key("/work/vendor/web/.git");
    let all = [a.clone(), b.clone()];

    assert_eq!(display_name(&a, &all), "api");
    assert_eq!(display_name(&b, &all), "web");
}
