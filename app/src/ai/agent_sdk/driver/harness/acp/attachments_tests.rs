use std::path::Path;

use base64::Engine as _;
use url::Url;

use super::*;

const PNG_HEADER: &[u8] = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR";

fn image_capable() -> PromptCapabilities {
    PromptCapabilities {
        image: true,
        audio: false,
        embedded_context: false,
    }
}

fn context_capable() -> PromptCapabilities {
    PromptCapabilities {
        image: false,
        audio: false,
        embedded_context: true,
    }
}

fn resolved(name: &str, mime_type: &str, contents: Option<Vec<u8>>) -> ResolvedFile {
    ResolvedFile {
        name: name.to_owned(),
        path: std::env::temp_dir().join("acp-attachments-test").join(name),
        mime_type: mime_type.to_owned(),
        contents,
    }
}

fn png(contents: Option<Vec<u8>>) -> ResolvedFile {
    resolved("screen shot.png", "image/png", contents)
}

fn text_file(contents: &str) -> ResolvedFile {
    resolved(
        "notes.md",
        "text/markdown",
        Some(contents.as_bytes().to_vec()),
    )
}

fn assert_link(block: &ContentBlock, expected: &ResolvedFile) {
    let ContentBlock::ResourceLink { uri, name } = block else {
        panic!("expected a resource link, got {block:?}");
    };
    assert_eq!(name.as_deref(), Some(expected.name.as_str()));
    let round_tripped = Url::parse(uri)
        .expect("resource link is a valid URL")
        .to_file_path()
        .expect("resource link is a file URL");
    assert_eq!(round_tripped, expected.path);
}

#[test]
fn text_only_prompt_is_a_single_text_block() {
    let blocks = prompt_content(
        PromptCapabilities::default(),
        "hello".to_owned(),
        Vec::new(),
        &[],
    );
    assert_eq!(blocks.len(), 1);
    assert!(matches!(&blocks[0], ContentBlock::Text { text } if text == "hello"));
}

#[test]
fn attachments_follow_the_prompt_text_in_order() {
    let screenshot = png(None);
    let blocks = prompt_content(
        PromptCapabilities::default(),
        "look at this".to_owned(),
        vec!["fn main() {}".to_owned()],
        std::slice::from_ref(&screenshot),
    );

    assert_eq!(blocks.len(), 3);
    assert!(matches!(&blocks[0], ContentBlock::Text { text } if text == "look at this"));
    assert!(matches!(
        &blocks[1],
        ContentBlock::Text { text } if text == "<attached_text>\nfn main() {}\n</attached_text>"
    ));
    assert_link(&blocks[2], &screenshot);
}

#[test]
fn image_is_inlined_when_the_agent_accepts_images() {
    let file = png(Some(PNG_HEADER.to_vec()));
    let ContentBlock::Image { data, mime_type } = file_block(image_capable(), &file) else {
        panic!("expected an inline image");
    };
    assert_eq!(mime_type, "image/png");
    assert_eq!(
        base64::engine::general_purpose::STANDARD
            .decode(data)
            .unwrap(),
        PNG_HEADER
    );
}

#[test]
fn image_is_linked_when_the_agent_does_not_accept_images() {
    let file = png(Some(PNG_HEADER.to_vec()));
    assert_link(&file_block(context_capable(), &file), &file);
    assert_link(&file_block(PromptCapabilities::default(), &file), &file);
}

#[test]
fn oversize_image_is_linked_even_when_images_are_accepted() {
    let file = png(Some(vec![0; MAX_INLINE_IMAGE_BYTES + 1]));
    assert_link(&file_block(image_capable(), &file), &file);
}

#[test]
fn text_file_is_embedded_when_the_agent_accepts_embedded_context() {
    let file = text_file("# Notes\n");
    let ContentBlock::Resource { resource } = file_block(context_capable(), &file) else {
        panic!("expected an embedded resource");
    };
    assert_eq!(resource.text.as_deref(), Some("# Notes\n"));
    assert_eq!(resource.mime_type.as_deref(), Some("text/markdown"));
    assert_eq!(
        Url::parse(&resource.uri).unwrap().to_file_path().unwrap(),
        file.path
    );
    assert!(resource.blob.is_none());
}

#[test]
fn text_file_is_linked_when_the_agent_does_not_accept_embedded_context() {
    let file = text_file("# Notes\n");
    assert_link(&file_block(image_capable(), &file), &file);
    assert_link(&file_block(PromptCapabilities::default(), &file), &file);
}

#[test]
fn oversize_text_file_is_linked_even_when_embedded_context_is_accepted() {
    let file = text_file(&"x".repeat(MAX_EMBEDDED_TEXT_BYTES + 1));
    assert_link(&file_block(context_capable(), &file), &file);
}

#[test]
fn binary_file_is_linked_regardless_of_capabilities() {
    let file = resolved(
        "archive.zip",
        "application/zip",
        Some(vec![0x50, 0x4b, 0x03, 0x04, 0xff]),
    );
    let everything = PromptCapabilities {
        image: true,
        audio: true,
        embedded_context: true,
    };
    assert_link(&file_block(everything, &file), &file);
}

#[test]
fn unreadable_file_is_linked() {
    let file = text_file("");
    let file = ResolvedFile {
        contents: None,
        ..file
    };
    assert_link(&file_block(context_capable(), &file), &file);
}

#[test]
fn utf8_file_with_unknown_extension_is_embedded_as_plain_text() {
    let file = resolved(
        "Makefile.custom",
        "application/octet-stream",
        Some(b"all:\n".to_vec()),
    );
    let ContentBlock::Resource { resource } = file_block(context_capable(), &file) else {
        panic!("expected an embedded resource");
    };
    assert_eq!(resource.mime_type.as_deref(), Some("text/plain"));
}

#[test]
fn blocks_serialize_in_acp_wire_shape() {
    let link = serde_json::to_value(file_block(PromptCapabilities::default(), &png(None))).unwrap();
    assert_eq!(link["type"], "resource_link");
    assert_eq!(link["name"], "screen shot.png");
    assert!(link["uri"].as_str().unwrap().starts_with("file://"));

    let image =
        serde_json::to_value(file_block(image_capable(), &png(Some(PNG_HEADER.to_vec())))).unwrap();
    assert_eq!(image["type"], "image");
    assert_eq!(image["mimeType"], "image/png");
    assert!(image["data"].is_string());

    let resource = serde_json::to_value(file_block(context_capable(), &text_file("hi"))).unwrap();
    assert_eq!(resource["type"], "resource");
    assert_eq!(resource["resource"]["text"], "hi");
    assert_eq!(resource["resource"]["mimeType"], "text/markdown");
    assert!(resource["resource"].get("blob").is_none());
}

/// A scratch directory removed when dropped, so a failing assertion does not leak files.
struct ScratchDir(std::path::PathBuf);

impl ScratchDir {
    fn new(label: &str) -> Self {
        let dir =
            std::env::temp_dir().join(format!("acp-attachments-{label}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    /// Writes `head` and extends the file with a hole to `len` bytes, so a file larger than
    /// the inline caps costs no disk or time to create.
    fn file(&self, name: &str, head: &[u8], len: u64) -> DownloadedAttachment {
        let path = self.0.join(name);
        std::fs::write(&path, head).unwrap();
        std::fs::OpenOptions::new()
            .write(true)
            .open(&path)
            .unwrap()
            .set_len(len)
            .unwrap();
        DownloadedAttachment {
            file_id: "att-1".to_owned(),
            file_name: name.to_owned(),
            file_path: path.to_string_lossy().into_owned(),
        }
    }
}

impl Drop for ScratchDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn read(file: &DownloadedAttachment, capabilities: PromptCapabilities) -> ResolvedFile {
    futures::executor::block_on(ResolvedFile::read(file, capabilities))
}

#[test]
fn reading_a_file_sniffs_its_mime_type_from_magic_bytes() {
    let dir = ScratchDir::new("sniff");
    let photo = dir.file("photo.txt", PNG_HEADER, PNG_HEADER.len() as u64);

    let resolved = read(&photo, image_capable());
    assert_eq!(resolved.mime_type, "image/png");
    assert_eq!(resolved.contents.as_deref(), Some(PNG_HEADER));
    assert!(matches!(
        file_block(image_capable(), &resolved),
        ContentBlock::Image { .. }
    ));

    let unread = read(&photo, PromptCapabilities::default());
    assert!(
        unread.contents.is_none(),
        "baseline agents never need the bytes"
    );
    assert_eq!(
        unread.mime_type, "text/plain",
        "falls back to the extension without bytes"
    );
}

#[test]
fn oversize_files_are_not_read() {
    let dir = ScratchDir::new("oversize");
    let everything = PromptCapabilities {
        image: true,
        audio: true,
        embedded_context: true,
    };

    let huge_image = dir.file("huge.png", PNG_HEADER, MAX_INLINE_IMAGE_BYTES as u64 + 1);
    let resolved = read(&huge_image, everything);
    assert!(resolved.contents.is_none());
    assert_link(&file_block(everything, &resolved), &resolved);

    let big_text = dir.file("big.txt", b"hello", MAX_EMBEDDED_TEXT_BYTES as u64 + 1);
    let resolved = read(&big_text, everything);
    assert!(
        resolved.contents.is_none(),
        "a text file over the text cap is not read even though it is under the image cap"
    );
    assert_link(&file_block(everything, &resolved), &resolved);

    let image = dir.file("ok.png", PNG_HEADER, MAX_INLINE_IMAGE_BYTES as u64);
    let resolved = read(&image, everything);
    assert_eq!(
        resolved.contents.as_ref().map(Vec::len),
        Some(MAX_INLINE_IMAGE_BYTES)
    );
    assert!(matches!(
        file_block(everything, &resolved),
        ContentBlock::Image { .. }
    ));
}

#[test]
fn files_of_a_kind_the_agent_does_not_take_inline_are_not_read() {
    let dir = ScratchDir::new("kind");
    let text = dir.file("notes.txt", b"hello", 5);
    let resolved = read(&text, image_capable());
    assert!(resolved.contents.is_none());
    assert_eq!(resolved.mime_type, "text/plain");

    let image = dir.file("photo.png", PNG_HEADER, PNG_HEADER.len() as u64);
    let resolved = read(&image, context_capable());
    assert!(resolved.contents.is_none());
    assert_eq!(resolved.mime_type, "image/png");

    let missing = DownloadedAttachment {
        file_id: "att-2".to_owned(),
        file_name: "gone.txt".to_owned(),
        file_path: dir.0.join("gone.txt").to_string_lossy().into_owned(),
    };
    let resolved = read(&missing, context_capable());
    assert!(resolved.contents.is_none());
    assert_link(&file_block(context_capable(), &resolved), &resolved);
}

#[test]
fn relative_paths_still_produce_a_file_uri() {
    assert_eq!(
        file_uri(Path::new("relative/x.png")),
        "file://relative/x.png"
    );
}
