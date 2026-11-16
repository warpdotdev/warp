use super::create_file_artifact_upload_target::FileArtifact;
use crate::error::UserFacingError;
use crate::request_context::RequestContext;
use crate::response_context::ResponseContext;
use crate::schema;

#[derive(cynic::QueryVariables, Debug)]
pub struct ConfirmFileArtifactUploadVariables {
    pub input: ConfirmFileArtifactUploadInput,
    pub request_context: RequestContext,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "RootMutation",
    variables = "ConfirmFileArtifactUploadVariables"
)]
pub struct ConfirmFileArtifactUpload {
    #[arguments(input: $input, requestContext: $request_context)]
    pub confirm_file_artifact_upload: ConfirmFileArtifactUploadResult,
}

crate::client::define_operation! {
    confirm_file_artifact_upload(ConfirmFileArtifactUploadVariables) -> ConfirmFileArtifactUpload;
}

#[derive(cynic::InlineFragments, Debug)]
pub enum ConfirmFileArtifactUploadResult {
    ConfirmFileArtifactUploadOutput(ConfirmFileArtifactUploadOutput),
    UserFacingError(UserFacingError),
    #[cynic(fallback)]
    Unknown,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct ConfirmFileArtifactUploadOutput {
    pub artifact: FileArtifact,
    pub response_context: ResponseContext,
}

#[derive(cynic::InputObject, Debug)]
#[cynic(graphql_type = "ConfirmFileArtifactUploadInput")]
pub struct ConfirmFileArtifactUploadInput {
    pub artifact_uid: cynic::Id,
    pub checksum: String,
}
