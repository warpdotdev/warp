use cynic::{GraphQlResponse, MutationBuilder};
use serde_json::json;

use super::*;
use crate::request_context::{ClientContext, OsContext};

fn request_context() -> RequestContext {
    RequestContext {
        client_context: ClientContext { version: None },
        os_context: OsContext {
            category: None,
            linux_kernel_version: None,
            name: None,
            version: None,
        },
    }
}

#[test]
fn builds_the_report_mutation() {
    let operation = ReportHarnessTransport::build(ReportHarnessTransportVariables {
        input: ReportHarnessTransportInput {
            transport: HarnessTransport::Acp,
            fallback_reason: None,
        },
        request_context: request_context(),
    });

    assert!(
        operation
            .query
            .starts_with("mutation ReportHarnessTransport(")
    );
    assert!(
        operation
            .query
            .contains("reportHarnessTransport(input: $input, requestContext: $requestContext)")
    );
    assert!(
        operation
            .query
            .contains("... on ReportHarnessTransportOutput")
    );
    assert!(operation.query.contains("... on UserFacingError"));
}

#[test]
fn serializes_the_transport_and_omits_a_missing_reason() {
    let operation = ReportHarnessTransport::build(ReportHarnessTransportVariables {
        input: ReportHarnessTransportInput {
            transport: HarnessTransport::Acp,
            fallback_reason: None,
        },
        request_context: request_context(),
    });

    let request = serde_json::to_value(operation).unwrap();
    assert_eq!(request["variables"]["input"], json!({"transport": "ACP"}));
}

#[test]
fn serializes_each_fallback_reason() {
    for (reason, wire) in [
        (HarnessTransportFallbackReason::NoSidecar, "NO_SIDECAR"),
        (
            HarnessTransportFallbackReason::SidecarWithoutAdapter,
            "SIDECAR_WITHOUT_ADAPTER",
        ),
    ] {
        let operation = ReportHarnessTransport::build(ReportHarnessTransportVariables {
            input: ReportHarnessTransportInput {
                transport: HarnessTransport::Pty,
                fallback_reason: Some(reason),
            },
            request_context: request_context(),
        });

        let request = serde_json::to_value(operation).unwrap();
        assert_eq!(
            request["variables"]["input"],
            json!({"transport": "PTY", "fallbackReason": wire})
        );
    }
}

#[test]
fn deserializes_the_output_and_a_rejection() {
    let output: GraphQlResponse<ReportHarnessTransport> = serde_json::from_value(json!({
        "data": {
            "reportHarnessTransport": {
                "__typename": "ReportHarnessTransportOutput",
                "responseContext": {"serverVersion": null}
            }
        }
    }))
    .unwrap();
    assert!(matches!(
        output.data.unwrap().report_harness_transport,
        ReportHarnessTransportResult::ReportHarnessTransportOutput(_)
    ));

    let rejected: GraphQlResponse<ReportHarnessTransport> = serde_json::from_value(json!({
        "data": {
            "reportHarnessTransport": {
                "__typename": "UserFacingError",
                "error": {"__typename": "InvalidInputError", "message": "fixed transport"},
                "responseContext": {"serverVersion": null}
            }
        }
    }))
    .unwrap();
    assert!(matches!(
        rejected.data.unwrap().report_harness_transport,
        ReportHarnessTransportResult::UserFacingError(_)
    ));
}
