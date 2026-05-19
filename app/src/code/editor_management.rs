use std::{
    collections::{hash_map::Entry, HashMap},
    path::PathBuf,
};

use super::buffer_location::LocalOrRemotePath;
use crate::ai::skills::SkillOpenOrigin;
use ai::skills::SkillReference;
use serde::{Deserialize, Serialize};
use warp_util::path::LineAndColumnArg;
use warpui::{AppContext, Entity, EntityId, ModelContext, SingletonEntity, ViewHandle, WindowId};

use crate::{
    ai::agent::AIAgentActionId,
    code_review::code_review_view::CodeReviewView,
    pane_group::{PaneGroup, PaneId},
    workspace::PaneViewLocator,
};

use super::view::CodeView;

pub struct CodeEditorSummary<'a> {
    pub unsaved_changes: Vec<&'a CodeEditorStatus>,
}

impl<'a> CodeEditorSummary<'a> {
    /// Create a summary from the currently open Code Editors.
    pub fn new(editors: &'a [CodeEditorStatus]) -> Self {
        let unsaved_changes = editors
            .iter()
            .filter(|editor| editor.unsaved_changes)
            .collect();

        Self { unsaved_changes }
    }
}

#[derive(Copy, Clone)]
pub struct CodeEditorStatus {
    unsaved_changes: bool,
}

impl CodeEditorStatus {
    pub fn new(unsaved_changes: bool) -> Self {
        Self { unsaved_changes }
    }

    /// Fetches all code editors open in the App.
    pub fn all_editors(app: &AppContext) -> impl Iterator<Item = Self> + '_ {
        app.window_ids()
            .flat_map(move |window_id| Self::editors_in_window(window_id, app))
    }

    /// Fetches all code editors in a given window.
    pub fn editors_in_window(
        window_id: WindowId,
        app: &AppContext,
    ) -> impl Iterator<Item = Self> + '_ {
        app.views_of_type::<CodeView>(window_id)
            .into_iter()
            .flat_map(move |editors| {
                editors
                    .into_iter()
                    .map(move |editor| Self::editor_status(&editor, app))
            })
    }

    /// Fetches all code editors in a given tab.
    pub fn editors_in_tab<'a>(
        tab: &ViewHandle<PaneGroup>,
        app: &'a AppContext,
    ) -> impl Iterator<Item = Self> + 'a {
        tab.as_ref(app)
            .code_panes(app)
            .map(move |(_, editor)| Self::editor_status(&editor, app))
    }

    pub fn editor_status(editor: &ViewHandle<CodeView>, app: &AppContext) -> Self {
        editor.read(app, |editor_view, ctx| Self {
            unsaved_changes: editor_view.contains_unsaved_changes(ctx),
        })
    }

    pub fn status_for_code_review(review: &ViewHandle<CodeReviewView>, app: &AppContext) -> Self {
        review.read(app, |review_view, ctx| Self {
            unsaved_changes: review_view.has_unsaved_changes(ctx),
        })
    }

    /// Fetches all code review views in a given window (including panel views).
    pub fn code_review_views_in_window(
        window_id: WindowId,
        app: &AppContext,
    ) -> impl Iterator<Item = Self> + '_ {
        app.views_of_type::<CodeReviewView>(window_id)
            .into_iter()
            .flat_map(move |editors| {
                editors
                    .into_iter()
                    .map(move |editor| Self::status_for_code_review(&editor, app))
            })
    }
}

#[derive(Debug, Hash, Eq, PartialEq, Clone, Serialize, Deserialize)]
pub enum CodeSource {
    /// A new code pane not attached to an existing file.
    New {
        /// When the new file is saved, open the file picker to this directory.
        default_directory: Option<PathBuf>,
    },
    /// Opened from file links.
    Link {
        path: PathBuf,
        range_start: Option<LineAndColumnArg>,
        range_end: Option<LineAndColumnArg>,
    },
    /// Opened from an active AI agent conversation.
    AIAction { id: AIAgentActionId },
    /// Opened from project rules (WARP.md) file.
    ProjectRules { path: PathBuf },
    /// Opened from file tree (local or remote).
    FileTree { location: LocalOrRemotePath },
    /// Opened from macOS Finder via "Open With".
    Finder { path: PathBuf },
    /// Opened from a skill.
    Skill {
        reference: SkillReference,
        path: PathBuf,
        origin: SkillOpenOrigin,
    },
}

impl CodeSource {
    pub fn default_directory(&self) -> Option<&PathBuf> {
        match self {
            Self::New {
                default_directory, ..
            } => default_directory.as_ref(),
            Self::Link { .. }
            | Self::AIAction { .. }
            | Self::ProjectRules { .. }
            | Self::FileTree { .. }
            | Self::Finder { .. }
            | Self::Skill { .. } => None,
        }
    }

    pub fn path(&self) -> Option<PathBuf> {
        match self {
            Self::New { .. } | Self::AIAction { .. } => None,
            Self::FileTree { location, .. } => match location {
                LocalOrRemotePath::Local(path) => Some(path.clone()),
                LocalOrRemotePath::Remote(_) => None,
            },
            Self::Link { path, .. }
            | Self::ProjectRules { path }
            | Self::Finder { path }
            | Self::Skill { path, .. } => Some(path.clone()),
        }
    }

    /// Returns the `LocalOrRemotePath` for file tree sources.
    pub fn file_location(&self) -> Option<&LocalOrRemotePath> {
        match self {
            Self::FileTree { location } => Some(location),
            _ => None,
        }
    }

    /// Returns the `LocalOrRemotePath` for any source that has a backing file.
    ///
    /// Unlike `path()` (which only returns local paths) and `file_location()`
    /// (which only covers `FileTree`), this covers every variant that maps to
    /// a file — local or remote.
    pub fn location(&self) -> Option<LocalOrRemotePath> {
        match self {
            Self::New { .. } | Self::AIAction { .. } => None,
            Self::FileTree { location } => Some(location.clone()),
            Self::Link { path, .. }
            | Self::ProjectRules { path }
            | Self::Finder { path }
            | Self::Skill { path, .. } => Some(LocalOrRemotePath::Local(path.clone())),
        }
    }

    /// Returns true if this is a bundled skill that should be read-only.
    pub fn is_bundled_skill(&self) -> bool {
        matches!(
            self,
            Self::Skill {
                reference: SkillReference::BundledSkillId(_),
                ..
            }
        )
    }

    pub fn omit_line_col(&self) -> CodeSource {
        if let CodeSource::Link { path, .. } = self {
            CodeSource::Link {
                path: path.clone(),
                range_start: None,
                range_end: None,
            }
        } else {
            self.clone()
        }
    }

    /// Returns the variant name as a string for telemetry purposes.
    pub fn telemetry_source_name(&self) -> &'static str {
        match self {
            Self::New { .. } => "new",
            Self::Link { .. } => "link",
            Self::AIAction { .. } => "ai_action",
            Self::ProjectRules { .. } => "project_rules",
            Self::FileTree {
                location: LocalOrRemotePath::Remote(_),
            } => "remote_file_tree",
            Self::FileTree { .. } => "file_tree",
            Self::Finder { .. } => "finder",
            Self::Skill { .. } => "skill",
        }
    }

    /// Returns `true` if this source should be restored across app restarts.
    ///
    /// `AIAction` is ephemeral (tied to a live conversation) and should not
    /// be restored.
    pub fn is_restorable(&self) -> bool {
        !matches!(
            self,
            Self::AIAction { .. }
                | Self::FileTree {
                    location: LocalOrRemotePath::Remote(_),
                }
        )
    }
}

struct CodePaneData {
    #[allow(unused)]
    window_id: WindowId,
    #[allow(unused)]
    locator: PaneViewLocator,
}

// Allow dead_code here for wasm compilation
#[allow(dead_code)]
pub enum CodeManagerEvent {
    EditCompleted { action_id: AIAgentActionId },
}

/// Singleton model for managing the state of open code panes. It is responsible for
/// 1) Allow caller to find an open code pane if exists.
/// 2) Allow other sources to listen for events emitted when code pane is closed.
#[derive(Default)]
pub struct CodeManager {
    source_to_pane_data: HashMap<CodeSource, CodePaneData>,
}

impl CodeManager {
    /// Register a new pane in the code manager.
    pub fn register_pane(
        &mut self,
        pane_group_id: EntityId,
        window_id: WindowId,
        pane_id: PaneId,
        source: CodeSource,
    ) {
        let entry = self.source_to_pane_data.entry(source.omit_line_col());
        if let Entry::Vacant(entry) = entry {
            entry.insert(CodePaneData {
                window_id,
                locator: PaneViewLocator {
                    pane_group_id,
                    pane_id,
                },
            });
        } else {
            log::warn!("Ignoring duplicate code pane registration");
        }
    }

    /// De-register an open code pane when it's removed from a pane group.
    pub fn deregister_pane(&mut self, source: &CodeSource) {
        self.source_to_pane_data.remove(&source.omit_line_col());
    }

    /// Returns the locator for a code pane that already has the given `LocalOrRemotePath`
    /// open in the given pane group. Works for both local and remote files.
    pub fn get_locator_for_location_in_tab(
        &self,
        pane_group_id: EntityId,
        location: &LocalOrRemotePath,
    ) -> Option<PaneViewLocator> {
        self.source_to_pane_data
            .iter()
            .find(|(source, data)| {
                data.locator.pane_group_id == pane_group_id
                    && source.location().as_ref() == Some(location)
            })
            .map(|(_, data)| data.locator)
    }

    // Allow dead_code here for wasm compilation
    #[allow(dead_code)]
    pub fn complete_pending_diffs(&mut self, source: CodeSource, ctx: &mut ModelContext<Self>) {
        if !self.source_to_pane_data.contains_key(&source) {
            log::warn!("Trying to complete an edit on a source that doesn't exist");
        }

        let CodeSource::AIAction { id } = source else {
            return;
        };

        ctx.emit(CodeManagerEvent::EditCompleted { action_id: id })
    }
}

impl Entity for CodeManager {
    type Event = CodeManagerEvent;
}

impl SingletonEntity for CodeManager {}
