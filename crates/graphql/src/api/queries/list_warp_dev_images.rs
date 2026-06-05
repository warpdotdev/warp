use crate::error::UserFacingError;
use crate::schema;

#[derive(cynic::QueryVariables, Debug)]
pub struct ListWarpDevImagesVariables {}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "RootQuery", variables = "ListWarpDevImagesVariables")]
pub struct ListWarpDevImages {
    #[cynic(rename = "listWarpDevImages")]
    pub list_warp_dev_images: ListWarpDevImagesResult,
}

#[derive(cynic::QueryFragment, Debug, Clone)]
pub struct ListWarpDevImagesOutput {
    pub images: Vec<ImageTag>,
}

#[derive(cynic::QueryFragment, Debug, Clone)]
pub struct ImageTag {
    pub image: String,
    pub repository: String,
    pub tag: String,
}

#[derive(cynic::InlineFragments, Debug)]
pub enum ListWarpDevImagesResult {
    ListWarpDevImagesOutput(ListWarpDevImagesOutput),
    UserFacingError(UserFacingError),
    #[cynic(fallback)]
    Unknown,
}

crate::client::define_operation! {
    ListWarpDevImages(ListWarpDevImagesVariables) -> ListWarpDevImages;
}
