use std::path::Path;

use url::Url;

use super::*;

fn downloaded(file_name: &str, file_path: &Path) -> DownloadedAttachment {
    DownloadedAttachment {
        file_id: "att-1".to_owned(),
        file_name: file_name.to_owned(),
        file_path: file_path.to_string_lossy().into_owned(),
    }
}

#[test]
fn text_only_prompt_is_a_single_text_block() {
    let blocks = prompt_content("hello".to_owned(), Vec::new(), &[]);
    assert_eq!(blocks.len(), 1);
    assert!(matches!(&blocks[0], ContentBlock::Text { text } if text == "hello"));
}

#[test]
fn attachments_follow_the_prompt_text_in_order() {
    let dir = std::env::temp_dir().join("acp-attachments-test");
    let screenshot = dir.join("screen shot.png");
    let blocks = prompt_content(
        "look at this".to_owned(),
        vec!["fn main() {}".to_owned()],
        &[downloaded("screen shot.png", &screenshot)],
    );

    assert_eq!(blocks.len(), 3);
    assert!(matches!(&blocks[0], ContentBlock::Text { text } if text == "look at this"));
    assert!(matches!(
        &blocks[1],
        ContentBlock::Text { text } if text == "<attached_text>\nfn main() {}\n</attached_text>"
    ));
    let ContentBlock::ResourceLink { uri, name } = &blocks[2] else {
        panic!("expected a resource link, got {:?}", blocks[2]);
    };
    assert_eq!(name.as_deref(), Some("screen shot.png"));
    let round_tripped = Url::parse(uri)
        .expect("resource link is a valid URL")
        .to_file_path()
        .expect("resource link is a file URL");
    assert_eq!(round_tripped, screenshot);
}

#[test]
fn resource_links_serialize_in_acp_wire_shape() {
    let dir = std::env::temp_dir().join("acp-attachments-test");
    let blocks = prompt_content(
        "p".to_owned(),
        Vec::new(),
        &[downloaded("notes.txt", &dir.join("notes.txt"))],
    );
    let json = serde_json::to_value(&blocks[1]).unwrap();
    assert_eq!(json["type"], "resource_link");
    assert_eq!(json["name"], "notes.txt");
    assert!(json["uri"].as_str().unwrap().starts_with("file://"));
}

#[test]
fn relative_paths_still_produce_a_file_uri() {
    assert_eq!(
        file_uri(Path::new("relative/x.png")),
        "file://relative/x.png"
    );
}
