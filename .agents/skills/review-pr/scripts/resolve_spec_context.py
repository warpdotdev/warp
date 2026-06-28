from __future__ import annotations

import argparse
import base64
import json
import os
import re
import urllib.error
import urllib.parse
import urllib.request
from datetime import datetime, timezone
from pathlib import Path
from typing import Any


API_ROOT = "https://api.github.com"
GRAPHQL_ROOT = f"{API_ROOT}/graphql"
NO_SPEC_CONTEXT_MESSAGE = "No approved or repository spec context was found for this PR."
REPO_ROOT = Path(
    (os.environ.get("OZ_REPO_ROOT") or "").strip()
    or Path(__file__).resolve().parents[4]
)

_CLOSING_ISSUES_QUERY = (
    "query($owner: String!, $name: String!, $number: Int!, $after: String) {"
    " repository(owner: $owner, name: $name) {"
    " pullRequest(number: $number) {"
    " closingIssuesReferences(first: 100, after: $after) {"
    " pageInfo { hasNextPage endCursor }"
    " nodes {"
    " number"
    " repository { owner { login } name }"
    " }"
    " }"
    " }"
    " }"
    " }"
)

_MANUAL_LINKED_ISSUES_QUERY = (
    "query($owner: String!, $name: String!, $number: Int!, $after: String) {"
    " repository(owner: $owner, name: $name) {"
    " pullRequest(number: $number) {"
    " timelineItems(first: 100, after: $after, itemTypes: [CONNECTED_EVENT, DISCONNECTED_EVENT]) {"
    " pageInfo { hasNextPage endCursor }"
    " nodes {"
    " __typename"
    " ... on ConnectedEvent {"
    " subject {"
    " __typename"
    " ... on Issue {"
    " number"
    " repository { owner { login } name }"
    " }"
    " }"
    " }"
    " ... on DisconnectedEvent {"
    " subject {"
    " __typename"
    " ... on Issue {"
    " number"
    " repository { owner { login } name }"
    " }"
    " }"
    " }"
    " }"
    " }"
    " }"
    " }"
    " }"
)


def _resolve_token() -> str:
    token = (os.environ.get("GH_TOKEN") or os.environ.get("GITHUB_TOKEN") or "").strip()
    if not token:
        raise SystemExit(
            "GH_TOKEN or GITHUB_TOKEN must be set to resolve PR spec context."
        )
    return token


def _parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Resolve approved or repository spec context for a pull request."
    )
    parser.add_argument(
        "--repo",
        required=True,
        help="Repository slug in OWNER/REPO format.",
    )
    parser.add_argument(
        "--pr",
        type=int,
        required=True,
        help="Pull request number to resolve spec context for.",
    )
    return parser.parse_args()


def _gh_request(
    path_or_url: str,
    *,
    token: str,
    accept: str = "application/vnd.github+json",
    params: dict[str, str] | None = None,
    method: str = "GET",
    payload: bytes | None = None,
    allow_http_error: bool = False,
) -> tuple[int, bytes, dict[str, str]]:
    url = path_or_url if path_or_url.startswith("https://") else f"{API_ROOT}{path_or_url}"
    if params:
        url = f"{url}?{urllib.parse.urlencode(params)}"
    request = urllib.request.Request(url, data=payload, method=method)  # noqa: S310
    request.add_header("Authorization", f"Bearer {token}")
    request.add_header("Accept", accept)
    request.add_header("X-GitHub-Api-Version", "2022-11-28")
    request.add_header("User-Agent", "oz-resolve-review-spec-context")
    if payload is not None:
        request.add_header("Content-Type", "application/json")
    try:
        with urllib.request.urlopen(request) as response:  # noqa: S310
            return response.status, response.read(), dict(response.headers)
    except urllib.error.HTTPError as exc:
        body = exc.read() if exc.fp is not None else b""
        if allow_http_error:
            return exc.code, body, dict(exc.headers or {})
        detail = body.decode("utf-8", errors="replace")[:500]
        raise SystemExit(
            f"GitHub API request failed ({exc.code}) for {path_or_url}: {detail}"
        ) from exc


def _gh_json(
    path: str,
    *,
    token: str,
    params: dict[str, str] | None = None,
    allow_http_error: bool = False,
) -> tuple[int, Any]:
    status, body, _headers = _gh_request(
        path,
        token=token,
        params=params,
        allow_http_error=allow_http_error,
    )
    return status, json.loads(body.decode("utf-8"))


def _parse_next_link(link_header: str) -> str | None:
    if not link_header:
        return None
    for piece in link_header.split(","):
        segment = piece.strip()
        if not segment.startswith("<"):
            continue
        end = segment.find(">")
        if end == -1:
            continue
        url = segment[1:end]
        rel_part = segment[end + 1 :]
        if 'rel="next"' not in rel_part:
            continue
        parsed = urllib.parse.urlparse(url)
        return parsed.path + (f"?{parsed.query}" if parsed.query else "")
    return None


def _gh_paginated_json(
    path: str,
    *,
    token: str,
    params: dict[str, str] | None = None,
    per_page: int = 100,
) -> list[Any]:
    merged_params = dict(params or {})
    merged_params.setdefault("per_page", str(per_page))
    next_path: str | None = (
        f"{path}?{urllib.parse.urlencode(merged_params)}" if merged_params else path
    )
    items: list[Any] = []
    while next_path:
        status, body, headers = _gh_request(next_path, token=token)
        if status != 200:
            raise SystemExit(f"GitHub API returned status {status} for {next_path}")
        page = json.loads(body.decode("utf-8"))
        if not isinstance(page, list):
            raise SystemExit(
                f"Expected JSON array from {next_path}, got {type(page).__name__}."
            )
        items.extend(page)
        next_path = _parse_next_link(headers.get("Link") or headers.get("link") or "")
    return items


def _gh_graphql_json(query: str, variables: dict[str, Any], *, token: str) -> dict[str, Any]:
    payload = json.dumps({"query": query, "variables": variables}).encode("utf-8")
    _status, body, _headers = _gh_request(
        GRAPHQL_ROOT,
        token=token,
        accept="application/json",
        method="POST",
        payload=payload,
    )
    data = json.loads(body.decode("utf-8"))
    errors = data.get("errors") or []
    if errors:
        raise SystemExit(f"GitHub GraphQL request failed: {errors}")
    return data


def parse_datetime(value: str) -> datetime:
    return datetime.fromisoformat(value.replace("Z", "+00:00")).astimezone(timezone.utc)


def read_local_spec_files(workspace: Path, issue_number: int) -> list[tuple[str, str]]:
    spec_dir_name = f"GH{issue_number}"
    spec_dir = workspace / "specs" / spec_dir_name
    results: list[tuple[str, str]] = []
    for name in ("product.md", "tech.md"):
        path = spec_dir / name
        if path.exists():
            results.append(
                (f"specs/{spec_dir_name}/{name}", path.read_text(encoding="utf-8").strip())
            )
    return results


def _fetch_pull(owner: str, repo: str, pr_number: int, *, token: str) -> dict[str, Any]:
    _status, payload = _gh_json(
        f"/repos/{owner}/{repo}/pulls/{pr_number}",
        token=token,
    )
    if not isinstance(payload, dict):
        raise SystemExit(
            f"Expected object payload for pull request #{pr_number}, got {type(payload).__name__}."
        )
    return payload


def _fetch_pull_files(
    owner: str,
    repo: str,
    pr_number: int,
    *,
    token: str,
) -> list[dict[str, Any]]:
    files = _gh_paginated_json(
        f"/repos/{owner}/{repo}/pulls/{pr_number}/files",
        token=token,
    )
    return [item for item in files if isinstance(item, dict)]


def _fetch_issue(
    owner: str,
    repo: str,
    issue_number: int,
    *,
    token: str,
) -> dict[str, Any] | None:
    status, payload = _gh_json(
        f"/repos/{owner}/{repo}/issues/{issue_number}",
        token=token,
        allow_http_error=True,
    )
    if status == 404:
        return None
    if not isinstance(payload, dict):
        raise SystemExit(
            f"Expected object payload for issue #{issue_number}, got {type(payload).__name__}."
        )
    return payload


def _fetch_file_contents(
    owner: str,
    repo: str,
    path: str,
    *,
    ref: str,
    token: str,
) -> str | None:
    encoded_path = urllib.parse.quote(path, safe="/")
    status, payload = _gh_json(
        f"/repos/{owner}/{repo}/contents/{encoded_path}",
        token=token,
        params={"ref": ref},
        allow_http_error=True,
    )
    if status == 404:
        return None
    if not isinstance(payload, dict):
        return None
    content = str(payload.get("content") or "").strip()
    encoding = str(payload.get("encoding") or "").strip().lower()
    if not content or encoding != "base64":
        return None
    return base64.b64decode(content.encode("utf-8")).decode("utf-8").strip()


def _normalize_github_linked_issue(node: Any, *, source: str) -> dict[str, Any] | None:
    if not isinstance(node, dict):
        return None
    number = node.get("number")
    if not isinstance(number, int):
        return None
    repository = node.get("repository") or {}
    owner = ((repository.get("owner") or {}).get("login") or "").strip()
    repo = str(repository.get("name") or "").strip()
    if not owner or not repo:
        return None
    return {
        "owner": owner,
        "repo": repo,
        "number": number,
        "source": source,
    }


def _graphql_pull_request_data(
    owner: str,
    repo: str,
    pr_number: int,
    query: str,
    *,
    token: str,
    after: str | None,
) -> dict[str, Any]:
    data = _gh_graphql_json(
        query,
        {
            "owner": owner,
            "name": repo,
            "number": int(pr_number),
            "after": after,
        },
        token=token,
    )
    return (((data.get("data") or {}).get("repository") or {}).get("pullRequest") or {})


def _fetch_closing_issue_references(
    owner: str,
    repo: str,
    pr_number: int,
    *,
    token: str,
) -> list[dict[str, Any]]:
    linked: dict[tuple[str, str, int, str], dict[str, Any]] = {}
    cursor: str | None = None
    while True:
        pr_data = _graphql_pull_request_data(
            owner,
            repo,
            pr_number,
            _CLOSING_ISSUES_QUERY,
            token=token,
            after=cursor,
        )
        closing_refs = pr_data.get("closingIssuesReferences") or {}
        for node in closing_refs.get("nodes") or []:
            issue_ref = _normalize_github_linked_issue(
                node,
                source="closingIssuesReferences",
            )
            if issue_ref is None:
                continue
            key = (
                issue_ref["owner"].lower(),
                issue_ref["repo"].lower(),
                int(issue_ref["number"]),
                str(issue_ref["source"]),
            )
            linked[key] = issue_ref
        page_info = closing_refs.get("pageInfo") or {}
        if not page_info.get("hasNextPage"):
            break
        cursor = page_info.get("endCursor")
        if not cursor:
            break
    return sorted(
        linked.values(),
        key=lambda item: (
            str(item["owner"]).lower(),
            str(item["repo"]).lower(),
            int(item["number"]),
            str(item["source"]),
        ),
    )


def _fetch_manual_linked_issue_references(
    owner: str,
    repo: str,
    pr_number: int,
    *,
    token: str,
) -> list[dict[str, Any]]:
    connected: dict[tuple[str, str, int], dict[str, Any]] = {}
    cursor: str | None = None
    while True:
        pr_data = _graphql_pull_request_data(
            owner,
            repo,
            pr_number,
            _MANUAL_LINKED_ISSUES_QUERY,
            token=token,
            after=cursor,
        )
        timeline_items = pr_data.get("timelineItems") or {}
        for node in timeline_items.get("nodes") or []:
            if not isinstance(node, dict):
                continue
            issue_ref = _normalize_github_linked_issue(
                node.get("subject"),
                source="manualLink",
            )
            if issue_ref is None:
                continue
            key = (
                issue_ref["owner"].lower(),
                issue_ref["repo"].lower(),
                int(issue_ref["number"]),
            )
            typename = str(node.get("__typename") or "")
            if typename == "ConnectedEvent":
                connected[key] = issue_ref
            elif typename == "DisconnectedEvent":
                connected.pop(key, None)
        page_info = timeline_items.get("pageInfo") or {}
        if not page_info.get("hasNextPage"):
            break
        cursor = page_info.get("endCursor")
        if not cursor:
            break
    return sorted(
        connected.values(),
        key=lambda item: (
            str(item["owner"]).lower(),
            str(item["repo"]).lower(),
            int(item["number"]),
            str(item["source"]),
        ),
    )


def _dedupe_ints(values: list[int]) -> list[int]:
    return list(dict.fromkeys(int(value) for value in values))


def _same_repo_issue_numbers(
    owner: str,
    repo: str,
    issue_refs: list[dict[str, Any]],
) -> list[int]:
    normalized_owner = owner.lower()
    normalized_repo = repo.lower()
    return _dedupe_ints(
        [
            int(issue_ref["number"])
            for issue_ref in issue_refs
            if str(issue_ref.get("owner") or "").lower() == normalized_owner
            and str(issue_ref.get("repo") or "").lower() == normalized_repo
        ]
    )


def _deterministic_issue_candidates(pr: dict[str, Any], changed_files: list[str]) -> list[int]:
    head_ref = str(((pr.get("head") or {}).get("ref")) or "")
    branch_issue_matches = [
        int(match.group(1))
        for match in re.finditer(
            r"(?:^|/)(?:spec|implement)-issue-(\d+)(?:$|[/-])",
            head_ref,
        )
    ]
    spec_file_issue_numbers = [
        int(match.group(1))
        for filename in changed_files
        for match in [re.match(r"^specs/GH(\d+)/(?:product|tech)\.md$", filename)]
        if match
    ]
    return _dedupe_ints(branch_issue_matches + spec_file_issue_numbers)


def _resolve_deterministic_issue_numbers(
    owner: str,
    repo: str,
    pr: dict[str, Any],
    changed_files: list[str],
    *,
    token: str,
) -> list[int]:
    resolved: list[int] = []
    for candidate in _deterministic_issue_candidates(pr, changed_files):
        issue = _fetch_issue(owner, repo, candidate, token=token)
        if issue is None:
            continue
        if not issue.get("pull_request"):
            resolved.append(candidate)
    return resolved


def resolve_issue_number_for_pr(
    owner: str,
    repo: str,
    pr_number: int,
    pr: dict[str, Any],
    changed_files: list[str],
    *,
    token: str,
) -> int | None:
    deterministic_issue_numbers = _resolve_deterministic_issue_numbers(
        owner,
        repo,
        pr,
        changed_files,
        token=token,
    )
    github_linked_issues = _fetch_closing_issue_references(
        owner,
        repo,
        pr_number,
        token=token,
    )
    github_linked_issues.extend(
        _fetch_manual_linked_issue_references(
            owner,
            repo,
            pr_number,
            token=token,
        )
    )
    same_repo_linked_numbers = _same_repo_issue_numbers(owner, repo, github_linked_issues)

    primary_issue_number: int | None = None
    if len(deterministic_issue_numbers) == 1:
        primary_issue_number = deterministic_issue_numbers[0]
    elif len(deterministic_issue_numbers) == 0 and len(same_repo_linked_numbers) == 1:
        primary_issue_number = same_repo_linked_numbers[0]
    return primary_issue_number


def find_matching_spec_prs(
    owner: str,
    repo: str,
    issue_number: int,
    *,
    token: str,
) -> tuple[list[dict[str, Any]], list[dict[str, Any]]]:
    expected_spec_branch = f"oz-agent/spec-issue-{issue_number}"
    matching = _gh_paginated_json(
        f"/repos/{owner}/{repo}/pulls",
        token=token,
        params={"state": "all", "head": f"{owner}:{expected_spec_branch}"},
    )
    approved: list[dict[str, Any]] = []
    unapproved: list[dict[str, Any]] = []
    for pr in matching:
        if not isinstance(pr, dict):
            continue
        pr_number = int(pr.get("number") or 0)
        if pr_number <= 0:
            continue
        issue_payload = _fetch_issue(owner, repo, pr_number, token=token) or {}
        labels = [
            str((label or {}).get("name") or "")
            for label in issue_payload.get("labels", []) or []
            if isinstance(label, dict)
        ]
        files = _fetch_pull_files(owner, repo, pr_number, token=token)
        spec_files = [
            str(file.get("filename") or "")
            for file in files
            if str(file.get("filename") or "").startswith("specs/")
        ]
        head = pr.get("head") or {}
        head_repo = head.get("repo") or {}
        entry = {
            "number": pr_number,
            "url": str(pr.get("html_url") or ""),
            "updated_at": str(pr.get("updated_at") or ""),
            "head_ref_name": str(head.get("ref") or ""),
            "head_repo_full_name": str(head_repo.get("full_name") or ""),
            "spec_files": spec_files,
        }
        if "plan-approved" in labels:
            approved.append(entry)
        else:
            unapproved.append(entry)
    approved.sort(key=lambda item: parse_datetime(item["updated_at"]), reverse=True)
    unapproved.sort(key=lambda item: parse_datetime(item["updated_at"]), reverse=True)
    return approved, unapproved


def resolve_spec_context_for_issue(
    owner: str,
    repo: str,
    issue_number: int,
    *,
    workspace: Path,
    token: str,
) -> dict[str, Any]:
    approved, unapproved = find_matching_spec_prs(owner, repo, issue_number, token=token)
    selected = approved[0] if approved else None
    local_specs = read_local_spec_files(workspace, issue_number)
    if selected and selected["head_repo_full_name"] != f"{owner}/{repo}":
        raise RuntimeError(
            f"Linked approved spec PR #{selected['number']} uses branch "
            f"{selected['head_repo_full_name']}:{selected['head_ref_name']}, which this workflow cannot push to."
        )

    spec_context_source = "approved-pr" if selected else "directory" if local_specs else ""
    spec_entries: list[dict[str, str]] = []
    if selected:
        for path in selected["spec_files"]:
            content = _fetch_file_contents(
                owner,
                repo,
                path,
                ref=selected["head_ref_name"],
                token=token,
            )
            if content:
                spec_entries.append({"path": path, "content": content})
    elif local_specs:
        for path, content in local_specs:
            spec_entries.append({"path": path, "content": content})

    return {
        "selected_spec_pr": selected,
        "approved_spec_prs": approved,
        "unapproved_spec_prs": unapproved,
        "spec_context_source": spec_context_source,
        "spec_entries": spec_entries,
    }


def resolve_spec_context_for_pr(
    owner: str,
    repo: str,
    pr_number: int,
    *,
    workspace: Path,
    token: str,
) -> dict[str, Any]:
    pr = _fetch_pull(owner, repo, pr_number, token=token)
    files = _fetch_pull_files(owner, repo, pr_number, token=token)
    changed_files = [str(file.get("filename") or "") for file in files]
    issue_number = resolve_issue_number_for_pr(
        owner,
        repo,
        pr_number,
        pr,
        changed_files,
        token=token,
    )
    if not issue_number:
        return {
            "issue_number": None,
            "spec_context_source": "",
            "selected_spec_pr": None,
            "spec_entries": [],
            "changed_files": changed_files,
        }
    spec_context = resolve_spec_context_for_issue(
        owner,
        repo,
        issue_number,
        workspace=workspace,
        token=token,
    )
    spec_context["issue_number"] = issue_number
    spec_context["changed_files"] = changed_files
    return spec_context


def _format_spec_context(spec_context: dict[str, object]) -> str:
    sections: list[str] = []
    selected_spec_pr = spec_context.get("selected_spec_pr")
    source = str(spec_context.get("spec_context_source") or "")
    if (
        source == "approved-pr"
        and isinstance(selected_spec_pr, dict)
        and selected_spec_pr.get("number")
        and selected_spec_pr.get("url")
    ):
        sections.append(
            f"Linked approved spec PR: [#{selected_spec_pr['number']}]({selected_spec_pr['url']})"
        )
    elif source == "directory":
        sections.append("Repository spec context was found in `specs/`.")
    for entry in spec_context.get("spec_entries", []):
        if not isinstance(entry, dict):
            continue
        path = str(entry.get("path") or "").strip()
        content = str(entry.get("content") or "").strip()
        if not path or not content:
            continue
        sections.append(f"## {path}\n\n{content}")
    return "\n\n".join(sections).strip() or NO_SPEC_CONTEXT_MESSAGE


def main() -> None:
    args = _parse_args()
    if "/" not in args.repo:
        raise SystemExit(
            f"Invalid repository slug: {args.repo!r}. Expected OWNER/REPO."
        )
    owner, repo = args.repo.split("/", 1)
    spec_context = resolve_spec_context_for_pr(
        owner,
        repo,
        args.pr,
        workspace=REPO_ROOT,
        token=_resolve_token(),
    )
    print(_format_spec_context(spec_context))


if __name__ == "__main__":
    main()
