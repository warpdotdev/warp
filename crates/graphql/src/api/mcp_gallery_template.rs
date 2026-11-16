use crate::schema;

#[derive(cynic::QueryFragment, Debug, Clone)]
#[cynic(graphql_type = "MCPTemplateVariable")]
pub struct MCPTemplateVariable {
    pub key: String,
    pub allowed_values: Option<Vec<String>>,
}

#[derive(cynic::QueryFragment, Debug, Clone)]
#[cynic(graphql_type = "MCPJsonTemplate")]
pub struct MCPJsonTemplate {
    pub json: String,
    pub variables: Vec<MCPTemplateVariable>,
}

#[derive(cynic::QueryFragment, Debug, Clone)]
#[cynic(graphql_type = "MCPGalleryTemplate")]
pub struct MCPGalleryTemplate {
    pub description: String,
    pub gallery_item_id: String,
    pub instructions_in_markdown: Option<String>,
    pub json_template: MCPJsonTemplate,
    pub template: String,
    pub title: String,
    pub version: i32,
}
