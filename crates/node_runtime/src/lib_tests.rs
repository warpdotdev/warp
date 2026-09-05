use super::{extract_zip, extract_zip_with_limits};
use std::io::Write as _;
use std::path::PathBuf;

/// Builds an in-memory zip archive from `(name, contents)` entries using the
/// `Stored` method so an entry's declared uncompressed size equals its length.
fn make_zip(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut buf = std::io::Cursor::new(Vec::new());
    {
        let mut writer = zip::ZipWriter::new(&mut buf);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        for (name, data) in entries {
            writer.start_file(*name, options).unwrap();
            writer.write_all(data).unwrap();
        }
        writer.finish().unwrap();
    }
    buf.into_inner()
}

/// Creates a unique temporary directory for a test and returns its path.
fn unique_temp_dir(tag: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let mut dir = std::env::temp_dir();
    dir.push(format!(
        "node_runtime_ziptest_{tag}_{}_{nanos}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

const NO_FILTER: Option<fn(&str) -> bool> = None;

#[test]
fn extracts_all_entries_streaming() {
    let zip = make_zip(&[
        ("bin/tool", b"hello world"),
        ("lib/data.bin", b"some other bytes"),
    ]);
    let dest = unique_temp_dir("all");

    futures::executor::block_on(extract_zip(&zip, &dest, NO_FILTER)).unwrap();

    assert_eq!(
        std::fs::read(dest.join("bin/tool")).unwrap(),
        b"hello world"
    );
    assert_eq!(
        std::fs::read(dest.join("lib/data.bin")).unwrap(),
        b"some other bytes"
    );

    std::fs::remove_dir_all(&dest).ok();
}

#[test]
fn respects_file_filter() {
    let zip = make_zip(&[("bin/clangd", b"binary"), ("README.md", b"docs")]);
    let dest = unique_temp_dir("filter");

    futures::executor::block_on(extract_zip(
        &zip,
        &dest,
        Some(|name: &str| name.ends_with("clangd")),
    ))
    .unwrap();

    assert!(dest.join("bin/clangd").exists());
    assert!(!dest.join("README.md").exists());

    std::fs::remove_dir_all(&dest).ok();
}

#[test]
fn errors_when_filter_matches_nothing() {
    let zip = make_zip(&[("README.md", b"docs")]);
    let dest = unique_temp_dir("nomatch");

    let result = futures::executor::block_on(extract_zip(
        &zip,
        &dest,
        Some(|name: &str| name.ends_with("clangd")),
    ));

    assert!(result.is_err(), "expected an error when nothing matches");
    std::fs::remove_dir_all(&dest).ok();
}

#[test]
fn rejects_entry_exceeding_per_entry_limit() {
    // A 100-byte entry against a 10-byte per-entry cap must be rejected.
    let zip = make_zip(&[("big.bin", &[0u8; 100])]);
    let dest = unique_temp_dir("entrycap");

    let result = futures::executor::block_on(extract_zip_with_limits(
        &zip, &dest, NO_FILTER, 10, 1_000_000,
    ));

    assert!(
        result.is_err(),
        "expected per-entry limit to reject the oversized entry"
    );
    // Nothing should have been written to disk.
    assert!(!dest.join("big.bin").exists());

    std::fs::remove_dir_all(&dest).ok();
}

#[test]
fn rejects_when_total_limit_exceeded() {
    // Two 100-byte entries: each is under the per-entry cap, but together they
    // exceed the 150-byte total cap.
    let zip = make_zip(&[("a.bin", &[1u8; 100]), ("b.bin", &[2u8; 100])]);
    let dest = unique_temp_dir("totalcap");

    let result =
        futures::executor::block_on(extract_zip_with_limits(&zip, &dest, NO_FILTER, 1_000, 150));

    assert!(
        result.is_err(),
        "expected total limit to reject once the cumulative size is exceeded"
    );

    std::fs::remove_dir_all(&dest).ok();
}
