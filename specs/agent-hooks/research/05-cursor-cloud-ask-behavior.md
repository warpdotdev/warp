# Cursor hook `permission: "ask"` in unattended contexts

**Answer.** Cursor’s public docs do not say whether native hook `permission: "ask"` produces an approval request, denial, execution, or a paused/failed run in managed Cloud Agents, Self-Hosted Machines, or `agent -p`. They do say Cloud Agents auto-run terminal commands, managed cloud does not support `beforeMCPExecution`, and `preToolUse`’s `ask` is not enforced. CLI auto-allow flags are documented, but their effect on hook `ask` is not. Community reports concern local IDE modes; I found no direct test of Cursor’s cloud/headless hook outcome.

## 1. Cursor-managed Cloud Agents

Cloud Agents load repository hooks; Enterprise teams can also distribute team and managed hooks. The support table includes `beforeShellExecution` and `preToolUse`, excludes `beforeMCPExecution`, and says hooks do not run in early read-only turns. ([Hooks](https://cursor.com/docs/hooks))

The Security overview says: “Cloud Agents auto-run terminal commands so they can iterate on tests without stopping for approval on every step.” ([Security overview](https://cursor.com/docs/cloud-agent/security))

**What docs do not say.** For cloud `beforeShellExecution: ask`, docs do not say whether an approval appears in the dashboard, Slack, GitHub, or mobile, or whether the command proceeds, is denied, or leaves the run waiting. A separate setting requires approval in a Project chat before a new machine/pool is used; that is placement approval, not command approval. The mobile release note says, “Get push notifications when an agent finishes, needs input, or is ready for review,” but does not tie this to hook `ask`. ([Cloud Agents settings](https://cursor.com/docs/cloud-agent/settings); [3.9 changelog](https://cursor.com/changelog/page/4))

## 2. Self-Hosted Machines and Pools

Self-Hosted Machines keeps the agent loop in Cursor’s cloud and runs tool calls on the worker. My Machines and Pools load command-based project hooks. The Pool page says, “The Cloud Agent limits that also apply on these tool-execution workers,” then lists command-based hooks only and no IDE-only hooks; `sessionStart`/`sessionEnd` run on workers. The general Cloud Agent security page says terminal commands auto-run but does not specify how hook `ask` changes that on a worker. ([Self-Hosted Machines](https://cursor.com/docs/cloud-agent/self-hosted); [My Machines](https://cursor.com/docs/cloud-agent/self-hosted/my-machines); [Team Pools, “Hooks”](https://cursor.com/docs/cloud-agent/self-hosted/pool#hooks); [Security overview](https://cursor.com/docs/cloud-agent/security))

**What docs do not say.** There is no self-hosted `ask` outcome or approval-surface/deny/pause/failure description. The Hooks matrix excludes `beforeMCPExecution` for managed cloud; Pool docs point to those Cloud Agent limits but do not restate that event’s status for workers. ([Hooks](https://cursor.com/docs/hooks); [Team Pools, “Hooks”](https://cursor.com/docs/cloud-agent/self-hosted/pool#hooks))

## 3. Cursor CLI `agent -p` / `--print`

`-p, --print` is documented for non-interactive use. `--force`/`--yolo` force-allow commands unless explicitly denied; `--approve-mcps` auto-approves MCP servers; `--trust` skips workspace trust. The headless guide says file changes are only proposed without `--force`. ([CLI parameters](https://cursor.com/docs/cli/reference/parameters); [Using Headless CLI](https://cursor.com/docs/cli/headless))

The interactive CLI guide says it asks for `y`/`n` before terminal commands, but docs do not say what a hook `ask` does in `-p`, whether it blocks, or whether `--force` overrides it. The CLI pages also differ on writes: one says print mode has “full write access,” while the headless guide says changes require `--force`. ([Using Agent in CLI](https://cursor.com/docs/cli/using); [Using Headless CLI](https://cursor.com/docs/cli/headless))

## 4. Exact `ask` wording in the Hooks reference

For `beforeShellExecution` and `beforeMCPExecution`, the reference says: “Called before any shell command or MCP tool is executed. Return a permission decision.” The output schema is:

> `"permission": "allow" | "deny" | "ask",`  
> `"user_message": "<message shown in client>",`  
> `"agent_message": "<message sent to agent>"`

The page does not define the unattended effect of this schema. Its exact caveats are:

> `"ask" is accepted by the schema but not enforced for preToolUse today.`
> `"ask" is not supported for subagentStart and is treated as "deny".`

It also says “any `deny` wins over `ask`, and `ask` wins over `allow`” when responses merge. No Auto-Run-mode rule appears in this hook contract. ([Hooks](https://cursor.com/docs/hooks))

## 5. Changelogs and release notes

No changelog entry found defines hook `ask` in cloud, self-hosted, or print mode. Related entries say “Users who choose to ‘Run Everything’ will never have their agents blocked,” describe `--yolo`/`--force` auto-run and MCP-confirmation flags, and add Cloud Agent lifecycle hooks; none specifies hook `ask`. The current CLI changelog covers Auto-review approval for risky deletes, not hook `ask`. ([2.4 changelog](https://cursor.com/changelog/2-4); [Feb. 18 CLI changelog](https://cursor.com/changelog/cli-feb-18-2026); [3.11 changelog](https://cursor.com/changelog/side-chat); [CLI changelog](https://cursor.com/docs/cli/changelog))

## 6. Community reports (not product documentation)

- **Forum, local IDE:** Reports vary by version/mode. In Feb. 2026, staff said “Right now, only `deny` and `allow` have a real effect.” In Oct. 2026, staff said: “You’re right: in Auto-review, when Auto-review itself decides a command is safe to run, the hook’s ‘ask’ answer currently isn’t applied, so no approval prompt appears.” Neither report tests Cloud Agents. ([Feb. thread](https://forum.cursor.com/t/hook-return-value-ask-has-no-practical-effect/152626); [Oct. thread](https://forum.cursor.com/t/beforeshellexecution-hook-returning-ask-is-ignored-in-auto-review-mode-honoured-in-allowlist-and-run-everything-3-21-16/173694))
- **Forum, headless CLI:** Staff says: “In headless/print mode without `--force`, every command that isn’t on the allowlist is silently denied (no prompt, the agent sees the failure and reacts).” This describes command approvals generally, not hook `ask`. A separate report says, “In print mode there’s no one to answer, so the CLI automatically rejects `AskQuestion`”; that is a different tool, not hook `permission: "ask"`. ([CLI permissions](https://forum.cursor.com/t/disable-permissions-ask-in-sandbox/157243); [AskQuestion](https://forum.cursor.com/t/cursor-agent-p-askquestion-is-auto-skipped-in-headless-mode-and-only-some-models-get-the-tool/173657))
- **GitHub community workaround:** One user-built hook says: “Cloud agents pick up project `.cursor/hooks.json`, but there is no local dialog there — an ask match will deny.” This is that wrapper’s behavior, not evidence Cursor maps native hook `ask` to deny. ([Gist README](https://gist.github.com/technusm1/fb8bf3591439a41caa8d9db5c059e851))

I found no direct forum or GitHub report that tests the Cursor host’s final disposition for native hook `ask` in a Cursor-managed Cloud Agent or Self-Hosted run.

## Comparisons

**Claude Code.** `PreToolUse` `ask` “prompts the user to confirm.” The hook reference says: “When Claude Code denies the call in a `-p` run where no one can answer that prompt, Claude reads the reason in the tool result instead.” An SDK callback or `--permission-prompt-tool` can answer it. ([Hooks reference](https://code.claude.com/docs/en/hooks); [Headless mode](https://code.claude.com/docs/en/headless))

**Codex.** Its Hooks guide says: “`permissionDecision: "ask"`, legacy `decision: "approve"`, `continue: false`, `stopReason`, and `suppressOutput` are parsed but not supported yet. Codex marks the hook run as failed, reports the error, and continues the tool call.” The separate `PermissionRequest` hook accepts `allow`/`deny` or no decision, which leaves normal approval flow. `codex exec` is documented as non-interactive and read-only by default; docs do not say how an unresolved normal approval is surfaced without a permission host. ([Codex Hooks](https://developers.openai.com/codex/hooks); [Non-interactive mode](https://developers.openai.com/codex/noninteractive); [generated schema](https://github.com/openai/codex/blob/main/codex-rs/hooks/schema/generated/pre-tool-use.command.output.schema.json))
