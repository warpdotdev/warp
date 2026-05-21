use crate::error::UserFacingError;
use crate::request_context::RequestContext;
use crate::response_context::ResponseContext;
use crate::scalars::Time;
use crate::schema;

#[derive(cynic::QueryVariables, Debug)]
pub struct ShareBlockVariables<'a> {
    pub block: BlockInput<'a>,
    pub request_context: RequestContext,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct ShareBlockOutput {
    pub url_ending: String,
    pub response_context: ResponseContext,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "RootMutation", variables = "ShareBlockVariables")]
pub struct ShareBlock {
    #[arguments(input: { block: $block }, requestContext: $request_context)]
    pub share_block: ShareBlockResult,
}
crate::client::define_operation! {
    ['a] share_block(ShareBlockVariables<'a>) -> ShareBlock;
}

#[derive(cynic::InlineFragments, Debug)]
pub enum ShareBlockResult {
    ShareBlockOutput(ShareBlockOutput),
    UserFacingError(UserFacingError),
    #[cynic(fallback)]
    Unknown,
}

#[derive(cynic::Enum, Clone, Debug)]
pub enum DisplaySetting {
    Command,
    CommandAndOutput,
    Output,
    #[cynic(fallback)]
    Other(String),
}

#[derive(cynic::InputObject, Debug)]
pub struct BlockInput<'a> {
    pub command: Option<&'a str>,
    pub embed_display_setting: DisplaySetting,
    pub output: Option<&'a str>,
    pub show_prompt: bool,
    pub stylized_command: Option<&'a str>,
    pub stylized_output: Option<&'a str>,
    pub stylized_prompt: Option<&'a str>,
    pub stylized_prompt_and_command: Option<&'a str>,
    pub time_started_term: Option<Time>,
    pub title: Option<&'a str>,
}
