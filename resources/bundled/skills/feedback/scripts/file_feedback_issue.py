#!/usr/bin/env python3

from __future__ import annotations

import argparse
import json
import os
import platform
import urllib.parse
import webbrowser
from pathlib import Path
import shutil
import subprocess
import sys

DEFAULT_HOSTNAME = "github.com"
FEEDBACK_LABEL = "in-app-feedback"

# GitHub's new-issue page accepts a prefilled title and body via query
# parameters, but browsers and intermediate servers commonly cap URLs around
# 8 KB. Keep a conservative threshold that leaves headroom for the base URL
# and percent-encoding overhead.
MAX_PREFILL_URL_LENGTH = 8000


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description=(
            "File a GitHub issue in the requested GitHub repository. The caller must choose "
            "the filing method via --use: `gh` to create the issue directly with the "
            "gh CLI, or `browser` to open the prefilled new-issue page in the browser "
            "(used when attachments require manual upload via GitHub's web UI)."
        )
    )
    parser.add_argument(
        "--use",
        dest="use_method",
        required=True,
        choices=["gh", "browser"],
        help=(
            "Filing method. `gh` creates the issue via the gh CLI (requires gh to be "
            "installed and authenticated). `browser` opens the prefilled new-issue "
            "page in the user's browser so attachments can be uploaded via GitHub's "
            "web UI."
        ),
    )

    parser.add_argument("--title", required=True, help="Issue title.")
    parser.add_argument(
        "--repo",
        required=True,
        help="GitHub repository to file the issue in, formatted as owner/name.",
    )
    parser.add_argument(
        "--body-file",
        type=Path,
        required=True,
        help="Path to a UTF-8 file containing the issue body.",
    )

    return parser.parse_args()


def read_text(path: Path, field_name: str) -> str:

    try:
        return path.read_text(encoding="utf-8")
    except OSError as exc:
        raise SystemExit(f"Failed to read {field_name} from {path}: {exc}") from exc


def normalize_title(title: str) -> str:
    normalized_title = " ".join(title.splitlines()).strip()
    if not normalized_title:
        raise SystemExit("Issue title must not be empty.")
    return normalized_title


def normalize_body(body: str) -> str:
    normalized_body = body.rstrip("\n")
    if not normalized_body.strip():
        raise SystemExit("Issue body must not be empty.")
    return normalized_body


def normalize_repo(repo: str) -> str:
    normalized_repo = repo.strip()
    if not normalized_repo or "/" not in normalized_repo:
        raise SystemExit("Issue repo must be provided in owner/name format.")
    owner, name = normalized_repo.split("/", 1)
    if not owner or not name or "/" in name:
        raise SystemExit("Issue repo must be provided in owner/name format.")
    return normalized_repo


def gh_path_if_authenticated() -> str | None:
    gh_path = shutil.which("gh")
    if gh_path is None:
        return None

    auth_result = subprocess.run(
        [gh_path, "auth", "status", "--hostname", DEFAULT_HOSTNAME],
        capture_output=True,
        text=True,
        check=False,
    )
    if auth_result.returncode != 0:
        return None

    return gh_path


def create_issue_with_gh(
    gh_path: str,
    repo: str,
    title: str,
    body: str,
) -> tuple[str | None, str | None]:
    command = [
        gh_path, "issue", "create",
        "--repo", repo,
        "--title", title,
        "--body", body,
        "--label", FEEDBACK_LABEL,
    ]

    result = subprocess.run(command, capture_output=True, text=True, check=False)
    if result.returncode != 0:
        error_output = "\n".join(part for part in [result.stdout.strip(), result.stderr.strip()] if part).strip()
        return None, error_output or "gh issue create failed"

    issue_url = result.stdout.strip().splitlines()[-1].strip()
    if not issue_url:
        return None, "gh issue create succeeded but did not return an issue URL"

    return issue_url, None


def build_new_issue_url(repo: str, title: str, body: str | None) -> str:
    """Build a GitHub new-issue URL with the provided title and optional body prefilled."""
    base = f"https://{DEFAULT_HOSTNAME}/{repo}/issues/new"
    params: list[tuple[str, str]] = [("title", title)]
    if body is not None:
        params.append(("body", body))
    return f"{base}?{urllib.parse.urlencode(params, quote_via=urllib.parse.quote)}"


def browser_is_available() -> tuple[bool, str | None]:
    """Return whether opening a browser is likely to succeed on this system.

    On macOS and Windows a GUI session is effectively always present for the
    user running this script. On Linux and other unix-likes ``webbrowser.open``
    can return True without actually opening a browser when no display server is
    running (for example, in a headless SSH session), so we require ``DISPLAY``
    or ``WAYLAND_DISPLAY`` to be set.
    """
    system = platform.system()
    if system in ("Darwin", "Windows"):
        return True, None
    if os.environ.get("DISPLAY") or os.environ.get("WAYLAND_DISPLAY"):
        return True, None
    return (
        False,
        (
            "No graphical display is available (neither DISPLAY nor WAYLAND_DISPLAY "
            "is set), so the GitHub new-issue page cannot be opened in a browser."
        ),
    )


def open_in_browser(url: str) -> bool:
    try:
        return webbrowser.open(url, new=2)
    except webbrowser.Error:
        return False


def fallback_to_browser(repo: str, title: str, body: str) -> int:
    """Open the GitHub new-issue page with a prefilled title (and body when it fits).

    Intended for the caller-selected ``--use browser`` path, which the feedback skill
    uses when image attachments need to be uploaded via GitHub's web UI. When the full
    prefill URL would exceed ``MAX_PREFILL_URL_LENGTH``, the body is omitted from the
    URL and surfaced in the JSON result under a ``body`` field so the caller can tell
    the user to paste it into the issue form manually.

    When the browser cannot be opened, the function attempts a ``gh issue create``
    fallback so the issue is still filed with the available text contents. The returned
    payload includes ``browser_unavailable: true`` when that fallback is used, so the
    caller can inform the user that the browser could not be opened and images were not
    uploaded. If both the browser and ``gh`` are unavailable, filing fails.
    """
    full_url = build_new_issue_url(repo, title, body)
    body_fits_in_url = len(full_url) <= MAX_PREFILL_URL_LENGTH
    url_to_open = full_url if body_fits_in_url else build_new_issue_url(repo, title, None)

    browser_available, browser_unavailable_reason = browser_is_available()

    base_payload: dict[str, object] = {
        "method": "browser",
        "repo": repo,
        "url": url_to_open,
    }
    if not body_fits_in_url:
        # Surface the body so the caller can instruct the user to paste it in.
        base_payload["body"] = body

    browser_failure_reason: str | None = None
    if not browser_available:
        browser_failure_reason = browser_unavailable_reason or "No display available."
    elif not open_in_browser(url_to_open):
        browser_failure_reason = "Unable to open a web browser for the prefilled new-issue page."

    if browser_failure_reason is not None:
        # Browser unavailable — try gh CLI as a fallback so the issue is still filed.
        gh_path = gh_path_if_authenticated()
        if gh_path is not None:
            issue_url, _gh_error = create_issue_with_gh(gh_path, repo, title, body)
            if issue_url is not None:
                print_result({
                    "status": "created",
                    "method": "gh",
                    "repo": repo,
                    "issue_url": issue_url,
                    "browser_unavailable": True,
                    "message": (
                        f"{browser_failure_reason} "
                        "The issue was filed programmatically with the available text contents. "
                        "Image attachments were not uploaded."
                    ),
                })
                return 0

        base_payload["status"] = "failed"
        base_payload["error"] = (
            browser_failure_reason
            + " Image attachments could not be handed off through the browser flow; "
            "no issue has been filed."
        )
        print_result(base_payload)
        return 1

    # The browser path is only used by the skill when image attachments are
    # present, so the user-facing message always references pasting/dropping them.
    if body_fits_in_url:
        message = (
            "Opened the GitHub new-issue page in your browser with the title and body prefilled. "
            "Paste or drag your attached screenshot(s) into the body at the placeholder line(s), "
            "then submit the issue."
        )
    else:
        message = (
            "Opened the GitHub new-issue page in your browser with the title prefilled. "
            "The drafted body was too long to include in the URL; paste the body (returned in "
            "this result's `body` field) into the issue form first, then paste or drag your "
            "attached screenshot(s) into the placeholder line(s), then submit."
        )

    base_payload["status"] = "browser_opened"
    base_payload["message"] = message
    print_result(base_payload)
    return 0


def print_result(payload: dict[str, object]) -> None:
    json.dump(payload, sys.stdout)
    sys.stdout.write("\n")


def file_with_gh(repo: str, title: str, body: str) -> int:
    """File the issue via the gh CLI. Returns status `unavailable` when gh isn't
    installed or not authenticated; the caller is responsible for choosing a
    different --use method in that case (no automatic fallback happens here).
    """
    gh_path = gh_path_if_authenticated()
    if gh_path is None:
        print_result(
            {
                "status": "unavailable",
                "method": "gh",
                "repo": repo,
                "message": f"GitHub CLI is not installed or not authenticated for {DEFAULT_HOSTNAME}.",
            }
        )
        return 0
    issue_url, gh_error = create_issue_with_gh(gh_path, repo, title, body)
    if issue_url is not None:
        print_result(
            {
                "status": "created",
                "method": "gh",
                "repo": repo,
                "issue_url": issue_url,
            }
        )
        return 0

    print_result(
        {
            "status": "failed",
            "method": "gh",
            "repo": repo,
            "gh_error": gh_error,
        }
    )
    return 1


def main() -> int:
    args = parse_args()

    title = normalize_title(args.title)
    repo = normalize_repo(args.repo)
    body = normalize_body(read_text(args.body_file, "body"))

    if args.use_method == "browser":
        return fallback_to_browser(repo, title, body)
    # argparse enforces choices=["gh", "browser"], so the remaining case is "gh".
    return file_with_gh(repo, title, body)


if __name__ == "__main__":
    raise SystemExit(main())
