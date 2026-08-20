#!/usr/bin/env python3
"""Validate a Factory file tree against warp-server.

Usage:
    python3 validate_factory_files.py [FACTORY_ROOT] [--json] [--server-root URL]

FACTORY_ROOT defaults to the current directory and must contain factory.yaml.

warp-server owns the Factory file format, so it is the only thing that decides
whether a tree is valid. This script collects the tree's resource files and
submits them to the validation endpoint, which runs the real parser plus the
state-independent checks the apply path would run next.

There is deliberately no local fallback. A bundled copy of the format is
routinely older than the server it is used against, and a stale copy does not
degrade gracefully: it reports confident, wrong diagnostics that invite an
agent to "repair" valid configuration by deleting it. Reporting that a tree
could not be checked is strictly safer than reporting the wrong answer, so when
the server cannot be reached this script says so and validates nothing.

That also means this script never parses YAML. It decides which files are
resource files from their paths alone and sends their bytes verbatim, so it
cannot disagree with the parser about what a document means.

The endpoint does not resolve server state. Model IDs, environment IDs, secret
names, runner references, MCP server IDs, integration availability, and the
values of provider name aliases are all checked when the plan is applied.

Exit codes:
    0  the server validated the tree and found no problem
    1  the server validated the tree and reported diagnostics
    2  the tree was not validated; the reason is printed
"""

from __future__ import annotations

import argparse
import json
import os
import sys
import urllib.error
import urllib.request
from pathlib import Path
from typing import Any, Optional

DEFAULT_SERVER_ROOT = "https://app.warp.dev"
VALIDATE_PATH = "/api/v1/factory-files/validate"

# Bounded so an unreachable or slow server reports quickly rather than stalling
# an authoring session.
REQUEST_TIMEOUT_SECONDS = 10.0
MAX_RESPONSE_BYTES = 8 * 1024 * 1024

# Mirrors the caps the endpoint enforces, so an oversized tree is reported here
# rather than collecting a 400.
MAX_REMOTE_FILES = 256
MAX_REMOTE_FILE_BYTES = 256 * 1024
MAX_REMOTE_CONTENT_BYTES = 2 * 1024 * 1024

SYMLINK_REFUSED = (
    "resource file is a symlink, or resolves outside the Factory root, and was not "
    "read. The server parses the repository tree, so it sees the link itself rather "
    "than its target and cannot accept this either. Replace it with a real file."
)

EXIT_VALID = 0
EXIT_DIAGNOSTICS = 1
EXIT_NOT_VALIDATED = 2


class NotValidated(Exception):
    """The tree was not checked. This is never a pass."""


class Problem:
    """One reported failure, located as precisely as the server allows."""

    def __init__(self, path: str, message: str, line: Optional[int] = None, pointer: str = ""):
        self.path = path
        self.message = message
        self.line = line
        self.pointer = pointer

    def as_dict(self) -> dict[str, Any]:
        return {
            "path": self.path,
            "line": self.line,
            "pointer": self.pointer,
            "message": self.message,
        }

    def render(self) -> str:
        location = self.path
        if self.line is not None:
            location += f":{self.line}"
        if self.pointer:
            return f"{location}: {self.pointer}: {self.message}"
        return f"{location}: {self.message}"


# ---------------------------------------------------------------------------
# Selecting the tree to submit
# ---------------------------------------------------------------------------


def classify(relative: str) -> tuple[str, str]:
    """Mirror the server's path classification. Returns (kind, name).

    This decides only which files are worth submitting. The server classifies
    them again and owns the verdict, so a disagreement here costs a wasted
    upload rather than a wrong answer.
    """
    if relative == "factory.yaml":
        return "factory", ""
    segments = relative.split("/")
    if len(segments) >= 2 and segments[0] == "skills":
        return "skill", ""
    if len(segments) >= 4 and segments[0] == "agents" and segments[2] == "skills":
        return "skill", ""
    if len(segments) == 3 and segments[0] == "agents" and segments[2] == "agent.md":
        return ("agent", segments[1]) if _valid_name(segments[1]) else ("invalid", "")
    if len(segments) == 3 and segments[0] == "automations" and segments[2] == "automation.md":
        return ("automation", segments[1]) if _valid_name(segments[1]) else ("invalid", "")
    if len(segments) == 2 and segments[0] == "automations" and segments[1].endswith(".md"):
        name = segments[1][: -len(".md")]
        return ("automation", name) if _valid_name(name) else ("invalid", "")
    if len(segments) == 2 and segments[0] == "runners" and segments[1].endswith(".yaml"):
        name = segments[1][: -len(".yaml")]
        return ("runner", name) if _valid_name(name) else ("invalid", "")
    if len(segments) == 3 and segments[0] == "scorers" and segments[2] == "scorer.md":
        return ("scorer", segments[1]) if _valid_name(segments[1]) else ("invalid", "")
    if len(segments) == 2 and segments[0] == "scorers" and segments[1].endswith(".md"):
        return "invalid", ""
    base = segments[-1]
    if segments[0] == "agents" and base == "agent.md":
        return "invalid", ""
    if segments[0] == "automations" and base == "automation.md":
        return "invalid", ""
    if segments[0] == "runners" and base.endswith(".yaml"):
        return "invalid", ""
    if segments[0] == "scorers" and base == "scorer.md":
        return "invalid", ""
    return "unrelated", ""


def _valid_name(name: str) -> bool:
    return name not in ("", ".", "..") and "/" not in name


def _resource_files(root: Path) -> list[Path]:
    files = [root / "factory.yaml"]
    for directory_name in ("agents", "automations", "runners", "scorers"):
        resource_root = root / directory_name
        if not resource_root.is_dir():
            continue
        for directory, child_directories, filenames in os.walk(resource_root):
            relative_directory = Path(directory).relative_to(root)
            parts = relative_directory.parts
            if directory_name == "agents" and len(parts) == 2:
                child_directories[:] = [name for name in child_directories if name != "skills"]
            files.extend(Path(directory) / filename for filename in filenames)
    return sorted(files)


def _leaves_factory_root(path: Path, root: Path) -> bool:
    """Report whether reading path would follow a link out of the tree.

    The server never resolves links: it parses an in-memory git tree, where a
    symlink is a blob whose content is the target path, so it sees the link
    itself. Following one here would both diverge from that and read a file the
    Factory does not contain - an untrusted repository could otherwise point a
    resource at any readable path and have its content uploaded.
    """
    if path.is_symlink():
        return True
    try:
        path.resolve().relative_to(root.resolve())
    except (OSError, ValueError, RuntimeError):
        return True
    return False


def collect_tree(root: Path) -> tuple[list[dict[str, str]], list[Problem]]:
    """Collect the resource files to submit, refusing symlinks as the server does."""
    if not (root / "factory.yaml").is_file():
        raise NotValidated(f"{root} has no factory.yaml, so it is not a Factory root")
    files: list[dict[str, str]] = []
    problems: list[Problem] = []
    total = 0
    for absolute in _resource_files(root):
        relative = absolute.relative_to(root).as_posix()
        kind, _ = classify(relative)
        if kind in ("unrelated", "skill", "invalid"):
            continue
        if _leaves_factory_root(absolute, root):
            problems.append(Problem(relative, SYMLINK_REFUSED))
            continue
        try:
            content = absolute.read_text(encoding="utf-8")
        except (OSError, UnicodeError) as error:
            problems.append(Problem(relative, f"could not read UTF-8 resource: {error}"))
            continue
        encoded = len(content.encode("utf-8"))
        if encoded > MAX_REMOTE_FILE_BYTES:
            raise NotValidated(f"{relative} is larger than the endpoint accepts")
        total += encoded
        if total > MAX_REMOTE_CONTENT_BYTES or len(files) >= MAX_REMOTE_FILES:
            raise NotValidated("the tree is larger than the endpoint accepts")
        files.append({"path": relative, "content": content})
    if not files:
        raise NotValidated("the tree has no resource files to submit")
    return files, problems


# ---------------------------------------------------------------------------
# Server-backed validation
# ---------------------------------------------------------------------------


class Outcome:
    """What the server found, and what it deliberately did not check."""

    def __init__(
        self,
        schema_version: str,
        problems: list[Problem],
        deferred: Optional[list[dict[str, Any]]] = None,
    ):
        self.schema_version = schema_version
        self.problems = problems
        self.deferred = deferred or []

    def disclosure(self) -> str:
        """The sentence the agent must repeat. Never claim more than ran."""
        return (
            f"Validated with the warp-server parser for {self.schema_version}; "
            "state-dependent apply checks were not run."
        )


def server_root(argument: Optional[str]) -> str:
    """Resolve the server to ask, so a local or staging root needs no code change.

    WARP_SERVER_ROOT_URL is the name an Oz sandbox exports; without it here, a
    sandbox that never sets the older WARP_SERVER_ROOT silently validates
    against production instead of the server it actually belongs to.
    """
    chosen = (
        argument
        or os.environ.get("WARP_SERVER_ROOT")
        or os.environ.get("WARP_SERVER_ROOT_URL")
        or DEFAULT_SERVER_ROOT
    )
    return chosen.rstrip("/")


def _fetch(url: str, data: Optional[bytes], token: Optional[str]) -> bytes:
    """Send one request and return its raw body, letting errors propagate."""
    headers = {"Accept": "application/json"}
    if data is not None:
        headers["Content-Type"] = "application/json"
    if token:
        headers["Authorization"] = "Bearer " + token
    request = urllib.request.Request(url, data=data, headers=headers)
    with urllib.request.urlopen(request, timeout=REQUEST_TIMEOUT_SECONDS) as response:
        return response.read(MAX_RESPONSE_BYTES + 1)


def _request_json(url: str, token: Optional[str] = None, payload: Optional[Any] = None) -> Any:
    """Post or fetch JSON, turning every failure class into NotValidated.

    A 401 or 403 drawn by a forwarded token is retried once without it: the
    endpoint needs no credential, so a stale or mismatched WARP_API_KEY must
    not be able to block validation. The drop is reported, because a key the
    server rejects is worth knowing about even when validation then succeeds.
    """
    data = json.dumps(payload).encode("utf-8") if payload is not None else None
    try:
        body = _fetch(url, data, token)
    except urllib.error.HTTPError as error:
        if token and error.code in (401, 403):
            print(
                f"WARP_API_KEY was rejected with HTTP {error.code}; retrying without it, "
                "since this endpoint needs no credential.",
                file=sys.stderr,
            )
            try:
                body = _fetch(url, data, token=None)
            except urllib.error.HTTPError as retry_error:
                raise NotValidated(f"the server answered HTTP {retry_error.code}") from retry_error
            except Exception as retry_error:  # DNS, TLS, connection, timeout, proxy, ...
                raise NotValidated(
                    f"the server could not be reached: {retry_error}"
                ) from retry_error
        else:
            raise NotValidated(f"the server answered HTTP {error.code}") from error
    except Exception as error:  # DNS, TLS, connection, timeout, proxy, ...
        raise NotValidated(f"the server could not be reached: {error}") from error
    if len(body) > MAX_RESPONSE_BYTES:
        raise NotValidated("the server response was implausibly large")
    try:
        return json.loads(body.decode("utf-8"))
    except (UnicodeError, ValueError) as error:
        raise NotValidated(f"the server response was not JSON: {error}") from error


def validate(root: Path, base_url: str) -> Outcome:
    """Submit the tree to the server, or raise NotValidated.

    The endpoint reads the declared schemaVersion itself and reports an
    unrecognized one as a diagnostic, so there is nothing to pre-flight and no
    reason for this script to read the tree's YAML.

    The endpoint needs no credential. WARP_API_KEY is forwarded when the
    environment already carries one, as an Oz sandbox does, so the request is
    attributable there; nothing requires it, because a local authoring agent
    runs in a shell that cannot see the Warp client's session. A stale or
    mismatched key is retried without itself, in _request_json, rather than
    blocking validation.
    """
    token = os.environ.get("WARP_API_KEY")
    files, problems = collect_tree(root)
    response = _request_json(base_url + VALIDATE_PATH, token=token, payload={"files": files})
    if not isinstance(response, dict) or not isinstance(response.get("diagnostics"), list):
        raise NotValidated("the validation response was malformed")

    for diagnostic in response["diagnostics"]:
        if not isinstance(diagnostic, dict):
            raise NotValidated("the validation response was malformed")
        code = str(diagnostic.get("code", ""))
        message = str(diagnostic.get("message", ""))
        line = diagnostic.get("line")
        problems.append(
            Problem(
                str(diagnostic.get("path", "")),
                f"{code}: {message}" if code else message,
                line=line if isinstance(line, int) else None,
            )
        )
    deferred = [
        entry for entry in response.get("deferred_resolutions", []) if isinstance(entry, dict)
    ]
    reported = response.get("schema_version")
    return Outcome(
        reported if isinstance(reported, str) and reported else "unknown",
        problems,
        deferred,
    )


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("root", nargs="?", default=".", help="Factory root containing factory.yaml")
    parser.add_argument("--json", action="store_true", help="emit machine-readable output")
    parser.add_argument(
        "--server-root",
        default=None,
        help="warp-server root to validate against; defaults to $WARP_SERVER_ROOT, then "
        "$WARP_SERVER_ROOT_URL, then " + DEFAULT_SERVER_ROOT,
    )
    args = parser.parse_args()

    root = Path(args.root).resolve()
    try:
        outcome = validate(root, server_root(args.server_root))
    except NotValidated as reason:
        report = (
            f"This tree was NOT validated: {reason}. Nothing here says the files are "
            "correct or incorrect. Validate against a reachable warp-server, and say "
            "plainly that validation did not run."
        )
        if args.json:
            print(json.dumps({"validated": False, "reason": str(reason)}, indent=2))
        else:
            print(report, file=sys.stderr)
        return EXIT_NOT_VALIDATED

    problems = outcome.problems
    if args.json:
        print(
            json.dumps(
                {
                    "validated": True,
                    "valid": not problems,
                    "schema_version": outcome.schema_version,
                    "disclosure": outcome.disclosure(),
                    "problems": [problem.as_dict() for problem in problems],
                    "deferred_resolutions": outcome.deferred,
                },
                indent=2,
            )
        )
        return EXIT_DIAGNOSTICS if problems else EXIT_VALID

    if problems:
        print(f"{len(problems)} problem(s) in {root}:", file=sys.stderr)
        for problem in problems:
            print(f"  {problem.render()}", file=sys.stderr)
        print(outcome.disclosure(), file=sys.stderr)
        return EXIT_DIAGNOSTICS

    print(f"{root}: factory files are valid.")
    print(outcome.disclosure())
    for entry in outcome.deferred:
        print(
            f"  deferred: {entry.get('path', '')} {entry.get('field', '')} "
            f"({entry.get('kind', '')}) is resolved when the plan is applied"
        )
    return EXIT_VALID


if __name__ == "__main__":
    sys.exit(main())
