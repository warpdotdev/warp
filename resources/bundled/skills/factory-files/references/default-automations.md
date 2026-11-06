# Default integration automations

Use these defaults to create or repair communication automations. Other Factory
agents do not have access to `warp-server` and cannot inspect its seeded
automations, so keep this reference self-contained.

## Before adding a listener

- Reuse existing listeners; preserve their names, scopes, and prompt bodies
  unless asked to change them. Defaults omit `agent` and use MAIN/FOREMAN.
- Add only the requested integration's listeners. Confirm unknown scopes and
  identities; do not copy another Factory's values or add duplicate listeners.
- A declared integration, connected account, and enabled matching automation
  are separate prerequisites. Report missing setup rather than widening filters.

## Communication defaults

Each dotted name below is `provider.event`.

| Integration | Seeded automation | Events and filters |
| --- | --- | --- |
| Slack | `Slack DMs` and `Slack app mentions` | Separate `slack.message_dm` and `slack.app_mention` listeners, each with `filter: {}`, targeting the Factory's Slack app. |
| Linear | `Linear agent sessions` | `linear.agent_session_created` with non-empty inclusive `team_ids` (or the `teams` name alias, not both). |
| Jira | `Jira agent sessions` | `jira.agent_session_created` with non-empty inclusive `project_keys`, using Jira project keys, not numeric IDs. |
| Microsoft Teams | `teams-app-mentions` | `teams.app_mention`, one trigger per Graph team with non-empty inclusive `team_ids` and `channel_ids`. |
| GitLab | `GitLab bot mentions` | `gitlab.bot_mentioned` with `filter: {}`. The server supplies `mentioned`; do not author it. Requires the workspace's GitLab automation capability. |
| Azure DevOps | `Azure DevOps mentions and assignments` | `azure_devops.pull_request_mentioned` with repository GUIDs in `repos`, plus `azure_devops.work_item_mentioned` and `azure_devops.work_item_assigned` with `filter: {}`. Keep this name and all three triggers so the server binds omitted `mentioned`/`assignees` to the ready Factory identity. Requires identity and webhook setup. |
| GitHub | `GitHub mentions and assignments` | `github.issue_mentioned`, `github.pull_request_mentioned`, `github.issue_assigned`, and `github.pull_request_assigned`. All four triggers use inclusive `repos` for the selected `owner/repo` repositories and `labels` for the Factory's own canonical `factory:<alias>` label. Mention events also use `mentioned`; assignment events use `assignees`. Both match the Factory account's bare logins, without `@` or `[bot]`: production currently accepts `warp` and legacy `warp-factory`; staging uses `warp-staging-factory`. Do not use legacy cloud-agent handles or app display names. Confirm the Factory account for other deployments. |

Use these native intake events, not generic comment or message listeners.

## Validation

Fetch the target server's schema and validate the tree as described in
`../SKILL.md`. These are recommended defaults, not a schema copy; parser
validation does not prove integration setup or event delivery.
