use remote_server::proto::{FileContextProto, file_context_proto};
use warp_util::host_id::HostId;
use warp_util::local_or_remote_path::LocalOrRemotePath;
use warp_util::remote_path::RemotePath;
use warp_util::standardized_path::StandardizedPath;

use super::*;

fn remote_path(host_id: &HostId, path: &str) -> LocalOrRemotePath {
    LocalOrRemotePath::Remote(RemotePath::new(
        host_id.clone(),
        StandardizedPath::try_new(path).unwrap(),
    ))
}

fn text_context(path: &LocalOrRemotePath, content: &str) -> FileContextProto {
    FileContextProto {
        file_name: path.as_remote().unwrap().path.as_str().to_string(),
        content: Some(file_context_proto::Content::TextContent(
            content.to_string(),
        )),
        line_range_start: None,
        line_range_end: None,
        last_modified_epoch_millis: None,
        line_count: content.lines().count() as u32,
    }
}

#[test]
fn remote_text_file_read_request_preserves_paths_and_limits() {
    let host_id = HostId::new("test-host".to_string());
    let first_path = remote_path(&host_id, "/repo/WARP.md");
    let second_path = remote_path(&host_id, "/repo/.agents/skills/test/SKILL.md");

    let request = remote_text_file_read_request(
        &[first_path.clone(), second_path.clone()],
        Some(REMOTE_CONTEXT_MAX_FILE_BYTES),
        Some(REMOTE_CONTEXT_MAX_BATCH_BYTES),
    );

    assert_eq!(request.max_file_bytes, Some(REMOTE_CONTEXT_MAX_FILE_BYTES));
    assert_eq!(
        request.max_batch_bytes,
        Some(REMOTE_CONTEXT_MAX_BATCH_BYTES)
    );
    assert!(request.files.iter().all(|file| file.line_ranges.is_empty()));
    assert_eq!(
        request
            .files
            .iter()
            .map(|file| file.path.as_str())
            .collect::<Vec<_>>(),
        vec![
            first_path.as_remote().unwrap().path.as_str(),
            second_path.as_remote().unwrap().path.as_str(),
        ]
    );

    let request_with_server_defaults =
        remote_text_file_read_request(&[first_path, second_path], None, None);
    assert_eq!(request_with_server_defaults.max_file_bytes, None);
    assert_eq!(request_with_server_defaults.max_batch_bytes, None);
}

#[test]
fn remote_text_file_contents_match_reordered_partial_responses_by_path() {
    let host_id = HostId::new("test-host".to_string());
    let first_path = remote_path(&host_id, "/repo/WARP.md");
    let missing_path = remote_path(&host_id, "/repo/AGENTS.md");
    let third_path = remote_path(&host_id, "/repo/.agents/skills/test/SKILL.md");

    let contents = pair_remote_text_file_contents(
        vec![first_path.clone(), missing_path, third_path.clone()],
        vec![
            text_context(&third_path, "third"),
            text_context(&first_path, "first"),
        ],
    );

    assert_eq!(
        contents,
        vec![
            (first_path, "first".to_string()),
            (third_path, "third".to_string()),
        ]
    );
}

#[test]
fn remote_text_file_contents_omit_non_text_responses() {
    let host_id = HostId::new("test-host".to_string());
    let path = remote_path(&host_id, "/repo/image.png");
    let context = FileContextProto {
        file_name: path.as_remote().unwrap().path.as_str().to_string(),
        content: None,
        line_range_start: None,
        line_range_end: None,
        last_modified_epoch_millis: None,
        line_count: 0,
    };

    assert!(pair_remote_text_file_contents(vec![path], vec![context]).is_empty());
}

#[test]
fn remote_text_file_contents_reject_mixed_locations() {
    let first_host = HostId::new("first-host".to_string());
    let second_host = HostId::new("second-host".to_string());
    let mixed_host_error = remote_context_host_id(&[
        remote_path(&first_host, "/repo/WARP.md"),
        remote_path(&second_host, "/repo/WARP.md"),
    ])
    .unwrap_err();
    assert_eq!(
        mixed_host_error.to_string(),
        "Remote context paths span multiple locations"
    );

    let mixed_local_remote_error = remote_context_host_id(&[
        remote_path(&first_host, "/repo/WARP.md"),
        LocalOrRemotePath::Local("/repo/AGENTS.md".into()),
    ])
    .unwrap_err();
    assert_eq!(
        mixed_local_remote_error.to_string(),
        "Remote context paths span multiple locations"
    );
}
