use serde_json::json;
use url::Url;

use super::*;

#[test]
fn initial_prompt_links_files_after_server_composed_text() {
    let dir = std::env::temp_dir().join("acp-initial-attachments");
    let screenshot = dir.join("image #雪%.png");
    let notes = dir.join("notes.txt");
    let mut harness = AcpHarness::new(
        Harness::Claude,
        AcpLaunchSpec::for_harness(Harness::Claude).unwrap(),
    );
    *harness.initial_attachments_mut().unwrap() = vec![
        DownloadedAttachment {
            file_id: "image-id".to_owned(),
            file_name: "image #雪%.png".to_owned(),
            file_path: screenshot.to_string_lossy().into_owned(),
        },
        DownloadedAttachment {
            file_id: "notes-id".to_owned(),
            file_name: "notes.txt".to_owned(),
            file_path: notes.to_string_lossy().into_owned(),
        },
    ];

    let blocks = harness.initial_prompt(
        "inspect these",
        Some("system"),
        Some("recovery"),
        Some("context"),
    );

    assert_eq!(blocks.len(), 3);
    let ContentBlock::ResourceLink { uri: image_uri, .. } = &blocks[1] else {
        panic!("expected an image resource link");
    };
    let ContentBlock::ResourceLink { uri: notes_uri, .. } = &blocks[2] else {
        panic!("expected a non-image resource link");
    };
    assert_eq!(
        serde_json::to_value(&blocks).unwrap(),
        json!([
            {"type": "text", "text": "recovery\n\ncontext\n\nsystem\n\ninspect these"},
            {"type": "resource_link", "uri": image_uri, "name": "image #雪%.png"},
            {"type": "resource_link", "uri": notes_uri, "name": "notes.txt"}
        ])
    );
    assert_eq!(
        Url::parse(image_uri).unwrap().to_file_path().unwrap(),
        screenshot
    );
    assert_eq!(
        Url::parse(notes_uri).unwrap().to_file_path().unwrap(),
        notes
    );
    assert!(image_uri.contains("image%20%23%E9%9B%AA%25.png"));
}

#[test]
fn initial_prompt_without_files_keeps_the_existing_text() {
    let harness = AcpHarness::new(
        Harness::Codex,
        AcpLaunchSpec::for_harness(Harness::Codex).unwrap(),
    );

    let blocks = harness.initial_prompt("hello", Some("staged in config"), Some(" "), None);

    assert_eq!(
        serde_json::to_value(blocks).unwrap(),
        json!([{"type": "text", "text": "hello"}])
    );
}
