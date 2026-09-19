//! The project-layout projection.
//!
//! Derives the set of projects from the currently-open tabs and maps each tab
//! to its project, so navigation and rendering can be scoped to a selected
//! project without ever filtering the raw `Workspace::tabs` vector in place.
//! This is the single source of truth the sidebar rail, the top tab bar, and
//! every navigation path consult.
//!
//! Recompute whenever tabs are added/removed/reordered or a tab's focused
//! repo/cwd changes; the result is a pure function of the tabs.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use warp_util::local_or_remote_path::LocalOrRemotePath;
use warpui::{AppContext, EntityId};

use super::project_key::ProjectKey;
use crate::tab::TabData;
use crate::terminal::CLIAgent;
use crate::terminal::cli_agent_sessions::handle_store::AgentSessionHandle;
use crate::terminal::cli_agent_sessions::session_scan::ScannedSession;

/// Identifies a project bucket in the sidebar. `Other` collects tabs with no
/// detectable repo/directory (for example a bare home-directory session).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ProjectId {
    Key(ProjectKey),
    Other,
}

impl ProjectId {
    /// The rail label for this project.
    pub fn display_name(&self) -> String {
        match self {
            Self::Key(key) => key.display_name(),
            Self::Other => "Other".to_owned(),
        }
    }
}

/// One project entry shown in the rail.
#[derive(Debug, Clone)]
pub struct ProjectEntry {
    pub id: ProjectId,
    pub display_name: String,
}

/// Where a dormant row came from, and therefore what may be done with it.
///
/// This is the rail-side face of the authority split documented on
/// [`session_scan`](crate::terminal::cli_agent_sessions::session_scan): only a
/// witnessed session has a pane, so only a witnessed session can be resumed in
/// the pane that ran it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DormantTaskOrigin {
    /// Warp spawned and witnessed this session, so the durable handle store
    /// holds its pane uuid and owns its pane binding.
    Handle,
    /// Found by scanning Claude Code's own transcripts. Warp never saw it run;
    /// it has no pane, no tab and no handle, and resumes only in a fresh tab
    /// at its directory.
    Scanned,
}

/// A dormant task row: a resumable session with no open tab, either from the
/// stored agent-session handles or discovered on disk. The rail renders these
/// under a project's live tasks; clicking one resumes the session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DormantTask {
    pub agent: CLIAgent,
    pub session_id: String,
    /// Resolved display label: the conversation title (scanned from the
    /// transcript when available, else the cached one), or the
    /// `Agent · shortid` floor. Never a path — replacing path-derived labels
    /// is the point of the rail.
    pub label: String,
    pub cwd: String,
    pub origin: DormantTaskOrigin,
}

/// A pure projection of the workspace's tabs into projects.
#[derive(Debug, Clone, Default)]
pub struct ProjectLayout {
    /// Distinct projects, in first-seen (stable) order.
    projects: Vec<ProjectEntry>,
    /// The project of each tab, parallel to `Workspace::tabs` by index.
    tab_project: Vec<ProjectId>,
    /// The pane-group id of each tab, parallel by index. Used to resolve a
    /// visible index back to a stable tab identity at the edge.
    tab_pane_group_ids: Vec<EntityId>,
    /// Dormant tasks per project, most-recently-seen first (the handle store's
    /// order). Populated only by [`Self::compute_with_handles`]; navigation
    /// paths that use [`Self::compute`] never see dormant rows.
    dormant: Vec<(ProjectId, DormantTask)>,
}

impl ProjectLayout {
    /// Computes the projection from the workspace tabs.
    pub fn compute(tabs: &[TabData], ctx: &AppContext) -> Self {
        let mut projects: Vec<ProjectEntry> = Vec::new();
        let mut tab_project = Vec::with_capacity(tabs.len());
        let mut tab_pane_group_ids = Vec::with_capacity(tabs.len());

        for tab in tabs {
            let id = Self::project_of_tab_data(tab, ctx);
            if !projects.iter().any(|entry| entry.id == id) {
                projects.push(ProjectEntry {
                    display_name: id.display_name(),
                    id: id.clone(),
                });
            }
            tab_project.push(id);
            tab_pane_group_ids.push(tab.pane_group.id());
        }

        Self {
            projects,
            tab_project,
            tab_pane_group_ids,
            dormant: Vec::new(),
        }
    }

    /// Like [`Self::compute`], but additionally buckets resumable sessions into
    /// their projects — including projects with **no open tabs**, which is what
    /// keeps the rail populated after a restart. Sessions that are live
    /// (`live_session_ids`) are suppressed: the live row wins.
    ///
    /// Two sources feed this, per the authority split on
    /// [`session_scan`](crate::terminal::cli_agent_sessions::session_scan):
    /// `handles` (witnessed sessions, which own their pane binding) and
    /// `scanned` (whatever Claude recorded on disk, which owns names and the
    /// existence of sessions Warp never saw). Scanned rows are appended after
    /// the handle rows so a project's own history cannot displace what Warp
    /// itself ran, and live/handle-bound ids are filtered out of the scanned
    /// set **before** the rail applies its row cap, so running sessions cannot
    /// hide resumable ones.
    ///
    /// The cwd → project resolution is memoised per distinct cwd within this
    /// single pass; nothing is persisted (repo detection may reclassify a path
    /// later, and the next compute simply picks that up).
    pub fn compute_with_handles(
        tabs: &[TabData],
        handles: &[AgentSessionHandle],
        live_session_ids: &HashSet<(CLIAgent, String)>,
        scanned: &[ScannedSession],
        ctx: &AppContext,
    ) -> Self {
        let mut layout = Self::compute(tabs, ctx);

        // Join key is the session UUID. A repo's worktrees bucket to one
        // project but have distinct cwds, so the same session can be reached
        // from more than one scanned directory; first one wins.
        let mut scanned_by_id: HashMap<&str, &ScannedSession> = HashMap::new();
        for session in scanned {
            scanned_by_id
                .entry(session.session_id.as_str())
                .or_insert(session);
        }

        let mut project_by_cwd: HashMap<&str, ProjectId> = HashMap::new();
        let mut bound_session_ids: HashSet<&str> = HashSet::new();
        for handle in handles {
            bound_session_ids.insert(handle.session_id.as_str());
            if live_session_ids.contains(&(handle.agent, handle.session_id.clone())) {
                continue;
            }
            // A tab still *hosts* this session (commonly: the agent exited but
            // its tab remains, or the tab was restored) — that tab already
            // shows this handle's title, so a dormant row would duplicate it.
            //
            // Both the persistent pane uuid and the directory must match. A
            // pane that has since `cd`-ed away, or restored to a different
            // startup directory, no longer hosts the session: it must not
            // absorb the row, or the task would be filed under whichever
            // project that pane now sits in rather than its own.
            if tabs.iter().any(|tab| {
                let pane_group = tab.pane_group.as_ref(ctx);
                pane_group
                    .find_terminal_pane_by_session_uuid(&handle.pane_uuid)
                    .is_some()
                    && pane_group.held_session_directory(ctx).as_deref()
                        == Some(handle.cwd.as_str())
            }) {
                continue;
            }
            let project = project_by_cwd
                .entry(handle.cwd.as_str())
                .or_insert_with(|| {
                    ProjectKey::for_path(&LocalOrRemotePath::Local(PathBuf::from(&handle.cwd)), ctx)
                        .map(ProjectId::Key)
                        .unwrap_or(ProjectId::Other)
                })
                .clone();
            if !layout.projects.iter().any(|entry| entry.id == project) {
                layout.projects.push(ProjectEntry {
                    display_name: project.display_name(),
                    id: project.clone(),
                });
            }
            layout.dormant.push((
                project,
                DormantTask {
                    agent: handle.agent,
                    session_id: handle.session_id.clone(),
                    label: dormant_label(
                        scanned_by_id
                            .get(handle.session_id.as_str())
                            .and_then(|session| session.label.as_deref()),
                        handle.title.as_deref(),
                        handle.agent,
                        &handle.session_id,
                    ),
                    cwd: handle.cwd.clone(),
                    origin: DormantTaskOrigin::Handle,
                },
            ));
        }

        for session in unwitnessed_sessions(scanned, &bound_session_ids, live_session_ids) {
            let project = project_by_cwd
                .entry(session.cwd.as_str())
                .or_insert_with(|| {
                    ProjectKey::for_path(
                        &LocalOrRemotePath::Local(PathBuf::from(&session.cwd)),
                        ctx,
                    )
                    .map(ProjectId::Key)
                    .unwrap_or(ProjectId::Other)
                })
                .clone();
            if !layout.projects.iter().any(|entry| entry.id == project) {
                layout.projects.push(ProjectEntry {
                    display_name: project.display_name(),
                    id: project.clone(),
                });
            }
            layout.dormant.push((
                project,
                DormantTask {
                    // The scan reads Claude Code's transcript layout, so every
                    // row it produces is a Claude session by construction.
                    agent: CLIAgent::Claude,
                    session_id: session.session_id.clone(),
                    label: dormant_label(
                        session.label.as_deref(),
                        None,
                        CLIAgent::Claude,
                        &session.session_id,
                    ),
                    cwd: session.cwd.clone(),
                    origin: DormantTaskOrigin::Scanned,
                },
            ));
        }
        layout
    }

    /// The dormant tasks bucketed to `selected`, in handle-store order
    /// (most recently seen first). Empty unless built by
    /// [`Self::compute_with_handles`].
    pub fn dormant_tasks_for_project(&self, selected: &ProjectId) -> Vec<&DormantTask> {
        self.dormant
            .iter()
            .filter_map(|(project, task)| (project == selected).then_some(task))
            .collect()
    }

    /// Resolves the project of a single tab from its focused session path.
    pub fn project_of_tab_data(tab: &TabData, ctx: &AppContext) -> ProjectId {
        let path: Option<PathBuf> = tab.pane_group.as_ref(ctx).active_session_path(ctx);
        path.and_then(|path| ProjectKey::for_path(&LocalOrRemotePath::Local(path), ctx))
            .map(ProjectId::Key)
            .unwrap_or(ProjectId::Other)
    }

    /// The distinct projects, in stable order.
    pub fn projects(&self) -> &[ProjectEntry] {
        &self.projects
    }

    /// The project of the tab at `index`, if in range.
    pub fn project_of_tab(&self, index: usize) -> Option<&ProjectId> {
        self.tab_project.get(index)
    }

    /// The pane-group id of the tab at `index`, if in range.
    pub fn pane_group_id_of_tab(&self, index: usize) -> Option<EntityId> {
        self.tab_pane_group_ids.get(index).copied()
    }

    /// Raw `Workspace::tabs` indices belonging to `selected`, in tab order.
    pub fn visible_tab_indices(&self, selected: &ProjectId) -> Vec<usize> {
        self.tab_project
            .iter()
            .enumerate()
            .filter_map(|(index, id)| (id == selected).then_some(index))
            .collect()
    }
}

/// The scanned sessions that earn a row of their own: everything the handle
/// store does not already cover, and nothing that is currently running.
///
/// The filtering happens **here**, before the rail applies its own row cap, so
/// running and already-witnessed sessions cannot consume the cap and hide
/// resumable ones behind it.
///
/// Newest first, by transcript mtime — the only recency signal an unwitnessed
/// session has, since Warp has no `last_seen_at` for one. Handle rows keep
/// their own store order and are not re-sorted into this: the two clocks mean
/// different things (when Warp last saw it, versus when Claude last wrote it)
/// and interleaving them would order rows by neither.
fn unwitnessed_sessions<'a>(
    scanned: &'a [ScannedSession],
    bound_session_ids: &HashSet<&str>,
    live_session_ids: &HashSet<(CLIAgent, String)>,
) -> Vec<&'a ScannedSession> {
    let mut unwitnessed: Vec<&ScannedSession> = scanned
        .iter()
        .filter(|session| {
            !bound_session_ids.contains(session.session_id.as_str())
                && !live_session_ids.contains(&(CLIAgent::Claude, session.session_id.clone()))
        })
        .collect();
    unwitnessed.sort_by(|left, right| right.modified.cmp(&left.modified));
    unwitnessed
}

/// The label for one dormant row.
///
/// The scanned name wins over the cached one even for a witnessed session: the
/// handle's title was cached when Warp last looked, but `/rename` lands in the
/// transcript afterwards, so disk is the fresher of the two. The floor is
/// `Agent · shortid` — never a path, since replacing path-derived labels is
/// the point of the rail.
fn dormant_label(
    scanned_label: Option<&str>,
    cached_title: Option<&str>,
    agent: CLIAgent,
    session_id: &str,
) -> String {
    scanned_label
        .or(cached_title)
        .map(str::to_owned)
        .unwrap_or_else(|| {
            let short: String = session_id.chars().take(8).collect();
            format!("{} · {short}", agent.display_name())
        })
}

/// The index reached by moving forward one step through `indices` from
/// `current`, wrapping at the end. If `current` is not in `indices`, returns
/// the first element (or `current` when empty). Used to cycle next-tab within a
/// project's visible tabs.
pub fn cycle_next(indices: &[usize], current: usize) -> usize {
    if indices.is_empty() {
        return current;
    }
    match indices.iter().position(|&index| index == current) {
        Some(pos) => indices[(pos + 1) % indices.len()],
        None => indices[0],
    }
}

/// The index reached by moving backward one step through `indices` from
/// `current`, wrapping at the start. If `current` is not in `indices`, returns
/// the first element (or `current` when empty).
pub fn cycle_prev(indices: &[usize], current: usize) -> usize {
    if indices.is_empty() {
        return current;
    }
    match indices.iter().position(|&index| index == current) {
        Some(pos) => indices[(pos + indices.len() - 1) % indices.len()],
        None => indices[0],
    }
}

#[cfg(test)]
#[path = "project_layout_tests.rs"]
mod tests;
