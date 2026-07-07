#!/usr/bin/env python3
"""Fetch PRs merged in a release range and extract explicit CHANGELOG markers.

Uses `gh` CLI (must be authenticated) and `git` — stdlib only, no pip deps.

Usage:
    python3 fetch_prs.py --repo warpdotdev/warp --base-ref <prev_tag> --head-ref <release_tag>

Outputs JSON to stdout.
"""

import argparse
import json
import re
import subprocess
import sys

# Matches lines like: CHANGELOG-NEW-FEATURE: Added dark mode
MARKER_RE = re.compile(
    r"^CHANGELOG-(NEW-FEATURE|IMPROVEMENT|BUG-FIX|IMAGE|OZ|NONE)\s*:?\s*(.*)$",
    re.MULTILINE,
)

# Matches issue-closing keywords: Fixes #123, Closes #456, Resolves #789
LINKED_ISSUE_RE = re.compile(
    r"(?:close[sd]?|fix(?:e[sd])?|resolve[sd]?)\s+#(\d+)",
    re.IGNORECASE,
)
PUBLIC_REPO = "warpdotdev/warp"
INTERNAL_REPO = "warpdotdev/warp-internal"
REPO_SYNC_AUTHORS = frozenset(
    {
        "app/warp-repo-sync",
        "warp-repo-sync",
        "warp-repo-sync[bot]",
    }
)
PUBLIC_PR_URL_RE = re.compile(r"https://github\.com/warpdotdev/warp/pull/(\d+)")


def run(cmd: list[str], *, check: bool = True) -> str:
    result = subprocess.run(cmd, capture_output=True, text=True, check=check)
    return result.stdout.strip()


def get_commits(base_ref: str, head_ref: str) -> list[str]:
    """Return SHAs of first-parent commits between base and head."""
    log = run(
        [
            "git",
            "log",
            "--first-parent",
            "--format=%H",
            f"{base_ref}..{head_ref}",
        ]
    )
    if not log:
        return []
    return log.splitlines()


def extract_pr_number(sha: str) -> int | None:
    """Extract PR number from a squash-merge commit subject line.

    Expects the GitHub squash format: 'feat: something (#1234)'.
    Matches the trailing parenthesized (#N) to avoid grabbing issue
    numbers from titles like 'Fixes #123 (#456)'.
    """
    msg = run(["git", "log", "-1", "--format=%s", sha])
    # Match the last (#N) in the subject — GitHub always appends the PR number
    m = re.search(r"\(#(\d+)\)\s*$", msg)
    if m:
        return int(m.group(1))
    # Fallback: first bare #N (for non-standard subjects)
    m = re.search(r"#(\d+)", msg)
    if m:
        return int(m.group(1))
    return None


def get_merged_commits(sha: str) -> list[str]:
    """For a merge commit, return the SHAs brought in by the merge.

    A merge commit has two parents: the first parent is the mainline, the
    second parent is the tip of the merged branch. The commits unique to
    the merge are those reachable from the second parent but not the first.
    Returns an empty list for non-merge commits.
    """
    parents = run(["git", "log", "-1", "--format=%P", sha]).split()
    if len(parents) < 2:
        return []
    log = run(
        ["git", "log", "--format=%H", f"{parents[0]}..{parents[1]}"],
        check=False,
    )
    if not log:
        return []
    return log.splitlines()


def fetch_pr_data(repo: str, pr_number: int) -> dict | None:
    """Fetch PR metadata and changed file paths via gh CLI."""
    fields = "number,title,author,body,labels,mergedAt,files,url"
    raw = run(
        ["gh", "pr", "view", str(pr_number), "--repo", repo, "--json", fields],
        check=False,
    )
    if not raw:
        return None
    try:
        return json.loads(raw)
    except json.JSONDecodeError:
        return None


def fetch_pr_commit_messages(repo: str, pr_number: int) -> list[str]:
    """Fetch commit messages for a PR via the GitHub API."""
    raw = run(
        ["gh", "api", f"repos/{repo}/pulls/{pr_number}/commits"],
        check=False,
    )
    if not raw:
        return []
    try:
        commits = json.loads(raw)
    except json.JSONDecodeError:
        return []

    messages = []
    for commit in commits:
        if not isinstance(commit, dict):
            continue
        commit_data = commit.get("commit")
        if isinstance(commit_data, dict):
            message = commit_data.get("message")
            if message:
                messages.append(message)
    return messages


def get_author_login(data: dict) -> str:
    """Extract a GitHub login from a gh PR JSON object."""
    if isinstance(data.get("author"), dict):
        return data["author"].get("login", "")
    if isinstance(data.get("author"), str):
        return data["author"]
    return ""


def get_label_names(data: dict) -> list[str]:
    """Extract label names from a gh PR JSON object."""
    label_names = []
    for lbl in data.get("labels", []) or []:
        if isinstance(lbl, dict):
            label_names.append(lbl.get("name", ""))
        else:
            label_names.append(str(lbl))
    return label_names


def get_file_paths(data: dict) -> list[str]:
    """Extract changed file paths from a gh PR JSON object."""
    file_paths = []
    for f in data.get("files", []) or []:
        if isinstance(f, dict):
            file_paths.append(f.get("path", ""))
    return file_paths


def is_repo_sync_pr(data: dict) -> bool:
    """Return whether this PR was created by the public-to-internal repo sync bot."""
    return get_author_login(data) in REPO_SYNC_AUTHORS


def should_include_pr(repo: str, data: dict) -> bool:
    """Return whether a PR should be exposed to changelog generation.

    Releases are cut from warp-internal, but non-sync-bot PRs merged there are
    private/internal changes. Do not expose them to the Oz changelog agent or to
    generated artifacts.
    """
    return repo != INTERNAL_REPO or is_repo_sync_pr(data)


def extract_public_pr_number(text: str) -> int | None:
    """Extract a public warpdotdev/warp PR number from text."""
    if not text:
        return None
    m = PUBLIC_PR_URL_RE.search(text)
    if m:
        return int(m.group(1))
    # Repo-sync commits commonly preserve the original public squash-merge
    # subject, such as "feat: add thing (#1234)".
    m = re.search(r"\(#(\d+)\)\s*$", text.splitlines()[0] if text else "")
    if m:
        return int(m.group(1))
    return None


def resolve_public_pr_number(repo: str, pr_number: int, data: dict) -> int | None:
    """Resolve a repo-sync PR back to its original public warpdotdev/warp PR."""
    public_pr_number = extract_public_pr_number(data.get("body", "") or "")
    if public_pr_number is not None:
        return public_pr_number

    for message in fetch_pr_commit_messages(repo, pr_number):
        public_pr_number = extract_public_pr_number(message)
        if public_pr_number is not None:
            return public_pr_number
    return None


def pr_reference(repo: str, pr_number: int, data: dict) -> dict:
    """Build a compact audit reference to a PR."""
    return {
        "number": data.get("number", pr_number),
        "url": data.get("url", ""),
        "author": get_author_login(data),
        "title": data.get("title", ""),
        "repo": repo,
    }


def normalize_pr_data(repo: str, pr_number: int, data: dict) -> tuple[str, dict, dict | None]:
    """Resolve repo-sync PRs to public PR metadata.

    The release workflow runs from warp-internal, where public PRs are mirrored
    as warp-repo-sync[bot] PRs with different PR numbers. For changelog output
    and contributor attribution, use the original public PR metadata when it can
    be resolved, and keep the internal PR under `internal_pr` for audit only.
    """
    internal_pr = pr_reference(repo, pr_number, data) if repo != PUBLIC_REPO else None
    if repo == PUBLIC_REPO or not is_repo_sync_pr(data):
        return repo, data, internal_pr

    public_pr_number = resolve_public_pr_number(repo, pr_number, data)
    if public_pr_number is None:
        return repo, data, internal_pr

    public_data = fetch_pr_data(PUBLIC_REPO, public_pr_number)
    if public_data is None:
        return repo, data, internal_pr

    return PUBLIC_REPO, public_data, internal_pr


def extract_linked_issues(body: str) -> list[int]:
    """Extract issue numbers from closing keywords in a PR body."""
    if not body:
        return []
    return sorted(set(int(m.group(1)) for m in LINKED_ISSUE_RE.finditer(body)))


def strip_html_comments(text: str) -> str:
    """Remove HTML comment blocks (<!-- ... -->) from text.

    This prevents template placeholders inside HTML comments from being
    parsed as real CHANGELOG markers.
    """
    return re.sub(r"<!--.*?-->", "", text, flags=re.DOTALL)


def extract_markers(body: str) -> list[dict]:
    """Extract CHANGELOG-* markers from a PR body."""
    if not body:
        return []
    # Strip HTML comments so template placeholders aren't treated as real markers
    cleaned = strip_html_comments(body)
    entries = []
    has_opt_out = False
    for m in MARKER_RE.finditer(cleaned):
        category = m.group(1)
        text = m.group(2).strip()
        # CHANGELOG-NONE is an explicit opt-out — skip all other markers
        if category == "NONE":
            has_opt_out = True
            continue
        # Skip template placeholders
        if text.startswith("{{") or text.startswith("{text") or not text:
            continue
        entries.append({"category": category, "text": text})
    # If the PR explicitly opted out, return a special marker
    if has_opt_out:
        return [{"category": "NONE", "text": ""}]
    return entries


def main() -> None:
    parser = argparse.ArgumentParser(description="Fetch PRs in a release range")
    parser.add_argument("--repo", required=True, help="GitHub repo (owner/name)")
    parser.add_argument("--base-ref", required=True, help="Previous release tag")
    parser.add_argument("--head-ref", required=True, help="Current release tag")
    args = parser.parse_args()

    commit_shas = get_commits(args.base_ref, args.head_ref)

    seen_prs: set[int] = set()
    prs: list[dict] = []

    def process_pr(pr_num: int) -> None:
        """Fetch and record a single PR by number."""
        data = fetch_pr_data(args.repo, pr_num)
        if data is None:
            return
        if not should_include_pr(args.repo, data):
            return
        source_repo, data, internal_pr = normalize_pr_data(args.repo, pr_num, data)
        author_login = get_author_login(data)
        label_names = get_label_names(data)

        body = data.get("body", "") or ""
        explicit_entries = extract_markers(body)
        linked_issues = extract_linked_issues(body)
        file_paths = get_file_paths(data)

        pr = {
            "number": data.get("number", pr_num),
            "url": data.get("url", "") if source_repo == PUBLIC_REPO else "",
            "title": data.get("title", ""),
            "author": author_login,
            "body": body,
            "labels": label_names,
            "merged_at": data.get("mergedAt", ""),
            "explicit_entries": explicit_entries,
            "linked_issues": linked_issues,
            "changed_files": file_paths,
            "source_repo": source_repo,
        }
        if internal_pr is not None:
            pr["internal_pr"] = internal_pr
        prs.append(pr)

    for sha in commit_shas:
        pr_num = extract_pr_number(sha)
        if pr_num is not None and pr_num not in seen_prs:
            # Normal squash-merge commit
            seen_prs.add(pr_num)
            process_pr(pr_num)
        else:
            # Merge commit fallback: walk the merged-in commits for PR numbers.
            # This handles branches merged via merge commit (e.g. security-patches)
            # rather than the usual squash merge.
            for merged_sha in get_merged_commits(sha):
                inner_pr = extract_pr_number(merged_sha)
                if inner_pr is not None and inner_pr not in seen_prs:
                    seen_prs.add(inner_pr)
                    process_pr(inner_pr)

    output = {
        "range": {"base": args.base_ref, "head": args.head_ref},
        "prs": prs,
    }
    json.dump(output, sys.stdout, indent=2)
    print()  # trailing newline


if __name__ == "__main__":
    main()
