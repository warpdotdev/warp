# Revision 01 — plan v1 → v2

> ⚠️ **ERRATUM — see [`REVISION-02.md`](./REVISION-02.md) §2 and §5.**
> §4 below claims the "at most one NULL-`session_id` row" invariant cannot be expressed as a partial
> unique index. **That is wrong** — `CREATE UNIQUE INDEX … ON t (pane_uuid, agent) WHERE session_id
> IS NULL` enforces it directly, verified against SQLite. The v2 index model was also wrong to put
> `pane_uuid` in the task-identity index. Both are corrected in plan v3. The original text is left
> unedited so this record stays honest about what was believed at the time.

**Date:** 2026-08-03
**Scope:** [`plan.md`](./plan.md) (rewritten), [`brainstorm.md`](./brainstorm.md) (Cursor section
updated)
**Trigger:** external review of plan v1, findings F1–F5
**Outcome:** all five findings accepted; one reviewer note rejected as a misread; one new decision
(D6) opened and settled; plan rewritten to remove contradictions introduced by incremental patching.

---

## 1. Findings, and how each was verified

Every finding was re-verified independently against this checkout before being applied — the review
was not taken on trust.

| # | Finding | Verified how | Verdict |
|---|---|---|---|
| **F1** | `CLIAgent` has **16** variants, not 15 — `Unknown` was omitted. With AGENTS.md's no-wildcard rule, a match missing it will not compile | Counted variants at `cli_agent.rs:140-158` → 16 | ✅ accepted |
| **F2** | D1 option "use the configured default CLI agent" references a setting that **does not exist** | `grep` across `app/src/settings` + `cli_agent.rs` for `default_cli_agent` / `default_agent` → no results | ✅ accepted |
| **F3** | `UNIQUE (pane_uuid, agent)` bakes in a history depth of one per pane | Reasoning check against the insert-early/update-late design | ✅ accepted → new decision **D6** |
| **F4** | Phase 0's hit rate is narrower than claimed: `--continue` scopes to the *exact* cwd, but Phase 0 launches at the project root | Confirmed against Claude Code session docs already fetched | ✅ accepted |
| **F5** | Cursor row needs reframing: sessions are directory-scoped, a handler is feasible via fs-watch, `--resume` with no id opens a picker | Verified on disk — see §2 | ✅ accepted |

### Rejected

- **"`ProjectLayout` struct is `:47` not `:59`."** The citation was for `ProjectLayout::compute`,
  which *is* line 59; `:47` is the struct. Same pattern for `cli_agent.rs:151` (the `CursorCli`
  variant) vs `:174` (its `command_prefix` match arm) — both correct, different referents. Left as
  written, but the referent is now named explicitly so it doesn't recur.

### Accepted line-citation corrections

- `tab_config.rs` fields: **128-129**, not 118-127. (Was wrong.)
- `claude_command` hardcoding: **210-218**, widened from `:211`.

---

## 2. New verified evidence: Cursor's on-disk session store

Reproduced locally rather than accepted from the review:

```
$ printf '%s' "/Users/sam/Documents/dev/tools/warp" | md5
6c96dda913d423db404e9cab3e4672cf          # ← a real dir under ~/.cursor/chats

$ cat ~/.cursor/chats/<md5>/<uuid>/meta.json
{"schemaVersion":1,"createdAtMs":…,"hasConversation":true,"title":"…","updatedAtMs":…,"cwd":"…"}
```

Layout: `~/.cursor/chats/<md5(cwd)>/<chat-uuid>/{meta.json, store.db}`.

The schema is slightly richer than the review reported — `schemaVersion` and `hasConversation` are
also present. `schemaVersion` matters: it gives a cheap drift signal before trusting undocumented
internals.

**What this changes:**

1. Cursor sessions are **directory-scoped** (the md5-of-cwd key is direct evidence) — resolves an
   open question that v1 listed as unknown.
2. Instrumenting Cursor is a **filesystem-watch job, not blocked on upstream hooks**. v1 claimed it
   "depends on Cursor exposing hooks"; that was wrong.
3. Cursor has no `--session-id` equivalent → it is a **late-id agent** like Codex, and needs no new
   schema accommodation.
4. Cursor still ships **last** — undocumented internals, gated, failing soft.

---

## 3. Contradictions removed in the rewrite

Incremental patching of v1 left the document internally inconsistent. The rewrite fixed:

| Problem | Fix |
|---|---|
| A stale "**Which harness?** … (a) default to the user's configured CLI agent" paragraph survived directly contradicting F2's correction two sections later | Removed; Phase 0 now points at D1 |
| Bullets for "Notes that affect implementation" were orphaned *after* the inserted Cursor storage section, appearing to belong to its numbered list | Reordered — all notes precede the Cursor subsection |
| The schema block still declared `UNIQUE (pane_uuid, agent)` while the prose below recommended full history | Schema now declares the partial index; D6 records the decision |
| Duplicate/contradictory risk rows for Cursor ("speculative — storage undocumented" vs. the new verified row) | Merged into one |
| `resume_command(harness, id)` in Phase 2 vs. `resume_command(agent, id)` everywhere else | Unified on `agent` |
| "The record you asked for" — conversational residue from the chat thread | Removed |

---

## 4. Decision log

| # | Decision | Status | Where |
|---|---|---|---|
| D1 | Phase 0 agent selection | **open** — proposed: PATH-filtered submenu, then last-used-per-project | plan §Decisions |
| D2 | Two groups in the rail, not one merged list | settled v2 | plan §Phase 2 |
| D3 | ~20 handles/project, 30 days | settled v2 | plan §Decisions |
| D4 | v1 agents = Claude, then Codex + OpenCode | settled v2 | plan §Decisions |
| D5 | New table, not an extension | settled v1 | plan §Decisions |
| **D6** | **Full history per pane, not depth-1** | **new, settled v2** | plan §Phase 1 |

**D6 rationale.** Under `UNIQUE (pane_uuid, agent)`, a pane that runs `claude`, exits, and runs
`claude` again upserts over the first row — the earlier session remains resumable on disk but
silently disappears from the rail. Depth-1 would reduce the feature to "resume the last one," which
Phase 0 already delivers without a schema. Full history is what makes resuming a *specific* task
meaningful.

Its cost: the "at most one NULL-`session_id` row per `(pane_uuid, agent)`" invariant cannot be
expressed as a partial unique index and must be enforced by the write path. Covered by a test in the
plan's testing table.

---

## 5. Carried forward unchanged

These v1 conclusions survived review intact and were re-confirmed:

- Two-enum split; key the command table off `CLIAgent`, not `AIAgentHarness`.
- `terminal_panes.uuid` as the local identity key — and the reviewer added supporting evidence:
  `sqlite.rs:1253` writes `terminal_snapshot.uuid` back on save, so uuids survive restore.
- The SQLite NULL-distinct unique-index trap, and the partial-index remedy.
- Never emit `--dangerously-skip-permissions` in a user-facing command.
- Tab configs (`directory` + `commands`) as Phase 0's launch mechanism — and the reviewer resolved
  the one open verification item: `open_tab_config` (`workspace/view.rs:7141`) is the programmatic
  entry point, and a param-less config launches with no modal.

---

## 6. Still not started

No code has been written. Next step is D1, then Phase 2 mocks (required by
[#9382](https://github.com/warpdotdev/warp/issues/9382)'s `needs-mocks` label), then Phase 0.
