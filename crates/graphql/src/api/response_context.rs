use crate::schema;

#[derive(cynic::QueryFragment, Debug)]
pub struct ResponseContext {
    pub server_version: Option<String>,
}
