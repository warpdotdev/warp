use super::*;
use crate::util::git::{
    parse_range, parse_unified_diff_header, sort_branches_main_first, BranchEntry,
};

#[test]
fn test_parse_range_with_comma() {
    let (start, count) =
        parse_range("10,5").expect("parse_range should succeed for range with count");
    assert_eq!(start, 10);
    assert_eq!(count, 5);
}

#[test]
fn test_parse_range_without_comma() {
    let (start, count) =
        parse_range("10").expect("parse_range should succeed for range without count");
    assert_eq!(start, 10);
    assert_eq!(count, 1);
}

#[test]
fn test_parse_unified_diff_header_basic() {
    let header = "@@ -10,5 +12,7 @@";
    let parsed = parse_unified_diff_header(header)
        .expect("parse_unified_diff_header should succeed for basic header");
    assert_eq!(parsed.old_start_line, 10);
    assert_eq!(parsed.old_line_count, 5);
    assert_eq!(parsed.new_start_line, 12);
    assert_eq!(parsed.new_line_count, 7);
}

#[test]
fn test_parse_unified_diff_header_with_context() {
    let header = "@@ -4978,33 +4978,43 @@ impl TerminalView {";
    let parsed = parse_unified_diff_header(header)
        .expect("parse_unified_diff_header should succeed for header with context");
    assert_eq!(parsed.old_start_line, 4978);
    assert_eq!(parsed.old_line_count, 33);
    assert_eq!(parsed.new_start_line, 4978);
    assert_eq!(parsed.new_line_count, 43);
}

#[test]
fn test_parse_unified_diff_header_single_line() {
    let header = "@@ -10 +12,3 @@";
    let parsed = parse_unified_diff_header(header)
        .expect("parse_unified_diff_header should succeed for single line header");
    assert_eq!(parsed.old_start_line, 10);
    assert_eq!(parsed.old_line_count, 1);
    assert_eq!(parsed.new_start_line, 12);
    assert_eq!(parsed.new_line_count, 3);
}

#[test]
fn test_sort_branches_main_first_empty() {
    let branches: Vec<BranchEntry> = vec![];
    let result: Vec<_> = sort_branches_main_first(&branches).collect();
    assert!(result.is_empty());
}

#[test]
fn test_sort_branches_main_first_no_main() {
    let branches = vec![
        BranchEntry {
            name: "feature-a".to_string(),
            is_main: false,
        },
        BranchEntry {
            name: "feature-b".to_string(),
            is_main: false,
        },
        BranchEntry {
            name: "feature-c".to_string(),
            is_main: false,
        },
    ];
    let result: Vec<_> = sort_branches_main_first(&branches).collect();
    // No main branches — order should be unchanged.
    assert_eq!(result, branches.iter().collect::<Vec<_>>());
}

#[test]
fn test_sort_branches_main_first_promotes_main() {
    let branches = vec![
        BranchEntry {
            name: "feature-a".to_string(),
            is_main: false,
        },
        BranchEntry {
            name: "main".to_string(),
            is_main: true,
        },
        BranchEntry {
            name: "feature-b".to_string(),
            is_main: false,
        },
    ];
    let result: Vec<_> = sort_branches_main_first(&branches)
        .map(|entry| entry.name.as_str())
        .collect();
    assert_eq!(result, vec!["main", "feature-a", "feature-b"]);
}

#[test]
fn test_sort_branches_main_first_main_already_first() {
    let branches = vec![
        BranchEntry {
            name: "main".to_string(),
            is_main: true,
        },
        BranchEntry {
            name: "feature-a".to_string(),
            is_main: false,
        },
        BranchEntry {
            name: "feature-b".to_string(),
            is_main: false,
        },
    ];
    let result: Vec<_> = sort_branches_main_first(&branches)
        .map(|entry| entry.name.as_str())
        .collect();
    assert_eq!(result, vec!["main", "feature-a", "feature-b"]);
}

#[test]
fn test_sort_branches_main_first_preserves_recency_order_for_non_main() {
    // Non-main branches should remain in their original (recency) order.
    let branches = vec![
        BranchEntry {
            name: "recent-feature".to_string(),
            is_main: false,
        },
        BranchEntry {
            name: "main".to_string(),
            is_main: true,
        },
        BranchEntry {
            name: "older-feature".to_string(),
            is_main: false,
        },
        BranchEntry {
            name: "oldest-feature".to_string(),
            is_main: false,
        },
    ];
    let result: Vec<_> = sort_branches_main_first(&branches)
        .map(|entry| entry.name.as_str())
        .collect();
    assert_eq!(
        result,
        vec!["main", "recent-feature", "older-feature", "oldest-feature"]
    );
}

#[test]
fn test_sort_branches_main_first_multiple_main_flags() {
    // Defensive: both flagged as main (shouldn't happen in practice, but
    // sort_branches_main_first should handle it gracefully).
    let branches = vec![
        BranchEntry {
            name: "feature".to_string(),
            is_main: false,
        },
        BranchEntry {
            name: "main".to_string(),
            is_main: true,
        },
        BranchEntry {
            name: "master".to_string(),
            is_main: true,
        },
    ];
    let result: Vec<_> = sort_branches_main_first(&branches)
        .map(|entry| entry.name.as_str())
        .collect();
    // Both main-flagged entries appear first, non-main last.
    assert_eq!(result, vec!["main", "master", "feature"]);
}

#[test]
fn test_parse_unified_diff_header_malformed() {
    let header = "not a diff header";
    let result = parse_unified_diff_header(header);
    assert!(result.is_err());

    let header2 = "@@ incomplete";
    let result2 = parse_unified_diff_header(header2);
    assert!(result2.is_err());
}

#[test]
fn test_parse_git_status_modified_file_with_spaces() {
    // Porcelain v2 output for a modified file with spaces in the name.
    // Format: 1 <XY> <sub> <mH> <mI> <mW> <hH> <hI> <path>
    let status_output = "1 .M N... 100644 100644 100644 abc1234 def5678 test file.txt";
    let result = LocalDiffStateModel::parse_git_status(status_output).unwrap();
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].0, "test file.txt");
    assert_eq!(result[0].1, GitFileStatus::Modified);
}

#[test]
fn test_parse_git_status_modified_file_with_multiple_spaces() {
    // Filename with multiple spaces.
    let status_output = "1 .M N... 100644 100644 100644 abc1234 def5678 path to/my test file.txt";
    let result = LocalDiffStateModel::parse_git_status(status_output).unwrap();
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].0, "path to/my test file.txt");
    assert_eq!(result[0].1, GitFileStatus::Modified);
}

#[test]
fn test_parse_git_status_new_file_with_spaces() {
    let status_output = "1 A. N... 000000 100644 100644 0000000 abc1234 new file name.rs";
    let result = LocalDiffStateModel::parse_git_status(status_output).unwrap();
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].0, "new file name.rs");
    assert_eq!(result[0].1, GitFileStatus::New);
}

#[test]
fn test_parse_git_status_renamed_file_with_spaces() {
    // Porcelain v2 renamed entry (type 2) with spaces in the new path.
    // Format: 2 <XY> <sub> <mH> <mI> <mW> <hH> <hI> <X><score> <path>\0<origPath>
    let status_output =
        "2 R. N... 100644 100644 100644 abc1234 def5678 R100 new name.txt\0old name.txt";
    let result = LocalDiffStateModel::parse_git_status(status_output).unwrap();
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].0, "new name.txt");
    assert!(matches!(
        &result[0].1,
        GitFileStatus::Renamed { old_path } if old_path == "old name.txt"
    ));
}

#[test]
fn test_parse_git_status_untracked_file_with_spaces() {
    let status_output = "? my untracked file.txt";
    let result = LocalDiffStateModel::parse_git_status(status_output).unwrap();
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].0, "my untracked file.txt");
    assert_eq!(result[0].1, GitFileStatus::Untracked);
}

#[test]
fn test_parse_git_status_unmerged_file_with_spaces() {
    // Porcelain v2 unmerged entry (type u) with spaces in the path.
    // Format: u <xy> <sub> <m1> <m2> <m3> <mW> <h1> <h2> <h3> <path>
    let status_output =
        "u UU N... 100644 100644 100644 100644 abc1234 def5678 ghi9012 conflict file.txt";
    let result = LocalDiffStateModel::parse_git_status(status_output).unwrap();
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].0, "conflict file.txt");
    assert_eq!(result[0].1, GitFileStatus::Conflicted);
}

#[test]
fn test_parse_git_status_mixed_entries_with_spaces() {
    // Multiple entries separated by NUL, mixing files with and without spaces.
    let status_output = "1 .M N... 100644 100644 100644 abc1234 def5678 test file.txt\0\
         1 .M N... 100644 100644 100644 abc1234 def5678 normal.txt\0\
         ? another file with spaces.rs";
    let result = LocalDiffStateModel::parse_git_status(status_output).unwrap();
    assert_eq!(result.len(), 3);
    assert_eq!(result[0].0, "test file.txt");
    assert_eq!(result[1].0, "normal.txt");
    assert_eq!(result[2].0, "another file with spaces.rs");
}

#[test]
fn test_parse_git_status_file_without_spaces_still_works() {
    // Ensure the splitn change doesn't break files without spaces.
    let status_output = "1 .M N... 100644 100644 100644 abc1234 def5678 simple.txt";
    let result = LocalDiffStateModel::parse_git_status(status_output).unwrap();
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].0, "simple.txt");
    assert_eq!(result[0].1, GitFileStatus::Modified);
}

#[tokio::test]
async fn untracked_directory_diff_is_empty_and_non_binary() {
    let repo_dir = tempfile::tempdir().expect("create temp repo dir");
    std::fs::create_dir(repo_dir.path().join("nested-repo")).expect("create nested dir");

    // `git status` reports a nested repo/worktree as a single untracked
    // directory entry (with a trailing slash). It must short-circuit to an
    // empty non-binary diff — the error fallback would otherwise mislabel it
    // as binary and the view would render "Binary file - no diff available"
    // instead of "New empty file".
    let diff = LocalDiffStateModel::get_file_diff(
        repo_dir.path(),
        "nested-repo/",
        &GitFileStatus::Untracked,
        false,
        None,
    )
    .await
    .expect("get_file_diff should succeed for an untracked directory");

    assert!(!diff.is_binary);
    assert_eq!(diff.hunks.len(), 0);
    assert_eq!(diff.status, GitFileStatus::Untracked);
}

#[tokio::test]
async fn untracked_directory_has_no_baseline_content() {
    let repo_dir = tempfile::tempdir().expect("create temp repo dir");
    std::fs::create_dir(repo_dir.path().join("nested-repo")).expect("create nested dir");
    std::fs::write(repo_dir.path().join("new-file.txt"), "hello\n").expect("write file");

    // No baseline for a directory entry, so no editor is constructed for it.
    let dir_content = LocalDiffStateModel::get_file_content_at_head(
        repo_dir.path(),
        "nested-repo/",
        &GitFileStatus::Untracked,
    )
    .await;
    assert_eq!(dir_content, None);

    // Regular untracked files keep their empty baseline.
    let file_content = LocalDiffStateModel::get_file_content_at_head(
        repo_dir.path(),
        "new-file.txt",
        &GitFileStatus::Untracked,
    )
    .await;
    assert_eq!(file_content, Some(String::new()));
}

#[tokio::test]
async fn num_lines_in_file_if_non_binary_counts_lines_in_text_file() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let file_path = dir.path().join("file.txt");
    std::fs::write(&file_path, "one\ntwo\nthree\n").expect("write file");

    let num_lines = LocalDiffStateModel::num_lines_in_file_if_non_binary(&file_path)
        .await
        .expect("counting a regular file should succeed");
    assert_eq!(num_lines, Some(3));
}

#[tokio::test]
async fn num_lines_in_file_if_non_binary_errors_for_directory() {
    let dir = tempfile::tempdir().expect("create temp dir");

    // Directories aren't countable. The metadata callers degrade this error
    // to a 0-line contribution per entry instead of failing the whole
    // metadata computation.
    let result = LocalDiffStateModel::num_lines_in_file_if_non_binary(dir.path()).await;
    assert!(result.is_err());
}

// ── Image preview classification and loading ───────────────────────────

fn encoded_image_bytes(format: image::ImageFormat, rgba: [u8; 4]) -> Vec<u8> {
    let img = image::RgbaImage::from_pixel(2, 3, image::Rgba(rgba));
    let mut bytes = Vec::new();
    image::DynamicImage::ImageRgba8(img)
        .write_to(&mut std::io::Cursor::new(&mut bytes), format)
        .expect("encode test image");
    bytes
}

fn git(repo: &std::path::Path, args: &[&str]) {
    let status = command::blocking::Command::new("git")
        .args(args)
        .current_dir(repo)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .status()
        .expect("run git");
    assert!(status.success(), "git {args:?} failed");
}

fn init_repo_with_committed_file(repo: &std::path::Path, file_name: &str, bytes: &[u8]) {
    git(repo, &["init", "--quiet"]);
    std::fs::write(repo.join(file_name), bytes).expect("write file");
    git(repo, &["add", file_name]);
    git(
        repo,
        &[
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.com",
            "-c",
            "commit.gpgsign=false",
            "commit",
            "--quiet",
            "-m",
            "add file",
        ],
    );
}

#[test]
fn classify_image_bytes_accepts_supported_raster_formats() {
    for (format, mime) in [
        (image::ImageFormat::Png, "image/png"),
        (image::ImageFormat::Jpeg, "image/jpeg"),
        (image::ImageFormat::Gif, "image/gif"),
        (image::ImageFormat::WebP, "image/webp"),
    ] {
        let bytes = encoded_image_bytes(format, [255, 0, 0, 255]);
        let byte_len = bytes.len() as u64;
        match LocalDiffStateModel::classify_image_bytes(bytes) {
            ImageSide::Image {
                mime: got_mime,
                width,
                height,
                byte_len: got_len,
                ..
            } => {
                assert_eq!(got_mime, mime);
                assert_eq!((width, height), (2, 3));
                assert_eq!(got_len, byte_len);
            }
            other => panic!("{mime} should classify as Image, got {other:?}"),
        }
    }
}

#[test]
fn classify_image_bytes_rejects_non_image_content() {
    // Content-only classification (product spec §8): none of these may
    // classify as an image regardless of what a file's extension claims.
    let svg = b"<svg xmlns=\"http://www.w3.org/2000/svg\"><rect/></svg>".to_vec();
    let plain_text = b"hello, definitely not a png".to_vec();
    let lfs_pointer = b"version https://git-lfs.github.com/spec/v1\n\
oid sha256:4d7a214614ab2935c943f9e0ff69d22eadbb8f32b1258daaa5e2ca24d17e2393\n\
size 12345\n"
        .to_vec();

    for bytes in [svg, plain_text, lfs_pointer] {
        assert_eq!(
            LocalDiffStateModel::classify_image_bytes(bytes),
            ImageSide::Rejected
        );
    }
}

#[test]
fn classify_image_bytes_rejects_truncated_image() {
    let mut bytes = encoded_image_bytes(image::ImageFormat::Png, [0, 255, 0, 255]);
    // Keep the magic bytes (so sniffing passes) but cut the data off.
    bytes.truncate(20);
    assert_eq!(
        LocalDiffStateModel::classify_image_bytes(bytes),
        ImageSide::Rejected
    );
}

#[test]
fn classify_image_bytes_caps_declared_dimensions_without_decoding() {
    // A tiny GIF whose logical screen descriptor is patched to declare
    // 65535×65535 (≈4.3 gigapixels; GIF has no checksums). The
    // decompression-bomb guard must classify it as TooLarge from the header
    // alone instead of allocating the pixel buffer.
    let mut bytes = encoded_image_bytes(image::ImageFormat::Gif, [255, 0, 0, 255]);
    bytes[6..10].copy_from_slice(&[0xFF, 0xFF, 0xFF, 0xFF]);
    let byte_len = bytes.len() as u64;

    assert_eq!(
        LocalDiffStateModel::classify_image_bytes(bytes),
        ImageSide::TooLarge { byte_len }
    );
}

#[tokio::test]
async fn working_image_side_over_byte_cap_is_too_large() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let byte_len = MAX_IMAGE_PREVIEW_BYTES + 1;
    let file = std::fs::File::create(dir.path().join("big.png")).expect("create file");
    file.set_len(byte_len).expect("grow file");
    drop(file);

    assert_eq!(
        LocalDiffStateModel::load_working_image_side(dir.path(), "big.png").await,
        ImageSide::TooLarge { byte_len }
    );
}

#[tokio::test]
async fn working_image_side_missing_file_is_unavailable() {
    let dir = tempfile::tempdir().expect("create temp dir");
    assert_eq!(
        LocalDiffStateModel::load_working_image_side(dir.path(), "missing.png").await,
        ImageSide::Unavailable
    );
}

#[tokio::test]
async fn image_preview_disabled_without_feature_flag() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let png = encoded_image_bytes(image::ImageFormat::Png, [255, 0, 0, 255]);
    std::fs::write(dir.path().join("new.png"), &png).expect("write png");

    let preview = LocalDiffStateModel::load_image_preview(
        dir.path(),
        "new.png",
        &GitFileStatus::Untracked,
        "HEAD",
    )
    .await;
    assert_eq!(preview, None);
}

#[tokio::test]
async fn image_preview_for_untracked_image_has_new_side_only() {
    let _flag = FeatureFlag::ImagePreviewInCodeReview.override_enabled(true);
    let dir = tempfile::tempdir().expect("create temp dir");
    let png = encoded_image_bytes(image::ImageFormat::Png, [255, 0, 0, 255]);
    std::fs::write(dir.path().join("new.png"), &png).expect("write png");

    let preview = LocalDiffStateModel::load_image_preview(
        dir.path(),
        "new.png",
        &GitFileStatus::Untracked,
        "HEAD",
    )
    .await
    .expect("untracked image should preview");
    assert_eq!(preview.old, None);
    assert!(matches!(preview.new, Some(ImageSide::Image { .. })));
}

#[tokio::test]
async fn image_preview_for_modified_image_has_both_sides() {
    let _flag = FeatureFlag::ImagePreviewInCodeReview.override_enabled(true);
    let dir = tempfile::tempdir().expect("create temp dir");
    let old_png = encoded_image_bytes(image::ImageFormat::Png, [255, 0, 0, 255]);
    let new_png = encoded_image_bytes(image::ImageFormat::Png, [0, 0, 255, 255]);
    init_repo_with_committed_file(dir.path(), "img.png", &old_png);
    std::fs::write(dir.path().join("img.png"), &new_png).expect("overwrite png");

    let preview = LocalDiffStateModel::load_image_preview(
        dir.path(),
        "img.png",
        &GitFileStatus::Modified,
        "HEAD",
    )
    .await
    .expect("modified image should preview");

    let (Some(ImageSide::Image { bytes: old, .. }), Some(ImageSide::Image { bytes: new, .. })) =
        (&preview.old, &preview.new)
    else {
        panic!("both sides should be images, got {preview:?}");
    };
    assert_eq!(**old, old_png);
    assert_eq!(**new, new_png);
}

#[tokio::test]
async fn image_preview_for_deleted_image_has_old_side_only() {
    let _flag = FeatureFlag::ImagePreviewInCodeReview.override_enabled(true);
    let dir = tempfile::tempdir().expect("create temp dir");
    let png = encoded_image_bytes(image::ImageFormat::Png, [255, 0, 0, 255]);
    init_repo_with_committed_file(dir.path(), "img.png", &png);
    std::fs::remove_file(dir.path().join("img.png")).expect("delete png");

    let preview = LocalDiffStateModel::load_image_preview(
        dir.path(),
        "img.png",
        &GitFileStatus::Deleted,
        "HEAD",
    )
    .await
    .expect("deleted image should preview");
    assert!(matches!(preview.old, Some(ImageSide::Image { .. })));
    assert_eq!(preview.new, None);
}

#[tokio::test]
async fn image_preview_for_unchanged_rename_collapses_to_single_side() {
    let _flag = FeatureFlag::ImagePreviewInCodeReview.override_enabled(true);
    let dir = tempfile::tempdir().expect("create temp dir");
    let png = encoded_image_bytes(image::ImageFormat::Png, [255, 0, 0, 255]);
    init_repo_with_committed_file(dir.path(), "old.png", &png);
    std::fs::rename(dir.path().join("old.png"), dir.path().join("new.png")).expect("rename png");

    let preview = LocalDiffStateModel::load_image_preview(
        dir.path(),
        "new.png",
        &GitFileStatus::Renamed {
            old_path: "old.png".to_string(),
        },
        "HEAD",
    )
    .await
    .expect("renamed image should preview");
    // No content change: single image, not a fake before/after (product §2).
    assert_eq!(preview.old, None);
    assert!(matches!(preview.new, Some(ImageSide::Image { .. })));
}

#[tokio::test]
async fn image_preview_with_corrupt_working_side_keeps_readable_base() {
    let _flag = FeatureFlag::ImagePreviewInCodeReview.override_enabled(true);
    let dir = tempfile::tempdir().expect("create temp dir");
    let png = encoded_image_bytes(image::ImageFormat::Png, [255, 0, 0, 255]);
    init_repo_with_committed_file(dir.path(), "img.png", &png);
    std::fs::write(dir.path().join("img.png"), b"corrupted, not an image").expect("corrupt png");

    let preview = LocalDiffStateModel::load_image_preview(
        dir.path(),
        "img.png",
        &GitFileStatus::Modified,
        "HEAD",
    )
    .await
    .expect("readable base side should keep the preview");
    assert!(matches!(preview.old, Some(ImageSide::Image { .. })));
    assert_eq!(preview.new, Some(ImageSide::Rejected));
}

#[tokio::test]
async fn image_preview_with_no_previewable_side_is_none() {
    let _flag = FeatureFlag::ImagePreviewInCodeReview.override_enabled(true);
    let dir = tempfile::tempdir().expect("create temp dir");
    // Untracked non-image binary: the only applicable side is Rejected, so
    // the file keeps the generic binary placeholder (product §8).
    std::fs::write(dir.path().join("blob.bin"), [0u8, 159, 146, 150]).expect("write binary file");

    let preview = LocalDiffStateModel::load_image_preview(
        dir.path(),
        "blob.bin",
        &GitFileStatus::Untracked,
        "HEAD",
    )
    .await;
    assert_eq!(preview, None);
}
