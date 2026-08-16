#!/usr/bin/env python3
"""Regression and packaging checks for the bundled factory-files skill.

Each case builds a throwaway Factory tree and asserts whether
resources/bundled/skills/factory-files/scripts/validate_factory_files.py
accepts it. The expected verdicts were verified against the authoritative Go
implementation in warp-server (factoryfile.ParseTree and
triggers.ValidateFilter), so a change here should be made only alongside a
matching change to the Factory file format.

The final check runs prepare_bundled_resources and compares the packaged skill
tree byte-for-byte with the canonical source.

Some VALID_CASES assert that the validator TOLERATES input the current server
rejects. Those are not mistakes and must not be "corrected" into INVALID_CASES:
the schemas ship inside a Warp release and are routinely older than the server,
so they defer catalogue and limit decisions rather than rejecting a tree built
for a newer server. If one of them starts failing, a schema was tightened;
reopen the schema rather than moving the case. See specs/REMOTE-2727/TECH.md.

Run directly, or via script/presubmit.
"""

from __future__ import annotations

import os
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
        "schedule-ids-not-in",
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
        "cloud-providers-current-key",
        tree(
            **{
                "factory.yaml": FACTORY
                + "cloudProviders:\n  aws:\n    roleArn: arn:aws:iam::123456789012:role/factory\n"
            }
        ),
    ),
    (
        "cloud-providers-current-and-legacy",
        tree(
            **{
                "factory.yaml": FACTORY
                + "cloudProviders:\n  aws:\n    roleArn: arn:aws:iam::123456789012:role/current\n"
                + "providers:\n  aws:\n    roleArn: arn:aws:iam::123456789012:role/legacy\n"
            }
        ),
    ),
    (
        "integrations-normalize-case-and-space",
        tree(**{"factory.yaml": FACTORY + "integrations:\n  - type: ' Slack '\n"}),
    ),
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
        "jira-agent-session-labels",
        tree(
            **{
                "automations/n/automation.md": "---\ntriggers:\n  - provider: jira\n"
                "    event: agent_session_created\n    filter:\n      project_keys: [ENG]\n"
                "      labels: [triage]\n      keywords: [urgent]\n---\nrun\n"
            }
        ),
    ),
    # Jira label matching is case-sensitive, so values differing only in case
    # are two distinct labels rather than a conflict.
    (
        "jira-labels-are-case-sensitive",
        tree(
            **{
                "automations/n/automation.md": "---\ntriggers:\n  - provider: jira\n"
                "    event: issue_labeled\n    filter:\n      labels:\n        in: [Bug]\n"
                "        not_in: [bug]\n---\nrun\n"
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
    (
        "nullable-overrides",
        tree(
            **{
                "factory.yaml": FACTORY + "credentialStrategy: null\n",
                "agents/main/agent.md": "---\nagentType: MAIN\nmodel: null\n"
                "credentialStrategy: null\nrunner: null\nenvironmentId: null\n---\nx\n",
            }
        ),
    ),
    (
        "sparse-harness-null-fields",
        tree(
            **{
                "agents/main/agent.md": "---\nagentType: MAIN\nharness:\n  type: claude\n"
                "  model: null\n  auth: null\n---\nx\n",
            }
        ),
    ),
    (
        "unicode-and-padded-alias",
        tree(**{"factory.yaml": FACTORY + "alias: '                                                            café '\n"}),
    ),
    (
        "large-linux-power-of-two-shape",
        tree(
            **{
                "runners/large.yaml": "instanceShape:\n  vcpus: 2048\n  memoryGb: 2048\n"
                "platform:\n  os: linux\n  linux:\n    dockerImage: ubuntu:24.04\n"
            }
        ),
    ),
    (
        "macos-default-version",
        tree(**{"runners/mac.yaml": "platform:\n  os: macos\n  arch: aarch64\n"}),
    ),
    (
        "named-cron-fields",
        tree(
            **{
                "automations/monthly/automation.md": "---\ntriggers:\n  - provider: schedule\n"
                "    event: cron_fired\n    schedule:\n      cron: 0 3 1 JAN MON-FRI\n---\nx\n"
            }
        ),
    ),
    (
        "scorer-complete",
        tree(
            **{
                "agents/implementer/agent.md": "---\n---\nImplement.\n",
                "scorers/tests/scorer.md": "---\nagents:\n  - implementer\nlabels:\n"
                "  - value: pass\n    score: 1\n  - value: fail\n    score: 0\n"
                "passingScore: 1\nsamplingRate: 25\nmodel: claude-4-5-haiku\n"
                "selfImprovement: true\n---\nEvaluate the run.\n",
            }
        ),
    ),
    # ---------------------------------------------------------------
    # Deliberate forward-compatibility tolerances.
    #
    # Each case below is input the CURRENT server rejects and this
    # validator accepts anyway, so that a Warp release older than the
    # server does not block valid work. Do not move these to
    # INVALID_CASES to "match the server" - that reintroduces exactly the
    # false rejections these were added to prevent.
    # ---------------------------------------------------------------
    (
        "new-event-on-known-provider-leaves-filter-open",
        tree(
            **{
                "automations/n/automation.md": "---\ntriggers:\n  - provider: github\n"
                "    event: brand_new_github_event\n    filter:\n"
                "      brand_new_filter_key: [x]\n---\nrun\n"
            }
        ),
    ),
    (
        "oz-harness-capability-limits-are-server-owned",
        tree(
            **{
                "agents/main/agent.md": "---\nagentType: MAIN\nharness:\n  type: oz\n"
                "  model: auto\n  reasoningLevel: high\n"
                "  auth:\n    source: managedSecret\n    secretName: K\n---\nx\n"
            }
        ),
    ),
    (
        "scorer-label-count-above-current-server-cap",
        tree(
            **{
                "scorers/many/scorer.md": "---\nagents: [main]\nlabels:\n"
                + "".join(
                    f"  - value: label_{index}\n    score: {1 if index == 0 else 0}\n"
                    for index in range(21)
                )
                + "passingScore: 1\nmodel: m\n---\nRubric.\n",
            }
        ),
    ),
    (
        "forward-compatible-unknowns",
        tree(
            **{
                "factory.yaml": FACTORY
                + "futureFactoryField: enabled\n"
                + "integrations:\n  - type: future-provider\n",
                "agents/main/agent.md": "---\nagentType: MAIN\nfutureAgentField: true\n"
                "credentialStrategy: FUTURE\nharness:\n  type: future-harness\n"
                "  model: future-model\nmcpServers:\n  future:\n    warpId: future\n"
                "    futureMcpField: true\n---\nx\n",
                "agents/future/agent.md": "---\nagentType: FUTURE\n---\nx\n",
                "automations/future/automation.md": "---\nfutureAutomationField: true\n"
                "triggers:\n  - provider: future-provider\n    event: future_event\n"
                "    filter:\n      future_filter: [value]\n---\nx\n",
                "runners/future.yaml": "futureRunnerField: true\nplatform:\n"
                "  os: future-os\n  arch: future-arch\n",
                "scorers/future/scorer.md": "---\nagents: [main]\nlabels:\n"
                "  - value: pass\n    score: 1\n  - value: fail\n    score: 0\n"
                "passingScore: 1\nmodel: future-model\nfutureScorerField: true\n---\nRubric.\n",
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
    ("alias-bad-char", tree(**{"factory.yaml": FACTORY + "alias: demo/factory\n"})),
    ("alias-too-long", tree(**{"factory.yaml": FACTORY + "alias: " + "a" * 61 + "\n"})),
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
        "unknown-filter-key-on-known-provider-and-event",
        tree(
            **{
                "automations/n/automation.md": "---\ntriggers:\n  - provider: slack\n"
                "    event: app_mention\n    filter:\n      channels: [C1]\n---\nrun\n"
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
    ("no-triggers", tree(**{"automations/n/automation.md": "---\nenabled: true\n---\nrun\n"})),
    ("empty-triggers", tree(**{"automations/n/automation.md": "---\ntriggers: []\n---\nrun\n"})),
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
    ("bad-agent-type", tree(**{"agents/main/agent.md": "---\nagentType: BOSS\n---\nx\n"})),
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
    (
        "canonical-matcher-conflict",
        tree(
            **{
                "automations/n/automation.md": "---\ntriggers:\n  - provider: slack\n"
                "    event: reaction_added\n    filter:\n      emojis:\n"
                "        in: [':eyes:']\n        not_in: [eyes]\n---\nrun\n"
            }
        ),
    ),
    ("alias-emoji", tree(**{"factory.yaml": FACTORY + "alias: factory🚀\n"})),
    (
        "trimmed-alias-too-long",
        tree(**{"factory.yaml": FACTORY + "alias: ' " + "a" * 61 + " '\n"}),
    ),
    (
        "empty-harness-model",
        tree(
            **{
                "agents/main/agent.md": "---\nagentType: MAIN\nharness:\n"
                "  type: claude\n  model: ''\n---\nx\n"
            }
        ),
    ),
    (
        "partial-runner-shape",
        tree(
            **{
                "runners/partial.yaml": "instanceShape:\n  vcpus: 4\nplatform:\n  os: linux\n"
                "  linux:\n    dockerImage: ubuntu:24.04\n"
            }
        ),
    ),
    (
        "empty-macos-config",
        tree(**{"runners/mac.yaml": "platform:\n  os: macos\n  arch: aarch64\n  mac: {}\n"}),
    ),
    (
        "cron-field-out-of-range",
        tree(
            **{
                "automations/bad/automation.md": "---\ntriggers:\n  - provider: schedule\n"
                "    event: cron_fired\n    schedule:\n      cron: 99 25 32 13 8\n---\nx\n"
            }
        ),
    ),
    (
        "cron-invalid-every-duration",
        tree(
            **{
                "automations/bad/automation.md": "---\ntriggers:\n  - provider: schedule\n"
                "    event: cron_fired\n    schedule:\n      cron: '@every someday'\n---\nx\n"
            }
        ),
    ),
    (
        "duplicate-inline-schedule-name",
        tree(
            **{
                "automations/dup/automation.md": "---\ntriggers:\n"
                "  - provider: schedule\n    event: cron_fired\n    schedule:\n"
                "      name: same\n      cron: 0 1 * * *\n"
                "  - provider: schedule\n    event: cron_fired\n    schedule:\n"
                "      name: same\n      cron: 0 2 * * *\n---\nx\n"
            }
        ),
    ),
    (
        "duplicate-trimmed-secrets",
        tree(**{"factory.yaml": FACTORY + "secrets: [' A ', A]\n"}),
    ),
    (
        "duplicate-trimmed-repositories",
        tree(
            **{
                "factory.yaml": "schemaVersion: v1alpha1\nname: demo\nrepositories:\n"
                "  - owner: ' warp '\n    name: repo\n"
                "  - owner: warp\n    name: repo\nagentDefaults:\n  model: auto\n"
            }
        ),
    ),
    (
        "quoted-scalar-with-trailing-junk",
        tree(**{"factory.yaml": FACTORY + 'description: "quoted" junk\n'}),
    ),
    (
        "unquoted-yaml-timestamp",
        tree(**{"factory.yaml": FACTORY + "description: 2026-08-15\n"}),
    ),
    (
        "scorer-empty-rubric",
        tree(
            **{
                "scorers/empty/scorer.md": "---\nagents: [main]\nlabels:\n"
                "  - value: pass\n    score: 1\n  - value: fail\n    score: 0\n"
                "passingScore: 1\nmodel: m\n---\n",
            }
        ),
    ),
    (
        "scorer-unknown-agent",
        tree(
            **{
                "scorers/unknown/scorer.md": "---\nagents: [missing]\nlabels:\n"
                "  - value: pass\n    score: 1\n  - value: fail\n    score: 0\n"
                "passingScore: 1\nmodel: m\n---\nRubric.\n",
            }
        ),
    ),
    (
        "scorer-all-pass",
        tree(
            **{
                "scorers/all-pass/scorer.md": "---\nagents: [main]\nlabels:\n"
                "  - value: pass\n    score: 1\n  - value: better\n    score: 0.9\n"
                "passingScore: 0.5\nmodel: m\n---\nRubric.\n",
            }
        ),
    ),
    (
        "scorer-zero-sampling",
        tree(
            **{
                "scorers/zero/scorer.md": "---\nagents: [main]\nlabels:\n"
                "  - value: pass\n    score: 1\n  - value: fail\n    score: 0\n"
                "passingScore: 1\nsamplingRate: 0\nmodel: m\n---\nRubric.\n",
            }
        ),
    ),
    (
        "scorer-flat-path",
        tree(
            **{
                "scorers/flat.md": "---\nagents: [main]\nlabels:\n"
                "  - value: pass\n    score: 1\n  - value: fail\n    score: 0\n"
                "passingScore: 1\nmodel: m\n---\nRubric.\n",
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
        body
        for _, body in re.findall(
            r"```(yaml|markdown)\n(.*?)```", EXAMPLES.read_text(encoding="utf-8"), re.S
        )
    ]
    expected_blocks = 11
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
        "scorers/tests-run/scorer.md": blocks[10],
    }
    return [
        ("documented-minimal-example", True, minimal),
        ("documented-full-example", True, full),
    ]


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
        source_files = {
            path.relative_to(SKILL)
            for path in SKILL.rglob("*")
            if path.is_file()
        }
        packaged_files = {
            path.relative_to(packaged)
            for path in packaged.rglob("*")
            if path.is_file()
        }
        if source_files != packaged_files:
            raise RuntimeError(
                f"packaged file set differs: missing={source_files - packaged_files}, "
                f"extra={packaged_files - source_files}"
            )
        for relative in source_files:
            if (SKILL / relative).read_bytes() != (packaged / relative).read_bytes():
                raise RuntimeError(f"packaged content differs for {relative}")

# Keywords that reject an otherwise well-formed value because it is not in a
# hard-coded list. Every one of these is a place a newer server could legitimately
# widen, so they are banned outside the narrow exceptions below.
_CLOSED_KEYWORDS = ("enum", "const", "maxItems")


def _walk_schema(node, path, found):
    if isinstance(node, dict):
        for keyword in _CLOSED_KEYWORDS:
            if keyword in node:
                found.append((path, keyword))
        if node.get("additionalProperties") is False:
            found.append((path, "additionalProperties:false"))
        for key, value in node.items():
            _walk_schema(value, f"{path}/{key}", found)
    elif isinstance(node, list):
        for index, value in enumerate(node):
            _walk_schema(value, f"{path}/{index}", found)


def assert_schemas_stay_forward_compatible() -> None:
    """Fail if a schema was closed back up against future server changes.

    The bundled schemas ship inside a Warp release and are routinely older than
    the warp-server they validate against, so rejecting unknown properties or
    unknown catalogue values would block configuration a newer server accepts.
    New values belong in an x-warp-known-values annotation instead.

    Three exceptions are allowed, all scoped so drift cannot trip them:

    - `if` conditions select which rule applies; they never reject on their own.
    - `$defs` named `declares*` exist only to be referenced from an `if`, so they
      are conditions too. Keep that naming convention for new ones.
    - Per-(provider, event) trigger filter objects close their key set, because
      a misspelled filter key is a common mistake that otherwise survives until
      apply. Those rules only fire when both provider and event are recognized.
    """
    import json

    offenders: list[str] = []
    for schema_path in sorted((SKILL / "schemas").glob("*.schema.json")):
        document = json.loads(schema_path.read_text(encoding="utf-8"))
        found: list[tuple[str, str]] = []
        _walk_schema(document, "", found)
        for path, keyword in found:
            if "/if/" in path or path.endswith("/if"):
                continue
            if path.startswith("/$defs/declares"):
                continue
            if keyword == "additionalProperties:false" and path.endswith("/then/properties/filter"):
                continue
            offenders.append(f"{schema_path.name}{path or '/'} uses {keyword}")
    if offenders:
        raise RuntimeError(
            "these schemas were tightened against future server changes, which "
            "would reject trees a newer server accepts; record new values in an "
            "x-warp-known-values annotation instead (see "
            "specs/REMOTE-2727/TECH.md):\n  " + "\n  ".join(offenders)
        )


def assert_symlinked_resources_are_refused() -> None:
    """A resource file that links out of the tree must not be read.

    The dict-based cases above cannot express a symlink, so this builds one
    directly. Two things are asserted: the tree is rejected, and the link
    target's content never reaches the output. The server parses an in-memory
    git tree, where a symlink is a blob holding the target path, so it never
    reads the target either.
    """
    canary = "CANARY-SHOULD-NEVER-BE-ECHOED"
    root = Path(tempfile.mkdtemp(prefix="factory-files-symlink-"))
    try:
        (root / "agents" / "main").mkdir(parents=True)
        (root / "runners").mkdir()
        (root / "factory.yaml").write_text(FACTORY, encoding="utf-8")
        (root / "agents" / "main" / "agent.md").write_text(MAIN_AGENT, encoding="utf-8")

        outside = Path(tempfile.mkdtemp(prefix="factory-files-outside-"))
        try:
            secret = outside / "secret.txt"
            secret.write_text(f"{canary}\nnot a mapping: [\n", encoding="utf-8")
            (root / "runners" / "linked.yaml").symlink_to(secret)

            result = subprocess.run(
                [sys.executable, str(VALIDATOR), str(root)],
                capture_output=True,
                text=True,
                check=False,
            )
            output = result.stdout + result.stderr
            if result.returncode == 0:
                raise RuntimeError("a symlinked resource file was accepted")
            if canary in output:
                raise RuntimeError(
                    "the validator echoed the content of a file outside the Factory root"
                )
            if "symlink" not in output:
                raise RuntimeError(f"expected a symlink diagnostic, got: {output.strip()}")
        finally:
            shutil.rmtree(outside, ignore_errors=True)

        # A link whose target is inside the root is refused too. The escape
        # check cannot see this one, so it exercises the is_symlink branch: the
        # server still reads the link rather than its target.
        (root / "runners" / "linked.yaml").unlink()
        (root / "runners" / "real.yaml").write_text(
            "platform:\n  linux:\n    dockerImage: ubuntu:24.04\n", encoding="utf-8"
        )
        (root / "runners" / "alias.yaml").symlink_to(root / "runners" / "real.yaml")
        result = subprocess.run(
            [sys.executable, str(VALIDATOR), str(root)],
            capture_output=True,
            text=True,
            check=False,
        )
        if result.returncode == 0:
            raise RuntimeError("a symlink pointing inside the Factory root was accepted")
    finally:
        shutil.rmtree(root, ignore_errors=True)


def main() -> int:
    # Run first: tightening a schema also fails several corpus cases, and this
    # explains why rather than leaving the reader to infer it from a rejection.
    assert_schemas_stay_forward_compatible()
    cases = [(name, True, files) for name, files in VALID_CASES]
    cases += [(name, False, files) for name, files in INVALID_CASES]
    cases += documented_example_cases()
    failures = [name for name, expect, files in cases if not run_case(name, expect, files)]
    if failures:
        print(f"{len(failures)}/{len(cases)} factory-files validator cases failed", file=sys.stderr)
        return 1
    assert_symlinked_resources_are_refused()
    assert_packaged_skill_matches()
    print(f"factory-files validator: {len(cases)}/{len(cases)} cases passed")
    print("factory-files schemas: still open to future server changes")
    print("factory-files resources: symlinks out of the tree are refused")
    print("factory-files packaging: source and bundled trees match")
    return 0


if __name__ == "__main__":
    sys.exit(main())
