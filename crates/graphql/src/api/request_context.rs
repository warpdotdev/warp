use crate::schema;

#[derive(cynic::InputObject, Debug)]
pub struct RequestContext {
    pub client_context: ClientContext,
    pub os_context: OsContext,
}

#[derive(cynic::InputObject, Debug)]
#[cynic(graphql_type = "OSContext")]
pub struct OsContext {
    pub category: Option<String>,
    pub linux_kernel_version: Option<String>,
    pub name: Option<String>,
    pub version: Option<String>,
}

#[derive(cynic::InputObject, Debug)]
pub struct ClientContext {
    pub version: Option<String>,
}
