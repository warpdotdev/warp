use warpui::elements::Empty;
use warpui::{AppContext, Element, Entity, TypedActionView, View, ViewContext};

use crate::app_state::PersistedAgentManagementFilters;
use crate::notebooks::NotebookId;
use crate::workflows::WorkflowType;

pub fn init(_app: &mut AppContext) {}

pub struct AgentManagementView;

impl AgentManagementView {
    pub fn new(
        _persisted_filters: Option<PersistedAgentManagementFilters>,
        _ctx: &mut ViewContext<Self>,
    ) -> Self {
        Self
    }

    pub(crate) fn show_setup_guide_from_link(&mut self, _ctx: &mut ViewContext<Self>) {}

    pub(crate) fn is_showing_setup_guide(&self) -> bool {
        true
    }

    pub(crate) fn get_filters(&self) -> PersistedAgentManagementFilters {
        PersistedAgentManagementFilters::default()
    }

    pub(crate) fn apply_environment_filter_from_link(
        &mut self,
        _environment_id: String,
        _ctx: &mut ViewContext<Self>,
    ) {
    }
}

impl Entity for AgentManagementView {
    type Event = AgentManagementViewEvent;
}

impl View for AgentManagementView {
    fn ui_name() -> &'static str {
        "AgentManagementView"
    }

    fn render(&self, _app: &AppContext) -> Box<dyn Element> {
        Empty::new().finish()
    }
}

#[derive(Clone, Debug)]
pub enum AgentManagementViewAction {
    FocusSearch,
}

pub enum AgentManagementViewEvent {
    OpenNewTabAndRunWorkflow(Box<WorkflowType>),
    OpenPlanNotebook { notebook_uid: NotebookId },
}

impl TypedActionView for AgentManagementView {
    type Action = AgentManagementViewAction;

    fn handle_action(&mut self, _action: &Self::Action, _ctx: &mut ViewContext<Self>) {}
}
