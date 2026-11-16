use crate::notebook::{UpdateNotebookEditAccessInput, UpdateNotebookEditAccessResult};
use crate::request_context::RequestContext;
use crate::schema;

#[derive(cynic::QueryVariables, Debug)]
pub struct GiveUpNotebookEditAccessVariables {
    pub input: UpdateNotebookEditAccessInput,
    pub request_context: RequestContext,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "RootMutation",
    variables = "GiveUpNotebookEditAccessVariables"
)]
pub struct GiveUpNotebookEditAccess {
    #[arguments(input: $input, requestContext: $request_context)]
    pub give_up_notebook_edit_access: UpdateNotebookEditAccessResult,
}
crate::client::define_operation! {
    give_up_notebook_edit_access(GiveUpNotebookEditAccessVariables) -> GiveUpNotebookEditAccess;
}
