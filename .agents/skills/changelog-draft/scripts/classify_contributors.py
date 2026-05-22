#!/usr/bin/env python3
"""Classify GitHub usernames as internal, external, or bot.

Uses `gh api` to check org membership — stdlib only, no pip deps.

Usage:
    python3 classify_contributors.py --org warpdotdev --authors user1,user2,user3

Outputs JSON to stdout.
"""

import argparse
import json
import subprocess
import sys

KNOWN_BOTS = frozenset(
    {
        "dependabot",
        "dependabot[bot]",
        "renovate",
        "renovate[bot]",
        "github-actions",
        "github-actions[bot]",
        "codecov",
        "codecov[bot]",
        "warp-bot",
        "warp-bot[bot]",
    }
)


def run(cmd: list[str], *, check: bool = True) -> subprocess.CompletedProcess:
    return subprocess.run(cmd, capture_output=True, text=True, check=check)


def check_org_membership(org: str, username: str) -> str:
    """Check if a user is a member of the given GitHub org via gh api.

    Returns:
        'internal' if the user is an org member (HTTP 204),
        'external' if the user is confirmed not a member (HTTP 404),
        'unknown' if the check failed due to auth/permission issues.
    """
    result = run(
        ["gh", "api", f"orgs/{org}/members/{username}", "--silent"],
        check=False,
    )
    if result.returncode == 0:
        return "internal"
    # Distinguish auth failures from genuine "not a member" responses.
    # gh api exits non-zero for both 404 (not a member) and 403/401 (no
    # read:org scope).  Only treat an explicit 404 as "external";
    # everything else (network errors, rate limits, auth issues) is "unknown"
    # to avoid publicly crediting internal or unverified users.
    stderr = result.stderr.lower()
    if "404" in stderr:
        return "external"
    return "unknown"


def main() -> None:
    parser = argparse.ArgumentParser(description="Classify contributor types")
    parser.add_argument("--org", required=True, help="GitHub org to check membership")
    parser.add_argument(
        "--authors",
        required=True,
        help="Comma-separated list of GitHub usernames",
    )
    args = parser.parse_args()

    authors = [a.strip() for a in args.authors.split(",") if a.strip()]

    internal: list[str] = []
    external: list[str] = []
    bot: list[str] = []
    unknown: list[str] = []

    for author in authors:
        if author.lower() in KNOWN_BOTS or author.endswith("[bot]"):
            bot.append(author)
        else:
            status = check_org_membership(args.org, author)
            if status == "internal":
                internal.append(author)
            elif status == "unknown":
                unknown.append(author)
            else:
                external.append(author)

    output = {"internal": internal, "external": external, "bot": bot, "unknown": unknown}
    json.dump(output, sys.stdout, indent=2)
    print()


if __name__ == "__main__":
    main()
