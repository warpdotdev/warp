use std::path::Path;

use warp_util::host_id::HostId;
use warp_util::remote_path::RemotePath;
use warp_util::standardized_path::StandardizedPath;

use super::{ProjectPriorities, RailProjectRow, rail_project_rows};
use crate::workspace::project_key::ProjectKey;
use crate::workspace::project_layout::{ProjectEntry, ProjectId};

fn std_path(path: &str) -> StandardizedPath {
    StandardizedPath::try_from_local(Path::new(path)).unwrap()
}

fn git(repo: &str) -> ProjectKey {
    ProjectKey::LocalGit(std_path(&format!("{repo}/.git")))
}

fn priorities(keys: &[&ProjectKey]) -> ProjectPriorities {
    ProjectPriorities(keys.iter().map(|key| key.to_storage_key()).collect())
}

fn entries(keys: &[&ProjectKey]) -> Vec<ProjectEntry> {
    keys.iter()
        .map(|key| ProjectEntry {
            display_name: key.display_name(),
            id: ProjectId::Key((*key).clone()),
        })
        .collect()
}

fn other_entry() -> ProjectEntry {
    ProjectEntry {
        id: ProjectId::Other,
        display_name: ProjectId::Other.display_name(),
    }
}

// ── Storage-key round trip ───────────────────────────────────────────

#[test]
fn storage_key_round_trips_every_variant() {
    let keys = [
        ProjectKey::LocalGit(std_path("/Users/sam/dev/warp/.git")),
        ProjectKey::LocalDir(std_path("/Users/sam/notes")),
        ProjectKey::Remote(RemotePath::new(
            HostId::new("host-abc".to_owned()),
            std_path("/srv/code/api"),
        )),
    ];
    for key in keys {
        let encoded = key.to_storage_key();
        assert_eq!(
            ProjectKey::from_storage_key(&encoded),
            Some(key.clone()),
            "round trip failed for {encoded}"
        );
    }
}

#[test]
fn storage_key_tags_distinguish_same_path_across_variants() {
    // A git repo and a plain directory can name the same path string; the
    // variant tag is what stops them sharing one settings entry.
    let git_key = ProjectKey::LocalGit(std_path("/Users/sam/dev/warp"));
    let dir_key = ProjectKey::LocalDir(std_path("/Users/sam/dev/warp"));
    assert_ne!(git_key.to_storage_key(), dir_key.to_storage_key());
}

#[test]
fn malformed_storage_keys_are_rejected() {
    // Hand-edited or newer-version entries must be skipped, not fatal.
    assert_eq!(ProjectKey::from_storage_key(""), None);
    assert_eq!(ProjectKey::from_storage_key("nonsense"), None);
    assert_eq!(ProjectKey::from_storage_key("bogus:/tmp"), None);
    // A remote entry with no host/path separator.
    assert_eq!(ProjectKey::from_storage_key("remote:justahost"), None);
}

// ── rank_of ──────────────────────────────────────────────────────────

#[test]
fn rank_of_is_the_list_index() {
    let (a, b, c) = (git("/dev/a"), git("/dev/b"), git("/dev/c"));
    let list = priorities(&[&a, &b]);
    assert_eq!(list.rank_of(&a), Some(0));
    assert_eq!(list.rank_of(&b), Some(1));
    assert_eq!(list.rank_of(&c), None);
}

#[test]
fn rank_of_survives_the_string_encoding() {
    // The stored form is a string, so a freshly-built key with no shared
    // allocation must still resolve to the stored rank.
    let stored = ProjectPriorities(vec![git("/dev/warp").to_storage_key()]);
    let rebuilt = ProjectKey::LocalGit(std_path("/dev/warp/.git"));
    assert_eq!(stored.rank_of(&rebuilt), Some(0));
}

#[test]
fn worktrees_of_one_repo_share_a_rank() {
    // Two worktrees of a repo resolve (via `common_git_dir`) to the same
    // shared `.git`, so they encode to one storage key and therefore one rank
    // — the whole reason priorities are keyed by `ProjectKey` and not by cwd.
    // The checkout → common-git-dir resolution itself lives in
    // `ProjectKey::for_path` / `repo_metadata`; here the shared `.git` is the
    // input.
    let common_git_dir = std_path("/Users/sam/dev/warp/.git");
    let from_main = ProjectKey::LocalGit(common_git_dir.clone());
    let from_worktree = ProjectKey::LocalGit(common_git_dir);
    let list = priorities(&[&from_main]);

    assert_eq!(from_main.to_storage_key(), from_worktree.to_storage_key());
    assert_eq!(list.rank_of(&from_worktree), Some(0));
    // A different repo must not borrow that rank.
    assert_eq!(list.rank_of(&git("/Users/sam/dev/orbit")), None);
}

#[test]
fn other_is_never_rankable() {
    let warp = git("/dev/warp");
    let list = priorities(&[&warp]);
    assert_eq!(list.rank_of_project(&ProjectId::Other), None);
    assert!(!list.contains(&ProjectId::Other));
    assert!(!list.can_move_up(&ProjectId::Other));
    assert!(!list.can_move_down(&ProjectId::Other));
}

// ── Mutators ─────────────────────────────────────────────────────────

#[test]
fn add_to_top_prepends() {
    let (a, b) = (git("/dev/a"), git("/dev/b"));
    let list = priorities(&[&a]).with_added_to_top(&b);
    assert_eq!(list.rank_of(&b), Some(0));
    assert_eq!(list.rank_of(&a), Some(1));
}

#[test]
fn add_to_top_promotes_instead_of_duplicating() {
    let (a, b, c) = (git("/dev/a"), git("/dev/b"), git("/dev/c"));
    let list = priorities(&[&a, &b, &c]).with_added_to_top(&c);
    assert_eq!(list.0.len(), 3, "re-adding must not duplicate the entry");
    assert_eq!(list.rank_of(&c), Some(0));
    assert_eq!(list.rank_of(&a), Some(1));
    assert_eq!(list.rank_of(&b), Some(2));
}

#[test]
fn remove_closes_the_gap() {
    let (a, b, c) = (git("/dev/a"), git("/dev/b"), git("/dev/c"));
    let list = priorities(&[&a, &b, &c]).with_removed(&b);
    assert_eq!(list.rank_of(&a), Some(0));
    assert_eq!(list.rank_of(&c), Some(1));
    assert_eq!(list.rank_of(&b), None);
}

#[test]
fn remove_of_unranked_project_is_a_no_op() {
    let (a, b) = (git("/dev/a"), git("/dev/b"));
    let list = priorities(&[&a]);
    assert_eq!(list.with_removed(&b), list);
}

#[test]
fn move_up_swaps_with_the_previous_rank() {
    let (a, b, c) = (git("/dev/a"), git("/dev/b"), git("/dev/c"));
    let list = priorities(&[&a, &b, &c]).with_moved_up(&c);
    assert_eq!(list.rank_of(&a), Some(0));
    assert_eq!(list.rank_of(&c), Some(1));
    assert_eq!(list.rank_of(&b), Some(2));
}

#[test]
fn move_down_swaps_with_the_next_rank() {
    let (a, b, c) = (git("/dev/a"), git("/dev/b"), git("/dev/c"));
    let list = priorities(&[&a, &b, &c]).with_moved_down(&a);
    assert_eq!(list.rank_of(&b), Some(0));
    assert_eq!(list.rank_of(&a), Some(1));
    assert_eq!(list.rank_of(&c), Some(2));
}

#[test]
fn moves_at_the_boundaries_do_nothing() {
    let (a, b) = (git("/dev/a"), git("/dev/b"));
    let list = priorities(&[&a, &b]);
    // Top project cannot rise; bottom project cannot sink — and neither may
    // wrap around to the other end.
    assert_eq!(list.with_moved_up(&a), list);
    assert_eq!(list.with_moved_down(&b), list);
}

#[test]
fn moves_of_unranked_projects_do_nothing() {
    let (a, unranked) = (git("/dev/a"), git("/dev/z"));
    let list = priorities(&[&a]);
    assert_eq!(list.with_moved_up(&unranked), list);
    assert_eq!(list.with_moved_down(&unranked), list);
}

#[test]
fn can_move_flags_match_the_boundaries() {
    let (a, b, c, unranked) = (git("/dev/a"), git("/dev/b"), git("/dev/c"), git("/dev/z"));
    let list = priorities(&[&a, &b, &c]);
    let id = |key: &ProjectKey| ProjectId::Key(key.clone());
    assert!(!list.can_move_up(&id(&a)));
    assert!(list.can_move_down(&id(&a)));
    assert!(list.can_move_up(&id(&b)));
    assert!(list.can_move_down(&id(&b)));
    assert!(list.can_move_up(&id(&c)));
    assert!(!list.can_move_down(&id(&c)));
    assert!(!list.can_move_up(&id(&unranked)));
    assert!(!list.can_move_down(&id(&unranked)));
}

// ── Banding ──────────────────────────────────────────────────────────

#[test]
fn ranked_band_comes_first_in_rank_order() {
    let (a, b, c) = (git("/dev/a"), git("/dev/b"), git("/dev/c"));
    // First-seen order a, b, c; priority order c, a.
    let rows = rail_project_rows(&entries(&[&a, &b, &c]), &priorities(&[&c, &a]));
    assert_eq!(
        rows,
        vec![
            RailProjectRow::Project {
                index: 2,
                rank: Some(0)
            },
            RailProjectRow::Project {
                index: 0,
                rank: Some(1)
            },
            RailProjectRow::UnrankedDivider,
            RailProjectRow::Project {
                index: 1,
                rank: None
            },
        ]
    );
}

#[test]
fn unranked_band_keeps_first_seen_order() {
    let (a, b, c, d) = (git("/dev/a"), git("/dev/b"), git("/dev/c"), git("/dev/d"));
    let rows = rail_project_rows(&entries(&[&a, &b, &c, &d]), &priorities(&[&c]));
    let unranked: Vec<usize> = rows
        .iter()
        .skip_while(|row| **row != RailProjectRow::UnrankedDivider)
        .skip(1)
        .filter_map(|row| match row {
            RailProjectRow::Project { index, rank: None } => Some(*index),
            RailProjectRow::Project { rank: Some(_), .. } | RailProjectRow::UnrankedDivider => None,
        })
        .collect();
    assert_eq!(unranked, vec![0, 1, 3]);
}

#[test]
fn divider_appears_only_when_both_bands_are_non_empty() {
    let (a, b) = (git("/dev/a"), git("/dev/b"));
    let has_divider = |rows: &[RailProjectRow]| rows.contains(&RailProjectRow::UnrankedDivider);

    // Nothing ranked: one band, no boundary to mark.
    assert!(!has_divider(&rail_project_rows(
        &entries(&[&a, &b]),
        &ProjectPriorities::default()
    )));
    // Everything ranked: likewise.
    assert!(!has_divider(&rail_project_rows(
        &entries(&[&a, &b]),
        &priorities(&[&a, &b])
    )));
    // Mixed: the divider earns its row.
    assert!(has_divider(&rail_project_rows(
        &entries(&[&a, &b]),
        &priorities(&[&a])
    )));
    // No projects at all.
    assert!(!has_divider(&rail_project_rows(&[], &priorities(&[&a]))));
}

#[test]
fn unrankable_other_stays_in_the_unranked_band() {
    let a = git("/dev/a");
    let mut projects = entries(&[&a]);
    projects.push(other_entry());
    let rows = rail_project_rows(&projects, &priorities(&[&a]));
    assert_eq!(
        rows,
        vec![
            RailProjectRow::Project {
                index: 0,
                rank: Some(0)
            },
            RailProjectRow::UnrankedDivider,
            RailProjectRow::Project {
                index: 1,
                rank: None
            },
        ]
    );
}

#[test]
fn stale_priority_entries_for_closed_projects_are_ignored() {
    // The list outlives the projects it names — a ranked project with no open
    // tabs and no dormant sessions simply has no row, and must not shift the
    // ranks of the projects that are present.
    let (open, closed) = (git("/dev/open"), git("/dev/closed"));
    let rows = rail_project_rows(&entries(&[&open]), &priorities(&[&closed, &open]));
    assert_eq!(
        rows,
        vec![RailProjectRow::Project {
            index: 0,
            rank: Some(1)
        }]
    );
}
