"""Tests for trim_diff_hunk and _parse_hunk_header.

Reference: app/src/code_review/comments/diff_hunk_parser_tests.rs
"""

import sys
import os
import unittest

sys.path.insert(0, os.path.dirname(__file__))

from trim_diff_hunk import trim_diff_hunk, _parse_hunk_header, _prepare_lines, _annotate_hunk_body, line_in_hunk, last_reachable_line


# ---------------------------------------------------------------------------
# Helper to build large hunks for testing
# ---------------------------------------------------------------------------

def _make_context_hunk(old_start, new_start, count):
    """Build a pure-context hunk with ``count`` lines."""
    header = f"@@ -{old_start},{count} +{new_start},{count} @@"
    body = [f" line {old_start + i}" for i in range(count)]
    return "\n".join([header] + body)


# ---------------------------------------------------------------------------
# _prepare_lines
# ---------------------------------------------------------------------------

class TestPrepareLines(unittest.TestCase):
    def test_splits_and_strips_trailing_empties(self):
        assert _prepare_lines("a\nb\nc\n\n") == ["a", "b", "c"]

    def test_no_trailing_empties(self):
        assert _prepare_lines("a\nb") == ["a", "b"]

    def test_single_line(self):
        assert _prepare_lines("@@ -1,1 +1,1 @@") == ["@@ -1,1 +1,1 @@"]

    def test_empty_string(self):
        assert _prepare_lines("") == []


# ---------------------------------------------------------------------------
# _parse_hunk_header
# ---------------------------------------------------------------------------

class TestParseHunkHeader(unittest.TestCase):
    def test_standard_header(self):
        assert _parse_hunk_header("@@ -10,5 +20,7 @@") == (10, 5, 20, 7, "")

    def test_header_with_context_text(self):
        result = _parse_hunk_header("@@ -10,5 +20,7 @@ fn main()")
        assert result == (10, 5, 20, 7, " fn main()")

    def test_omitted_counts_default_to_one(self):
        assert _parse_hunk_header("@@ -10 +20 @@") == (10, 1, 20, 1, "")

    def test_invalid_headers(self):
        assert _parse_hunk_header("not a header") is None
        assert _parse_hunk_header("@@ invalid @@") is None
        assert _parse_hunk_header("") is None

    def test_only_old_count_omitted(self):
        assert _parse_hunk_header("@@ -10 +20,3 @@") == (10, 1, 20, 3, "")

    def test_only_new_count_omitted(self):
        assert _parse_hunk_header("@@ -10,3 +20 @@") == (10, 3, 20, 1, "")


# ---------------------------------------------------------------------------
# trim_diff_hunk – basic / passthrough cases
# ---------------------------------------------------------------------------

class TestTrimPassthrough(unittest.TestCase):
    """Cases where trim_diff_hunk should return the input unchanged."""

    def test_empty_string(self):
        assert trim_diff_hunk("", 1) == ""

    def test_none_input(self):
        assert trim_diff_hunk(None, 1) is None

    def test_small_hunk_unchanged(self):
        hunk = "@@ -1,2 +1,3 @@\n first line\n+added line\n last line"
        assert trim_diff_hunk(hunk, 2, context_lines=3) == hunk

    def test_invalid_header_unchanged(self):
        hunk = "not a valid header\n+line1\n+line2\n+line3\n+line4\n+line5"
        assert trim_diff_hunk(hunk, 1) == hunk

    def test_target_not_found_unchanged(self):
        hunk = _make_context_hunk(1, 1, 20)
        assert trim_diff_hunk(hunk, 999) == hunk


# ---------------------------------------------------------------------------
# trim_diff_hunk – trimming behaviour
# ---------------------------------------------------------------------------

class TestTrimBehaviour(unittest.TestCase):
    def test_trims_to_context_window(self):
        """Large hunk trimmed to ±3 lines around target."""
        hunk = _make_context_hunk(1, 1, 20)
        result = trim_diff_hunk(hunk, 10, context_lines=3)
        result_lines = result.split("\n")

        # Target ± 3 → lines 7-13 = 7 body lines + header
        assert result_lines[0] == "@@ -7,7 +7,7 @@"
        assert " line 7" in result
        assert " line 10" in result
        assert " line 13" in result
        assert " line 6" not in result
        assert " line 14" not in result

    def test_target_at_beginning(self):
        hunk = _make_context_hunk(1, 1, 20)
        result = trim_diff_hunk(hunk, 1, context_lines=3)
        result_lines = result.split("\n")

        # Can't go before line 1 → lo=0, hi=3
        assert result_lines[0] == "@@ -1,4 +1,4 @@"
        assert " line 1" in result
        assert " line 4" in result
        assert " line 5" not in result

    def test_target_at_end(self):
        hunk = _make_context_hunk(1, 1, 20)
        result = trim_diff_hunk(hunk, 20, context_lines=3)
        result_lines = result.split("\n")

        assert result_lines[0] == "@@ -17,4 +17,4 @@"
        assert " line 17" in result
        assert " line 20" in result
        assert " line 16" not in result

    def test_preserves_whitespace(self):
        """Mirrors Rust test_parse_preserves_whitespace."""
        lines = ["@@ -1,15 +1,16 @@"]
        for i in range(1, 8):
            lines.append(f" line {i}")
        lines.append("+    heavily indented")  # new line 8
        for i in range(8, 16):
            lines.append(f" line {i}")
        hunk = "\n".join(lines)

        result = trim_diff_hunk(hunk, 8, side="RIGHT")
        assert "+    heavily indented" in result

    def test_preserves_header_context_text(self):
        lines = ["@@ -1,20 +1,20 @@ fn example()"]
        for i in range(1, 21):
            lines.append(f" line {i}")
        hunk = "\n".join(lines)

        result = trim_diff_hunk(hunk, 10)
        assert result.split("\n")[0].endswith(" fn example()")

    def test_trailing_empty_lines_stripped(self):
        hunk = _make_context_hunk(1, 1, 20) + "\n\n\n"
        result = trim_diff_hunk(hunk, 10)
        assert not result.endswith("\n")


# ---------------------------------------------------------------------------
# trim_diff_hunk – LEFT / RIGHT side targeting
# ---------------------------------------------------------------------------

class TestSideTargeting(unittest.TestCase):
    def test_right_side_skips_deletions(self):
        """RIGHT side tracks new-file line numbers; deletions are invisible."""
        lines = ["@@ -1,12 +1,14 @@"]
        for i in range(1, 4):
            lines.append(f" line {i}")         # old 1-3, new 1-3
        lines.append("-deleted A")              # old 4
        lines.append("-deleted B")              # old 5
        lines.append("+added A")                # new 4
        lines.append("+added B")                # new 5
        lines.append("+added C")                # new 6
        lines.append("+added D")                # new 7
        for i in range(6, 14):
            lines.append(f" line {i}")          # old 6-13, new 8-15
        hunk = "\n".join(lines)

        result = trim_diff_hunk(hunk, 6, side="RIGHT")
        assert "+added C" in result

    def test_left_side_skips_additions(self):
        """LEFT side tracks old-file line numbers; additions are invisible."""
        lines = ["@@ -10,15 +10,16 @@"]
        for i in range(15):
            if i == 7:
                lines.append("+added line")     # new-only, no old num
            lines.append(f" context {i}")
        hunk = "\n".join(lines)

        # old line 15 = old_start(10) + 5 context lines → " context 5"
        result = trim_diff_hunk(hunk, 15, side="LEFT")
        assert " context 5" in result

    def test_left_targets_deletion(self):
        """Targeting a deleted line by old-file number."""
        lines = ["@@ -1,12 +1,10 @@"]
        for i in range(1, 5):
            lines.append(f" line {i}")
        lines.append("-removed A")              # old 5
        lines.append("-removed B")              # old 6
        for i in range(5, 13):
            lines.append(f" line {i}")
        hunk = "\n".join(lines)

        result = trim_diff_hunk(hunk, 5, side="LEFT")
        assert "-removed A" in result


# ---------------------------------------------------------------------------
# trim_diff_hunk – multi-line comment ranges
# ---------------------------------------------------------------------------

class TestMultilineRange(unittest.TestCase):
    def test_keeps_full_range_plus_context(self):
        hunk = _make_context_hunk(1, 1, 20)
        # start_line=8, target=12, context=3 → window is [8-3, 12+3] = [5, 15]
        result = trim_diff_hunk(hunk, 12, start_line=8, context_lines=3)
        result_lines = result.split("\n")

        assert result_lines[0] == "@@ -5,11 +5,11 @@"
        assert " line 5" in result
        assert " line 8" in result
        assert " line 12" in result
        assert " line 15" in result
        assert " line 4" not in result
        assert " line 16" not in result

    def test_range_start_at_hunk_boundary(self):
        hunk = _make_context_hunk(1, 1, 20)
        # start_line=1, context=3 → lo clamped to 0
        result = trim_diff_hunk(hunk, 5, start_line=1, context_lines=3)
        assert " line 1" in result
        assert " line 8" in result


# ---------------------------------------------------------------------------
# trim_diff_hunk – pure additions / pure deletions
# ---------------------------------------------------------------------------

class TestPureAdditionsAndDeletions(unittest.TestCase):
    def test_only_additions(self):
        """Old-file count should be 0 in trimmed header."""
        lines = ["@@ -5,0 +5,15 @@"]
        for i in range(5, 20):
            lines.append(f"+new line {i}")
        hunk = "\n".join(lines)

        result = trim_diff_hunk(hunk, 10, side="RIGHT")
        header = result.split("\n")[0]
        # All trimmed lines are additions → old count = 0
        assert ",0 +" in header
        assert "+new line 10" in result

    def test_only_deletions(self):
        """New-file count should be 0 in trimmed header."""
        lines = ["@@ -5,15 +5,0 @@"]
        for i in range(5, 20):
            lines.append(f"-old line {i}")
        hunk = "\n".join(lines)

        result = trim_diff_hunk(hunk, 10, side="LEFT")
        header = result.split("\n")[0]
        assert "+5,0 @@" in header
        assert "-old line 10" in result


# ---------------------------------------------------------------------------
# trim_diff_hunk – special markers
# ---------------------------------------------------------------------------

class TestSpecialMarkers(unittest.TestCase):
    def test_no_newline_marker_does_not_shift_line_numbers(self):
        r"""'\ No newline at end of file' must not affect line counting."""
        lines = ["@@ -1,15 +1,16 @@"]
        for i in range(1, 8):
            lines.append(f" line {i}")
        lines.append("+added line")                     # new line 8
        lines.append("\\ No newline at end of file")
        for i in range(8, 16):
            lines.append(f" line {i}")
        hunk = "\n".join(lines)

        # Use context=3 so the marker is within the trim window
        result = trim_diff_hunk(hunk, 8, side="RIGHT", context_lines=3)
        assert "+added line" in result

    def test_no_newline_marker_excluded_from_counts(self):
        r"""The marker should not inflate old/new counts in the header."""
        lines = ["@@ -1,15 +1,16 @@"]
        for i in range(1, 8):
            lines.append(f" line {i}")
        lines.append("+added line")                     # new line 8
        lines.append("\\ No newline at end of file")
        for i in range(8, 16):
            lines.append(f" line {i}")
        hunk = "\n".join(lines)

        # Use context=3 so the marker is within the trim window
        result = trim_diff_hunk(hunk, 8, side="RIGHT", context_lines=3)
        header = result.split("\n")[0]
        parsed = _parse_hunk_header(header)
        old_count, new_count = parsed[1], parsed[3]
        # Count body lines manually to verify
        body = result.split("\n")[1:]
        expected_old = sum(
            1 for l in body if l and l[0] not in ("+", "\\")
        )
        expected_new = sum(
            1 for l in body if l and l[0] not in ("-", "\\")
        )
        assert old_count == expected_old
        assert new_count == expected_new


# ---------------------------------------------------------------------------
# trim_diff_hunk – zero context (default)
# ---------------------------------------------------------------------------

class TestZeroContext(unittest.TestCase):
    """With context_lines=0, only the exact commented line(s) are kept."""

    def test_isolates_single_context_line(self):
        hunk = _make_context_hunk(1, 1, 20)
        result = trim_diff_hunk(hunk, 10)
        result_lines = result.split("\n")

        assert result_lines[0] == "@@ -10,1 +10,1 @@"
        assert len(result_lines) == 2
        assert " line 10" in result

    def test_isolates_addition(self):
        lines = ["@@ -1,5 +1,6 @@"]
        for i in range(1, 4):
            lines.append(f" line {i}")
        lines.append("+added line")  # new line 4
        for i in range(4, 6):
            lines.append(f" line {i}")
        hunk = "\n".join(lines)

        result = trim_diff_hunk(hunk, 4, side="RIGHT")
        result_lines = result.split("\n")
        assert len(result_lines) == 2
        assert "+added line" in result
        header = _parse_hunk_header(result_lines[0])
        assert header[1] == 0  # old count = 0
        assert header[3] == 1  # new count = 1

    def test_isolates_deletion(self):
        lines = ["@@ -1,6 +1,5 @@"]
        for i in range(1, 4):
            lines.append(f" line {i}")
        lines.append("-deleted line")  # old line 4
        for i in range(5, 7):
            lines.append(f" line {i}")
        hunk = "\n".join(lines)

        result = trim_diff_hunk(hunk, 4, side="LEFT")
        result_lines = result.split("\n")
        assert len(result_lines) == 2
        assert "-deleted line" in result
        header = _parse_hunk_header(result_lines[0])
        assert header[1] == 1  # old count = 1
        assert header[3] == 0  # new count = 0

    def test_multiline_range_no_context(self):
        hunk = _make_context_hunk(1, 1, 20)
        result = trim_diff_hunk(hunk, 12, start_line=8)
        result_lines = result.split("\n")

        assert result_lines[0] == "@@ -8,5 +8,5 @@"
        assert " line 8" in result
        assert " line 12" in result
        assert " line 7" not in result
        assert " line 13" not in result


# ---------------------------------------------------------------------------
# line_in_hunk
# ---------------------------------------------------------------------------

class TestLineInHunk(unittest.TestCase):
    def test_target_within_range(self):
        hunk = _make_context_hunk(10, 20, 5)
        assert line_in_hunk(hunk, 22, side="RIGHT") is True

    def test_target_within_range_left(self):
        hunk = _make_context_hunk(10, 20, 5)
        assert line_in_hunk(hunk, 12, side="LEFT") is True

    def test_target_past_end(self):
        hunk = _make_context_hunk(10, 20, 5)  # new 20-24
        assert line_in_hunk(hunk, 30, side="RIGHT") is False

    def test_target_before_start(self):
        hunk = _make_context_hunk(10, 20, 5)  # new 20-24
        assert line_in_hunk(hunk, 5, side="RIGHT") is False

    def test_right_target_in_deletion_only_hunk(self):
        """A hunk with only deletions has no reachable RIGHT lines."""
        hunk = "@@ -5,3 +5,0 @@\n-del A\n-del B\n-del C"
        assert line_in_hunk(hunk, 5, side="RIGHT") is False

    def test_empty_hunk(self):
        assert line_in_hunk("", 1) is False

    def test_none_hunk(self):
        assert line_in_hunk(None, 1) is False

    def test_target_past_hunk_end(self):
        """Comment 2892844016: target 11078, hunk new 10816-10822."""
        hunk = (
            "@@ -10808,9 +10816,7 @@ impl Workspace {\n"
            "                 comment,\n"
            "                 diff_mode,\n"
            "             } => {\n"
            "-                if !pane_group.as_ref(ctx).right_panel_open {\n"
            "-                    self.open_code_review_panel_from_arg(open_code_review, pane_group.clone(), ctx);\n"
            "-                }\n"
            "+                self.open_code_review_panel_from_arg(open_code_review, pane_group.clone(), ctx);"
        )
        assert line_in_hunk(hunk, 11078, side="RIGHT") is False

    def test_target_before_hunk_start(self):
        """Comment 2898341466: target 2899, hunk new 3066-3082."""
        hunk = (
            "@@ -2926,6 +3066,17 @@ fn render_response_footer(props: Props, app: &AppContext) -> Option<Box<dyn Elem\n"
            "         }\n"
            "     }\n"
            " \n"
            "+    // Bulk-import review comments button, shown on the latest exchange when the conversation\n"
            "+    // has any imported review comments.\n"
            "+    if props.conversation_has_imported_comments && !props.shared_session_status.is_viewer() {"
        )
        assert line_in_hunk(hunk, 2899, side="RIGHT") is False

    def test_truncated_hunk(self):
        """Comment 2954102612: header claims +4770,109 but body has only 5 lines.

        Simulates a truncated hunk where the target (4918) is past the body.
        """
        lines = ["@@ -4769,6 +4770,109 @@ impl AIBlock {"]
        for i in range(4770, 4775):
            lines.append(f"+    line {i}")
        hunk = "\n".join(lines)
        assert line_in_hunk(hunk, 4918, side="RIGHT") is False


# ---------------------------------------------------------------------------
# last_reachable_line
# ---------------------------------------------------------------------------

class TestLastReachableLine(unittest.TestCase):
    def test_context_hunk(self):
        hunk = _make_context_hunk(10, 20, 5)  # new 20-24
        assert last_reachable_line(hunk, side="RIGHT") == 24

    def test_addition_hunk(self):
        hunk = "@@ -5,0 +5,3 @@\n+new A\n+new B\n+new C"
        assert last_reachable_line(hunk, side="RIGHT") == 7

    def test_deletion_only_right(self):
        hunk = "@@ -5,3 +5,0 @@\n-del A\n-del B\n-del C"
        assert last_reachable_line(hunk, side="RIGHT") is None

    def test_deletion_only_left(self):
        hunk = "@@ -5,3 +5,0 @@\n-del A\n-del B\n-del C"
        assert last_reachable_line(hunk, side="LEFT") == 7

    def test_empty_hunk(self):
        assert last_reachable_line("", side="RIGHT") is None

    def test_none_hunk(self):
        assert last_reachable_line(None, side="RIGHT") is None


# ---------------------------------------------------------------------------
# trim_diff_hunk – multi-hunk inputs
# ---------------------------------------------------------------------------

class TestMultiHunk(unittest.TestCase):
    """trim_diff_hunk should find and trim the correct sub-hunk."""

    _MULTI = (
        "@@ -1,3 +1,4 @@ fn foo()\n"
        " ctx1\n"
        "+added_early\n"
        " ctx2\n"
        " ctx3\n"
        "@@ -100,3 +101,4 @@ fn bar()\n"
        " ctx100\n"
        "+added_late\n"
        " ctx101\n"
        " ctx102"
    )

    def test_target_in_second_hunk(self):
        """Target 102 (new-file line in second sub-hunk) is found and trimmed."""
        result = trim_diff_hunk(self._MULTI, 102, side="RIGHT")
        assert "+added_late" in result
        # The result should NOT contain the first hunk's content.
        assert "+added_early" not in result
        header = _parse_hunk_header(result.split("\n")[0])
        assert header is not None
        # new_start should be from the second hunk's range (101+)
        assert header[2] == 102  # trimmed to just line 102

    def test_target_in_first_hunk(self):
        result = trim_diff_hunk(self._MULTI, 2, side="RIGHT")
        assert "+added_early" in result
        assert "+added_late" not in result

    def test_target_in_no_hunk(self):
        result = trim_diff_hunk(self._MULTI, 9999, side="RIGHT")
        assert result == self._MULTI

    def test_multi_hunk_line_in_hunk(self):
        """line_in_hunk should search across all sub-hunks."""
        assert line_in_hunk(self._MULTI, 102, side="RIGHT") is True
        assert line_in_hunk(self._MULTI, 2, side="RIGHT") is True
        assert line_in_hunk(self._MULTI, 9999, side="RIGHT") is False


# ---------------------------------------------------------------------------
# Edge case: markdown list items in pure-addition hunks
# ---------------------------------------------------------------------------

class TestMarkdownListItem(unittest.TestCase):
    r"""Hunks from where commented lines are markdown list items.

    The line `+- \`specs/<issue-number>/TECH.md\`` starts with `+-`.  The `-`
    is a markdown list marker, NOT a diff deletion prefix.
    """

    _HUNK = (
        "@@ -0,0 +1,116 @@\n"
        "+---\n"
        "+name: write-tech-spec\n"
        "+description: desc\n"
        "+---\n"
        "+\n"
        "+# write-tech-spec\n"
        "+\n"
        "+Write a spec.\n"
        "+\n"
        "+## Overview\n"
        "+\n"
        "+The tech spec overview.\n"
        "+\n"
        "+Write specs into source control under:\n"
        "+\n"
        "+- `specs/<issue-number>/TECH.md`"
    )

    def test_annotate_hunk_body_classifies_plus_dash_as_addition(self):
        """_annotate_hunk_body must treat `+-` lines as additions."""
        body = _prepare_lines(self._HUNK)[1:]  # skip header
        annotated = _annotate_hunk_body(body, 0, 1)
        # Line 16 (new-file) should be the markdown list item.
        target_entry = annotated[15]  # 0-indexed
        old_num, new_num, text = target_entry
        assert old_num is None, f"Expected no old-file line number, got {old_num}"
        assert new_num == 16, f"Expected new-file line 16, got {new_num}"
        assert text == "+- `specs/<issue-number>/TECH.md`"

    def test_trim_preserves_plus_dash_line(self):
        """trim_diff_hunk must preserve the full `+-` line text."""
        result = trim_diff_hunk(self._HUNK, 16, side="RIGHT")
        assert "+- `specs/<issue-number>/TECH.md`" in result

    def test_trim_header_counts_plus_dash_as_new(self):
        """Trimmed header must count the `+-` line as a new-file line."""
        result = trim_diff_hunk(self._HUNK, 16, side="RIGHT")
        header = _parse_hunk_header(result.split("\n")[0])
        assert header is not None
        old_count, new_count = header[1], header[3]
        # A single addition line: old_count=0, new_count=1
        assert old_count == 0, f"Expected old_count=0, got {old_count}"
        assert new_count == 1, f"Expected new_count=1, got {new_count}"

    def test_line_in_hunk_finds_plus_dash_line(self):
        """line_in_hunk must locate line 16 on the RIGHT side."""
        assert line_in_hunk(self._HUNK, 16, side="RIGHT") is True

    def test_last_reachable_line_includes_plus_dash(self):
        """last_reachable_line must reach line 16 on the RIGHT side."""
        assert last_reachable_line(self._HUNK, side="RIGHT") == 16


if __name__ == "__main__":
    unittest.main()
