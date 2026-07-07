use crate::schema;

#[derive(cynic::Scalar, Debug, Clone)]
pub struct ContentHash(pub String);

#[derive(cynic::Enum, Clone, Copy, Debug)]
pub enum EmbeddingConfig {
    #[cynic(rename = "OPENAI_TEXT_SMALL_3_256")]
    OpenaiTextSmall3256,
    #[cynic(rename = "VOYAGE_CODE_3_512")]
    VoyageCode3512,
    #[cynic(rename = "VOYAGE_3_5_512")]
    Voyage35512,
    #[cynic(rename = "VOYAGE_3_5_LITE_512")]
    Voyage35Lite512,
    #[cynic(rename = "VOYAGE_4_512")]
    Voyage4512,
}

#[derive(cynic::Scalar, Debug, Clone)]
pub struct NodeHash(pub String);

#[derive(cynic::InputObject, Debug)]
pub struct Fragment {
    pub content: String,
    pub content_hash: ContentHash,
}

#[derive(cynic::InputObject, Debug)]
pub struct RepoMetadata {
    pub path: Option<String>,
}
