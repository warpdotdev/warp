---
name: cross-platform-cloud-verification
description: Orchestrates cost-conscious cloud verification across available operating-system and architecture runners after cheaper checks pass. Use whenever a code change, bug fix, build, packaging flow, native dependency, UI behavior, filesystem behavior, or test has material cross-platform implications, even if the user only asks to verify or test the change. Consult domain verification skills for what to test; use this skill to choose and access the smallest relevant set of platforms.
compatibility: Requires the Oz CLI (`oz-dev`) for runner discovery and `run_agents` support for `remote.runner_id`.
---

# Cross-platform cloud verification

Verify a change on the smallest useful set of cloud platforms. This skill owns
runner discovery, platform selection, child orchestration, and result
aggregation. It does not define the product-specific verification procedure.

Cross-platform execution consumes remote compute and credits, so use it as the
last verification layer rather than as an exploratory first step.

## Workflow

### 1. Establish the local verification gate

Finish the cheaper feedback loops first:

1. Inspect the change and identify the behavior being verified.
2. Run the relevant local build, focused tests, lint, type checks, or manual
   verification.
3. Fix deterministic local failures before launching cloud runs.
4. Confirm the exact commit or branch containing the change can be checked out
   by cloud agents. Do not test the default branch when the intended change is
   only local.

Do not commit or push changes merely to satisfy this workflow unless the user
has authorized that action. If the change is not remotely reachable, report the
precondition and ask for the minimum action needed.

Skip cloud fan-out when local verification already shows the change is broken.
Do not use remote platforms as a substitute for diagnosing an ordinary local
failure.

### 2. Delegate test design to verification skills

Inspect the available skill descriptions and read every skill that materially
defines how to verify the affected surface. Examples include GUI computer-use
verification, TUI live verification, integration testing, unit testing, CI
diagnosis, packaging, or repository-specific validation.

Extract from those skills:

- setup and authentication requirements
- commands or interactions to perform
- expected observations and pass criteria
- required screenshots, recordings, logs, or text captures
- platform-specific caveats

Build one shared verification procedure from that guidance. Tell each child
which verification skill to read when it is available in the child environment,
and include the essential procedure directly so the run remains actionable if
the skill is unavailable there.

This skill decides where to run that procedure, not what the procedure should
be.

### 3. Discover runners at execution time

List runners immediately before selecting platforms:

```sh
oz-dev runner list --output-format json
```

Use the returned runner metadata rather than hard-coding names or IDs. Relevant
fields include `uid`, `name`, `os`, `arch`, compute capacity, image or macOS
version, and setup commands.

If runner discovery fails because of authentication, permissions, or service
availability, report the blocker instead of inventing a runner matrix.

### 4. Infer the affected platform dimensions

Determine which dimensions the change can plausibly affect from the diff,
repository configuration, reported bug, and verification guidance.

Treat the change as **OS-sensitive** when it touches or depends on items such as:

- OS-gated code or platform modules
- windowing, desktop integration, installers, packaging, or signing
- shells, process creation, permissions, filesystems, paths, or line endings
- OS-specific APIs, toolchains, or user-facing behavior

Treat the change as **architecture-sensitive** only with evidence such as:

- architecture gates or assembly
- unsafe code, FFI, ABI, alignment, endianness, or binary serialization
- SIMD, native libraries, architecture-specific dependencies, or packaging
- an architecture-specific bug report or explicit user requirement

Do not infer architecture sensitivity merely because both x86-64 and AArch64
runners exist.

### 5. Select the minimal runner matrix

Filter discovered runners to those that can execute the verification procedure,
then apply these defaults:

1. Select only affected operating systems.
2. Select one representative runner per affected OS.
3. Add another architecture for an OS only when the change is
   architecture-sensitive.
4. Do not add an unaffected control platform by default.
5. Deduplicate equivalent OS/architecture candidates.

When several runners cover the same platform, prefer in order:

1. the exact OS/version/architecture from the bug report or target
2. the repository's normal CI or release platform
3. a runner whose image, setup, capacity, and tools match the procedure
4. the lower-setup-cost candidate

For architecture-insensitive Linux changes, prefer the repository's primary
Linux architecture; if the repository gives no signal, use x86-64 as the single
representative.

Before launching, record a concise selection rationale and explicitly list
relevant platforms omitted because no runner is available. Missing runners do
not block useful verification on available relevant platforms.

State each omission once. After the matrix is decided, do not keep repeating
that an irrelevant or redundant platform was not selected in child prompts,
result rows, evidence, and the conclusion.

### 6. Construct verification-only child prompts

Every child prompt should contain:

- repository, branch, and exact commit to check out
- selected runner name, OS, and architecture
- verification skill names and the distilled procedure
- setup and state preconditions
- exact commands or interactions
- expected results and required evidence
- a prohibition on fixing or pushing code

Trust orchestration to route the child to the requested runner. Do not make every
child re-confirm its OS and architecture. Ask for platform introspection only
when the verification depends on an exact OS version or capability, or when
there is evidence of a routing problem.

Ask each child to return:

```text
Platform: <runner name; OS; architecture; version if relevant>
Commit: <tested SHA>
Status: <passed | failed | blocked>
Checks:
- <command or interaction>: <result>
Evidence:
- <artifact, screenshot, log, or concise observation>
Deviations:
- <difference from the requested procedure, or none>
```

Children may make temporary, uncommitted setup adjustments required by the
verification skill, but they must report them and must not push source changes.

### 7. Launch with `run_agents`

Use `run_agents` so child IDs, messages, lifecycle events, and artifacts remain
part of the parent orchestration flow. Use the same repository environment for
every selected platform and set `remote.runner_id` to the discovered runner UID.

```text
summary: Verifying the change on <OS/architecture>.
base_prompt: <shared verification-only instructions>
remote:
  environment_id: <repository environment ID>
  runner_id: <selected runner UID>
  computer_use_enabled: <true only when the verification procedure needs it>
agent_run_configs:
- name: <short platform-specific name>
  prompt: <platform-specific procedure and expected evidence>
```

If no suitable repository environment is already known, inspect
`oz-dev environment list` and choose one that checks out the target repository.
Do not silently create or mutate an environment.

`remote.runner_id` is run-wide, so runners with different UIDs require separate
`run_agents` calls. Use one single-child batch per selected runner, or group
multiple independently useful children only when they share the same runner and
run-wide configuration. Distinct runner IDs are a legitimate reason for
separate batches; do not place children for different platforms in one batch.

Omit `model_id` and `remote.harness` unless the user requested them. Attach
relevant verification skills when the children need them. If an approved
orchestration config is active, ensure its resolved runner matches the selected
runner because config resolution takes precedence over call fields.

Capture each trusted `agent_id` from the launched result. Coordinate through
pushed child messages and lifecycle events; do not poll `oz-dev run get` or
`list_messages_from_agents`. Read notified messages, intervene only for a
failure or actionable block, and use `wait_for_events` when no other work can
proceed. Allow at most one retry for a clearly transient infrastructure
failure. A product failure is evidence, not a reason to spend more credits
repeating the same check.

### 8. Aggregate without hiding gaps

Wait for every launched child, then report:

```text
Cross-platform verification: <passed | failed | incomplete>
Local gate: <checks completed before cloud launch>
Results:
- <OS/arch; runner>: <why selected> - <passed/failed/blocked>
Unverified:
- <relevant OS/arch>: unverified because no suitable runner was available

Evidence:
- <platform>: <commands, observations, and artifact/run links>

Conclusion:
- <what the results establish and what remains unverified>
```

Use `passed` only when every selected platform passed and no required platform
is unavailable. Use `failed` when any verification check failed. Use
`incomplete` when runs were blocked or a relevant platform lacked a runner.

Keep dry-run proposals and final summaries compact. Include the local gate, the
selected matrix, one concise omission rationale when it is material, the
verification procedure, and verdict rules. Do not repeat setup mechanics or
platform exclusions in multiple sections. A dry-run proposal should usually fit
in roughly 300-500 words; expand only when the domain verification procedure
genuinely requires more detail.

## Cost and scope guardrails

- Run this workflow after local verification, not during implementation.
- Choose platforms from evidence in the change, not from runner availability.
- Keep one child per selected platform unless distinct state variants are
  independently necessary.
- State runner-selection and omission decisions once.
- Preserve negative results and platform gaps; do not broaden the matrix merely
  to obtain a passing result.
- Do not let children implement fixes. Return failures to the parent workflow,
  fix locally, rerun cheap checks, and only then decide whether another cloud
  verification pass is justified.
