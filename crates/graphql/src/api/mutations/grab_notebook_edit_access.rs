use crate::notebook::{UpdateNotebookEditAccessInput, UpdateNotebookEditAccessResult};
use crate::request_context::RequestContext;
use crate::schema;

#[derive(cynic::QueryVariables, Debug)]
pub struct GrabNotebookEditAccessVariables {
    pub input: UpdateNotebookEditAccessInput,
    pub request_context: RequestContext,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "RootMutation",
    variables = "GrabNotebookEditAccessVariables"
)]
pub struct GrabNotebookEditAccess {
    #[arguments(requestContext: $request_context, input: $input)]
    pub grab_notebook_edit_access: UpdateNotebookEditAccessResult,
}
crate::client::define_operation! {
    grab_notebook_edit_access(GrabNotebookEditAccessVariables) -> GrabNotebookEditAccess;
}
