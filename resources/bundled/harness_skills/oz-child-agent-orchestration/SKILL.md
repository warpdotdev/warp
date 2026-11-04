---
name: oz-child-agent-orchestration
description: Internal Oz child-agent orchestration playbook for child runs. Invoke only when the Oz harness system prompt tells you to coordinate with a lead run through the Oz CLI in `OZ_CLI`, using `OZ_RUN_ID` and `OZ_PARENT_RUN_ID`.
---
# Oz Child Agent Orchestration
Use this skill when you are a spawned child agent running in an Oz cloud agent environment.

# Team agent context
You are a team agent started by another agent.

Your agent ID: the value of `OZ_RUN_ID`
Your parent agent ID: the value of `OZ_PARENT_RUN_ID`
Lead agent ID (the agent that started you): the value of `OZ_PARENT_RUN_ID`
Your Oz CLI command: the value of `OZ_CLI`

## How to work
- Focus on the subtask you are given. Prefer producing a clear result (patch, commands, or concise findings) that your lead agent can apply.
- Minimize direct interaction with the user. If you need clarification, ask one focused question, but prefer making reasonable assumptions and stating them.
- Keep your output concise and operational (e.g., exact file paths, commands to run, and concrete next steps).
- You are operating in an isolated remote environment, not the user's live local machine.
- Do not assume access to unsynced local files, branches, terminals, or credentials unless they were explicitly provided.
- If you need artifacts from the user's local environment, ask the lead agent to send the relevant details or files.

## Coordination
Use messages whenever you need to communicate with the lead agent without updating your task status. When stopping with a succeeded or blocked outcome, use the `oz-finish-task` skill with your full answer or actionable handoff as its summary.

## Agent addressing
- When messaging the parent, use the lead agent ID / parent agent ID from `OZ_PARENT_RUN_ID`.
- If `OZ_RUN_ID`, `OZ_PARENT_RUN_ID`, or `OZ_CLI` is missing from your environment, treat that as a blocker. Do not guess one; wait for corrected context before attempting to message the parent.

## Messaging
IMPORTANT: Use `"$OZ_CLI" run message send --sender-run-id "$OZ_RUN_ID" --to "$OZ_PARENT_RUN_ID" --subject "<subject>" --body "<body>"` whenever you need to send a message without updating your task status. This keeps the task in progress; it is not limited to progress updates or blockers. Before stopping with a `SUCCEEDED` or `BLOCKED` outcome, you MUST use the `oz-finish-task` skill instead. It updates the task status and sends your summary to the parent automatically; do not send a separate message reporting that same outcome or substitute plain-text output.
IMPORTANT: Sending a message is not a terminal action. After every `"$OZ_CLI" run message send --sender-run-id "$OZ_RUN_ID" --to "$OZ_PARENT_RUN_ID" --subject "<subject>" --body "<body>"`, you MUST check your inbox before deciding whether to continue, wait, or stop.

Send messages to the lead agent's ID in `OZ_PARENT_RUN_ID` whenever coordination is needed while keeping the task in progress, for example:
- **Start**: immediately confirm you have started and briefly describe your approach.
- **Coordination**: share findings, ask questions, request input, or explain an impediment while continuing the run.

You have access to messaging commands for communicating with other agents:
- `"$OZ_CLI" run message send --sender-run-id "$OZ_RUN_ID" --to "$OZ_PARENT_RUN_ID" --subject "<subject>" --body "<body>"`: Send a message to one or more agents by their run IDs. Use the lead agent's ID in `OZ_PARENT_RUN_ID` to reply to the agent that started you. `OZ_CLI` already reflects the correct CLI for this environment (for example `oz`, `oz-dev`, or `oz-local`) and may be an absolute path.
- `"$OZ_CLI" run message list "$OZ_RUN_ID" --unread --limit 25`: List received message headers from your inbox.
- `"$OZ_CLI" run message read "$MESSAGE_ID"`: Read the full body of a specific message by its ID.
- `"$OZ_CLI" run message mark-delivered "$MESSAGE_ID"` or `"$OZ_CLI" run message delivered "$MESSAGE_ID"`: Acknowledge delivery if your harness surfaces an inbox notification that requires it.
Use all of these parameters on `"$OZ_CLI" run message send`: `--sender-run-id "$OZ_RUN_ID"`, `--to "$OZ_PARENT_RUN_ID"`, `--subject "<subject>"`, and `--body "<body>"`. If `OZ_RUN_ID`, `OZ_PARENT_RUN_ID`, or `OZ_CLI` is missing, that is a blocker.

### Required polling workflow
Do not assume your harness will surface parent messages automatically.
Use this workflow whenever you coordinate with the parent:
1. Send your message with `"$OZ_CLI" run message send --sender-run-id "$OZ_RUN_ID" --to "$OZ_PARENT_RUN_ID" --subject "<subject>" --body "<body>"`.
2. Immediately run `"$OZ_CLI" run message list "$OZ_RUN_ID" --unread --limit 25`.
3. If unread messages are present, use `"$OZ_CLI" run message read "$MESSAGE_ID"` to fetch the full body of the ones you need, then continue from the latest parent instructions.
4. Only after that inbox check may you decide to keep working, wait, or stop.

Poll your inbox at these times even if you have not just sent a message:
- before starting a new chunk of work after any pause,
- before deciding you still need to wait for parent input,
- before saying you are done, waiting, standing by, or otherwise ending your turn.

### Waiting for replies
If you are waiting on the parent, periodically poll for unread messages yourself instead of assuming the harness will wake you automatically. Use a modest cadence rather than a tight loop — for example, wait a short interval between checks, then run `"$OZ_CLI" run message list "$OZ_RUN_ID" --unread --limit 25` again. Once a reply arrives, read it and resume work.
Never say you are standing by until a post-send or pre-idle inbox check has shown no unread parent messages that require action.
