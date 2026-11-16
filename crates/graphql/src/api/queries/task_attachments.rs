use crate::error::UserFacingError;
use crate::request_context::RequestContext;
use crate::schema;

/// A GraphQL query to fetch attachments for a specific task.
///
/// This query is used by Agent Mode VMs to retrieve file attachments (images, PDFs, etc.)
/// that have been made available to them.
#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "RootQuery", variables = "TaskVariables")]
pub struct Task {
    #[arguments(input: $input, requestContext: $request_context)]
    pub task: TaskResult,
}

crate::client::define_operation! {
    task(TaskVariables) -> Task;
}

#[derive(cynic::QueryVariables, Debug)]
pub struct TaskVariables {
    pub input: TaskInput,
    pub request_context: RequestContext,
}

#[derive(cynic::InputObject, Debug)]
pub struct TaskInput {
    pub task_id: cynic::Id,
}

#[derive(cynic::InlineFragments, Debug)]
pub enum TaskResult {
    TaskOutput(TaskOutput),
    UserFacingError(UserFacingError),
    #[cynic(fallback)]
    Unknown,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct TaskOutput {
    pub task: TaskData,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "Task")]
pub struct TaskData {
    pub task_id: cynic::Id,
    pub attachments: Vec<TaskAttachment>,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct TaskAttachment {
    pub file_id: cynic::Id,
    pub filename: String,
    pub download_url: String,
    pub mime_type: String,
}
