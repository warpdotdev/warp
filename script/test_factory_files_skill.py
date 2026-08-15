#!/usr/bin/env python3
"""Regression corpus for the bundled factory-files validator.

Each case builds a throwaway Factory tree and asserts whether
resources/bundled/skills/factory-files/scripts/validate_factory_files.py
accepts it. The expected verdicts were verified against the authoritative Go
implementation in warp-server (factoryfile.ParseTree and
triggers.ValidateFilter), so a change here should be made only alongside a
matching change to the Factory file format.

Run directly, or via script/presubmit.
"""

from __future__ import annotations

import re
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
SKILL = REPO_ROOT / "resources" / "bundled" / "skills" / "factory-files"
VALIDATOR = SKILL / "scripts" / "validate_factory_files.py"
EXAMPLES = SKILL / "references" / "examples.md"

FACTORY = """schemaVersion: v1alpha1
name: demo
repositories:
  - owner: warpdotdev
    name: warp
agentDefaults:
  model: auto
"""

MAIN_AGENT = "---\nagentType: MAIN\n---\nDo the thing.\n"


def tree(**files: str) -> dict[str, str]:
    """A minimal valid tree, overridden by the given files."""
    base = {"factory.yaml": FACTORY, "agents/main/agent.md": MAIN_AGENT}
    base.update(files)
    return base


VALID_CASES: list[tuple[str, dict[str, str]]] = [
    ("minimal", tree()),
    (
        "empty-agent-frontmatter",
        tree(
            **{
                "agents/main/agent.md": "---\nagentType: MAIN\n---\n",
                "agents/aux/agent.md": "---\n---\nhelp out\n",
            }
        ),
    ),
    (
        "harness-claude-full",
        tree(
            **{
                "agents/main/agent.md": "---\nagentType: MAIN\nharness:\n  type: claude\n"
                "  model: opus\n  reasoningLevel: high\n  auth:\n    source: managedSecret\n"
                "    secretName: ANTHROPIC_KEY\n---\nx\n"
            }
        ),
    ),
    (
        "harness-claude-code-alias",
        tree(
            **{
                "agents/main/agent.md": "---\nagentType: MAIN\nharness:\n  type: claude-code\n"
                "  model: opus\n---\nx\n"
            }
        ),
    ),
    (
        "harness-auth-clear",
        tree(
            **{
                "agents/main/agent.md": "---\nagentType: MAIN\nharness:\n  type: codex\n"
                "  model: gpt-5.5\n  auth: null\n---\nx\n"
            }
        ),
    ),
    (
        "harness-oz-model-only",
        tree(
            **{
                "agents/main/agent.md": "---\nagentType: MAIN\nharness:\n  type: oz\n"
                "  model: auto\n---\nx\n"
            }
        ),
    ),
    (
        "agentdefaults-harness",
        tree(
            **{
                "factory.yaml": FACTORY.replace(
                    "  model: auto\n", "  harness:\n    type: claude\n    model: opus\n"
                )
            }
        ),
    ),
    (
        "inline-schedule",
        tree(
            **{
                "automations/nightly/automation.md": "---\ntriggers:\n  - provider: schedule\n"
                "    event: cron_fired\n    schedule:\n      name: nightly\n"
                "      cron: 0 3 * * *\n---\nrun\n"
            }
        ),
    ),
    (
        "schedule-ids-filter",
        tree(
            **{
                "automations/n/automation.md": "---\ntriggers:\n  - provider: schedule\n"
                "    event: cron_fired\n    filter:\n      schedule_ids: [sched_1]\n---\nrun\n"
            }
        ),
    ),
    (
        "descriptor-cron",
        tree(
            **{
                "automations/n/automation.md": "---\ntriggers:\n  - provider: schedule\n"
                "    event: cron_fired\n    schedule:\n      cron: '@daily'\n---\nrun\n"
            }
        ),
    ),
    (
        "macos-runner",
        tree(
            **{
                "runners/mac.yaml": "platform:\n  os: macos\n  arch: aarch64\n  mac:\n"
                "    version: '15'\ninstanceShape:\n  vcpus: 6\n  memoryGb: 14\n"
            }
        ),
    ),
    (
        "linux-runner",
        tree(
            **{
                "runners/lin.yaml": "description: CI\nsetupCommands:\n  - apt-get update -y\n"
                "instanceShape:\n  vcpus: 4\n  memoryGb: 8\nplatform:\n  os: linux\n"
                "  arch: x86_64\n  linux:\n    dockerImage: ubuntu:22.04\n"
            }
        ),
    ),
    (
        "linux-runner-default-os",
        tree(**{"runners/lin.yaml": "platform:\n  linux:\n    dockerImage: ubuntu:22.04\n"}),
    ),
    (
        "integrations",
        tree(
            **{
                "factory.yaml": FACTORY
                + "integrations:\n  - type: slack\n  - type: linear\n  - type: jira\n"
            }
        ),
    ),
    ("integrations-empty", tree(**{"factory.yaml": FACTORY + "integrations: []\n"})),
    (
        "workerhost-clear",
        tree(**{"agents/main/agent.md": "---\nagentType: MAIN\nworkerHost: null\n---\nx\n"}),
    ),
    (
        "legacy-flat-automation",
        tree(
            **{
                "automations/flat.md": "---\ntriggers:\n  - provider: github\n    event: push\n"
                "    filter:\n      repos: [warpdotdev/warp]\n---\nrun\n"
            }
        ),
    ),
    (
        "not-in-matcher",
        tree(
            **{
                "automations/n/automation.md": "---\ntriggers:\n  - provider: github\n"
                "    event: pull_request_opened\n    filter:\n      labels:\n        in: [ready]\n"
                "        not_in: [wip]\n      pr_numbers: [12, 13]\n---\nrun\n"
            }
        ),
    ),
    ("alias-ok", tree(**{"factory.yaml": FACTORY + "alias: Demo Factory-1.0_x\n"})),
    (
        "skills-are-ignored",
        tree(
            **{
                "agents/main/skills/x/SKILL.md": "---\nname: x\n---\n",
                "skills/y/SKILL.md": "---\nname: y\n---\n",
            }
        ),
    ),
    (
        "block-scalar-description",
        tree(
            **{
                "factory.yaml": "schemaVersion: v1alpha1\nname: demo\ndescription: |\n  line one\n"
                "  line two\nrepositories:\n  - owner: warpdotdev\n    name: warp\n"
                "agentDefaults:\n  model: auto\n"
            }
        ),
    ),
    (
        "comments-and-quotes",
        tree(
            **{
                "factory.yaml": "# leading comment\nschemaVersion: v1alpha1 # trailing\n"
                'name: "demo: with colon"\nrepositories:\n  - owner: warpdotdev\n    name: warp\n'
                "agentDefaults:\n  model: auto\n"
            }
        ),
    ),
    # Prose in a plain scalar is not a quoted string, so an apostrophe must not
    # read as an unterminated quote.
    (
        "apostrophe-in-prose",
        tree(**{"factory.yaml": FACTORY + "description: It's Warp's factory\n"}),
    ),
    # A block scalar's body is opaque text: emphasis, a document marker, and an
    # ampersand are all literal there.
    (
        "block-scalar-prose",
        tree(
            **{
                "factory.yaml": "schemaVersion: v1alpha1\nname: demo\ndescription: |\n"
                "  It's a summary with *emphasis*\n  ---\n  A & B\n"
                "repositories:\n  - owner: warpdotdev\n    name: warp\n"
                "agentDefaults:\n  model: auto\n"
            }
        ),
    ),
    (
        "gitlab-and-factory-providers",
        tree(
            **{
                "automations/n/automation.md": "---\ntriggers:\n  - provider: gitlab\n"
                "    event: merge_request\n    filter:\n      repos: [acme/platform]\n"
                "      actions: [open]\n  - provider: factory\n"
                "    event: work_item_stage_changed\n    filter:\n      stages: [REVIEW]\n---\nrun\n"
            }
        ),
    ),
    (
        "empty-and-null-matchers",
        tree(
            **{
                "automations/n/automation.md": "---\ntriggers:\n  - provider: github\n"
                "    event: pull_request_opened\n    filter:\n      labels: {}\n"
                "      authors: null\n---\nrun\n"
            }
        ),
    ),
]

INVALID_CASES: list[tuple[str, dict[str, str]]] = [
    (
        "model-and-harness",
        tree(
            **{
                "agents/main/agent.md": "---\nagentType: MAIN\nmodel: auto\nharness:\n"
                "  type: oz\n  model: auto\n---\nx\n"
            }
        ),
    ),
    (
        "agentdefaults-neither",
        tree(
            **{
                "factory.yaml": FACTORY.replace(
                    "agentDefaults:\n  model: auto\n", "agentDefaults:\n  runner: r\n"
                )
            }
        ),
    ),
    (
        "oz-reasoning-level",
        tree(
            **{
                "agents/main/agent.md": "---\nagentType: MAIN\nharness:\n  type: oz\n"
                "  model: auto\n  reasoningLevel: high\n---\nx\n"
            }
        ),
    ),
    (
        "oz-auth",
        tree(
            **{
                "agents/main/agent.md": "---\nagentType: MAIN\nharness:\n  type: oz\n"
                "  model: auto\n  auth:\n    source: managedSecret\n    secretName: K\n---\nx\n"
            }
        ),
    ),
    (
        "worker-env-with-secret",
        tree(
            **{
                "agents/main/agent.md": "---\nagentType: MAIN\nharness:\n  type: claude\n"
                "  model: opus\n  auth:\n    source: workerEnvironment\n    secretName: K\n---\nx\n"
            }
        ),
    ),
    (
        "managed-secret-without-name",
        tree(
            **{
                "agents/main/agent.md": "---\nagentType: MAIN\nharness:\n  type: claude\n"
                "  model: opus\n  auth:\n    source: managedSecret\n---\nx\n"
            }
        ),
    ),
    ("empty-harness", tree(**{"agents/main/agent.md": "---\nagentType: MAIN\nharness: {}\n---\nx\n"})),
    (
        "unknown-harness-type",
        tree(
            **{
                "agents/main/agent.md": "---\nagentType: MAIN\nharness:\n  type: claude_code\n"
                "  model: opus\n---\nx\n"
            }
        ),
    ),
    ("alias-bad-char", tree(**{"factory.yaml": FACTORY + "alias: demo/factory\n"})),
    ("alias-too-long", tree(**{"factory.yaml": FACTORY + "alias: " + "a" * 61 + "\n"})),
    ("integration-github", tree(**{"factory.yaml": FACTORY + "integrations:\n  - type: github\n"})),
    ("two-main-agents", tree(**{"agents/other/agent.md": "---\nagentType: FOREMAN\n---\nx\n"})),
    (
        "no-main-agent",
        {"factory.yaml": FACTORY, "agents/main/agent.md": "---\nagentType: REVIEW\n---\nx\n"},
    ),
    (
        "unknown-agent-ref",
        tree(
            **{
                "automations/n/automation.md": "---\nagent: nope\ntriggers:\n  - provider: github\n"
                "    event: push\n---\nrun\n"
            }
        ),
    ),
    (
        "schedule-on-wrong-trigger",
        tree(
            **{
                "automations/n/automation.md": "---\ntriggers:\n  - provider: github\n"
                "    event: push\n    schedule:\n      cron: 0 3 * * *\n---\nrun\n"
            }
        ),
    ),
    (
        "schedule-and-schedule-ids",
        tree(
            **{
                "automations/n/automation.md": "---\ntriggers:\n  - provider: schedule\n"
                "    event: cron_fired\n    filter:\n      schedule_ids: [s1]\n    schedule:\n"
                "      cron: 0 3 * * *\n---\nrun\n"
            }
        ),
    ),
    (
        "schedule-neither",
        tree(
            **{
                "automations/n/automation.md": "---\ntriggers:\n  - provider: schedule\n"
                "    event: cron_fired\n---\nrun\n"
            }
        ),
    ),
    (
        "cron-six-fields",
        tree(
            **{
                "automations/n/automation.md": "---\ntriggers:\n  - provider: schedule\n"
                "    event: cron_fired\n    schedule:\n      cron: 0 0 3 * * *\n---\nrun\n"
            }
        ),
    ),
    (
        "cron-tz-prefix",
        tree(
            **{
                "automations/n/automation.md": "---\ntriggers:\n  - provider: schedule\n"
                "    event: cron_fired\n    schedule:\n      cron: CRON_TZ=UTC 0 3 * * *\n---\nrun\n"
            }
        ),
    ),
    (
        "unknown-filter-key",
        tree(
            **{
                "automations/n/automation.md": "---\ntriggers:\n  - provider: slack\n"
                "    event: app_mention\n    filter:\n      channels: [C1]\n---\nrun\n"
            }
        ),
    ),
    (
        "schedule-ids-not-in",
        tree(
            **{
                "automations/n/automation.md": "---\ntriggers:\n  - provider: schedule\n"
                "    event: cron_fired\n    filter:\n      schedule_ids:\n        not_in: [s1]\n"
                "    schedule:\n      cron: 0 3 * * *\n---\nrun\n"
            }
        ),
    ),
    (
        "unknown-event",
        tree(
            **{
                "automations/n/automation.md": "---\ntriggers:\n  - provider: github\n"
                "    event: issue_closed\n---\nrun\n"
            }
        ),
    ),
    (
        "unknown-provider",
        tree(
            **{
                "automations/n/automation.md": "---\ntriggers:\n  - provider: gitlab\n"
                "    event: push\n---\nrun\n"
            }
        ),
    ),
    ("no-triggers", tree(**{"automations/n/automation.md": "---\nenabled: true\n---\nrun\n"})),
    ("empty-triggers", tree(**{"automations/n/automation.md": "---\ntriggers: []\n---\nrun\n"})),
    (
        "mcp-extra-key",
        tree(
            **{
                "agents/main/agent.md": "---\nagentType: MAIN\nmcpServers:\n  gh:\n"
                "    warpId: m1\n    command: x\n---\nx\n"
            }
        ),
    ),
    ("duplicate-secrets", tree(**{"factory.yaml": FACTORY + "secrets:\n  - A\n  - A\n"})),
    (
        "empty-repositories",
        tree(
            **{
                "factory.yaml": "schemaVersion: v1alpha1\nname: demo\nrepositories: []\n"
                "agentDefaults:\n  model: auto\n"
            }
        ),
    ),
    ("bad-schema-version", tree(**{"factory.yaml": FACTORY.replace("v1alpha1", "v1beta1")})),
    ("linux-no-docker-image", tree(**{"runners/lin.yaml": "platform:\n  os: linux\n  arch: x86_64\n"})),
    ("runner-no-platform", tree(**{"runners/lin.yaml": "description: nothing\n"})),
    (
        "linux-shape-not-power-of-two",
        tree(
            **{
                "runners/lin.yaml": "instanceShape:\n  vcpus: 3\n  memoryGb: 8\nplatform:\n"
                "  os: linux\n  linux:\n    dockerImage: u\n"
            }
        ),
    ),
    (
        "macos-with-linux-section",
        tree(**{"runners/mac.yaml": "platform:\n  os: macos\n  linux:\n    dockerImage: u\n"}),
    ),
    ("macos-x86", tree(**{"runners/mac.yaml": "platform:\n  os: macos\n  arch: x86_64\n"})),
    (
        "macos-bad-version",
        tree(**{"runners/mac.yaml": "platform:\n  os: macos\n  mac:\n    version: '13'\n"}),
    ),
    (
        "macos-bad-shape",
        tree(
            **{
                "runners/mac.yaml": "platform:\n  os: macos\n  mac:\n    version: '15'\n"
                "instanceShape:\n  vcpus: 5\n  memoryGb: 9\n"
            }
        ),
    ),
    ("bad-agent-type", tree(**{"agents/main/agent.md": "---\nagentType: BOSS\n---\nx\n"})),
    (
        "bad-credential-strategy",
        tree(**{"agents/main/agent.md": "---\nagentType: MAIN\ncredentialStrategy: OWNER\n---\nx\n"}),
    ),
    (
        "unknown-agent-field",
        tree(**{"agents/main/agent.md": "---\nagentType: MAIN\nprompt: inline\n---\nx\n"}),
    ),
    ("nested-agent-path", tree(**{"agents/team/extra/agent.md": "---\n---\nx\n"})),
    (
        "duplicate-automation-name",
        tree(
            **{
                "automations/dup.md": "---\ntriggers:\n  - provider: github\n    event: push\n---\nrun\n",
                "automations/dup/automation.md": "---\ntriggers:\n  - provider: github\n"
                "    event: push\n---\nrun\n",
            }
        ),
    ),
    (
        "yaml-anchor",
        tree(
            **{
                "factory.yaml": "schemaVersion: v1alpha1\nname: demo\nrepositories:\n  - &base\n"
                "    owner: warpdotdev\n    name: warp\nagentDefaults:\n  model: auto\n"
            }
        ),
    ),
    (
        "yaml-alias",
        tree(**{"agents/main/agent.md": "---\nagentType: MAIN\nrunner: *base\n---\nx\n"}),
    ),
    (
        "yaml-tag",
        tree(**{"agents/main/agent.md": "---\nagentType: MAIN\nrunner: !!str lin\n---\nx\n"}),
    ),
    (
        "matcher-in-and-not-in-conflict",
        tree(
            **{
                "automations/n/automation.md": "---\ntriggers:\n  - provider: github\n"
                "    event: pull_request_opened\n    filter:\n      labels:\n"
                "        in: [ready]\n        not_in: [ready]\n---\nrun\n"
            }
        ),
    ),
]


def run_case(name: str, expect_valid: bool, files: dict[str, str]) -> bool:
    root = Path(tempfile.mkdtemp(prefix="factory-files-case-"))
    try:
        for relative, content in files.items():
            path = root / relative
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(content, encoding="utf-8")
        result = subprocess.run(
            [sys.executable, str(VALIDATOR), str(root)],
            capture_output=True,
            text=True,
            check=False,
        )
        if (result.returncode == 0) == expect_valid:
            return True
        expected = "valid" if expect_valid else "invalid"
        print(f"FAIL {name}: expected {expected}", file=sys.stderr)
        output = (result.stdout + result.stderr).rstrip()
        print(output or "  (no output)", file=sys.stderr)
        return False
    finally:
        shutil.rmtree(root, ignore_errors=True)


def documented_example_cases() -> list[tuple[str, bool, dict[str, str]]]:
    """Assemble the code blocks in references/examples.md into valid trees.

    The reference teaches by example, so an example that no longer validates is
    a defect in the documentation. Block indices are positional: adding or
    reordering a block in the reference means updating this mapping.
    """
    blocks = [
        body for _, body in re.findall(r"```(yaml|markdown)\n(.*?)```", EXAMPLES.read_text(), re.S)
    ]
    expected_blocks = 10
    if len(blocks) != expected_blocks:
        raise SystemExit(
            f"examples.md has {len(blocks)} example blocks, expected {expected_blocks}; "
            "update documented_example_cases()"
        )
    minimal = {"factory.yaml": blocks[0], "agents/foreman/agent.md": blocks[1]}
    full = {
        "factory.yaml": blocks[2],
        "agents/foreman/agent.md": blocks[1],
        "agents/implementer/agent.md": blocks[3],
        "agents/reviewer/agent.md": blocks[4],
        "agents/investigator/agent.md": blocks[5],
        "automations/pr-review/automation.md": blocks[6],
        "automations/nightly-sweep/automation.md": blocks[7],
        "runners/linux-standard.yaml": blocks[8],
        "runners/macos-standard.yaml": blocks[9],
    }
    return [
        ("documented-minimal-example", True, minimal),
        ("documented-full-example", True, full),
    ]


def main() -> int:
    cases = [(name, True, files) for name, files in VALID_CASES]
    cases += [(name, False, files) for name, files in INVALID_CASES]
    cases += documented_example_cases()
    failures = [name for name, expect, files in cases if not run_case(name, expect, files)]
    if failures:
        print(f"{len(failures)}/{len(cases)} factory-files validator cases failed", file=sys.stderr)
        return 1
    print(f"factory-files validator: {len(cases)}/{len(cases)} cases passed")
    return 0


if __name__ == "__main__":
    sys.exit(main())
