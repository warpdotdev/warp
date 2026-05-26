use super::object::ObjectType;
use crate::response_context::ResponseContext;
use crate::schema;

#[derive(cynic::QueryFragment, Debug)]
pub struct UserFacingError {
    pub error: UserFacingErrorInterface,
    pub response_context: ResponseContext,
}

#[derive(cynic::InlineFragments, Debug)]
pub enum UserFacingErrorInterface {
    SharedObjectsLimitExceeded(SharedObjectsLimitExceeded),
    PersonalObjectsLimitExceeded(PersonalObjectsLimitExceeded),
    AccountDelinquencyError(AccountDelinquencyError),
    GenericStringObjectUniqueKeyConflict(GenericStringObjectUniqueKeyConflict),
    BudgetExceededError(BudgetExceededError),
    PaymentMethodDeclinedError(PaymentMethodDeclinedError),
    InvalidAttachmentError(InvalidAttachmentError),
    #[cynic(fallback)]
    Unknown(UserFacingErrorFallback),
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(graphql_type = "UserFacingErrorInterface")]
pub struct UserFacingErrorFallback {
    pub message: String,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct SharedObjectsLimitExceeded {
    pub limit: i32,
    pub object_type: ObjectType,
    pub message: String,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct PersonalObjectsLimitExceeded {
    pub limit: i32,
    pub object_type: ObjectType,
    pub message: String,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct AccountDelinquencyError {
    pub message: String,
}

#[derive(cynic::QueryFragment, Debug)]
pub struct GenericStringObjectUniqueKeyConflict {
    pub message: String,
}

#[derive(cynic::QueryFragment, Debug, thiserror::Error)]
#[error("{message}")]
pub struct BudgetExceededError {
    pub message: String,
}

#[derive(cynic::QueryFragment, Debug, thiserror::Error)]
#[error("{message}")]
pub struct PaymentMethodDeclinedError {
    pub message: String,
}

#[derive(cynic::QueryFragment, Debug, thiserror::Error)]
#[error("{message}")]
pub struct InvalidAttachmentError {
    pub message: String,
}
