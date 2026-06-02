use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::path::PathBuf;

use string_offset::ByteOffset;

use super::super::{ContentHash, Fragment, FragmentLocation, FragmentMetadata};
use super::{build_fragments_from_file_contents, fragments_to_context_locations};
use crate::index::locations::{CodeContextLocation, FileFragmentLocation};

fn metadata(
    path: &str,
    byte_range: Range<ByteOffset>,
    start_line: usize,
    end_line: usize,
) -> FragmentMetadata {
    FragmentMetadata {
        absolute_path: PathBuf::from(path),
        location: super::super::fragment_metadata::FragmentLocation {
            start_line,
            end_line,
            byte_range,
        },
    }
}

fn fragment(content: &str, path: &str, byte_range: Range<ByteOffset>) -> Fragment {
    Fragment {
        content: content.to_string(),
        content_hash: ContentHash::from_content(content),
        location: FragmentLocation {
            absolute_path: PathBuf::from(path),
            byte_range,
        },
    }
}

#[test]
fn builds_fragments_from_exact_byte_ranges() {
    let path = PathBuf::from("/repo/src/lib.rs");
    let content = "before\nneedle\nπ-after".to_string();
    let fragment_content = "needle";
    let start = content.find(fragment_content).unwrap();
    let end = start + fragment_content.len();
    let content_hash = ContentHash::from_content(fragment_content);
    let metadata = metadata(
        path.to_string_lossy().as_ref(),
        ByteOffset::from(start)..ByteOffset::from(end),
        2,
        2,
    );

    let result = build_fragments_from_file_contents(
        [(content_hash.clone(), metadata)],
        &HashMap::from([(path.clone(), content)]),
    );

    assert_eq!(result.fail_to_read.len(), 0);
    assert_eq!(result.successfully_read.len(), 1);
    let fragment = &result.successfully_read[0];
    assert_eq!(fragment.content, fragment_content);
    assert_eq!(fragment.content_hash, content_hash);
    assert_eq!(fragment.location.absolute_path, path);
}

#[test]
fn rejects_invalid_hashes_and_byte_ranges() {
    let path = PathBuf::from("/repo/src/lib.rs");
    let content = "abcπdef".to_string();
    let bad_hash_metadata = metadata(
        path.to_string_lossy().as_ref(),
        ByteOffset::from(0)..ByteOffset::from(3),
        1,
        1,
    );
    let invalid_boundary_metadata = metadata(
        path.to_string_lossy().as_ref(),
        ByteOffset::from(4)..ByteOffset::from(5),
        1,
        1,
    );

    let result = build_fragments_from_file_contents(
        [
            (ContentHash::from_content("not abc"), bad_hash_metadata),
            (ContentHash::from_content("π"), invalid_boundary_metadata),
        ],
        &HashMap::from([(path.clone(), content)]),
    );

    assert!(result.successfully_read.is_empty());
    assert_eq!(result.fail_to_read.len(), 2);
    assert_eq!(result.fail_to_read_path, vec![path]);
}

#[test]
fn shapes_fragments_into_merged_context_locations() {
    let path = "/repo/src/lib.rs";
    let fragment_a = fragment("a", path, ByteOffset::from(0)..ByteOffset::from(1));
    let fragment_b = fragment("b", path, ByteOffset::from(2)..ByteOffset::from(3));
    let metadata_a = metadata(path, ByteOffset::from(0)..ByteOffset::from(1), 10, 12);
    let metadata_b = metadata(path, ByteOffset::from(2)..ByteOffset::from(3), 15, 17);
    let metadata_by_hash = HashMap::from([
        (fragment_a.content_hash.clone(), vec![metadata_a]),
        (fragment_b.content_hash.clone(), vec![metadata_b]),
    ]);

    let result = fragments_to_context_locations(
        vec![fragment_a, fragment_b],
        |hash| metadata_by_hash.get(hash).map(Vec::as_slice),
        2,
    );

    assert_eq!(
        result,
        HashSet::from([CodeContextLocation::Fragment(FileFragmentLocation {
            path: PathBuf::from(path),
            line_ranges: std::iter::once(8..20).collect(),
        })])
    );
}

#[test]
fn falls_back_to_whole_file_when_metadata_is_missing() {
    let path = "/repo/src/lib.rs";
    let fragment = fragment("a", path, ByteOffset::from(0)..ByteOffset::from(1));
    let result = fragments_to_context_locations(vec![fragment], |_| None, 2);

    assert_eq!(
        result,
        HashSet::from([CodeContextLocation::WholeFile(PathBuf::from(path))])
    );
}
