# Automation triggers
A trigger names a `provider` and an `event`, and may narrow which deliveries
match with a `filter`.

```yaml
triggers:
  - provider: github
    event: pull_request_opened
    filter:
      repos: [warpdotdev/warp-server]
      base_branches: [main]
```

## Why filter keys need care
The Factory file parser accepts any mapping as a `filter`. Filter keys are
validated later, when the plan is applied. A wrong key therefore parses fine
and fails at apply time, so treat the catalogue below as the contract. Keys are
`snake_case`; a camelCase spelling such as `baseBranches` is rejected.

Fields combine with AND. An absent field is a wildcard.

## Filter values
Each filter field takes a matcher. A bare array is sugar for `{in: [...]}`.

```yaml
labels: [ready]                   # ANY-of
labels:
  in: [ready, urgent]             # ANY-of
  not_in: [wip]                   # excludes ANY-of
```

A value present in both `in` and `not_in` is rejected: the filter could never
match. Some fields compare canonical forms too — for example, case-insensitive
usernames or emoji names — so equivalent values can conflict even when their
source spelling differs. `schedule_ids` supports only `in`, because excluding
one schedule would match every other schedule in scope.

All values are strings except `pr_numbers`, which takes integers.

## github
- `push` — `repos`, `branches`, `paths`
- `issue_created`, `issue_labeled` — `repos`, `labels`, `assignees`, `authors`
- `pull_request_opened`, `pull_request_ready`, `pull_request_closed`,
  `pull_request_merged`, `pull_request_labeled`, `pull_request_synchronized`,
  `pull_request_reopened` — `repos`, `base_branches`, `pr_numbers`, `paths`,
  `assignees`, `authors`, `labels`
- `issue_mentioned`, `pull_request_mentioned` — `repos`, `mentioned`, `labels`
- `issue_assigned`, `pull_request_assigned` — `repos`, `assignees`, `labels`
- `pull_request_review_requested` — `repos`, `reviewers`, `reviewer_teams`
- `pull_request_review_submitted` — `repos`, `mentioned`, `labels`,
  `review_states`
- `check_suite_completed` — `repos`, `conclusions`, `branches`, `labels`,
  `authors`
- `workflow_run_completed` — `repos`, `conclusions`, `branches`, `workflows`,
  `labels`, `authors`
- `check_run_rerequested`, `check_suite_rerequested` — `repos`

`repos` values are `owner/name`. Branch values may be written with or without a
`refs/heads/` prefix.

## gitlab
- `merge_request` — `repos`, `actions`, `base_branches`
- `bot_mentioned` — `repos`

`mentioned` is accepted on `bot_mentioned` but the server seeds it, so declaring
it has no effect.

## factory
- `work_item_stage_changed` — `stages`

A Factory delivery is already scoped to one factory, so the stage the work item
moved into is the only dimension worth constraining.

## linear
- `issue_created`, `issue_labeled`, `issue_state_changed`, `issue_assigned` —
  `team_ids`, `project_ids`, `labels`, `state_ids`, `assignee_ids`,
  `mentioned_user_ids`, `creator_ids`
- `comment_created` — `team_ids`, `project_ids`, `labels`, `state_ids`,
  `issue_ids`, `mentioned_user_ids`, `creator_ids`
- `agent_session_created` — `team_ids`, `creator_ids`, `keywords`

Linear `*_ids` fields take durable Linear UUIDs, not display names or keys.
`labels` matches by name, case-insensitively.

## jira
- `issue_created`, `issue_labeled` — `project_keys`, `labels`
- `status_changed` — `project_keys`, `status_ids`
- `agent_session_created` — `project_keys`, `labels`, `keywords`

Jira `labels` match case-sensitively, unlike the other providers. On
`agent_session_created` the session payload does not carry labels, so a
constrained subscription resolves them through a best-effort issue fetch and
fails closed when that returns none.

## slack
- `app_mention`, `message_dm`, `message_im`, `message_mpim`, `message_posted` —
  `channel_ids`, `user_ids`, `keywords`
- `reaction_added` — `channel_ids`, `user_ids`, `emojis`, `keywords`,
  `item_user_ids`
- `member_joined_channel` — `channel_ids`, `user_ids`

`channel_ids` and `user_ids` take Slack IDs (`C…`, `U…`), not `#channel` or
`@user` names. `emojis` are emoji names; colons and skin-tone suffixes are
ignored. `keywords` match message text case-insensitively as substrings.

A channel message that mentions the app produces both a `message_posted` and an
`app_mention` delivery, so subscribe to one kind or the other, not both.

## schedule
- `cron_fired` — `schedule_ids`

A `schedule.cron_fired` trigger must name exactly one source of schedule:

```yaml
# Declare the schedule inline.
triggers:
  - provider: schedule
    event: cron_fired
    schedule:
      name: nightly-sweep
      cron: 0 3 * * *

# Or watch schedules that already exist.
triggers:
  - provider: schedule
    event: cron_fired
    filter:
      schedule_ids: [sched_abc123]
```

Declaring both, or neither, is an error: a trigger with neither would subscribe
to every schedule delivery for the team.

`schedule` is only valid on a `schedule.cron_fired` trigger.

### Inline schedules
- `cron` — a standard five-field expression or a descriptor (`@daily`,
  `@hourly`, `@every 1h`). Always interpreted in UTC, so a `CRON_TZ=` or `TZ=`
  prefix is rejected, as is the six-field form carrying seconds. Field ranges,
  lists, steps, and month/day names follow the server's robfig/cron grammar.
- `name` — the declaration's stable identity within its automation. Editing
  `cron` under an unchanged `name` updates the running schedule in place;
  changing `name` replaces it. At most one inline schedule per automation may
  omit `name`, and names must be unique within the automation.
