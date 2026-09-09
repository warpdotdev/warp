use super::NewlineNormalizer;

fn normalize_chunks(chunks: &[&[u8]]) -> Vec<u8> {
    let mut normalizer = NewlineNormalizer::new();
    let mut out = Vec::new();
    for chunk in chunks {
        out.extend(normalizer.push(chunk));
    }
    out.extend(normalizer.finish());
    out
}

#[test]
fn devcontainer_text_stream_normalizes_newlines_across_chunks() {
    let chunks: &[&[u8]] = &[b"first\r", b"\nsecond\n", b"third\r", b"fourth\n"];
    assert_eq!(
        normalize_chunks(chunks),
        b"first\r\nsecond\r\nthird\rfourth\r\n"
    );

    assert_eq!(
        normalize_chunks(&[b"hello\nworld\n"]),
        b"hello\r\nworld\r\n"
    );
    assert_eq!(normalize_chunks(&[b"keep\r\ncrlf"]), b"keep\r\ncrlf");
    assert_eq!(normalize_chunks(&[b"stand\ralone"]), b"stand\ralone");
    assert_eq!(normalize_chunks(&[b"", b"x", b""]), b"x");
    assert_eq!(normalize_chunks(&[b"ends-with-cr\r"]), b"ends-with-cr\r");
    assert_eq!(normalize_chunks(&[b"split\r", b"\nline"]), b"split\r\nline");
    assert_eq!(
        normalize_chunks(&[b"lone-lf-at-end\n", b"next"]),
        b"lone-lf-at-end\r\nnext"
    );
}
