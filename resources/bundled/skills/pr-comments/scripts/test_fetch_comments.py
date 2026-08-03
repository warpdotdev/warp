"""Tests for _resolve_comment_line and _resolve_line (no network calls).

Uses synthetic GitHub API comment dicts and diff hunks to exercise the
fallback chain: line → original_line → last reachable line → None.
"""

import sys
import os
import unittest

sys.path.insert(0, os.path.dirname(__file__))

from fetch_github_review_comments import (
    _resolve_comment_line,
    _resolve_line,
    base_repo_from_url,
)


# ---------------------------------------------------------------------------
# A small reusable hunk: new file lines 20-24, old file lines 10-14.
# ---------------------------------------------------------------------------
_CONTEXT_HUNK = "\n".join(
    ["@@ -10,5 +20,5 @@"] + [f" line {10 + i}" for i in range(5)]
)


def _gh_comment(**overrides):
    """Build a minimal GitHub-API-shaped comment dict."""
    base = {
        "side": "RIGHT",
        "line": None,
        "original_line": None,
        "start_line": None,
        "original_start_line": None,
    }
    base.update(overrides)
    return base


# ---------------------------------------------------------------------------
# _resolve_line
# ---------------------------------------------------------------------------

class TestResolveLine(unittest.TestCase):
    def test_line_matches(self):
        assert _resolve_line(_CONTEXT_HUNK, 22, None, "RIGHT") == 22

    def test_line_mismatches_original_matches(self):
        assert _resolve_line(_CONTEXT_HUNK, 9999, 23, "RIGHT") == 23

    def test_both_mismatch_falls_back_to_last_reachable(self):
        # last new-file line in the hunk is 24
        assert _resolve_line(_CONTEXT_HUNK, 9999, 8888, "RIGHT") == 24

    def test_empty_hunk_returns_none(self):
        assert _resolve_line("", 22, None, "RIGHT") is None

    def test_none_candidates_falls_back_to_last_reachable(self):
        assert _resolve_line(_CONTEXT_HUNK, None, None, "RIGHT") == 24

    def test_left_side(self):
        assert _resolve_line(_CONTEXT_HUNK, 12, None, "LEFT") == 12


# ---------------------------------------------------------------------------
# _resolve_comment_line
# ---------------------------------------------------------------------------

class TestResolveCommentLine(unittest.TestCase):
    def test_line_matches_hunk(self):
        c = _gh_comment(side="RIGHT", line=22)
        result = _resolve_comment_line(c, _CONTEXT_HUNK)
        assert result == (22, None, "RIGHT")

    def test_line_mismatches_original_matches(self):
        c = _gh_comment(side="RIGHT", line=9999, original_line=23)
        result = _resolve_comment_line(c, _CONTEXT_HUNK)
        assert result == (23, None, "RIGHT")

    def test_both_mismatch_returns_last_reachable(self):
        c = _gh_comment(side="RIGHT", line=9999, original_line=8888)
        result = _resolve_comment_line(c, _CONTEXT_HUNK)
        # last reachable new-file line is 24
        assert result == (24, None, "RIGHT")

    def test_both_mismatch_no_reachable_lines_returns_none(self):
        deletion_only = "@@ -5,3 +5,0 @@\n-del A\n-del B\n-del C"
        c = _gh_comment(side="RIGHT", line=9999, original_line=8888)
        result = _resolve_comment_line(c, deletion_only)
        assert result is None

    def test_empty_hunk_returns_none(self):
        c = _gh_comment(side="RIGHT", line=22)
        result = _resolve_comment_line(c, "")
        assert result is None

    def test_both_line_and_original_none_falls_back(self):
        c = _gh_comment(side="RIGHT")
        result = _resolve_comment_line(c, _CONTEXT_HUNK)
        # No explicit line, but the hunk has reachable lines → last reachable
        assert result == (24, None, "RIGHT")

    def test_start_line_resolved(self):
        c = _gh_comment(side="RIGHT", line=23, start_line=21)
        result = _resolve_comment_line(c, _CONTEXT_HUNK)
        assert result == (23, 21, "RIGHT")

    def test_start_line_falls_back_to_original(self):
        c = _gh_comment(
            side="RIGHT", line=23,
            start_line=9999, original_start_line=20,
        )
        result = _resolve_comment_line(c, _CONTEXT_HUNK)
        assert result == (23, 20, "RIGHT")

    def test_side_defaults_to_right(self):
        c = _gh_comment(line=22)
        c.pop("side")
        result = _resolve_comment_line(c, _CONTEXT_HUNK)
        assert result[2] == "RIGHT"

    # -- Inverted range: start_line > end_line after fallback ----------------

    def test_start_line_cleared_when_inverted(self):
        """Both start_line and original_start_line are stale.

        _resolve_line falls back to last_reachable_line (24) for the start,
        but end_line resolved to an earlier line (21).  The fix must clear
        start_line to None instead of returning an inverted range.
        """
        c = _gh_comment(
            side="RIGHT",
            line=21,
            start_line=9999,
            original_start_line=8888,
        )
        result = _resolve_comment_line(c, _CONTEXT_HUNK)
        end_line, start_line, side = result
        assert end_line == 21
        assert start_line is None
        assert side == "RIGHT"

    def test_start_line_kept_when_not_inverted(self):
        """start_line resolves to a line before end_line — should be kept."""
        c = _gh_comment(
            side="RIGHT",
            line=23,
            start_line=21,
        )
        result = _resolve_comment_line(c, _CONTEXT_HUNK)
        assert result == (23, 21, "RIGHT")

    def test_start_line_cleared_when_equal_to_end(self):
        """start_line == end_line is fine — only strictly greater is inverted."""
        c = _gh_comment(
            side="RIGHT",
            line=22,
            start_line=22,
        )
        result = _resolve_comment_line(c, _CONTEXT_HUNK)
        assert result == (22, 22, "RIGHT")

    def test_start_fallback_exceeds_end_on_left_side(self):
        """Same inverted-range scenario but on the LEFT side."""
        c = _gh_comment(
            side="LEFT",
            line=11,
            start_line=9999,
            original_start_line=8888,
        )
        result = _resolve_comment_line(c, _CONTEXT_HUNK)
        end_line, start_line, side = result
        assert end_line == 11
        # last_reachable_line on LEFT = 14, which > 11 → cleared
        assert start_line is None
        assert side == "LEFT"

    # -- Regression: PR #22932 outdated comment ----------------------------

    def test_regression_outdated_comment_falls_back(self):
        """Comment 2898341466: line=2899 (repositioned), hunk +3066.

        Neither line nor original_line is in the hunk, so we expect the
        last reachable line (3071 in the original 6-body-line hunk).
        """
        hunk = (
            "@@ -2926,6 +3066,17 @@ fn render_response_footer\n"
            "         }\n"
            "     }\n"
            " \n"
            "+    // Bulk-import review comments button\n"
            "+    // has any imported review comments.\n"
            "+    if props.conversation_has_imported_comments {"
        )
        c = _gh_comment(side="RIGHT", line=2899, original_line=2899)
        result = _resolve_comment_line(c, hunk)
        assert result is not None
        end_line, _start, side = result
        assert side == "RIGHT"
        # Should fall back to last reachable new-file line in the hunk body.
        assert end_line == 3071


# ---------------------------------------------------------------------------
# base_repo_from_url
# ---------------------------------------------------------------------------

class TestBaseRepoFromUrl(unittest.TestCase):
    def test_same_repo_pr(self):
        url = "https://github.com/warpdotdev/warp/pull/1"
        assert base_repo_from_url(url) == ("warpdotdev", "warp")

    def test_fork_pr_resolves_to_base_repo(self):
        """Regression: a PR opened from a fork must resolve to the base repo.

        The PR URL always points at the base repo (warpdotdev/warp), so even
        though the head/fork is th1nkful/warp, comments are fetched from the
        base repo instead of 404ing against the fork.
        """
        url = "https://github.com/warpdotdev/warp/pull/9850"
        assert base_repo_from_url(url) == ("warpdotdev", "warp")

    def test_enterprise_host(self):
        url = "https://ghe.example.com/acme/widgets/pull/42"
        assert base_repo_from_url(url) == ("acme", "widgets")


if __name__ == "__main__":
    unittest.main()
