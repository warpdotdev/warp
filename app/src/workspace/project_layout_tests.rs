use std::collections::HashSet;
use std::path::Path;
use std::time::{Duration, SystemTime};

use warp_util::standardized_path::StandardizedPath;

use super::super::project_key::ProjectKey;
use super::{
    DormantTask, DormantTaskOrigin, ProjectEntry, ProjectId, ProjectLayout, ScannedSession,
    dormant_label, unwitnessed_sessions,
};
use crate::terminal::CLIAgent;

fn git_key(path: &str) -> ProjectKey {
    ProjectKey::LocalGit(StandardizedPath::try_from_local(Path::new(path)).unwrap())
}

/// Builds a layout directly from a per-tab project list, mirroring the
/// first-seen dedupe in `compute` (which itself needs a live app + pane groups
/// and is exercised by the GUI verification instead).
fn layout_from(tab_project: Vec<ProjectId>) -> ProjectLayout {
    let mut projects: Vec<ProjectEntry> = Vec::new();
    for id in &tab_project {
        if !projects.iter().any(|entry| &entry.id == id) {
            projects.push(ProjectEntry {
                display_name: id.display_name(),
                id: id.clone(),
            });
        }
    }
    ProjectLayout {
        projects,
        tab_project,
        tab_pane_group_ids: Vec::new(),
        dormant: Vec::new(),
    }
}

/// A dormant task bucketed to `project`, as `compute_with_handles` would emit.
fn dormant(project: ProjectId, session_id: &str, label: &str) -> (ProjectId, DormantTask) {
    (
        project,
        DormantTask {
            agent: CLIAgent::Claude,
            session_id: session_id.to_owned(),
            label: label.to_owned(),
            cwd: "/dev/example".to_owned(),
            origin: DormantTaskOrigin::Handle,
        },
    )
}

#[test]
fn visible_tab_indices_selects_only_that_project() {
    let warp = ProjectId::Key(git_key("/Users/sam/dev/warp/.git"));
    let orbit = ProjectId::Key(git_key("/Users/sam/dev/orbit/.git"));
    let layout = layout_from(vec![
        warp.clone(),
        orbit.clone(),
        warp.clone(),
        ProjectId::Other,
    ]);
    assert_eq!(layout.visible_tab_indices(&warp), vec![0, 2]);
    assert_eq!(layout.visible_tab_indices(&orbit), vec![1]);
    assert_eq!(layout.visible_tab_indices(&ProjectId::Other), vec![3]);
}

#[test]
fn projects_are_distinct_in_first_seen_order() {
    let warp = ProjectId::Key(git_key("/Users/sam/dev/warp/.git"));
    let orbit = ProjectId::Key(git_key("/Users/sam/dev/orbit/.git"));
    let layout = layout_from(vec![orbit.clone(), warp.clone(), orbit.clone()]);
    let names: Vec<_> = layout
        .projects()
        .iter()
        .map(|entry| entry.display_name.clone())
        .collect();
    assert_eq!(names, vec!["orbit", "warp"]);
}

#[test]
fn other_bucket_is_named_other() {
    assert_eq!(ProjectId::Other.display_name(), "Other");
    let layout = layout_from(vec![ProjectId::Other]);
    assert_eq!(layout.projects().len(), 1);
}

#[test]
fn cycle_next_and_prev_wrap_within_subset() {
    use super::{cycle_next, cycle_prev};
    let visible = [0usize, 2, 5];
    assert_eq!(cycle_next(&visible, 0), 2);
    assert_eq!(cycle_next(&visible, 5), 0); // wraps to start
    assert_eq!(cycle_prev(&visible, 0), 5); // wraps to end
    assert_eq!(cycle_prev(&visible, 2), 0);
    // A current index outside the subset falls back to the first visible tab.
    assert_eq!(cycle_next(&visible, 3), 0);
    assert_eq!(cycle_prev(&visible, 3), 0);
    // Empty subset returns current unchanged.
    assert_eq!(cycle_next(&[], 4), 4);
}

#[test]
fn dormant_tasks_are_scoped_to_their_project_in_store_order() {
    let orbit = ProjectId::Key(git_key("/repos/orbit/.git"));
    let warp = ProjectId::Key(git_key("/repos/warp/.git"));
    let mut layout = layout_from(vec![orbit.clone()]);
    layout.dormant = vec![
        dormant(orbit.clone(), "aaaa", "Fix retry backoff"),
        dormant(warp.clone(), "bbbb", "Two-line tabs"),
        dormant(orbit.clone(), "cccc", "Rerank eval harness"),
    ];

    let labels: Vec<_> = layout
        .dormant_tasks_for_project(&orbit)
        .into_iter()
        .map(|task| task.label.as_str())
        .collect();
    assert_eq!(labels, vec!["Fix retry backoff", "Rerank eval harness"]);

    // A project with only dormant rows still resolves its own tasks.
    assert_eq!(layout.dormant_tasks_for_project(&warp).len(), 1);
    // And a project with none gets an empty list, not the whole set.
    assert!(
        layout
            .dormant_tasks_for_project(&ProjectId::Other)
            .is_empty()
    );
}

#[test]
fn compute_alone_never_yields_dormant_rows() {
    // Navigation paths use `compute`, which must stay tabs-only so dormant
    // tasks can never reach the tab bar or the cycle-next/prev order.
    let orbit = ProjectId::Key(git_key("/repos/orbit/.git"));
    let layout = layout_from(vec![orbit.clone()]);
    assert!(layout.dormant_tasks_for_project(&orbit).is_empty());
}

fn scanned(session_id: &str, label: Option<&str>, modified_secs: u64) -> ScannedSession {
    ScannedSession {
        session_id: session_id.to_owned(),
        cwd: "/repos/orbit".to_owned(),
        // The rail consumes the resolved label; which candidate produced it is
        // the scan model's business, so these rows carry none.
        #[cfg(not(target_family = "wasm"))]
        names: Default::default(),
        label: label.map(str::to_owned),
        modified: SystemTime::UNIX_EPOCH + Duration::from_secs(modified_secs),
    }
}

#[test]
fn unwitnessed_rows_exclude_live_and_handle_bound_sessions() {
    let sessions = [
        scanned("aaaa", Some("Witnessed"), 100),
        scanned("bbbb", Some("Unwitnessed"), 200),
        scanned("cccc", Some("Running right now"), 300),
    ];
    let bound: HashSet<&str> = ["aaaa"].into_iter().collect();
    let live: HashSet<(CLIAgent, String)> = [(CLIAgent::Claude, "cccc".to_owned())]
        .into_iter()
        .collect();

    let rows = unwitnessed_sessions(&sessions, &bound, &live);

    // Filtered here, before the rail's row cap, so a burst of running or
    // already-witnessed sessions cannot bury the resumable ones.
    assert_eq!(
        rows.iter()
            .map(|session| session.session_id.as_str())
            .collect::<Vec<_>>(),
        vec!["bbbb"]
    );
}

#[test]
fn unwitnessed_rows_are_newest_first() {
    // mtime is the only recency signal an unwitnessed session has.
    let sessions = [
        scanned("old", Some("Old"), 100),
        scanned("newest", Some("Newest"), 900),
        scanned("middle", Some("Middle"), 500),
    ];
    let rows = unwitnessed_sessions(&sessions, &HashSet::new(), &HashSet::new());
    assert_eq!(
        rows.iter()
            .map(|session| session.session_id.as_str())
            .collect::<Vec<_>>(),
        vec!["newest", "middle", "old"]
    );
}

#[test]
fn scanned_names_beat_the_cached_handle_title() {
    // `/rename` lands in the transcript after the handle was written, so disk
    // is the fresher of the two names.
    assert_eq!(
        dormant_label(
            Some("Renamed on disk"),
            Some("Stale cache"),
            CLIAgent::Claude,
            "aaaa"
        ),
        "Renamed on disk"
    );
    // Without a scan result the cached title still stands.
    assert_eq!(
        dormant_label(None, Some("Stale cache"), CLIAgent::Claude, "aaaa"),
        "Stale cache"
    );
    // And with neither, the floor is an id — never a path.
    assert_eq!(
        dormant_label(None, None, CLIAgent::Claude, "61f785ca-1c31-4671"),
        format!("{} · 61f785ca", CLIAgent::Claude.display_name())
    );
}

/// A dormant or scanned row is never labelled blank either. The handle store
/// writes whatever title a plugin event carried, verbatim, so an empty or
/// whitespace-only one reaches here and has to fall through to the id floor
/// rather than draw an empty row.
#[test]
fn a_dormant_row_is_never_labelled_blank() {
    const BLANKS: [&str; 4] = ["", " ", "   ", "\n\t "];
    let floor = format!("{} · 61f785ca", CLIAgent::Claude.display_name());

    for scanned in BLANKS {
        for cached in BLANKS {
            assert_eq!(
                dormant_label(
                    Some(scanned),
                    Some(cached),
                    CLIAgent::Claude,
                    "61f785ca-1c31-4671-9d0e-2b0f5b7d1234"
                ),
                floor,
                "blank labels ({scanned:?}, {cached:?}) should fall through to the id floor"
            );
        }
    }

    // A blank scan result must not shadow a cached title that does say
    // something — the fall-through is per candidate, not all-or-nothing.
    for blank in BLANKS {
        assert_eq!(
            dormant_label(
                Some(blank),
                Some("  Fix the parser "),
                CLIAgent::Claude,
                "61f785ca"
            ),
            "Fix the parser"
        );
    }
}
