use std::sync::OnceLock;

use apollo_compiler::validation::Valid;
use apollo_compiler::{ExecutableDocument, Schema};
use cynic::queries::OperationType;
use cynic::{QueryFragment, QueryVariables};
use http::{HeaderMap, HeaderValue, StatusCode};

use super::{ANONYMOUS_SESSION_REJECTED_HEADER, is_anonymous_session_rejection};
use crate::mutations::generate_code_embeddings::GenerateCodeEmbeddingsResult;
use crate::mutations::update_merkle_tree::UpdateMerkleTreeResult;
use crate::queries::get_relevant_fragments::GetRelevantFragmentsResult;
use crate::queries::rerank_fragments::RerankFragmentsResult;
use crate::queries::sync_merkle_tree::SyncMerkleTreeResult;

pub(crate) fn assert_valid_operation<Q: QueryFragment, V: QueryVariables>() {
    static SCHEMA: OnceLock<Valid<Schema>> = OnceLock::new();
    let schema = SCHEMA.get_or_init(|| {
        Schema::parse_and_validate(
            include_str!("../../warp_graphql_schema/api/schema.graphql"),
            "schema.graphql",
        )
        .unwrap()
    });
    let operation_type = match Q::TYPE {
        Some("RootQuery") => OperationType::Query,
        Some("RootMutation") => OperationType::Mutation,
        Some("RootSubscription") => OperationType::Subscription,
        other => panic!("Expected a GraphQL operation root, got {other:?}"),
    };
    let document = cynic::queries::build_executable_document::<Q, V>(
        operation_type,
        Q::name().as_deref(),
        Default::default(),
        None,
    );

    ExecutableDocument::parse_and_validate(schema, document, "operation.graphql").unwrap();
}

#[test]
fn warp_drive_subscription_is_valid() {
    assert_valid_operation::<crate::subscriptions::get_warp_drive_updates::GetWarpDriveUpdates, ()>(
    );
}

macro_rules! error_response_test {
    ($name:ident, $result:ident, $variant:ident) => {
        #[test]
        fn $name() {
            let scalar_error: $result = serde_json::from_value(serde_json::json!({
                "__typename": stringify!($variant),
                "message": "Request failed",
            }))
            .unwrap();
            let $result::$variant(error) = scalar_error else {
                panic!("Expected a scalar error");
            };
            assert_eq!(error.message, "Request failed");

            let user_facing_error: $result = serde_json::from_value(serde_json::json!({
                "__typename": "UserFacingError",
                "error": {
                    "__typename": "BudgetExceededError",
                    "message": "Budget exceeded",
                },
                "responseContext": { "serverVersion": null },
            }))
            .unwrap();
            let $result::UserFacingError(error) = user_facing_error else {
                panic!("Expected a user-facing error");
            };
            assert_eq!(
                super::get_user_facing_error_message(error),
                "Budget exceeded",
            );
        }
    };
}

error_response_test!(
    sync_merkle_tree_errors_deserialize,
    SyncMerkleTreeResult,
    SyncMerkleTreeError
);
error_response_test!(
    get_relevant_fragments_errors_deserialize,
    GetRelevantFragmentsResult,
    GetRelevantFragmentsError
);
error_response_test!(
    rerank_fragments_errors_deserialize,
    RerankFragmentsResult,
    RerankFragmentsError
);
error_response_test!(
    update_merkle_tree_errors_deserialize,
    UpdateMerkleTreeResult,
    UpdateMerkleTreeError
);
error_response_test!(
    generate_code_embeddings_errors_deserialize,
    GenerateCodeEmbeddingsResult,
    GenerateCodeEmbeddingsError
);

fn headers_with_rejection(value: &'static str) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(
        ANONYMOUS_SESSION_REJECTED_HEADER,
        HeaderValue::from_static(value),
    );
    headers
}

#[test]
fn unauthorized_with_rejection_header_is_anonymous_session_rejection() {
    assert!(is_anonymous_session_rejection(
        StatusCode::UNAUTHORIZED,
        &headers_with_rejection("true"),
    ));
}

#[test]
fn unauthorized_without_rejection_header_is_not_anonymous_session_rejection() {
    assert!(!is_anonymous_session_rejection(
        StatusCode::UNAUTHORIZED,
        &HeaderMap::new(),
    ));
}

#[test]
fn rejection_header_requires_unauthorized_status() {
    assert!(!is_anonymous_session_rejection(
        StatusCode::FORBIDDEN,
        &headers_with_rejection("true"),
    ));
}

#[test]
fn rejection_header_requires_true_value() {
    assert!(!is_anonymous_session_rejection(
        StatusCode::UNAUTHORIZED,
        &headers_with_rejection("false"),
    ));
}
