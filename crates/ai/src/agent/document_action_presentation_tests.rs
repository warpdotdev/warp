use super::DocumentActionPresentation;
use crate::agent::action::{
    AIAgentActionType, CreateDocumentsRequest, DocumentDiff, DocumentToCreate, EditDocumentsRequest,
};
use crate::agent::action_result::{
    AIAgentActionResultType, CreateDocumentsResult, DocumentContext, EditDocumentsResult,
};
use crate::document::{AIDocumentId, AIDocumentVersion, DEFAULT_PLANNING_DOCUMENT_TITLE};

#[test]
fn streamed_create_uses_canonical_fallback_titles() {
    let action = create_action([("", "first\nbody"), ("Named plan", "second")]);

    let presentation = DocumentActionPresentation::resolve(&action, None).unwrap();

    assert_eq!(
        presentation
            .documents
            .iter()
            .map(|document| document.title.as_str())
            .collect::<Vec<_>>(),
        vec!["Document 1", "Named plan"]
    );
}

#[test]
fn completed_create_combines_request_titles_with_result_documents() {
    let action = create_action([("", "streamed")]);
    let document_id = AIDocumentId::new();
    let result = AIAgentActionResultType::CreateDocuments(CreateDocumentsResult::Success {
        created_documents: vec![document_context(document_id, "final")],
    });

    let presentation = DocumentActionPresentation::resolve(&action, Some(&result)).unwrap();
    let document = &presentation.documents[0];

    assert_eq!(document.title, DEFAULT_PLANNING_DOCUMENT_TITLE);
    assert_eq!(document.content, "final");
    assert_eq!(document.document_id, Some(document_id));
    assert_eq!(
        document.document_version,
        Some(AIDocumentVersion::default())
    );
}

#[test]
fn completed_edit_uses_result_content_and_references() {
    let document_id = AIDocumentId::new();
    let action = AIAgentActionType::EditDocuments(EditDocumentsRequest {
        diffs: vec![DocumentDiff {
            document_id,
            search: "old".to_owned(),
            replace: "new".to_owned(),
        }],
    });
    let result = AIAgentActionResultType::EditDocuments(EditDocumentsResult::Success {
        updated_documents: vec![document_context(document_id, "updated")],
    });

    let presentation = DocumentActionPresentation::resolve(&action, Some(&result)).unwrap();

    assert_eq!(presentation.documents[0].content, "updated");
}

#[test]
fn unsuccessful_result_discards_streamed_documents() {
    let action = create_action([("Plan", "streamed")]);
    let result = AIAgentActionResultType::CreateDocuments(CreateDocumentsResult::Cancelled);

    let presentation = DocumentActionPresentation::resolve(&action, Some(&result)).unwrap();

    assert!(presentation.documents.is_empty());
}

fn create_action<const N: usize>(documents: [(&str, &str); N]) -> AIAgentActionType {
    AIAgentActionType::CreateDocuments(CreateDocumentsRequest {
        documents: documents
            .into_iter()
            .map(|(title, content)| DocumentToCreate {
                title: title.to_owned(),
                content: content.to_owned(),
            })
            .collect(),
    })
}

fn document_context(document_id: AIDocumentId, content: &str) -> DocumentContext {
    DocumentContext {
        document_id,
        document_version: AIDocumentVersion::default(),
        content: content.to_owned(),
        line_ranges: Vec::new(),
    }
}
