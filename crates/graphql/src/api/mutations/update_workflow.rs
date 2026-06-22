use crate::error::UserFacingError;
use crate::object::ObjectUpdateSuccess;
use crate::request_context::RequestContext;
use crate::response_context::ResponseContext;
use crate::scalars::Time;
use crate::schema;
use crate::workflow::Workflow;

#[derive(cynic::QueryVariables, Debug)]
pub struct UpdateWorkflowVariables {
    pub input: UpdateWorkflowInput,
    pub request_context: RequestContext,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct WorkflowUpdateRejected {
    pub conflicting_workflow: Workflow,
    pub revision_ts: Time,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct UpdateWorkflowOutput {
    pub response_context: ResponseContext,
    pub update: WorkflowUpdate,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "RootMutation", variables = "UpdateWorkflowVariables")]
pub struct UpdateWorkflow {
    #[arguments(input: $input, requestContext: $request_context)]
    pub update_workflow: UpdateWorkflowResult,
}
crate::client::define_operation! {
    update_workflow(UpdateWorkflowVariables) -> UpdateWorkflow;
}

#[derive(cynic::InlineFragments, Debug)]
#[allow(clippy::large_enum_variant)]
pub enum UpdateWorkflowResult {
    UpdateWorkflowOutput(UpdateWorkflowOutput),
    UserFacingError(UserFacingError),
    #[cynic(fallback)]
    Unknown,
}

#[derive(cynic::InlineFragments, Debug)]
#[allow(clippy::large_enum_variant)]
pub enum WorkflowUpdate {
    ObjectUpdateSuccess(ObjectUpdateSuccess),
    WorkflowUpdateRejected(WorkflowUpdateRejected),
    #[cynic(fallback)]
    Unknown,
}

#[derive(cynic::InputObject, Debug)]
pub struct UpdateWorkflowInput {
    pub data: String,
    pub revision_ts: Option<Time>,
    pub uid: cynic::Id,
}
