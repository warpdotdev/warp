---
name: oz-child-agent-orchestration
description: Coordinate with the parent of an Oz cloud child run. Read before starting delegated work, and when checking messages, reporting a blocker, or handing off results.
compatibility: Requires Bash and jq in the cloud sandbox.
---
# Child coordination
Use the CLI in `OZ_CLI`, your current run ID in `OZ_RUN_ID`, and your parent's run ID in `OZ_PARENT_RUN_ID`. Do not guess missing IDs. If any is absent, report the missing coordination context instead of sending to an inferred recipient.

## Receive instructions
Before substantive work, between milestones, and immediately before finishing or waiting, list unread messages:

```sh
"$OZ_CLI" run message list "$OZ_RUN_ID" --unread
```

For each returned message ID, read its body, then mark it delivered only after reading it:

```sh
"$OZ_CLI" run message read <message-id>
"$OZ_CLI" run message mark-delivered <message-id>
```

Continue from the latest parent instructions. To wait, run this Bash recipe. It returns after one notification or a timeout, stopping and reaping the watcher in either case. Set `wait_seconds` below your command tool's execution limit (default: 300 seconds). Keep `last_sequence` from the previous event, or start at zero:

```bash
last_sequence="${last_sequence:-0}"
wait_seconds="${wait_seconds:-300}"
if event=$(
  wait_dir=$(mktemp -d) || exit 1
  watch_pid=
  cleanup() {
    if [ -n "$watch_pid" ]; then
      kill "$watch_pid" 2>/dev/null || true
      wait "$watch_pid" 2>/dev/null || true
    fi
    rm -rf -- "$wait_dir"
  }
  trap cleanup EXIT
  trap 'exit 130' INT
  trap 'exit 143' TERM
  mkfifo "$wait_dir/events" || exit 1
  "$OZ_CLI" run message watch "$OZ_RUN_ID" \
    --since-sequence "$last_sequence" --output-format ndjson > "$wait_dir/events" &
  watch_pid=$!
  if IFS= read -r -t "$wait_seconds" event < "$wait_dir/events"; then
    printf '%s\n' "$event"
  else
    read_status=$?
    if [ "$read_status" -gt 128 ]; then exit 124; fi
    exit 1
  fi
); then
  last_sequence=$(jq -er '.sequence' <<< "$event")
else
  wait_status=$?
  if [ "$wait_status" -ne 124 ]; then
    printf 'Message watch failed (status %s).\n' "$wait_status" >&2
  fi
fi
"$OZ_CLI" run message list "$OZ_RUN_ID" --unread
```

`event` holds the notification JSON and `last_sequence` is the reconnect cursor. After the recipe lists unread headers, read and mark each message delivered as above before resuming delegated work. On timeout, retain the cursor, drain the inbox, then wait again only if still blocked; do not send repeated status messages or claim completion while waiting. On a watch error, report the failure to the parent. The stream is a notification, not a substitute for reading the full body.

## Report and hand off
Send blockers and questions to the parent before stopping, and report material results without narrating every command:

```sh
"$OZ_CLI" run message send --sender-run-id "$OZ_RUN_ID" \
  --to "$OZ_PARENT_RUN_ID" --subject '<subject>' --body '<message>'
```

Include exact plan IDs when referencing plans. Use plan content supplied by the parent when document tools are unavailable. You are in an isolated cloud sandbox: use the parent's requested handoff and push durable branches or draft PRs for code changes, reporting their references and validation results. Do not leave important work only on the sandbox filesystem.

When complete, perform the final unread-message check, then use `oz-finish-task` as your last action. Its summary is the completion handoff to the parent; include findings or deliverables themselves, not merely a description of having produced them. Do not claim completion while blocked waiting for an answer.
