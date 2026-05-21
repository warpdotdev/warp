use crate::workflows::{
    command_parser::{compute_workflow_display_data, WorkflowArgumentIndex},
    workflow::Argument,
    WorkflowType,
};
use warp_server_client::ids::SyncId;
use warpui::{elements::Empty, AppContext, Element, Entity, TypedActionView, View, ViewContext};

pub const WORKFLOW_PARAMETER_HIGHLIGHT_COLOR: u32 = 0x42C0FA4D;

pub struct SelectedWorkflowState {
    currently_selected_argument: WorkflowArgumentIndex,
    num_arguments: WorkflowArgumentIndex,
    argument_cycling_enabled: bool,
}

impl SelectedWorkflowState {
    pub fn increment_argument_index(&mut self) {
        if *self.num_arguments > 0 && self.argument_cycling_enabled {
            self.currently_selected_argument =
                ((*self.currently_selected_argument + 1) % *self.num_arguments).into();
        }
    }

    pub fn set_argument_index(&mut self, index: WorkflowArgumentIndex) {
        if *index < *self.num_arguments {
            self.currently_selected_argument = index;
        } else {
            log::error!(
                "Tried to set the argument index to {:?} but the len is {:?}",
                *index,
                *self.num_arguments
            );
        }
    }

    pub fn currently_selected_argument(&self) -> WorkflowArgumentIndex {
        self.currently_selected_argument
    }

    pub fn set_argument_cycling_enabled(&mut self, new_val: bool) {
        self.argument_cycling_enabled = new_val;
    }
}

pub struct WorkflowsMoreInfoView {
    workflow: WorkflowType,
    pub info_box_expanded: bool,
    pub selected_workflow_state: SelectedWorkflowState,
    pub show_shift_tab_treatment: bool,
}

impl WorkflowsMoreInfoView {
    pub fn new(
        info_box_expanded: bool,
        workflow: WorkflowType,
        show_shift_tab_treatment: bool,
        _ctx: &mut ViewContext<Self>,
    ) -> Self {
        let num_arguments = workflow.as_workflow().arguments().len();
        let _ = compute_workflow_display_data(workflow.as_workflow());

        Self {
            workflow,
            info_box_expanded,
            selected_workflow_state: SelectedWorkflowState {
                currently_selected_argument: 0.into(),
                num_arguments: num_arguments.into(),
                argument_cycling_enabled: true,
            },
            show_shift_tab_treatment,
        }
    }

    pub fn selected_argument(&self) -> Option<&Argument> {
        if !self.selected_workflow_state.argument_cycling_enabled {
            return None;
        }

        self.workflow
            .as_workflow()
            .arguments()
            .get(*self.selected_workflow_state.currently_selected_argument)
    }

    pub fn set_environment_variables_selection(
        &mut self,
        _env_vars_id: Option<SyncId>,
        _ctx: &mut ViewContext<Self>,
    ) {
    }
}

#[derive(Debug)]
pub enum WorkflowsInfoBoxViewEvent {
    PrefixCommandWithEnvironmentVariables(Option<SyncId>),
}

#[derive(Debug, Clone)]
pub enum WorkflowsInfoBoxViewAction {
    CollapseOrExpand,
    SelectEnvironmentVariables(Option<SyncId>),
}

impl Entity for WorkflowsMoreInfoView {
    type Event = WorkflowsInfoBoxViewEvent;
}

impl TypedActionView for WorkflowsMoreInfoView {
    type Action = WorkflowsInfoBoxViewAction;

    fn handle_action(&mut self, action: &WorkflowsInfoBoxViewAction, ctx: &mut ViewContext<Self>) {
        match action {
            WorkflowsInfoBoxViewAction::CollapseOrExpand => {
                self.info_box_expanded = !self.info_box_expanded;
            }
            WorkflowsInfoBoxViewAction::SelectEnvironmentVariables(env_vars) => ctx.emit(
                WorkflowsInfoBoxViewEvent::PrefixCommandWithEnvironmentVariables(env_vars.clone()),
            ),
        }
    }
}

impl View for WorkflowsMoreInfoView {
    fn ui_name() -> &'static str {
        "WorkflowsInfoBoxView"
    }

    fn render(&self, _app: &AppContext) -> Box<dyn Element> {
        Empty::new().finish()
    }
}
