use remote_server::proto::{file_context_proto, FileContextProto};
use warp_util::host_id::HostId;
use warp_util::local_or_remote_path::LocalOrRemotePath;
use warp_util::remote_path::RemotePath;
use warp_util::standardized_path::StandardizedPath;

use super::{pair_remote_rule_paths_with_contents, remote_rule_read_request};

fn remote_rule_path(host_id: &HostId, name: &str) -> LocalOrRemotePath {
    LocalOrRemotePath::Remote(RemotePath::new(
        host_id.clone(),
        StandardizedPath::try_new(format!("/repo/{name}").as_str()).unwrap(),
    ))
}

fn remote_rule_file_context(path: &LocalOrRemotePath, content: &str) -> FileContextProto {
    let LocalOrRemotePath::Remote(remote) = path else {
        panic!("Expected a remote rule path");
    };

    FileContextProto {
        file_name: remote.path.as_str().to_string(),
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
fn remote_rule_contents_match_reordered_responses_by_path() {
    let host = HostId::new("test-host".to_string());
    let first_path = remote_rule_path(&host, "WARP.md");
    let second_path = remote_rule_path(&host, "nested/AGENTS.md");

    let contents = pair_remote_rule_paths_with_contents(
        vec![first_path.clone(), second_path.clone()],
        vec![
            remote_rule_file_context(&second_path, "second rules"),
            remote_rule_file_context(&first_path, "first rules"),
        ],
    );

    assert_eq!(
        contents,
        vec![
            (first_path, "first rules".to_string()),
            (second_path, "second rules".to_string()),
        ]
    );
}

#[test]
fn remote_rule_contents_keep_paths_aligned_after_missing_reads() {
    let host = HostId::new("test-host".to_string());
    let missing_path = remote_rule_path(&host, "WARP.md");
    let present_path = remote_rule_path(&host, "nested/AGENTS.md");

    let contents = pair_remote_rule_paths_with_contents(
        vec![missing_path, present_path.clone()],
        vec![remote_rule_file_context(&present_path, "present rules")],
    );

    assert_eq!(contents, vec![(present_path, "present rules".to_string())]);
}

#[test]
fn remote_rule_read_request_preserves_discovered_paths() {
    let host = HostId::new("test-host".to_string());
    let first_path = remote_rule_path(&host, "WARP.md");
    let second_path = remote_rule_path(&host, "nested/AGENTS.md");

    let request = remote_rule_read_request(&[first_path.clone(), second_path.clone()]);

    assert_eq!(request.max_file_bytes, None);
    assert_eq!(request.max_batch_bytes, None);
    assert_eq!(request.files.len(), 2);
    let LocalOrRemotePath::Remote(first_remote) = first_path else {
        panic!("Expected a remote rule path");
    };
    let LocalOrRemotePath::Remote(second_remote) = second_path else {
        panic!("Expected a remote rule path");
    };
    assert_eq!(request.files[0].path, first_remote.path.as_str());
    assert_eq!(request.files[1].path, second_remote.path.as_str());
}
