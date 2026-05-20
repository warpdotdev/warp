use crate::{
    error::UserFacingError, request_context::RequestContext, response_context::ResponseContext,
    schema,
};

#[derive(cynic::QueryVariables, Debug)]
pub struct CreateFileArtifactUploadTargetVariables {
    pub input: CreateFileArtifactUploadTargetInput,
    pub request_context: RequestContext,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    graphql_type = "RootMutation",
    variables = "CreateFileArtifactUploadTargetVariables"
)]
pub struct CreateFileArtifactUploadTarget {
    #[arguments(input: $input, requestContext: $request_context)]
    pub create_file_artifact_upload_target: CreateFileArtifactUploadTargetResult,
}

crate::client::define_operation! {
    create_file_artifact_upload_target(CreateFileArtifactUploadTargetVariables) -> CreateFileArtifactUploadTarget;
}

#[derive(cynic::InlineFragments, Debug)]
pub enum CreateFileArtifactUploadTargetResult {
    CreateFileArtifactUploadTargetOutput(CreateFileArtifactUploadTargetOutput),
    UserFacingError(UserFacingError),
    #[cynic(fallback)]
    Unknown,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct CreateFileArtifactUploadTargetOutput {
    pub artifact: FileArtifact,
    pub response_context: ResponseContext,
    pub upload_target: FileArtifactUploadTarget,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct FileArtifact {
    pub artifact_uid: cynic::Id,
    pub filepath: String,
    pub description: Option<String>,
    pub mime_type: String,
    pub size_bytes: Option<i32>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "CreateUploadTarget")]
pub struct FileArtifactUploadTarget {
    pub url: String,
    pub method: String,
    pub headers: Vec<FileArtifactUploadHeader>,
    pub fields: Vec<FileArtifactUploadField>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "CreateUploadTargetHeader")]
pub struct FileArtifactUploadHeader {
    pub name: String,
    pub value: String,
}

/// A single multipart form field for a presigned POST upload target.
#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "UploadField")]
pub struct FileArtifactUploadField {
    pub name: String,
    pub value: FileArtifactUploadFieldValue,
}

/// The value of a multipart form field on a presigned POST upload target.
#[derive(cynic::InlineFragments, Debug)]
#[cynic(graphql_type = "UploadFieldValue")]
pub enum FileArtifactUploadFieldValue {
    StaticUploadFieldValue(StaticUploadFieldValue),
    ContentCRC32CFieldValue(ContentCRC32CFieldValue),
    ContentDataFieldValue(ContentDataFieldValue),
    #[cynic(fallback)]
    Unknown,
}

/// Literal string value known at URL-generation time.
#[derive(cynic::QueryFragment, Debug)]
pub struct StaticUploadFieldValue {
    pub value: String,
}

/// Signals the client to compute CRC32C of the upload, base64-encode the 4-byte
/// big-endian result, and send it as the field's value. The GraphQL type
/// carries no payload beyond a placeholder `_: Boolean`; the client only cares
/// about the variant tag, but cynic's `QueryFragment` derive requires at least
/// one field to select.
#[derive(cynic::QueryFragment, Debug, Default)]
pub struct ContentCRC32CFieldValue {
    #[cynic(rename = "_")]
    _placeholder: Option<bool>,
}

/// Signals the client to use the raw upload bytes as the field's value. Must
/// be the final entry in `fields`. See [`ContentCRC32CFieldValue`] for why we
/// select a placeholder `_: Boolean`.
#[derive(cynic::QueryFragment, Debug, Default)]
pub struct ContentDataFieldValue {
    #[cynic(rename = "_")]
    _placeholder: Option<bool>,
}

#[derive(cynic::InputObject, Debug)]
#[cynic(graphql_type = "CreateFileArtifactUploadTargetInput")]
pub struct CreateFileArtifactUploadTargetInput {
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub conversation_id: Option<cynic::Id>,
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub run_id: Option<cynic::Id>,
    pub filepath: String,
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub size_bytes: Option<i32>,
}
