use super::*;

/// The synchronous and asynchronous diff entry points produce identical
/// edits, since they share the same underlying diff core.
#[test]
fn text_diff_sync_matches_async_text_diff() {
    let old = "line1\nline2\nline3\nline4\nline5\n";
    let new = "line1\nCHANGED\nline3\nline4\nline5\n";

    let sync_result = text_diff_sync(old, new);
    let async_result = futures_lite::future::block_on(text_diff(old, new));

    assert_eq!(sync_result, async_result);
}

/// Applying the edits from `text_diff_sync` to `old`, in reverse order (so
/// earlier byte ranges aren't shifted by later replacements), must
/// reconstruct `new` exactly.
#[test]
fn text_diff_sync_edits_transform_old_into_new() {
    let old = "the quick brown fox\njumps over\nthe lazy dog\n";
    let new = "the quick brown fox\nleaps over\nthe lazy dog\n";

    let diff = text_diff_sync(old, new);
    assert!(!diff.is_empty());

    let mut result = old.to_string();
    for (range, replacement) in diff.edits.iter().rev() {
        result.replace_range(range.clone(), replacement);
    }
    assert_eq!(result, new);
}

#[test]
fn text_diff_sync_is_empty_for_identical_text() {
    let text = "unchanged content\nacross multiple lines\n";
    assert!(text_diff_sync(text, text).is_empty());
}

/// The dangerous case for a diff-based replace of a large, mostly-unchanged
/// buffer (APP-5357's `resolve_conflict`): a change to one line out of many
/// must produce a small diff, not one that touches the whole text.
#[test]
fn text_diff_sync_scopes_a_single_line_change_in_a_large_buffer() {
    let line = |i: usize| format!("line{i:03}\n");
    let old: String = (0..200).map(line).collect();
    let new: String = (0..200)
        .map(|i| {
            if i == 100 {
                "CHANGED\n".to_string()
            } else {
                line(i)
            }
        })
        .collect();

    let diff = text_diff_sync(&old, &new);

    assert_eq!(diff.edits.len(), 1, "expected exactly one changed hunk");
    let (range, replacement) = &diff.edits[0];
    assert_eq!(replacement, "CHANGED\n");
    assert!(
        range.end - range.start < old.len() / 2,
        "expected a scoped edit far smaller than the full buffer ({} bytes), got {} bytes",
        old.len(),
        range.end - range.start
    );
}
