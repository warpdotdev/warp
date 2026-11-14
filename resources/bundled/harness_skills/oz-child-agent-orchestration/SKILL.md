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
Use messages whenever you need to communicate with the lead agent while work is still in progress. If you cannot continue until an expected parent reply arrives, use the `wait-for-events` skill after checking your inbox. When your assigned work is complete or you are stopping with a blocker rather than awaiting that reply, use the `oz-finish-task` skill to send the outcome and update task status.

## Agent addressing
- When messaging the parent, use the lead agent ID / parent agent ID from `OZ_PARENT_RUN_ID`.
- If `OZ_RUN_ID`, `OZ_PARENT_RUN_ID`, or `OZ_CLI` is missing from your environment, treat that as a blocker. Do not guess one; wait for corrected context before attempting to message the parent.

## Messaging
IMPORTANT: Use `"$OZ_CLI" run message send --sender-run-id "$OZ_RUN_ID" --to "$OZ_PARENT_RUN_ID" --subject "<subject>" --body "<body>"` whenever you need to send a message while work is still in progress. This does not update the task status, leaving it in progress.
- When messaging the parent, use the lead agent ID / parent agent ID from `OZ_PARENT_RUN_ID`.
- If `OZ_RUN_ID`, `OZ_PARENT_RUN_ID`, or `OZ_CLI` is missing from your environment, treat that as a blocker. Do not guess one; wait for corrected context before attempting to message the parent.
- Sending a message this way is not a terminal action. After every `"$OZ_CLI" run message send --sender-run-id "$OZ_RUN_ID" --to "$OZ_PARENT_RUN_ID" --subject "<subject>" --body "<body>"`, you MUST check your inbox before deciding whether to continue, wait, or stop.

You should send a message to the lead agent's ID in `OZ_PARENT_RUN_ID` immediately once you have started work, briefly describing your approach. Afterwards, send as needed while continuing work, such as if you were asked to share preliminary findings, or if you need to ask questions.

If your assigned work is complete or you are stopping with a blocker rather than waiting for an expected parent response, do not send your outcome to the parent this way. You MUST use the `oz-finish-task` skill, which sends the outcome and updates task status. If you have instead asked your parent a question and expect its answer before you can continue, send the question, check your inbox, then follow `wait-for-events`. Never wait after completing the assigned task.

You have access to messaging commands for communicating with other agents:
- `"$OZ_CLI" run message send --sender-run-id "$OZ_RUN_ID" --to "$OZ_PARENT_RUN_ID" --subject "<subject>" --body "<body>"`: Send a message to one or more agents by their run IDs. Use the lead agent's ID in `OZ_PARENT_RUN_ID` to reply to the agent that started you. `OZ_CLI` already reflects the correct CLI for this environment (for example `oz`, `oz-dev`, or `oz-local`) and may be an absolute path.
- `"$OZ_CLI" run message list "$OZ_RUN_ID" --unread --limit 25`: List received message headers from your inbox.
- `"$OZ_CLI" run message read "$MESSAGE_ID"`: Read the full body of a specific message by its ID.
- `"$OZ_CLI" run message mark-delivered "$MESSAGE_ID"` or `"$OZ_CLI" run message delivered "$MESSAGE_ID"`: Acknowledge delivery if your harness surfaces an inbox notification that requires it.
Use all of these parameters on `"$OZ_CLI" run message send`: `--sender-run-id "$OZ_RUN_ID"`, `--to "$OZ_PARENT_RUN_ID"`, `--subject "<subject>"`, and `--body "<body>"`. If `OZ_RUN_ID`, `OZ_PARENT_RUN_ID`, or `OZ_CLI` is missing, that is a blocker.

### Checking for messages
Your harness will surface messages for you automatically, but you may also check for messages proactively by running
`"$OZ_CLI" run message list "$OZ_RUN_ID" --unread --limit 25`.

Omit the `--unread` flag if you expect new messages to exist but don't see them - it's possible they were mistakenly marked as read even if you didn't actually read them.

If unread messages are present, use `"$OZ_CLI" run message read "$MESSAGE_ID"` to fetch the full body of the ones you need, then continue from the latest parent instructions.

It's a good idea to check your inbox at these times:
- you just sent a message
- before starting a new chunk of work after any pause,
- before deciding you still need to wait for parent input,
- before saying you are done, waiting, standing by, or otherwise ending your turn.

Do NOT start a background poll checking for messages. Only check as needed.
