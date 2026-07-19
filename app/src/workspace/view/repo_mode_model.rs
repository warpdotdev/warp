//! Registry bridge and selection helpers for repo mode.
//!
//! Selection state lives on [`Workspace`] (`selected_repo_root`). This module
//! owns list/add/remove/select operations against `ProjectManagementModel`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use chrono::NaiveDateTime;
use repo_mode::{canonicalize_repo_path, display_name_for_path, is_dead_path, RepoEntryKind};
use warpui::{AppContext, SingletonEntity, ViewContext};

use super::Workspace;
use crate::features::FeatureFlag;
use crate::pane_group::{NewTerminalOptions, PanesLayout};
use crate::projects::ProjectManagementModel;
use crate::workspace::tab_group::{TabGroup, TabGroupId};
use crate::workspace::WorkspaceRegistry;

/// Snapshot of a registry entry for UI rendering, ordered by recency at launch.
#[derive(Clone, Debug)]
pub struct RepoModeListEntry {
    pub path: PathBuf,
    pub display_name: String,
    pub kind: RepoEntryKind,
    pub is_dead: bool,
    pub last_opened_ts: Option<NaiveDateTime>,
    pub added_ts: NaiveDateTime,
}

impl Workspace {
    /// True when repo mode is compiled in and the runtime flag is on.
    pub(super) fn repo_mode_enabled() -> bool {
        FeatureFlag::RepoMode.is_enabled()
    }

    /// Ordered registry list captured for the section (recency: last_opened then added).
    pub(super) fn repo_mode_entries(&self, ctx: &AppContext) -> Vec<RepoModeListEntry> {
        if !Self::repo_mode_enabled() {
            return Vec::new();
        }
        let mut entries: Vec<RepoModeListEntry> =
            ProjectManagementModel::handle(ctx).read(ctx, |projects, _| {
                projects
                    .all_projects()
                    .map(|project| {
                        let path = PathBuf::from(&project.path);
                        let kind = if path.join(".git").exists() {
                            RepoEntryKind::Repo
                        } else {
                            RepoEntryKind::Folder
                        };
                        RepoModeListEntry {
                            display_name: display_name_for_path(&path),
                            is_dead: is_dead_path(&path),
                            path,
                            kind,
                            last_opened_ts: project.last_opened_ts,
                            added_ts: project.added_ts,
                        }
                    })
                    .collect()
            });
        entries.sort_by(|a, b| {
            b.last_opened_ts
                .cmp(&a.last_opened_ts)
                .then(b.added_ts.cmp(&a.added_ts))
                .then(a.display_name.cmp(&b.display_name))
        });
        entries
    }

    pub(super) fn open_folder_picker_for_repo_mode(&mut self, ctx: &mut ViewContext<Self>) {
        if !Self::repo_mode_enabled() {
            return;
        }
        ctx.open_file_picker(
            move |result, ctx| {
                let Ok(paths) = result else { return };
                let Some(path) = paths.into_iter().next() else {
                    return;
                };
                let Ok(canonical) = canonicalize_repo_path(Path::new(&path)) else {
                    return;
                };
                ProjectManagementModel::handle(ctx).update(ctx, |projects, ctx| {
                    projects.upsert_project(canonical.clone(), ctx);
                });
                // Select the new entry and open its first group tab (F1).
                let window_ids: Vec<_> = ctx.window_ids().collect();
                for window_id in window_ids {
                    if let Some(workspace) = WorkspaceRegistry::as_ref(ctx).get(window_id, ctx) {
                        let path = canonical.clone();
                        workspace.update(ctx, |workspace, ctx| {
                            workspace.select_repo_mode_entry(&path, ctx);
                        });
                        break;
                    }
                }
            },
            warpui::platform::FilePickerConfiguration::new().folders_only(),
        );
    }

    pub(super) fn remove_repo_mode_entry(&mut self, path: &Path, ctx: &mut ViewContext<Self>) {
        if !Self::repo_mode_enabled() {
            return;
        }
        let path_buf = canonicalize_repo_path(path).unwrap_or_else(|_| path.to_path_buf());
        let path_str = path_buf.to_string_lossy().into_owned();

        let group_ids: Vec<TabGroupId> = self
            .tab_groups
            .values()
            .filter(|g| g.repo_root.as_deref() == Some(path_str.as_str()))
            .map(|g| g.id)
            .collect();
        for group_id in group_ids {
            self.ungroup_tabs(group_id, ctx);
        }

        if self.selected_repo_root.as_deref() == Some(path_str.as_str()) {
            self.selected_repo_root = None;
        }

        ProjectManagementModel::handle(ctx).update(ctx, |projects, ctx| {
            projects.remove_project(path_buf, ctx);
        });
        ctx.notify();
    }

    pub(super) fn select_repo_mode_all(&mut self, ctx: &mut ViewContext<Self>) {
        if !Self::repo_mode_enabled() {
            return;
        }
        self.selected_repo_root = None;
        ctx.notify();
    }

    pub(super) fn select_repo_mode_entry(&mut self, path: &Path, ctx: &mut ViewContext<Self>) {
        if !Self::repo_mode_enabled() {
            return;
        }
        let path_buf = canonicalize_repo_path(path).unwrap_or_else(|_| path.to_path_buf());
        let path_str = path_buf.to_string_lossy().into_owned();
        self.selected_repo_root = Some(path_str.clone());

        // Do not upsert on select — that would bump last_opened_ts and reorder
        // the list mid-session (R3 launch-fixed order).

        let has_group = self
            .tab_groups
            .values()
            .any(|g| g.repo_root.as_deref() == Some(path_str.as_str()));
        if !has_group {
            self.create_repo_mode_group_with_tab(&path_buf, ctx);
        } else {
            self.activate_repo_mode_group_mru(&path_buf, ctx);
        }
        ctx.notify();
    }

    pub(super) fn create_repo_mode_group_with_tab(
        &mut self,
        path: &Path,
        ctx: &mut ViewContext<Self>,
    ) {
        let mut group = TabGroup::new();
        group.repo_root = Some(path.to_string_lossy().into_owned());
        let group_id = group.id;
        self.tab_groups.insert(group_id, group);

        self.add_tab_with_pane_layout(
            PanesLayout::SingleTerminal(Box::new(NewTerminalOptions {
                initial_directory: Some(path.to_path_buf()),
                hide_homepage: true,
                ..Default::default()
            })),
            Arc::new(HashMap::new()),
            None,
            ctx,
        );
        if let Some(tab) = self.tabs.get_mut(self.active_tab_index) {
            tab.group_id = Some(group_id);
        }
    }

    fn activate_repo_mode_group_mru(&mut self, path: &Path, ctx: &mut ViewContext<Self>) {
        let path_str = path.to_string_lossy();
        let Some(group_id) = self
            .tab_groups
            .values()
            .find(|g| g.repo_root.as_deref() == Some(path_str.as_ref()))
            .map(|g| g.id)
        else {
            return;
        };
        let already_active = self
            .tabs
            .get(self.active_tab_index)
            .is_some_and(|t| t.group_id == Some(group_id));
        if already_active {
            return;
        }
        if let Some(index) = self.tabs.iter().position(|t| t.group_id == Some(group_id)) {
            self.activate_tab(index, ctx);
        }
    }

    /// Bound tab-group id for the current selection, if any.
    pub(super) fn selected_repo_mode_group_id(&self) -> Option<TabGroupId> {
        let selected = self.selected_repo_root.as_deref()?;
        self.tab_groups
            .values()
            .find(|g| g.repo_root.as_deref() == Some(selected))
            .map(|g| g.id)
    }

    /// Tabs visible under the current repo-mode selection (all tabs when "All"/flag off).
    /// When a repo is selected but its group is missing, returns an empty list (R10) —
    /// never ungrouped tabs.
    pub(super) fn repo_mode_visible_tab_indices(&self) -> Option<Vec<usize>> {
        if !Self::repo_mode_enabled() {
            return None;
        }
        let Some(selected) = self.selected_repo_root.as_deref() else {
            return None;
        };
        let Some(group_id) = self
            .tab_groups
            .values()
            .find(|g| g.repo_root.as_deref() == Some(selected))
            .map(|g| g.id)
        else {
            return Some(Vec::new());
        };
        Some(
            self.tabs
                .iter()
                .enumerate()
                .filter(|(_, tab)| tab.group_id == Some(group_id))
                .map(|(i, _)| i)
                .collect(),
        )
    }
}
