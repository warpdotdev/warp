#!/usr/bin/env python3
"""Behavioral and packaging checks for the bundled factory-files skill.

The skill no longer carries a copy of the Factory file format. warp-server owns
the format and decides whether a tree is valid, so there is nothing here that
asserts which documents the format accepts - those cases live beside the parser
in warp-server (logic/factoryfile).

What is left to check is everything the client is still responsible for:

- which files it selects and uploads, and which it refuses to read
- that it reports the server's verdict as the server worded it
- that every way of not reaching a verdict is reported as "not validated"
  rather than as a pass

That last one is the point of the whole design. A stale local copy of the
format used to answer confidently and wrongly; the replacement must never
imply a tree was checked when it was not.

Run directly, or via script/presubmit.
"""

from __future__ import annotations

import contextlib
import http.server
import json
import os
import shutil
import subprocess
import sys
import tempfile
import threading
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
SKILL = REPO_ROOT / "resources" / "bundled" / "skills" / "factory-files"
VALIDATOR = SKILL / "scripts" / "validate_factory_files.py"

EXIT_VALID = 0
EXIT_DIAGNOSTICS = 1
EXIT_NOT_VALIDATED = 2

DISCLOSURE = "Validated with the warp-server parser"
NOT_VALIDATED = "was NOT validated"

FACTORY = """schemaVersion: v1alpha1
name: demo
repositories:
  - owner: warpdotdev
    name: warp
agentDefaults:
  model: auto
"""

MAIN_AGENT = "---\nagentType: MAIN\n---\nDo the thing.\n"

CLEAN_RESPONSE = {"schema_version": "v1alpha1", "valid": True, "diagnostics": []}


class _FakeServer:
    """A warp-server stand-in.

    Records what the validator submitted so a test can assert the tree it sent,
    not just the verdict it printed.
    """

    def __init__(self, httpd):
        self._httpd = httpd
        self.url = f"http://127.0.0.1:{httpd.server_address[1]}"

    def submitted_files(self) -> list[dict]:
        return self._httpd.submitted_files

    def authorization(self) -> str:
        return self._httpd.authorization


@contextlib.contextmanager
def fake_server(validate_body, validate_status: int = 200, raw_body: bytes | None = None):
    class Handler(http.server.BaseHTTPRequestHandler):
        def log_message(self, *_args):  # keep the corpus output readable
            pass

        def do_POST(self):  # noqa: N802 - BaseHTTPRequestHandler's naming
            length = int(self.headers.get("Content-Length", "0"))
            payload = json.loads(self.rfile.read(length) or b"{}")
            self.server.submitted_files.extend(payload.get("files", []))
            self.server.authorization = self.headers.get("Authorization", "")
            if raw_body is not None:
                encoded = raw_body
            else:
                encoded = json.dumps(validate_body).encode("utf-8")
            self.send_response(validate_status)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(encoded)))
            self.end_headers()
            self.wfile.write(encoded)

    httpd = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    httpd.submitted_files = []
    httpd.authorization = ""
    thread = threading.Thread(target=httpd.serve_forever, daemon=True)
    thread.start()
    try:
        yield _FakeServer(httpd)
    finally:
        httpd.shutdown()
        httpd.server_close()
        thread.join(timeout=5)


def run_validator(root: Path, server_url: str, api_key: str = "", extra: list[str] | None = None):
    environment = os.environ.copy()
    environment.pop("WARP_API_KEY", None)
    environment.pop("WARP_SERVER_ROOT", None)
    if api_key:
        environment["WARP_API_KEY"] = api_key
    return subprocess.run(
        [sys.executable, str(VALIDATOR), str(root), "--server-root", server_url]
        + (extra or []),
        capture_output=True,
        text=True,
        check=False,
        env=environment,
    )


@contextlib.contextmanager
def factory_tree(**files: str):
    """A minimal Factory root, plus any extra files, cleaned up afterwards."""
    root = Path(tempfile.mkdtemp(prefix="factory-files-"))
    try:
        contents = {"factory.yaml": FACTORY, "agents/main/agent.md": MAIN_AGENT}
        contents.update(files)
        for relative, content in contents.items():
            path = root / relative
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(content, encoding="utf-8")
        yield root
    finally:
        shutil.rmtree(root, ignore_errors=True)


def assert_submits_only_resource_files() -> None:
    """Only canonical resource paths are uploaded.

    Skills can hold anything a repository wants to give an agent, and unrelated
    files are none of the server's business, so neither is sent.
    """
    with factory_tree(
        **{
            "automations/nightly/automation.md": "---\nagent: main\n---\nrun\n",
            "runners/linux.yaml": "platform:\n  linux:\n    dockerImage: ubuntu:24.04\n",
            "scorers/tests/scorer.md": "---\nagents: [main]\n---\nRubric.\n",
            "skills/helper/SKILL.md": "---\nname: helper\n---\nhelp\n",
            "agents/main/skills/inner/SKILL.md": "---\nname: inner\n---\nhelp\n",
            "README.md": "unrelated",
            "agents/main/notes.txt": "unrelated",
        }
    ) as root:
        with fake_server(CLEAN_RESPONSE) as server:
            result = run_validator(root, server.url)
        if result.returncode != EXIT_VALID:
            raise RuntimeError(f"expected a clean pass, got: {result.stdout}{result.stderr}")
        submitted = {entry["path"] for entry in server.submitted_files()}
        expected = {
            "factory.yaml",
            "agents/main/agent.md",
            "automations/nightly/automation.md",
            "runners/linux.yaml",
            "scorers/tests/scorer.md",
        }
        if submitted != expected:
            raise RuntimeError(
                f"unexpected submitted tree:\n  missing={sorted(expected - submitted)}"
                f"\n  extra={sorted(submitted - expected)}"
            )


def assert_reports_the_servers_verdict() -> None:
    """Diagnostics are relayed as the server worded them, never reinterpreted."""
    with factory_tree() as root:
        response = {
            "schema_version": "v1alpha1",
            "valid": False,
            "diagnostics": [
                {
                    "path": "factory.yaml",
                    "line": 2,
                    "column": 1,
                    "code": "FF_UNKNOWN_FIELD",
                    "message": 'unknown field "bogus"',
                }
            ],
        }
        with fake_server(response) as server:
            result = run_validator(root, server.url)
        if result.returncode != EXIT_DIAGNOSTICS:
            raise RuntimeError(f"expected exit {EXIT_DIAGNOSTICS}, got {result.returncode}")
        for fragment in ("FF_UNKNOWN_FIELD", 'unknown field "bogus"', "factory.yaml:2"):
            if fragment not in result.stderr:
                raise RuntimeError(f"the server diagnostic was not relayed: {result.stderr}")

        # An unrecognized schemaVersion is just a server diagnostic now. The
        # client has no opinion about versions because it no longer reads YAML.
        unsupported = {
            "schema_version": "v9alpha1",
            "valid": False,
            "diagnostics": [
                {
                    "path": "factory.yaml",
                    "line": 1,
                    "column": 1,
                    "code": "FF_UNSUPPORTED_VERSION",
                    "message": "unsupported schemaVersion v9alpha1",
                }
            ],
        }
        with fake_server(unsupported) as server:
            result = run_validator(root, server.url)
        if result.returncode != EXIT_DIAGNOSTICS or "FF_UNSUPPORTED_VERSION" not in result.stderr:
            raise RuntimeError(f"an unsupported version was not relayed: {result.stderr}")


def assert_surfaces_deferred_resolutions() -> None:
    """A deferred provider alias is reported, not silently dropped.

    These are the values the endpoint deliberately did not prove. Hiding them
    would let a clean result read as a guarantee the tree will apply.
    """
    with factory_tree() as root:
        response = {
            "schema_version": "v1alpha1",
            "valid": True,
            "diagnostics": [],
            "deferred_resolutions": [
                {
                    "path": "automations/t/automation.md",
                    "field": "triggers[0].filter.teams",
                    "kind": "linear_name_alias",
                }
            ],
        }
        with fake_server(response) as server:
            result = run_validator(root, server.url)
        if result.returncode != EXIT_VALID:
            raise RuntimeError("a deferred alias should not fail the tree")
        for fragment in ("deferred", "linear_name_alias", "triggers[0].filter.teams"):
            if fragment not in result.stdout:
                raise RuntimeError(f"deferred resolutions were not reported: {result.stdout}")


def assert_unreached_verdicts_are_never_a_pass() -> None:
    """Every way of failing to reach the server exits 2 and says so.

    This is the check that matters most. The previous design answered from a
    bundled copy of the format when the server was unavailable, and a stale
    copy produced confident, wrong diagnostics. Silence is the safe failure,
    but only if it is loud about being silence.
    """
    cases = {
        "http 401": dict(validate_status=401),
        "http 429": dict(validate_status=429),
        "http 500": dict(validate_status=500),
        "response is not JSON": dict(raw_body=b"<html>nope</html>"),
        "response is missing diagnostics": dict(validate_body={"schema_version": "v1alpha1"}),
        "diagnostics are not objects": dict(
            validate_body={"schema_version": "v1alpha1", "diagnostics": ["nope"]}
        ),
    }
    with factory_tree() as root:
        for name, options in cases.items():
            body = options.pop("validate_body", CLEAN_RESPONSE)
            with fake_server(body, **options) as server:
                result = run_validator(root, server.url)
            if result.returncode != EXIT_NOT_VALIDATED:
                raise RuntimeError(
                    f"{name}: expected exit {EXIT_NOT_VALIDATED}, got {result.returncode}"
                )
            combined = result.stdout + result.stderr
            if NOT_VALIDATED not in combined:
                raise RuntimeError(f"{name}: did not report that nothing was validated")
            if DISCLOSURE in combined:
                raise RuntimeError(f"{name}: claimed a server verdict it never received")

        # An unreachable server is the same case, and must not hang.
        result = run_validator(root, "http://127.0.0.1:9")
        if result.returncode != EXIT_NOT_VALIDATED or NOT_VALIDATED not in result.stderr:
            raise RuntimeError(f"an unreachable server was not reported: {result.stderr}")

        # A directory that is not a Factory root is also not a verdict.
        empty = Path(tempfile.mkdtemp(prefix="factory-files-empty-"))
        try:
            result = run_validator(empty, "http://127.0.0.1:9")
            if result.returncode != EXIT_NOT_VALIDATED or "factory.yaml" not in result.stderr:
                raise RuntimeError(f"a non-Factory directory was not reported: {result.stderr}")
        finally:
            shutil.rmtree(empty, ignore_errors=True)


def assert_credentials_are_optional_but_forwarded() -> None:
    """The endpoint needs no key; one is forwarded when the environment has it.

    Requiring a key would disable validation for local authoring agents, whose
    shell cannot see the Warp client's session.
    """
    with factory_tree() as root:
        with fake_server(CLEAN_RESPONSE) as server:
            result = run_validator(root, server.url)
            if result.returncode != EXIT_VALID:
                raise RuntimeError("validation should not require a credential")
            if server.authorization():
                raise RuntimeError("an Authorization header was sent without a key")

        with fake_server(CLEAN_RESPONSE) as server:
            result = run_validator(root, server.url, api_key="wk-1.abc")
            if result.returncode != EXIT_VALID:
                raise RuntimeError("validation failed when a credential was present")
            if server.authorization() != "Bearer wk-1.abc":
                raise RuntimeError(f"the key was not forwarded: {server.authorization()!r}")


def assert_symlinked_resources_are_refused() -> None:
    """A symlinked resource is reported and never uploaded.

    git stores a symlink as a blob holding the target path, so the server sees
    the link rather than its target and cannot accept one either. Following it
    here would both diverge from that and upload a file the Factory does not
    contain.
    """
    canary = "CANARY-SHOULD-NEVER-BE-UPLOADED"
    outside = Path(tempfile.mkdtemp(prefix="factory-files-outside-"))
    try:
        secret = outside / "secret.txt"
        secret.write_text(f"{canary}\n", encoding="utf-8")

        # A link out of the tree.
        with factory_tree() as root:
            (root / "runners").mkdir(exist_ok=True)
            (root / "runners" / "escapes.yaml").symlink_to(secret)
            with fake_server(CLEAN_RESPONSE) as server:
                result = run_validator(root, server.url)
                uploaded = json.dumps(server.submitted_files())
            combined = result.stdout + result.stderr
            if result.returncode != EXIT_DIAGNOSTICS or "symlink" not in combined:
                raise RuntimeError(f"an escaping symlink was not refused: {combined}")
            if canary in combined or canary in uploaded:
                raise RuntimeError("the symlink target's content escaped")

        # A link whose target is inside the tree is refused too: the server
        # still sees the link, not the file it points at.
        with factory_tree() as root:
            (root / "runners").mkdir(exist_ok=True)
            (root / "runners" / "real.yaml").write_text(
                "platform:\n  linux:\n    dockerImage: ubuntu:24.04\n", encoding="utf-8"
            )
            (root / "runners" / "alias.yaml").symlink_to(root / "runners" / "real.yaml")
            with fake_server(CLEAN_RESPONSE) as server:
                result = run_validator(root, server.url)
                submitted = {entry["path"] for entry in server.submitted_files()}
            if result.returncode != EXIT_DIAGNOSTICS:
                raise RuntimeError("an in-tree symlink was accepted")
            if "runners/alias.yaml" in submitted:
                raise RuntimeError("an in-tree symlink was uploaded")
            if "runners/real.yaml" not in submitted:
                raise RuntimeError("the real file beside the symlink was not uploaded")
    finally:
        shutil.rmtree(outside, ignore_errors=True)


def assert_oversized_trees_are_reported() -> None:
    """A tree past the endpoint's caps is reported here, not sent and rejected."""
    with factory_tree(**{"runners/big.yaml": "#" * (256 * 1024 + 1)}) as root:
        with fake_server(CLEAN_RESPONSE) as server:
            result = run_validator(root, server.url)
            if server.submitted_files():
                raise RuntimeError("an oversized tree was uploaded anyway")
        if result.returncode != EXIT_NOT_VALIDATED or "larger than" not in result.stderr:
            raise RuntimeError(f"an oversized file was not reported: {result.stderr}")


def assert_json_output_never_implies_a_verdict() -> None:
    """--json distinguishes "checked and clean" from "not checked"."""
    with factory_tree() as root:
        with fake_server(CLEAN_RESPONSE) as server:
            result = run_validator(root, server.url, extra=["--json"])
        payload = json.loads(result.stdout)
        if payload != {
            "validated": True,
            "valid": True,
            "schema_version": "v1alpha1",
            "disclosure": payload["disclosure"],
            "problems": [],
            "deferred_resolutions": [],
        }:
            raise RuntimeError(f"unexpected clean payload: {payload}")

        result = run_validator(root, "http://127.0.0.1:9", extra=["--json"])
        payload = json.loads(result.stdout)
        if payload.get("validated") is not False or "valid" in payload:
            raise RuntimeError(
                f"an unvalidated tree must not report a validity verdict: {payload}"
            )


def assert_no_format_copy_remains() -> None:
    """The skill must not regrow a local copy of the Factory file format.

    A bundled copy ships inside a Warp release and goes stale against the
    server, which is what produced confidently wrong diagnostics before. If a
    future change needs the format, fetch it from the server.
    """
    schemas = list(SKILL.rglob("*.schema.json"))
    if schemas:
        raise RuntimeError(
            "the skill has regrown bundled schemas, which go stale against the "
            "server and produce false rejections; fetch the format instead:\n  "
            + "\n  ".join(str(path.relative_to(SKILL)) for path in schemas)
        )
    source = VALIDATOR.read_text(encoding="utf-8")
    for banned in ("import yaml", "def load_yaml", "jsonschema"):
        if banned in source:
            raise RuntimeError(
                f"the validator parses the format again ({banned!r}); it should send "
                "bytes to the server and relay the verdict"
            )


def assert_packaged_skill_matches() -> None:
    with tempfile.TemporaryDirectory(prefix="factory-files-bundle-") as destination:
        environment = os.environ.copy()
        environment.update({"SKIP_SETTINGS_SCHEMA": "1", "NO_LICENSES": "1"})
        subprocess.run(
            [str(REPO_ROOT / "script" / "prepare_bundled_resources"), destination, "stable"],
            cwd=REPO_ROOT,
            env=environment,
            capture_output=True,
            text=True,
            check=True,
        )
        packaged = Path(destination) / "bundled" / "skills" / "factory-files"
        source_files = {path.relative_to(SKILL) for path in SKILL.rglob("*") if path.is_file()}
        packaged_files = {
            path.relative_to(packaged) for path in packaged.rglob("*") if path.is_file()
        }
        if source_files != packaged_files:
            raise RuntimeError(
                f"packaged file set differs: missing={source_files - packaged_files}, "
                f"extra={packaged_files - source_files}"
            )
        for relative in source_files:
            if (SKILL / relative).read_bytes() != (packaged / relative).read_bytes():
                raise RuntimeError(f"packaged content differs for {relative}")


CHECKS = (
    ("only resource files are submitted", assert_submits_only_resource_files),
    ("the server's verdict is relayed verbatim", assert_reports_the_servers_verdict),
    ("deferred resolutions are surfaced", assert_surfaces_deferred_resolutions),
    ("an unreached verdict is never a pass", assert_unreached_verdicts_are_never_a_pass),
    ("credentials are optional but forwarded", assert_credentials_are_optional_but_forwarded),
    ("symlinked resources are refused", assert_symlinked_resources_are_refused),
    ("oversized trees are reported", assert_oversized_trees_are_reported),
    ("json output never implies a verdict", assert_json_output_never_implies_a_verdict),
    ("no local copy of the format remains", assert_no_format_copy_remains),
    ("source and bundled trees match", assert_packaged_skill_matches),
)


def main() -> int:
    failures = 0
    for description, check in CHECKS:
        try:
            check()
        except Exception as error:  # noqa: BLE001 - the report is the point
            failures += 1
            print(f"FAIL factory-files: {description}\n  {error}", file=sys.stderr)
        else:
            print(f"factory-files: {description}")
    if failures:
        print(f"{failures}/{len(CHECKS)} factory-files checks failed", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
