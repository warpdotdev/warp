# Default integration automations

Use these defaults when asked to let people communicate with a Factory from
an integration. They are recommended listeners, not a copy of the schema.
Fetch the target server's schema and validate the resulting tree as described
in `../SKILL.md`; a parser pass does not prove an integration will deliver.

## Before adding a listener

- Inspect the Factory's existing automations, declared integrations, connected
  accounts, and intended repositories, teams, projects, or channels. Repair or
  reuse an existing listener rather than adding a duplicate.
- Preserve seeded names, scopes, and prompt bodies unless the request changes
  them. Omit `agent` to route to the MAIN/FOREMAN agent, as the defaults do.
  Intake defaults have empty prompt bodies; do not invent a task that
  overrides the incoming user's request.
- A declared integration, a connected provider account, and an enabled matching
  automation are separate prerequisites. Do not promise that tagging works
  merely because an account is connected. Report missing setup rather than
  broadening filters to work around it.
- Defaults depend on the integrations and routing scopes selected at creation.
  Do not add every row below to every Factory. Confirm missing scope or identity
  values instead of copying another Factory's values. Custom workflows may use
  other events; they are not replacements for the default intake listeners.

## Communication defaults

Each dotted name below is `provider.event`.

| Integration | Seeded automation | Events and filters |
| --- | --- | --- |
| Slack | `Slack DMs` and `Slack app mentions` | Separate listeners for `slack.message_dm` and `slack.app_mention`, each with `filter: {}`. These events already target the Factory's Slack app; do not substitute a workspace-wide `message_posted` listener. |
| Linear | `Linear agent sessions` | `linear.agent_session_created`, with a non-empty inclusive `team_ids` list for the selected Linear teams. The `teams` name alias is also available; do not declare both. |
| Jira | `Jira agent sessions` | `jira.agent_session_created`, with a non-empty inclusive `project_keys` list, such as `[APP]`. Use Jira project keys, not numeric IDs or the Factory alias. |
| Microsoft Teams | `teams-app-mentions` | `teams.app_mention`, one trigger per Graph team, with that team's `team_ids` and selected `channel_ids`. Both scopes are non-empty. |
| GitLab | `GitLab bot mentions` | `gitlab.bot_mentioned`, authored with `filter: {}`. The server supplies the Factory's service-account `mentioned` matcher; do not commit or guess it. Requires the workspace's GitLab automation capability. |
| Azure DevOps | `Azure DevOps mentions and assignments` | Three triggers: `azure_devops.pull_request_mentioned` with selected repository GUIDs in `repos`, plus `azure_devops.work_item_mentioned` and `azure_devops.work_item_assigned` with `filter: {}`. Preserve this canonical name and complete shape: the server binds omitted `mentioned`/`assignees` to the ready Factory identity. Requires Azure DevOps identity and webhook setup. |
| GitHub | `GitHub mentions and assignments` | Four triggers: `github.issue_mentioned`, `github.pull_request_mentioned`, `github.issue_assigned`, and `github.pull_request_assigned`. See identity and scope rules below. |

For Linear and Jira communication, use the agent-session event rather than
trying to reconstruct mentions or delegation from issue/comment webhooks.

## GitHub identity and routing

GitHub Factory communication uses the **Factory account**, not the legacy
cloud-agent account or the app's display name. Resolve the target server's
accepted Factory handles from its seeded automation or configuration:

- Production currently accepts `warp` and legacy `warp-factory`.
- Staging currently uses `warp-staging-factory`.
- Filter values are bare logins, without `@` or `[bot]`. Do not substitute
  `warp-agent`, `warp-agent-staging`, or a Slack app mention.

Every default GitHub trigger carries inclusive `repos` for the Factory's
selected `owner/repo` repositories and `labels` for its own canonical
`factory:<alias>` label. Reuse the actual Factory label; do not copy another
Factory's alias. Mention events additionally filter `mentioned` on the
accepted Factory handles; assignment events instead filter `assignees`.
Keep all four events: PR mentions alone miss issues and assignments.

For example, the staging mention/assignment listener for an illustrative
Factory with alias `example` and repository `acme/service` is:

```markdown
---
enabled: true
triggers:
  - provider: github
    event: issue_mentioned
    filter:
      repos: [acme/service]
      labels: ["factory:example"]
      mentioned: [warp-staging-factory]
  - provider: github
    event: pull_request_mentioned
    filter:
      repos: [acme/service]
      labels: ["factory:example"]
      mentioned: [warp-staging-factory]
  - provider: github
    event: issue_assigned
    filter:
      repos: [acme/service]
      labels: ["factory:example"]
      assignees: [warp-staging-factory]
  - provider: github
    event: pull_request_assigned
    filter:
      repos: [acme/service]
      labels: ["factory:example"]
      assignees: [warp-staging-factory]
---
```

Do not replace this with `pull_request_commented` plus a keyword or a
`pull_request_review_submitted` listener: those are different workflows.

### Other seeded GitHub follow-ups

These also use the same repository and Factory-label scopes:

- `GitHub PR closed or merged`: `pull_request_closed` and
  `pull_request_merged`.
- `GitHub PR labeled`: `pull_request_labeled`.
- `GitHub issue labeled`: `issue_labeled` and `issue_created` so an issue
  created with the label is included.
- `GitHub PR review submitted`: `pull_request_review_submitted`.

The closed/merged, PR-labeled, and review-submitted defaults carry dedicated
prompt bodies; preserve them when repairing triggers. The issue-labeled
default, like mention/assignment intake, has an empty body.

## Source of truth

Defaults and environment identities can change independently of this bundled
skill. Prefer the target Factory's current seeded configuration. When source
is available, verify against warp-server's
[`logic/factorysource/defaults/automations.go`](https://github.com/warpdotdev/warp-server/blob/develop/logic/factorysource/defaults/automations.go),
`logic/factorysource/defaults/azure_devops_automation.go`, and
`config/github.go` plus the target environment's configuration. The target
server's schema remains authoritative for accepted events and filter fields.
