#!/usr/bin/env python3
"""Fetch GitHub PR review comments and output JSON for insert_code_review_comments.

Requires: gh CLI (authenticated), git.
Must be run from within a git repository whose current branch has an open PR.

Prints JSON to stdout matching the insert_code_review_comments tool schema.
"""

import json
import os
import subprocess
import sys
from urllib.parse import urlparse

from trim_diff_hunk import trim_diff_hunk, line_in_hunk, last_reachable_line


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

def run_command(args, error_msg="Command failed"):
    """Run a command and return stdout. Exits on failure."""
    result = subprocess.run(
        args,
        capture_output=True,
        text=True,
        encoding="utf-8",
        env={**os.environ, "GH_PAGER": ""},
    )
    if result.returncode != 0:
        print(f"{error_msg}: {result.stderr.strip()}", file=sys.stderr)
        sys.exit(1)
    return result.stdout


def base_repo_from_url(url):
    """Return ``(owner, repo)`` for the base repository from a PR URL.

    PR review/issue comments live on the **base** repository (the repo that
    owns the PR), not the head/fork repo. For a PR opened from a fork the two
    differ, so addressing the head repo 404s. A PR's ``url`` always points at
    the base repo (e.g. ``https://github.com/OWNER/REPO/pull/123``), so parse
    owner/repo from it to resolve correctly for both same-repo and fork PRs.
    """
    parts = urlparse(url).path.strip("/").split("/")
    if len(parts) < 2 or not parts[0] or not parts[1]:
        print(f"Could not parse owner/repo from PR URL: {url!r}", file=sys.stderr)
        sys.exit(1)
    return parts[0], parts[1]


def run_gh_api(endpoint):
    """Run ``gh api --paginate`` and return a list of JSON objects.

    Handles the case where ``gh`` outputs multiple concatenated JSON arrays
    (one per page).
    """
    text = run_command(
        ["gh", "api", endpoint, "--paginate"],
        error_msg=f"gh api {endpoint} failed",
    ).strip()
    if not text:
        return []

    items = []
    decoder = json.JSONDecoder()
    pos = 0
    try:
        while pos < len(text):
            while pos < len(text) and text[pos] in " \t\n\r":
                pos += 1
            if pos >= len(text):
                break
            obj, end = decoder.raw_decode(text, pos)
            items.extend(obj if isinstance(obj, list) else [obj])
            pos = end
    except json.JSONDecodeError as exc:
        print(f"Failed to parse API response: {exc}", file=sys.stderr)
        sys.exit(1)
    return items


# ---------------------------------------------------------------------------
# Comment building
# ---------------------------------------------------------------------------

def _comment(cid, author, ts, body, url, location=None, reply_to=None):
    """Build a dict matching the insert_code_review_comments comment schema."""
    c = {
        "comment_id": cid,
        "author": author,
        "last_modified_timestamp": ts,
        "comment_body": body,
        "html_url": url,
    }
    if reply_to:
        c["reply_metadata"] = {"parent_comment_id": reply_to}
    elif location:
        c["location_metadata"] = location
    return c


def _resolve_line(hunk, line, original_line, side):
    """Pick the first line number that is reachable in the hunk on *side*.

    Tries *line* first (current diff position), then *original_line*
    (position when the comment was placed).  If neither is reachable,
    falls back to the last reachable line in the hunk on *side*.

    Returns the resolved line number, or ``None`` if nothing is reachable.
    """
    if line and line_in_hunk(hunk, line, side):
        return line
    if original_line and line_in_hunk(hunk, original_line, side):
        return original_line
    return last_reachable_line(hunk, side)


def _resolve_comment_line(comment, hunk):
    """Resolve validated (end_line, start_line, side) for a diff comment.

    Uses ``side`` from the GitHub API as the authoritative diff side.
    Returns ``(end_line, start_line | None, side)`` or ``None`` when the
    comment cannot be attached to any line in the hunk.
    """
    side = comment.get("side") or "RIGHT"

    end_line = _resolve_line(
        hunk,
        comment.get("line"),
        comment.get("original_line"),
        side,
    )
    if end_line is None:
        return None

    raw_start = comment.get("start_line")
    raw_original_start = comment.get("original_start_line")
    if raw_start or raw_original_start:
        start_line = _resolve_line(hunk, raw_start, raw_original_start, side)
        if start_line is not None and start_line > end_line:
            start_line = None
    else:
        start_line = None

    return (end_line, start_line, side)


# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------

def main():
    repo_root = run_command(
        ["git", "rev-parse", "--show-toplevel"],
        "Not a git repository",
    ).strip()

    pr = json.loads(
        run_command(
            [
                "gh", "pr", "view",
                "--json", "number,url,baseRefName",
            ],
            "Failed to get PR info (is there an open PR on this branch?)",
        )
    )
    number = pr["number"]
    base = pr["baseRefName"]
    # Comments live on the base repo, which the PR URL points at — this is
    # correct even when the PR was opened from a fork.
    owner, repo = base_repo_from_url(pr["url"])

    api = f"/repos/{owner}/{repo}"

    issue_comments = run_gh_api(f"{api}/issues/{number}/comments")
    diff_comments = run_gh_api(f"{api}/pulls/{number}/comments")
    reviews = run_gh_api(f"{api}/pulls/{number}/reviews")

    comments = []

    # -- Issue comments (PR-level, no location or reply metadata) -----------
    for c in issue_comments:
        comments.append(
            _comment(
                str(c["id"]),
                c["user"]["login"] if c.get("user") else "[deleted]",
                c["updated_at"],
                c["body"],
                c["html_url"],
            )
        )

    # -- Diff comments (line-level, with location or reply) -----------------
    for c in diff_comments:
        cid = str(c["id"])
        author = c["user"]["login"] if c.get("user") else "[deleted]"
        ts = c["updated_at"]
        body = c["body"]
        url = c["html_url"]

        reply_to_id = c.get("in_reply_to_id")
        if reply_to_id:
            comments.append(
                _comment(cid, author, ts, body, url, reply_to=str(reply_to_id))
            )
            continue

        hunk = c.get("diff_hunk", "")
        resolved = _resolve_comment_line(c, hunk)

        loc = {"filepath": c["path"]}
        if resolved:
            end_line, start_line, side = resolved
            if hunk:
                loc["diff_hunk"] = trim_diff_hunk(
                    hunk, end_line, side=side, start_line=start_line
                )
            loc["end_line"] = end_line
            if start_line:
                loc["start_line"] = start_line
            loc["side"] = side

        comments.append(_comment(cid, author, ts, body, url, location=loc))

    # -- Reviews (PR-level, no location) ------------------------------------
    for r in reviews:
        if not r.get("body"):
            continue
        comments.append(
            _comment(
                str(r["id"]),
                r["user"]["login"] if r.get("user") else "[deleted]",
                r.get("submitted_at", ""),
                r["body"],
                r["html_url"],
            )
        )

    result = {
        "local_repository_path": repo_root,
        "base_branch": base,
        "comments": comments,
    }

    json.dump(result, sys.stdout, indent=2)


if __name__ == "__main__":
    main()
