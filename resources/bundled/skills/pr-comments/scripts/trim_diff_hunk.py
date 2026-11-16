"""Diff-hunk trimming for unified diffs.

Mirrors the logic in app/src/code_review/comments/diff_hunk_parser.rs:
walk hunk lines tracking old/new file line numbers, locate the target line,
then trim unneeded lines from the start and end (never the middle) and
rewrite the hunk header to match the trimmed window.
"""

import re

_HUNK_HEADER_RE = re.compile(
    r"^@@ -(\d+)(?:,(\d+))? \+(\d+)(?:,(\d+))? @@(.*)$"
)


def _parse_hunk_header(line):
    m = _HUNK_HEADER_RE.match(line)
    if not m:
        return None
    return (
        int(m.group(1)),
        int(m.group(2)) if m.group(2) is not None else 1,
        int(m.group(3)),
        int(m.group(4)) if m.group(4) is not None else 1,
        m.group(5),
    )


def _annotate_hunk_body(body_lines, old_start, new_start):
    """Annotate *body_lines* with ``(old_num | None, new_num | None, text)``."""
    old_num, new_num = old_start, new_start
    annotated = []
    for text in body_lines:
        ch = text[0] if text else " "
        if ch == "+":
            annotated.append((None, new_num, text))
            new_num += 1
        elif ch == "-":
            annotated.append((old_num, None, text))
            old_num += 1
        elif ch == "\\":
            annotated.append((None, None, text))
        else:
            annotated.append((old_num, new_num, text))
            old_num += 1
            new_num += 1
    return annotated


def _split_hunks(lines):
    """Split *lines* (with trailing empties already stripped) into sub-hunks.

    Returns a list of ``(header_tuple, header_text, body_lines)`` where
    *header_tuple* is the parsed ``(old_start, old_count, new_start, new_count,
    ctx)`` and *body_lines* are the raw diff body strings.
    """
    hunks = []
    current_header = None
    current_header_text = None
    current_body = []

    for line in lines:
        parsed = _parse_hunk_header(line)
        if parsed is not None:
            if current_header is not None:
                hunks.append((current_header, current_header_text, current_body))
            current_header = parsed
            current_header_text = line
            current_body = []
        else:
            current_body.append(line)

    if current_header is not None:
        hunks.append((current_header, current_header_text, current_body))

    return hunks


def _find_target_idx(annotated, target_line, use_new):
    """Return the index into *annotated* where the target line lives, or None."""
    for i, (o, n, _) in enumerate(annotated):
        num = n if use_new else o
        if num is not None and num == target_line:
            return i
    return None


# ---------------------------------------------------------------------------
# Public helpers used by fetch_github_review_comments.py
# ---------------------------------------------------------------------------

def _prepare_lines(diff_hunk):
    """Split *diff_hunk* into lines and strip trailing empty strings."""
    lines = diff_hunk.split("\n")
    while lines and lines[-1] == "":
        lines.pop()
    return lines


def line_in_hunk(diff_hunk, target_line, side="RIGHT"):
    """Return ``True`` if *target_line* is reachable on *side* of the hunk."""
    if not diff_hunk:
        return False

    lines = _prepare_lines(diff_hunk)
    use_new = side != "LEFT"

    for header, _, body in _split_hunks(lines):
        old_start, _, new_start, _, _ = header
        annotated = _annotate_hunk_body(body, old_start, new_start)
        if _find_target_idx(annotated, target_line, use_new) is not None:
            return True

    return False


def last_reachable_line(diff_hunk, side="RIGHT"):
    """Return the last line number reachable on *side*, or ``None``."""
    if not diff_hunk:
        return None

    lines = _prepare_lines(diff_hunk)
    use_new = side != "LEFT"
    last = None

    for header, _, body in _split_hunks(lines):
        old_start, _, new_start, _, _ = header
        for o, n, _ in _annotate_hunk_body(body, old_start, new_start):
            num = n if use_new else o
            if num is not None:
                last = num

    return last


# ---------------------------------------------------------------------------
# trim_diff_hunk
# ---------------------------------------------------------------------------

def trim_diff_hunk(diff_hunk, target_line, side="RIGHT", start_line=None, context_lines=0):
    """Return *diff_hunk* trimmed to ±*context_lines* around *target_line*.

    The hunk header is rewritten so the line numbers stay correct.
    If the hunk is already small enough, it is returned unchanged.
    """
    if not diff_hunk:
        return diff_hunk

    lines = _prepare_lines(diff_hunk)

    hunks = _split_hunks(lines)
    if not hunks:
        return diff_hunk

    use_new = side != "LEFT"

    # Find the sub-hunk that contains the target line.
    for header, _header_text, body in hunks:
        old_start, _, new_start, _, hdr_ctx = header
        annotated = _annotate_hunk_body(body, old_start, new_start)

        target_idx = _find_target_idx(annotated, target_line, use_new)
        if target_idx is None:
            continue

        # Found the right sub-hunk — trim within it.
        if len(annotated) <= context_lines * 2 + 1:
            # Small enough already — return just this sub-hunk.
            new_hdr = _rewrite_header(annotated, old_start, new_start, hdr_ctx)
            return "\n".join([new_hdr] + [t for _, _, t in annotated])

        range_start_idx = None
        if start_line:
            range_start_idx = _find_target_idx(annotated, start_line, use_new)

        first = range_start_idx if range_start_idx is not None else target_idx
        lo = max(0, first - context_lines)
        hi = min(len(annotated) - 1, target_idx + context_lines)
        trimmed = annotated[lo : hi + 1]

        new_hdr = _rewrite_header(trimmed, old_start, new_start, hdr_ctx)
        return "\n".join([new_hdr] + [t for _, _, t in trimmed])

    # Target not found in any sub-hunk — return original unchanged.
    return diff_hunk


def _rewrite_header(trimmed, old_start, new_start, hdr_ctx):
    """Build a unified diff header from a trimmed annotation window."""
    t_os = t_ns = None
    t_oc = t_nc = 0
    for o, n, text in trimmed:
        ch = text[0] if text else " "
        if ch == "+":
            t_nc += 1
            if t_ns is None and n is not None:
                t_ns = n
        elif ch == "-":
            t_oc += 1
            if t_os is None and o is not None:
                t_os = o
        elif ch == "\\":
            pass
        else:
            t_oc += 1
            t_nc += 1
            if t_os is None and o is not None:
                t_os = o
            if t_ns is None and n is not None:
                t_ns = n

    return f"@@ -{t_os or old_start},{t_oc} +{t_ns or new_start},{t_nc} @@{hdr_ctx}"
