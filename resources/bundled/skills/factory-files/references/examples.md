# Examples
Every value below is a placeholder. Never copy IDs, secret names, ARNs,
project numbers, or images from another factory into a new one.

## Minimal factory
```
factory.yaml
agents/foreman/agent.md
```

`factory.yaml`:

```yaml
schemaVersion: v1alpha1
name: platform-ops
description: Keeps the platform repositories healthy.
repositories:
  - owner: acme
    name: platform
agentDefaults:
  model: auto
```

`agents/foreman/agent.md`:

```markdown
---
description: Entry point for platform work.
agentType: MAIN
---
You are the platform foreman. Triage incoming work, decide whether it needs a
spec, and delegate implementation.
```

## Factory with secrets, MCP, and integrations
```yaml
schemaVersion: v1alpha1
name: platform-ops
alias: Platform Ops
credentialStrategy: EXECUTOR
repositories:
  - owner: acme
    name: platform
  - owner: acme
    name: platform-config
secrets:
  - DEPLOY_TOKEN
mcpServers:
  github:
    warpId: mcp_placeholder_github
integrations:
  - type: linear
  - type: slack
cloudProviders:
  gcp:
    projectNumber: "000000000000"
    workloadIdentityFederationPoolId: example-pool
    workloadIdentityFederationProviderId: example-provider
    serviceAccountEmail: factory@example.iam.gserviceaccount.com
agentDefaults:
  model: auto
  runner: linux-standard
  secrets:
    - DEPLOY_TOKEN
```

## Agent on a non-Oz harness
```markdown
---
description: Implements approved specs.
agentType: IMPLEMENT
harness:
  type: claude
  model: opus
  reasoningLevel: high
  auth:
    source: managedSecret
    secretName: ANTHROPIC_API_KEY
runner: linux-standard
---
Implement the approved spec. Keep the change scoped to what the spec describes.
```

The Oz shorthand is a single field, and is mutually exclusive with `harness`:

```markdown
---
agentType: REVIEW
model: auto
---
Review the change against the repository's conventions.
```

## Agent that clears an inherited value
```markdown
---
description: Runs on the shared workspace default host.
workerHost: null
secrets: []
---
Investigate the failure and report what you find.
```

`workerHost: null` clears an inherited host. `secrets: []` replaces the
inherited list with nothing; it does not merge.

## Event-driven automation
`automations/pr-review/automation.md`:

```markdown
---
agent: reviewer
triggers:
  - provider: github
    event: pull_request_opened
    filter:
      repos: [acme/platform]
      base_branches: [main]
      labels:
        not_in: [wip]
  - provider: github
    event: pull_request_synchronized
    filter:
      repos: [acme/platform]
---
Review the pull request that triggered this run.
```

## Scheduled automation
```markdown
---
enabled: true
triggers:
  - provider: schedule
    event: cron_fired
    schedule:
      name: nightly-sweep
      cron: 0 3 * * *
---
Sweep for stale branches and open a cleanup pull request if any are found.
```

## Linux runner
`runners/linux-standard.yaml`:

```yaml
description: Standard Linux build runner.
setupCommands:
  - apt-get update -y
  - apt-get install -y build-essential
instanceShape:
  vcpus: 4
  memoryGb: 8
platform:
  os: linux
  arch: x86_64
  linux:
    dockerImage: ubuntu:24.04
```

## macOS runner
`runners/macos-standard.yaml`:

```yaml
description: macOS runner for Apple platform builds.
instanceShape:
  vcpus: 6
  memoryGb: 14
platform:
  os: macos
  arch: aarch64
  mac:
    version: "15"
```
## Scorer
`scorers/tests-run/scorer.md`:

```markdown
---
description: Checks whether implementation runs include test evidence.
agents:
  - implementer
labels:
  - value: tests_run
    score: 1
  - value: tests_skipped
    score: 0
passingScore: 1
samplingRate: 25
model: claude-4-5-haiku
---
Evaluate whether the agent ran the relevant tests before finishing. Return
`tests_run` when the transcript contains the command and result; otherwise
return `tests_skipped`.
```

## Scoped skills
A skill under `skills/` is available to every agent in the factory. A skill
under `agents/<name>/skills/` is available only to that agent.

```
skills/release-checklist/SKILL.md          every agent
agents/reviewer/skills/review-style/SKILL.md   the reviewer agent only
```

These are agent playbooks, not Factory schema. Moving one between the two
locations changes who can use it, so do not relocate a skill unless that is
what was asked for.
