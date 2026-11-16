use crate::error::UserFacingError;
use crate::object::ObjectMetadata;
use crate::object_permissions::Owner;
use crate::request_context::RequestContext;
use crate::response_context::ResponseContext;
use crate::schema;

#[derive(cynic::QueryVariables, Debug)]
pub struct TransferWorkflowOwnerVariables {
    pub input: TransferWorkflowOwnerInput,
    pub request_context: RequestContext,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct TransferWorkflowOwnerOutput {
    pub metadata: ObjectMetadata,
    pub response_context: ResponseContext,
    pub success: bool,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "RootMutation",
    variables = "TransferWorkflowOwnerVariables"
)]
pub struct TransferWorkflowOwner {
    #[arguments(requestContext: $request_context, input: $input)]
    pub transfer_workflow_owner: TransferWorkflowOwnerResult,
}
crate::client::define_operation! {
    transfer_workflow_owner(TransferWorkflowOwnerVariables) -> TransferWorkflowOwner;
}

#[derive(cynic::InlineFragments, Debug)]
pub enum TransferWorkflowOwnerResult {
    TransferWorkflowOwnerOutput(TransferWorkflowOwnerOutput),
    UserFacingError(UserFacingError),
    #[cynic(fallback)]
    Unknown,
}

#[derive(cynic::InputObject, Debug)]
pub struct TransferWorkflowOwnerInput {
    pub owner: Owner,
    pub uid: cynic::Id,
}
