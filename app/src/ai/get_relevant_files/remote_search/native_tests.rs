use std::path::PathBuf;

use remote_server::proto::{FileContextProto, ReadFileContextResponse, file_context_proto};

use super::file_contents_from_response;

#[test]
fn file_contents_from_response_keeps_only_whole_text_files() {
    let response = ReadFileContextResponse {
        file_contexts: vec![
            FileContextProto {
                file_name: "/repo/src/lib.rs".to_string(),
                content: Some(file_context_proto::Content::TextContent(
                    "content".to_string(),
                )),
                line_range_start: None,
                line_range_end: None,
                last_modified_epoch_millis: None,
                line_count: 1,
            },
            FileContextProto {
                file_name: "/repo/src/fragment.rs".to_string(),
                content: Some(file_context_proto::Content::TextContent(
                    "fragment".to_string(),
                )),
                line_range_start: Some(1),
                line_range_end: Some(2),
                last_modified_epoch_millis: None,
                line_count: 1,
            },
        ],
        failed_files: vec![],
    };

    let file_contents = file_contents_from_response(response);

    assert_eq!(file_contents.len(), 1);
    assert_eq!(
        file_contents.get(&PathBuf::from("/repo/src/lib.rs")),
        Some(&"content".to_string())
    );
}
