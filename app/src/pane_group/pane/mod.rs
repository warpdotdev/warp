//! The pane module contains the interfaces that must be implemented and followed by any concrete pane type.
//!
//! Each kind of pane involves both a [`BackingView`] implementation and a [`PaneContent`] implementation.
//! APIs for managing a pane as part of a pane group, like getting its title or moving it from one
//! tab to another, belong in [`PaneContent`]. APIs for rendering or interacting with an individual pane,
//! like building its context menu, belong in [`BackingView`].
//!
//! The [`PaneContent`] interface requires implementers to maintain a [`PaneId`] for their pane.
//! The [`PaneId`] must be created via a [`PaneView<BackingView>`]. The [`PaneId`] is consequently
//! used to render a [`PaneView`] which internally renders the pane, including the [`BackingView`].
pub(super) mod ai_document_pane;
pub(super) mod ai_fact_pane;
pub(super) mod code_diff_pane;
pub(super) mod code_diff_pane_model;
pub(super) mod code_pane;
pub(super) mod env_var_collection_pane;
pub(crate) mod environment_management_pane;
pub(super) mod execution_profile_editor_pane;
pub(super) mod file_pane;
pub(super) mod get_started_pane;
pub(super) mod get_started_view;
#[cfg(not(target_family = "wasm"))]
pub(super) mod local_harness_launch;
pub(super) mod network_log_pane;
pub(super) mod notebook_pane;
pub(super) mod settings_pane;
pub(super) mod terminal_pane;
pub mod view;
pub(super) mod welcome_pane;
pub(crate) mod welcome_view;
pub mod workflow_pane;

use std::{any::Any, fmt::Display};

use crate::pane_group::focus_state::PaneFocusHandle;
use crate::pane_group::pane::get_started_view::GetStartedView;
use crate::view_components::action_button::ActionButton;
use crate::{
    ai::execution_profiles::editor::ExecutionProfileEditorView,
    ai::{
        ai_document_view::AIDocumentView, blocklist::inline_action::code_diff_view::CodeDiffView,
        facts::AIFactView,
    },
    code::view::CodeView,
    drive::sharing::ShareableObject,
    env_vars::view::env_var_collection::EnvVarCollectionView,
    menu::MenuItem,
    notebooks::{file::FileNotebookView, notebook::NotebookView},
    server::network_log_view::NetworkLogView,
    server::telemetry::SharingDialogSource,
    settings::PaneSettings,
    settings_view::{environments_page::EnvironmentsPageView, SettingsView},
    terminal::{available_shells::AvailableShell, TerminalView},
    workflows::workflow_view::WorkflowView,
};
use serde::{Deserialize, Serialize};
use url::Url;
use warp_util::remote_path::RemotePath;
use warpui::{
    elements::{DispatchEventResult, EventHandler, MouseInBehavior},
    presenter::ChildView,
    Action, AppContext, Element, Entity, EntityId, ModelContext, ModelHandle, SingletonEntity,
    View, ViewContext, ViewHandle, WeakModelHandle,
};

pub use self::view::PaneHeaderAction;
pub use self::view::PaneHeaderCustomAction;
pub use self::view::PaneView;
pub use self::view::PaneViewEvent;

use welcome_view::WelcomeView;

use super::{ActivationReason, LeafContents, PaneGroup, PaneGroupAction};

pub(super) fn init(app: &mut AppContext) {
    self::view::init(app);
    welcome_view::init(app);
    get_started_view::init(app);
}

/// The opaque identifier for an arbitrary pane. Consumers
/// should not be concerned with the internal IDs that are used;
/// instead, consumers should use the [`PaneGroup`] APIs to get
/// panes of concrete types from a pane ID.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct PaneId(IPaneId);

#[derive(Debug, Clone, Copy)]
pub enum ActionOrigin {
    // If a drag/drop action started from an editor tab, we pass its index forward.
    EditorTab(usize),
    Pane,
}

/// A [`PaneId`] that is known to belong to a terminal pane.
/// Generally, prefer [`PaneId`], except for logic/features that will only
/// ever apply to terminal sessions (like synced inputs and the block-sharing modal).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct TerminalPaneId(EntityId);

impl From<TerminalPaneId> for PaneId {
    fn from(terminal_pane: TerminalPaneId) -> Self {
        PaneId(IPaneId {
            pane_type: IPaneType::Terminal,
            pane_view_id: terminal_pane.0,
        })
    }
}

impl TerminalPaneId {
    /// Creates a [`TerminalPaneId`] for a dummy terminal pane.
    #[cfg(test)]
    pub fn dummy_terminal_pane_id() -> Self {
        Self(EntityId::new())
    }
}

/// An internal representation of a pane ID. Specifically, we don't want to allow
/// consumers to derive the underlying view ID from a pane ID. Instead, consumers
/// should use the relevant [`crate::PaneGroup`] APIs to access pane content (which
/// can provide the underlying view).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
struct IPaneId {
    /// The type of pane. Needs to match the BackingView.
    pane_type: IPaneType,

    /// The entity id of the PaneView<BackingView>.
    pane_view_id: EntityId,
}

impl Display for IPaneId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Pane {} ({})", self.pane_type, self.pane_view_id)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub(crate) enum IPaneType {
    Terminal,
    Notebook,
    File,
    Code,
    CodeDiff,
    EnvVarCollection,
    EnvironmentManagement,
    Workflow,
    Settings,
    AIFact,
    AIDocument,
    ExecutionProfileEditor,
    GetStarted,
    NetworkLog,
    Welcome,
    DeferredPlaceholder,
    /// A pane type only for tests.
    #[cfg(test)]
    Dummy,
}

impl Display for IPaneType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            IPaneType::Terminal => write!(f, "Terminal"),
            IPaneType::Notebook => write!(f, "Notebook"),
            IPaneType::File => write!(f, "File"),
            IPaneType::Code => write!(f, "Code"),
            IPaneType::CodeDiff => write!(f, "Code Diff"),
            IPaneType::EnvVarCollection => write!(f, "Environment Variable Collection"),
            IPaneType::EnvironmentManagement => write!(f, "Environment Management"),
            IPaneType::Workflow => write!(f, "Workflow"),
            IPaneType::Settings => write!(f, "Settings"),
            IPaneType::AIFact => write!(f, "AI Fact"),
            IPaneType::AIDocument => write!(f, "AI Document"),
            IPaneType::ExecutionProfileEditor => write!(f, "Execution Profile Editor"),
            IPaneType::GetStarted => write!(f, "GetStarted"),
            IPaneType::NetworkLog => write!(f, "Network Log"),
            IPaneType::Welcome => write!(f, "Welcome"),
            IPaneType::DeferredPlaceholder => write!(f, "Placeholder"),
            #[cfg(test)]
            IPaneType::Dummy => write!(f, "Dummy"),
        }
    }
}

impl PaneId {
    fn new<T: BackingView>(pane_type: IPaneType, pane_view: &ViewHandle<PaneView<T>>) -> Self {
        Self(IPaneId {
            pane_type,
            pane_view_id: pane_view.id(),
        })
    }

    fn new_from_ctx<T: BackingView>(pane_type: IPaneType, ctx: &ViewContext<PaneView<T>>) -> Self {
        Self(IPaneId {
            pane_type,
            pane_view_id: ctx.view_id(),
        })
    }

    /// Creates a [`PaneId`] from a [`ViewContext<PaneView<TerminalView>>`]
    pub fn from_terminal_pane_ctx(ctx: &ViewContext<terminal_pane::TerminalPaneView>) -> Self {
        Self::new_from_ctx(IPaneType::Terminal, ctx)
    }

    /// Creates a [`PaneId`] from a [`ViewContext<PaneView<FileNotebookView>>`]
    pub fn from_file_pane_ctx(ctx: &ViewContext<PaneView<FileNotebookView>>) -> Self {
        Self::new_from_ctx(IPaneType::File, ctx)
    }

    /// Creates a [`PaneId`] from a [`ViewContext<PaneView<NotebookView>>`]
    pub fn from_notebook_pane_ctx(ctx: &ViewContext<PaneView<NotebookView>>) -> Self {
        Self::new_from_ctx(IPaneType::Notebook, ctx)
    }

    /// Creates a [`PaneId`] from a [`ViewContext<PaneView<EnvVarCollectionView>>`]
    pub fn from_env_var_collection_pane_ctx(
        ctx: &ViewContext<PaneView<EnvVarCollectionView>>,
    ) -> Self {
        Self::new_from_ctx(IPaneType::EnvVarCollection, ctx)
    }

    /// Creates a [`PaneId`] from a [`ViewContext<PaneView<EnvironmentsPageView>>`]
    pub fn from_environment_management_pane_ctx(
        ctx: &ViewContext<PaneView<EnvironmentsPageView>>,
    ) -> Self {
        Self::new_from_ctx(IPaneType::EnvironmentManagement, ctx)
    }

    /// Creates a [`PaneId`] from a [`ViewContext<PaneView<WorkflowView>>`]
    pub fn from_workflow_pane_ctx(ctx: &ViewContext<PaneView<WorkflowView>>) -> Self {
        Self::new_from_ctx(IPaneType::Workflow, ctx)
    }

    /// Creates a [`PaneId`] from a [`ViewContext<PaneView<TextView>>`]
    pub fn from_code_pane_ctx(ctx: &ViewContext<PaneView<CodeView>>) -> Self {
        Self::new_from_ctx(IPaneType::Code, ctx)
    }

    /// Creates a [`PaneId`] from a [`ViewContext<PaneView<CodeDiffView>>`]
    pub fn from_code_diff_pane_ctx(ctx: &ViewContext<PaneView<CodeDiffView>>) -> Self {
        Self::new_from_ctx(IPaneType::CodeDiff, ctx)
    }

    /// Creates a [`PaneId`] from a [`ViewContext<PaneView<SettingsView>>`]
    pub fn from_settings_pane_ctx(ctx: &ViewContext<PaneView<SettingsView>>) -> Self {
        Self::new_from_ctx(IPaneType::Settings, ctx)
    }

    /// Creates a [`PaneId`] from a [`ViewContext<PaneView<AIFactView>>`]
    pub fn from_ai_fact_pane_ctx(ctx: &ViewContext<PaneView<AIFactView>>) -> Self {
        Self::new_from_ctx(IPaneType::AIFact, ctx)
    }

    /// Creates a [`PaneId`] from a [`ViewContext<PaneView<AIDocumentView>>`]
    pub fn from_ai_document_pane_ctx(ctx: &ViewContext<PaneView<AIDocumentView>>) -> Self {
        Self::new_from_ctx(IPaneType::AIDocument, ctx)
    }

    /// Creates a [`PaneId`] from a [`ViewContext<PaneView<ExecutionProfileEditorView>>`]
    pub fn from_execution_profile_editor_pane_ctx(
        ctx: &ViewContext<PaneView<ExecutionProfileEditorView>>,
    ) -> Self {
        Self::new_from_ctx(IPaneType::ExecutionProfileEditor, ctx)
    }

    pub fn from_welcome_pane_ctx(ctx: &ViewContext<PaneView<WelcomeView>>) -> Self {
        Self::new_from_ctx(IPaneType::Welcome, ctx)
    }

    pub fn from_get_started_pane_ctx(ctx: &ViewContext<PaneView<GetStartedView>>) -> Self {
        Self::new_from_ctx(IPaneType::GetStarted, ctx)
    }

    /// Creates a [`PaneId`] from a [`ViewContext<PaneView<NetworkLogView>>`].
    pub fn from_network_log_pane_ctx(ctx: &ViewContext<PaneView<NetworkLogView>>) -> Self {
        Self::new_from_ctx(IPaneType::NetworkLog, ctx)
    }

    /// Creates a [`PaneId`] from a [`PaneView<TerminalView>`] entity ID.
    pub fn from_terminal_pane_view(
        terminal_pane_view: &ViewHandle<terminal_pane::TerminalPaneView>,
    ) -> Self {
        Self::new(IPaneType::Terminal, terminal_pane_view)
    }

    /// Creates a [`PaneId`] from a [`PaneView<NotebookView>`] entity ID.
    pub fn from_notebook_pane_view(
        notebook_pane_view: &ViewHandle<PaneView<NotebookView>>,
    ) -> Self {
        Self::new(IPaneType::Notebook, notebook_pane_view)
    }

    /// Creates a [`PaneId`] from a [`PaneView<FileNotebookView>`] entity ID.
    pub fn from_file_pane_view(file_pane_view: &ViewHandle<PaneView<FileNotebookView>>) -> Self {
        Self::new(IPaneType::File, file_pane_view)
    }

    /// Creates a [`PaneId`] from a [`PaneView<TextView>`] entity ID.
    pub fn from_code_pane_view(code_pane_view: &ViewHandle<PaneView<CodeView>>) -> Self {
        Self::new(IPaneType::Code, code_pane_view)
    }

    /// Creates a [`PaneId`] from a [`PaneView<CodeDiffView>`] entity ID.
    pub fn from_code_diff_pane_view(
        code_diff_pane_view: &ViewHandle<PaneView<CodeDiffView>>,
    ) -> Self {
        Self::new(IPaneType::CodeDiff, code_diff_pane_view)
    }

    /// Creates a [`PaneId`] from a [`PaneView<EnvVarCollection>`] entity ID.
    pub fn from_env_var_collection_view(
        env_var_collection_view: &ViewHandle<PaneView<EnvVarCollectionView>>,
    ) -> Self {
        Self::new(IPaneType::EnvVarCollection, env_var_collection_view)
    }

    /// Creates a [`PaneId`] from a [`PaneView<EnvironmentsPageView>`] entity ID.
    pub fn from_environment_management_pane_view(
        environment_management_pane_view: &ViewHandle<PaneView<EnvironmentsPageView>>,
    ) -> Self {
        Self::new(
            IPaneType::EnvironmentManagement,
            environment_management_pane_view,
        )
    }

    /// Creates a [`PaneId`] from a [`PaneView<WorkflowView>`] entity ID.
    pub fn from_workflow_pane_view(
        workflow_pane_view: &ViewHandle<PaneView<WorkflowView>>,
    ) -> Self {
        Self::new(IPaneType::Workflow, workflow_pane_view)
    }

    /// Creates a [`PaneId`] from a [`PaneView<SettingsView>`] entity ID.
    pub fn from_settings_pane_view(
        settings_pane_view: &ViewHandle<PaneView<SettingsView>>,
    ) -> Self {
        Self::new(IPaneType::Settings, settings_pane_view)
    }

    /// Creates a [`PaneId`] from a [`PaneView<AIFactView>`] entity ID.
    pub fn from_ai_fact_pane_view(ai_fact_pane_view: &ViewHandle<PaneView<AIFactView>>) -> Self {
        Self::new(IPaneType::AIFact, ai_fact_pane_view)
    }

    /// Creates a [`PaneId`] from a [`PaneView<AIDocumentView>`] entity ID.
    pub fn from_ai_document_pane_view(
        ai_document_pane_view: &ViewHandle<PaneView<AIDocumentView>>,
    ) -> Self {
        Self::new(IPaneType::AIDocument, ai_document_pane_view)
    }

    /// Creates a [`PaneId`] from a [`PaneView<ExecutionProfileEditorView>`] entity ID.
    pub fn from_execution_profile_editor_pane_view(
        execution_profile_editor_pane_view: &ViewHandle<PaneView<ExecutionProfileEditorView>>,
    ) -> Self {
        Self::new(
            IPaneType::ExecutionProfileEditor,
            execution_profile_editor_pane_view,
        )
    }

    pub fn from_get_started_pane_view(
        get_started_pane_view: &ViewHandle<PaneView<GetStartedView>>,
    ) -> Self {
        Self::new(IPaneType::GetStarted, get_started_pane_view)
    }

    pub fn from_welcome_pane_view(welcome_pane_view: &ViewHandle<PaneView<WelcomeView>>) -> Self {
        Self::new(IPaneType::Welcome, welcome_pane_view)
    }

    /// Creates a [`PaneId`] from a [`PaneView<NetworkLogView>`] entity ID.
    pub fn from_network_log_pane_view(
        network_log_pane_view: &ViewHandle<PaneView<NetworkLogView>>,
    ) -> Self {
        Self::new(IPaneType::NetworkLog, network_log_pane_view)
    }

    #[cfg_attr(not(feature = "local_fs"), allow(dead_code))]
    pub(super) fn deferred_placeholder_pane_id() -> Self {
        Self(IPaneId {
            pane_type: IPaneType::DeferredPlaceholder,
            pane_view_id: warpui::EntityId::new(),
        })
    }

    /// Creates a [`PaneId`] for a dummy pane.
    #[cfg(test)]
    pub fn dummy_pane_id() -> Self {
        Self(IPaneId {
            pane_type: IPaneType::Dummy,
            pane_view_id: warpui::EntityId::new(),
        })
    }

    /// Returns a [`TerminalPaneId`] for the pane, if this is a terminal pane ID.
    pub fn as_terminal_pane_id(&self) -> Option<TerminalPaneId> {
        if matches!(self.0.pane_type, IPaneType::Terminal) {
            Some(TerminalPaneId(self.0.pane_view_id))
        } else {
            None
        }
    }

    pub(crate) fn pane_type(&self) -> IPaneType {
        self.0.pane_type
    }

    pub(crate) fn creation_order_id(&self) -> EntityId {
        self.0.pane_view_id
    }

    pub fn is_terminal_pane(&self) -> bool {
        matches!(self.0.pane_type, IPaneType::Terminal)
    }

    pub fn is_notebook_pane(&self) -> bool {
        matches!(self.0.pane_type, IPaneType::Notebook)
    }

    pub fn is_code_pane(&self) -> bool {
        matches!(self.0.pane_type, IPaneType::Code)
    }

    pub fn is_file_pane(&self) -> bool {
        matches!(self.0.pane_type, IPaneType::File)
    }

    pub fn is_code_diff_pane(&self) -> bool {
        matches!(self.0.pane_type, IPaneType::CodeDiff)
    }

    pub fn is_environment_management_pane(&self) -> bool {
        matches!(self.0.pane_type, IPaneType::EnvironmentManagement)
    }

    /// Returns true if this pane contains a Warp Drive object (notebook, workflow, etc.).
    pub fn is_warp_drive_object_pane(&self) -> bool {
        matches!(
            self.0.pane_type,
            IPaneType::Notebook
                | IPaneType::Workflow
                | IPaneType::EnvVarCollection
                | IPaneType::AIFact
        )
    }

    /// Renders the child view backing this pane.
    pub fn render(self, app: &AppContext) -> Box<dyn Element> {
        let mut element = match self.0.pane_type {
            IPaneType::Terminal => {
                ChildView::<PaneView<TerminalView>>::with_id(self.0.pane_view_id).finish()
            }
            IPaneType::Notebook => {
                ChildView::<PaneView<NotebookView>>::with_id(self.0.pane_view_id).finish()
            }
            IPaneType::File => {
                ChildView::<PaneView<FileNotebookView>>::with_id(self.0.pane_view_id).finish()
            }
            IPaneType::Code => {
                ChildView::<PaneView<CodeView>>::with_id(self.0.pane_view_id).finish()
            }
            IPaneType::CodeDiff => {
                ChildView::<PaneView<CodeDiffView>>::with_id(self.0.pane_view_id).finish()
            }
            IPaneType::EnvVarCollection => {
                ChildView::<PaneView<EnvVarCollectionView>>::with_id(self.0.pane_view_id).finish()
            }
            IPaneType::EnvironmentManagement => {
                ChildView::<PaneView<EnvironmentsPageView>>::with_id(self.0.pane_view_id).finish()
            }
            IPaneType::Workflow => {
                ChildView::<PaneView<WorkflowView>>::with_id(self.0.pane_view_id).finish()
            }
            IPaneType::Settings => {
                ChildView::<PaneView<SettingsView>>::with_id(self.0.pane_view_id).finish()
            }
            IPaneType::AIFact => {
                ChildView::<PaneView<AIFactView>>::with_id(self.0.pane_view_id).finish()
            }
            IPaneType::AIDocument => {
                ChildView::<PaneView<AIDocumentView>>::with_id(self.0.pane_view_id).finish()
            }
            IPaneType::ExecutionProfileEditor => {
                ChildView::<PaneView<ExecutionProfileEditorView>>::with_id(self.0.pane_view_id)
                    .finish()
            }
            IPaneType::GetStarted => {
                ChildView::<PaneView<GetStartedView>>::with_id(self.0.pane_view_id).finish()
            }
            IPaneType::NetworkLog => {
                ChildView::<PaneView<NetworkLogView>>::with_id(self.0.pane_view_id).finish()
            }
            IPaneType::Welcome => {
                ChildView::<PaneView<WelcomeView>>::with_id(self.0.pane_view_id).finish()
            }
            IPaneType::DeferredPlaceholder => warpui::elements::Empty::new().finish(),
            #[cfg(test)]
            IPaneType::Dummy => warpui::elements::Empty::new().finish(),
        };
        if *PaneSettings::as_ref(app).focus_panes_on_hover {
            element = EventHandler::new(element)
                .on_mouse_in(
                    move |ctx, _, _| {
                        ctx.dispatch_typed_action(PaneGroupAction::Activate(
                            self,
                            ActivationReason::Hover,
                        ));
                        DispatchEventResult::PropagateToParent
                    },
                    Some(MouseInBehavior {
                        // Don't fire on synthetic events because we don't want to steal focus
                        // when a user creates a new pane.
                        fire_on_synthetic_events: false,

                        // Don't fire when covered because we don't want to steal focus when
                        // modals are open on top of panes.
                        fire_when_covered: false,
                    }),
                )
                .finish();
        }
        element
    }

    pub fn position_id(&self) -> String {
        self.to_string()
    }
}

impl Display for PaneId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Pane {}", self.0)
    }
}

#[derive(Debug, Clone, Copy)]
pub enum DetachType {
    // Pane has been permanently closed and should have resources cleaned up.
    Closed,

    // Pane has been temporarily hidden for close.
    HiddenForClose,

    // Pane detached during a move.
    Moved,
}

pub enum ShareableLink {
    /// The base app url should be used for the browser url bar
    Base,
    /// The url for the active pane to use for the browser url bar
    Pane { url: Url },
}

#[derive(Debug)]
pub enum ShareableLinkError {
    /// An expected error occurred when attempting to get the shareable link of the active pane.
    /// For example the pane is not yet in a state where it has a shareable link but will soon.
    Expected,
    /// An unexpected error while trying to get the shareable link of the active pane.
    Unexpected(String),
}

/// The contents of a leaf pane.
///
/// The [`PaneData`] tree references panes by their [`PaneId`], while the [`PaneGroup`] view owns
/// all their contents through [`PaneContent`].
///
/// See [`BackingView`] for
pub trait PaneContent: 'static {
    /// The corresponding identifier for this pane.
    fn id(&self) -> PaneId;

    /// Pre-attachment hook that allows panes to perform some work before attachment.
    /// Returns true if the pane can be attached normally, false if attachment should be prevented.
    fn pre_attach(&self, _group: &PaneGroup, _ctx: &mut ViewContext<PaneGroup>) -> bool {
        // Default implementation allows all panes to attach
        true
    }

    /// Attempts to attach this pane to a group, calling pre_attach first.
    /// Returns true if attachment succeeded, false if pre_attach prevented it.
    fn try_attach(
        &self,
        group: &PaneGroup,
        focus_handle: PaneFocusHandle,
        ctx: &mut ViewContext<PaneGroup>,
    ) -> bool {
        if self.pre_attach(group, ctx) {
            self.attach(group, focus_handle, ctx);
            true
        } else {
            false
        }
    }

    /// Callback for when this leaf pane is added to a pane group.
    ///
    /// This is called after the pane is added to the group's set of leaf panes, but before the
    /// new pane is focused.
    fn attach(
        &self,
        group: &PaneGroup,
        focus_handle: PaneFocusHandle,
        ctx: &mut ViewContext<PaneGroup>,
    );

    /// Callback for when this leaf pane is removed from a pane group.
    ///
    /// This is called when:
    /// - The pane is about to be closed
    /// - The pane group is closed, but may be restored
    /// - The pane is being moved to another tab, or upgraded to its own tab
    fn detach(&self, group: &PaneGroup, detach_type: DetachType, ctx: &mut ViewContext<PaneGroup>);

    /// Snapshot this pane for session restoration.
    fn snapshot(&self, app: &AppContext) -> LeafContents;

    /// Returns whether or not application focus is within this pane.
    fn has_application_focus(&self, ctx: &mut ViewContext<PaneGroup>) -> bool;

    /// Focus this pane's contents.
    fn focus(&self, ctx: &mut ViewContext<PaneGroup>);

    /// Get the shareable link for the pane.
    ///
    /// This is called when the focused pane changes. It is used to get the link to the
    /// for the active pane (if there is one). This link is used to update the browser's
    /// url bar.
    fn shareable_link(
        &self,
        ctx: &mut ViewContext<PaneGroup>,
    ) -> Result<ShareableLink, ShareableLinkError>;

    /// Pane-agnostic state that all panes have.
    fn pane_configuration(&self) -> ModelHandle<PaneConfiguration>;

    fn is_pane_being_dragged(&self, ctx: &AppContext) -> bool;
}

/// Trait for untyped pane contents. This is a workaround for trait upcasting being
/// unstable: https://github.com/rust-lang/rust/issues/65991
pub trait AnyPaneContent {
    /// This pane's contents, as [`Any`], to allow downcasting.
    fn as_any(&self) -> &dyn Any;
    fn as_pane(&self) -> &dyn PaneContent;
    fn pre_attach(&self, group: &PaneGroup, ctx: &mut ViewContext<PaneGroup>) -> bool;
    fn try_attach(
        &self,
        group: &PaneGroup,
        focus_handle: PaneFocusHandle,
        ctx: &mut ViewContext<PaneGroup>,
    ) -> bool;
}

impl<T> AnyPaneContent for T
where
    T: PaneContent,
{
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_pane(&self) -> &dyn PaneContent {
        self
    }

    fn pre_attach(&self, group: &PaneGroup, ctx: &mut ViewContext<PaneGroup>) -> bool {
        PaneContent::pre_attach(self, group, ctx)
    }

    fn try_attach(
        &self,
        group: &PaneGroup,
        focus_handle: PaneFocusHandle,
        ctx: &mut ViewContext<PaneGroup>,
    ) -> bool {
        PaneContent::try_attach(self, group, focus_handle, ctx)
    }
}

/// A helper struct to group together pane-agnostic properties.
/// Implemented as a model so that underlying panes and the generic
/// views ([`PaneView`] and [`PaneHeader`]) can communicate changes
/// to one another.
pub struct PaneConfiguration {
    title: String,
    title_secondary: String,
    custom_vertical_tabs_title: Option<String>,
    show_active_pane_indicator: bool,

    /// If true, we draw an accent border around the pane.
    show_accent_border: bool,

    /// Pane views set this when they have an open modal. We dim the pane header along with the
    /// pane contents.
    has_open_modal: bool,

    /// Although the pane can think its focused, we actually now allow for left and right panels to be focused so the pane should be dimmed instead
    dim_even_if_focused: bool,

    /// Extra left inset (in px) for the pane header's left-side controls.
    /// Used to shift controls right to make room for a floating button overlay.
    pub header_left_inset: f32,
}

impl Entity for PaneConfiguration {
    type Event = PaneConfigurationEvent;
}

impl PaneConfiguration {
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            title_secondary: String::from(""),
            custom_vertical_tabs_title: None,
            show_active_pane_indicator: false,
            show_accent_border: false,
            has_open_modal: false,
            dim_even_if_focused: false,
            header_left_inset: 0.,
        }
    }

    pub fn title(&self) -> &str {
        &self.title
    }

    pub fn title_secondary(&self) -> &str {
        &self.title_secondary
    }

    pub fn custom_vertical_tabs_title(&self) -> Option<&str> {
        self.custom_vertical_tabs_title.as_deref()
    }

    pub fn dim_even_if_focused(&self) -> bool {
        self.dim_even_if_focused
    }

    pub fn set_show_active_pane_indicator(
        &mut self,
        show_active_pane_indicator: bool,
        ctx: &mut ModelContext<Self>,
    ) {
        self.show_active_pane_indicator = show_active_pane_indicator;
        ctx.emit(PaneConfigurationEvent::ShowActivePaneIndicatorUpdated);
        ctx.emit(PaneConfigurationEvent::HeaderContentChanged);
    }

    pub fn set_title(&mut self, title: impl Into<String>, ctx: &mut ModelContext<Self>) {
        let title = title.into();
        if self.title != title {
            self.title = title;
            ctx.emit(PaneConfigurationEvent::TitleUpdated);
            ctx.emit(PaneConfigurationEvent::HeaderContentChanged);
        }
    }

    pub fn set_title_secondary(
        &mut self,
        secondary: impl Into<String>,
        ctx: &mut ModelContext<Self>,
    ) {
        let secondary = secondary.into();
        if self.title_secondary != secondary {
            self.title_secondary = secondary;
            ctx.emit(PaneConfigurationEvent::TitleUpdated);
            ctx.emit(PaneConfigurationEvent::HeaderContentChanged);
        }
    }

    pub fn set_custom_vertical_tabs_title(
        &mut self,
        title: impl Into<String>,
        ctx: &mut ModelContext<Self>,
    ) {
        let title = title.into();
        let title = title.trim();
        let title = (!title.is_empty()).then(|| title.to_string());
        if self.custom_vertical_tabs_title != title {
            self.custom_vertical_tabs_title = title;
            ctx.emit(PaneConfigurationEvent::VerticalTabsTitleUpdated);
        }
    }

    pub fn clear_custom_vertical_tabs_title(&mut self, ctx: &mut ModelContext<Self>) {
        if self.custom_vertical_tabs_title.take().is_some() {
            ctx.emit(PaneConfigurationEvent::VerticalTabsTitleUpdated);
        }
    }

    pub fn set_dim_even_if_focused(
        &mut self,
        dim_even_if_focused: bool,
        ctx: &mut ModelContext<Self>,
    ) {
        if self.dim_even_if_focused != dim_even_if_focused {
            self.dim_even_if_focused = dim_even_if_focused;
            ctx.emit(PaneConfigurationEvent::DimEvenIfFocusedUpdated);
        }
    }

    pub fn set_show_accent_border(
        &mut self,
        show_accent_border: bool,
        ctx: &mut ModelContext<Self>,
    ) {
        self.show_accent_border = show_accent_border;
        ctx.emit(PaneConfigurationEvent::ShowAccentBorderUpdated);
    }

    pub fn set_has_open_modal(&mut self, has_open_modal: bool, ctx: &mut ModelContext<Self>) {
        self.has_open_modal = has_open_modal;
        ctx.emit(PaneConfigurationEvent::OpenModalUpdated);
        ctx.emit(PaneConfigurationEvent::HeaderContentChanged);
    }

    pub fn refresh_pane_header_overflow_menu_items(&mut self, ctx: &mut ModelContext<Self>) {
        ctx.emit(PaneConfigurationEvent::RefreshPaneHeaderOverflowMenuItems);
        ctx.emit(PaneConfigurationEvent::HeaderContentChanged);
    }

    /// Sets the shareable object in the current pane. If `None`, the share button is removed.
    pub fn set_shareable_object(
        &mut self,
        shareable_object: Option<ShareableObject>,
        ctx: &mut ModelContext<Self>,
    ) {
        ctx.emit(PaneConfigurationEvent::ShareableObjectChanged(
            shareable_object,
        ));
    }

    pub fn toggle_sharing_dialog(
        &mut self,
        source: SharingDialogSource,
        ctx: &mut ModelContext<Self>,
    ) {
        ctx.emit(PaneConfigurationEvent::ToggleSharingDialog(source));
    }

    /// Notifies that the header content has changed and the pane header should re-render.
    /// Use this when the backing view's state has changed in a way that affects the header
    /// content returned by `render_header_content()`.
    pub fn notify_header_content_changed(&mut self, ctx: &mut ModelContext<Self>) {
        ctx.emit(PaneConfigurationEvent::HeaderContentChanged);
    }

    pub fn set_header_left_inset(&mut self, inset: f32, ctx: &mut ModelContext<Self>) {
        if (self.header_left_inset - inset).abs() > f32::EPSILON {
            self.header_left_inset = inset;
            ctx.emit(PaneConfigurationEvent::HeaderContentChanged);
        }
    }
}

pub enum PaneConfigurationEvent {
    TitleUpdated,
    VerticalTabsTitleUpdated,
    ShowActivePaneIndicatorUpdated,
    RenderElementFnUpdated,
    ShowAccentBorderUpdated,
    OpenModalUpdated,
    RefreshPaneHeaderOverflowMenuItems,
    ShareableObjectChanged(Option<ShareableObject>),
    ToggleSharingDialog(SharingDialogSource),
    DimEvenIfFocusedUpdated,
    /// The header content has changed and should be re-rendered.
    /// This is used when the backing view's state changes in a way that
    /// affects what `render_header_content()` returns.
    HeaderContentChanged,
}

/// Event emitted when the pane stack changes.
pub enum PaneStackEvent<P: View> {
    /// A view was added to the stack.
    ViewAdded(ViewHandle<P>),
    /// A view was removed from the stack.
    ViewRemoved(ViewHandle<P>),
}

/// A navigation stack of backing views for a pane.
///
/// This model allows panes to support a stack of views, where only the topmost
/// view is active/rendered. Views can be pushed onto the stack and popped to
/// return to the previous view.
///
/// The stack is guaranteed to always have at least one view.
///
/// Each view in the stack can have associated data of type [`BackingView::AssociatedData`].
/// This is useful for storing per-view metadata that should be tied to the view's own lifetime.
pub struct PaneStack<P: BackingView> {
    /// The stack of backing views with associated data. The last element is the active (topmost) view.
    children: vec1::Vec1<(P::AssociatedData, ViewHandle<P>)>,
}

impl<P: BackingView> Entity for PaneStack<P> {
    type Event = PaneStackEvent<P>;
}

impl<P: BackingView<AssociatedData = ()>> PaneStack<P> {
    /// Pushes a new view onto the stack, making it the active view. This is a convenience wrapper
    /// around [`Self::push`] for panes with no associated data.
    pub fn push_view(&mut self, view: ViewHandle<P>, ctx: &mut ModelContext<Self>) {
        self.push((), view, ctx);
    }
}

impl<P: BackingView> PaneStack<P> {
    /// Creates a new pane stack with a single initial view and its associated data.
    pub fn new(
        data: P::AssociatedData,
        initial_view: ViewHandle<P>,
        ctx: &mut ModelContext<Self>,
    ) -> Self {
        let weak_handle = ctx.handle();
        initial_view.update(ctx, |view, ctx| {
            view.set_pane_stack(weak_handle, ctx);
        });
        Self {
            children: vec1::vec1![(data, initial_view)],
        }
    }

    /// Returns the topmost (active) view in the stack.
    pub fn active_view(&self) -> &ViewHandle<P> {
        &self.children.last().1
    }

    /// Returns the associated data for the topmost (active) view in the stack.
    pub fn active_data(&self) -> &P::AssociatedData {
        &self.children.last().0
    }

    /// Returns a mutable reference to the associated data for the topmost (active) view.
    pub fn active_data_mut(&mut self) -> &mut P::AssociatedData {
        &mut self.children.last_mut().0
    }

    /// Returns all views in the stack.
    pub fn views(&self) -> impl Iterator<Item = &ViewHandle<P>> {
        self.children.iter().map(|(_, view)| view)
    }

    /// Returns all entries in the stack as (data, view) pairs.
    pub fn entries(&self) -> &[(P::AssociatedData, ViewHandle<P>)] {
        &self.children
    }

    /// Returns the depth (number of views) in the stack.
    pub fn depth(&self) -> usize {
        self.children.len()
    }

    /// Pushes a new view with associated data onto the stack, making it the active view.
    pub fn push(
        &mut self,
        data: P::AssociatedData,
        view: ViewHandle<P>,
        ctx: &mut ModelContext<Self>,
    ) {
        let weak_handle = ctx.handle();
        view.update(ctx, |view, ctx| {
            view.set_pane_stack(weak_handle, ctx);
        });
        self.children.push((data, view.clone()));
        ctx.emit(PaneStackEvent::ViewAdded(view));
    }

    /// Pops the topmost view from the stack.
    /// Returns the popped view and its associated data, or `None` if the stack has only one view.
    pub fn pop(
        &mut self,
        ctx: &mut ModelContext<Self>,
    ) -> Option<(P::AssociatedData, ViewHandle<P>)> {
        let popped = self.children.pop().ok()?;
        ctx.emit(PaneStackEvent::ViewRemoved(popped.1.clone()));
        Some(popped)
    }
}

/// Toolbelt buttons appear in a group together on the left-hand side of the pane header.
pub struct ToolbeltButton {
    pub action_button: ViewHandle<ActionButton>,
}

/// The view that is rendered as the pane's contents.
pub trait BackingView: View {
    type PaneHeaderOverflowMenuAction: Action + Clone;
    type CustomAction: Action + Clone;
    /// Associated data type stored with each view in the [`PaneStack`].
    type AssociatedData: 'static + Send;

    /// Processes the corresponding action when one of the
    /// overflow menu items is selected. Allows implementers
    /// to add pre-/post-processing logic (e.g. telemetry).
    ///
    // Note: even if the [`PaneHeaderOverflowMenuAction`] was [`TypedActionView::Action`]
    // (assuming [`TypedActionView`] was one of the trait bounds for [`BackingView`]),
    // the pane header cannot simply dispatch the action because the underlying view
    // is not one of its ancestors and thus won't be in the responder chain.
    fn handle_pane_header_overflow_menu_action(
        &mut self,
        action: &Self::PaneHeaderOverflowMenuAction,
        ctx: &mut ViewContext<Self>,
    );

    /// Handles a CustomAction in the pane header being clicked.
    fn handle_custom_action(
        &mut self,
        _custom_action: &Self::CustomAction,
        _ctx: &mut ViewContext<Self>,
    ) {
    }

    /// Closes the pane.
    fn close(&mut self, ctx: &mut ViewContext<Self>);

    /// Focus the pane contents. This is similar to [`PaneContent::focus`], but called from within
    /// the pane when focus shifts from the header or pane controls to the pane contents.
    fn focus_contents(&mut self, ctx: &mut ViewContext<Self>);

    /// The menu items that should be displayed in the pane header's overflow menu.
    /// When these should be updated, fire the [`PaneConfigurationEvent::RefreshPaneHeaderOverflowMenuItems`]
    /// and the pane header will fetch the new items via this API.
    fn pane_header_overflow_menu_items(
        &self,
        _ctx: &AppContext,
    ) -> Vec<MenuItem<Self::PaneHeaderOverflowMenuAction>> {
        vec![]
    }

    fn on_pane_header_overflow_menu_toggled(
        &mut self,
        _is_open: bool,
        _ctx: &mut ViewContext<Self>,
    ) {
    }

    /// Action buttons rendered on the left side of the pane header in the returned order in the vec.
    fn pane_header_toolbelt_buttons(&self, _ctx: &AppContext) -> Vec<ToolbeltButton> {
        vec![]
    }

    fn should_render_header(&self, _app: &AppContext) -> bool {
        true
    }

    /// Called when this view is added to a [`PaneStack`]. Views are given a handle to their owning stack so that they can:
    /// * Check if they're part of a stack
    /// * Push/pop views from the stack
    ///
    /// Because the pane stack holds a strong reference to each view, views *must not* store a strong `ModelHandle` to the
    // stack.
    fn set_pane_stack(
        &mut self,
        _pane_stack: WeakModelHandle<PaneStack<Self>>,
        _ctx: &mut ViewContext<Self>,
    ) where
        Self: Sized,
    {
    }

    /// Returns pane header content for this view.
    ///
    /// The framework handles wrapping the returned content with draggable behavior,
    /// so implementations don't need to worry about drag-and-drop.
    ///
    /// # Return values
    /// - `HeaderContent::Standard { .. }`: Standard header with customization points
    /// - `HeaderContent::Custom(element)`: Fully custom header, auto-wrapped with draggable
    /// - `HeaderContent::CustomWithExplicitDraggable(element)`: Custom header where the view
    ///   is responsible for calling `PaneHeader::render_pane_header_draggable()` on appropriate elements
    fn render_header_content(
        &self,
        ctx: &view::HeaderRenderContext<'_>,
        app: &AppContext,
    ) -> view::HeaderContent;

    /// Set the handle that tracks whether the pane containing this view is focused.
    fn set_focus_handle(&mut self, focus_handle: PaneFocusHandle, ctx: &mut ViewContext<Self>);
}

/// Common event type for all panes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PaneEvent {
    Close,
    CloseAndFocus {
        pane_to_focus: PaneId,
    },
    SplitLeft(Option<AvailableShell>),
    SplitRight(Option<AvailableShell>),
    SplitUp(Option<AvailableShell>),
    SplitDown(Option<AvailableShell>),
    ToggleMaximized,
    /// Make this pane the focused pane.
    FocusSelf,
    FocusActiveSession,
    /// The session-restoration state for this pane must be updated.
    AppStateChanged,
    /// Repo for this pane's terminal has changed
    RepoChanged,
    /// A remote server resolved the repo root for a session in this pane.
    RemoteRepoNavigated {
        remote_path: RemotePath,
    },
    /// Split the current pane into two. If `initial_query` is `Some` fill the new pane's input with
    /// its value.
    NewPaneInAIMode {
        initial_query: Option<String>,
    },
    ClearHoveredTabIndex,
    #[cfg(feature = "local_fs")]
    ReplaceWithCodePane {
        path: std::path::PathBuf,
        source: Option<crate::code::editor_management::CodeSource>,
    },
    #[cfg(feature = "local_fs")]
    ReplaceWithFilePane {
        path: std::path::PathBuf,
        source: Option<crate::code::editor_management::CodeSource>,
    },
}
