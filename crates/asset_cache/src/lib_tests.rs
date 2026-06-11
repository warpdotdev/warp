use super::*;

/// Drive the fetch closure of an [`AssetSource::Async`] to completion.
fn fetch_bytes(source: &AssetSource) -> Result<Bytes> {
    match source {
        AssetSource::Async { fetch, .. } => futures::executor::block_on(fetch()),
        other => panic!("expected an Async source, got {other:?}"),
    }
}

#[test]
fn data_uri_source_decodes_base64_payload() {
    let source = data_uri_source("data:image/png;base64,iVBORw0KGgo=")
        .expect("base64 data URI should produce a source");
    let bytes = fetch_bytes(&source).expect("payload should decode");
    assert_eq!(
        bytes.as_ref(),
        BASE64_STANDARD.decode("iVBORw0KGgo=").unwrap().as_slice()
    );
}

#[test]
fn data_uri_source_strips_embedded_whitespace() {
    // base64 payloads saved in notebooks frequently contain newlines.
    let source =
        data_uri_source("data:image/png;base64,iVBO\nRw0K Ggo=").expect("should produce a source");
    let bytes = fetch_bytes(&source).expect("payload should decode after stripping whitespace");
    assert_eq!(
        bytes.as_ref(),
        BASE64_STANDARD.decode("iVBORw0KGgo=").unwrap().as_slice()
    );
}

#[test]
fn data_uri_source_rejects_non_base64_data_uris() {
    assert!(data_uri_source("https://example.com/a.png").is_none());
    assert!(data_uri_source("/abs/path.png").is_none());
    assert!(data_uri_source("relative/path.png").is_none());
    // A `data:` URI without the `;base64` marker is not a renderable asset.
    assert!(data_uri_source("data:text/plain,hello").is_none());
    // A `data:` URI without a comma separator is malformed.
    assert!(data_uri_source("data:image/png;base64").is_none());
}

#[test]
fn data_uri_source_invalid_base64_fails_on_fetch() {
    // Detection succeeds on the prefix/marker, but decoding the bad payload
    // surfaces as a fetch error (FailedToLoad) rather than a panic.
    let source =
        data_uri_source("data:image/png;base64,not valid base64!").expect("detected as data URI");
    assert!(fetch_bytes(&source).is_err());
}
