# Revision 02 — plan v2 → v3

**Date:** 2026-08-03
**Trigger:** NO-GO review of plan v2 — three implementation-blocking contradictions plus two
readiness gaps
**Scope:** [`plan.md`](./plan.md) rewritten as v3; erratum filed against
[`REVISION-01.md`](./REVISION-01.md)
**Outcome:** all findings accepted, all verified independently. Four scope decisions taken by
interview. Phase 0 dropped; four new decisions recorded (D7–D10).

---

## 1. Findings — all confirmed, one of them against my own prior claim

| # | Finding | Verification | Verdict |
|---|---|---|---|
| **B1** | `ProjectKey` cannot safely be persisted at write time: `for_path` may return `LocalDir` then upgrade to `LocalGit` after repo detection; and `ProjectKey` has no serialization contract | Read `project_key.rs:20` → `#[derive(Debug, Clone, PartialEq, Eq, Hash)]` — no `Serialize`. Upgrade behaviour documented in the file's own comment at `:58-60` | ✅ accepted |
| **B2a** | "A partial index cannot enforce one NULL row" is **false** | Ran it against SQLite — see §2 | ✅ accepted; **my error** |
| **B2b** | Including `pane_uuid` in the task index lets one upstream session duplicate across panes | Same test: `(agent, session_id) WHERE NOT NULL` correctly rejects the cross-pane duplicate | ✅ accepted |
| **B3** | Phase 1 and Phase 3 overlap populations — Warp-driven runs would appear as both a handle and a conversation, contradicting D5 | Read back v2 §Phase 1 write paths vs §Phase 3 | ✅ accepted |
| **B4** | `resume_command` interpolates externally-discovered ids into shell commands with no validation or quoting policy | `grep` for `Uuid::parse_str` / `validate` in `cli_agent_sessions` → **no results**; ids are unvalidated | ✅ accepted |
| **B5** | D1 sequencing is circular: "Phase 0 → D1" while Phase 0's completion criterion hardcodes Claude | Read v2 §Sequencing vs §Phase 0 "Done when" | ✅ accepted |
| **B6** | "Everything else inserts a row that stays NULL-id" is inaccurate — unsupported agents have no handler, so no event and no row | `create_handler` returns `None` (`listener/mod.rs:70-78`) | ✅ accepted |

### §2 — The index test, run rather than argued

```sql
CREATE UNIQUE INDEX i_null ON h(pane, agent) WHERE session_id IS NULL;
CREATE UNIQUE INDEX i_id   ON h(agent, session_id) WHERE session_id IS NOT NULL;
```

| Attempt | Result |
|---|---|
| Second NULL row, same `(pane, agent)` | `UNIQUE constraint failed: h.pane, h.agent` |
| NULL row, different pane | accepted |
| Same `session_id` from a different pane | `UNIQUE constraint failed: h.agent, h.session_id` |

So the NULL invariant **is** indexable, and the task identity **must not** include `pane_uuid`.
v2 asserted the opposite on both counts.

### Evidence the review asked for: is `(agent, session_id)` unique without cwd scoping?

Yes for every agent in scope — all ids are UUIDs. Warp mints Claude's via `Uuid::new_v4()`; Codex's
rollout id is a `Uuid`; Cursor's chat directories are UUIDs (observed:
`998add3f-6339-423a-9ea9-2fc6e2d3fe9b`). An agent with only locally-unique ids would need a
cwd-scoped index — noted in the plan as a check before adding one.

---

## 3. Scope decisions taken by interview

| Question | Decision | Consequence |
|---|---|---|
| Handles table ownership | **CLI-launched only** | Resolves B3 outright. Agent-Mode drivers are untouched by Phase A; no cross-population dedup exists to define. D5's rationale becomes true rather than aspirational |
| Keep the zero-storage Phase 0? | **Dropped** | Resolves B5 by deletion — D1 existed only to pick Phase 0's agent. Also removes the silent sub-directory miss |
| Click behaviour | **Prefill, don't execute** | Materially reduces B4's blast radius, and makes the command visible before it runs |
| Agent picker with no handle | Submenu filtered by resume-support **and** `PATH` | Survives as the pattern for "start a new agent here"; no new setting needed |

---

## 4. What changed in the plan

- **Phase 0 deleted.** Phases renamed A/B/C to avoid confusion with v2's numbering.
- **`project_key` removed from the schema.** `cwd` is now canonical and the project is resolved at
  **read time** through one shared memoized `HashMap<cwd, ProjectId>` built once per refresh. This
  kills B1 at the root: nothing stale is stored, no versioned representation is needed, and
  `ProjectKey` needs no `Serialize`. Phase C uses the same helper, so the two populations cannot
  disagree about a task's project.
- **Index model corrected** to the two partial indexes above, with a documented merge rule for
  resuming a known session in a new pane (refresh the task row, drop the in-flight row).
- **`pane_uuid` demoted** from identity to provenance + in-flight slot key.
- **New "Command safety" section** — validate on ingest (`[A-Za-z0-9_-]{1,64}`), reject control
  characters, quote at the boundary, mandatory adversarial tests. Includes the non-obvious point
  that *prefill is not self-securing*: a newline inside an id would submit the line on arrival.
- **B6 sentence corrected** — unsupported agents produce no rows at all.
- **Testing table extended** with the injection suite, the cross-pane merge case, and a
  `LocalDir`→`LocalGit` bucketing test.
- **New decisions D7–D10** recorded; **D1 retired**.

---

## 5. Erratum against REVISION-01

REVISION-01 §4 repeats v2's incorrect claim that the one-NULL-row invariant "cannot be expressed as
a partial unique index and must be upheld by the write path." That is wrong — see §2. A pointer has
been added to REVISION-01; the original text is left in place so the trail stays honest about what
was believed at the time.

The write path still owns the *merge* rule (resuming a known session in a new pane), which is
genuine application logic, not an index constraint.

---

## 6. Status

Decisions are settled and the identity model is empirically grounded. Remaining before code:
Phase B mocks, required by [#9382](https://github.com/warpdotdev/warp/issues/9382)'s `needs-mocks`
label. Command-safety work ships with Phase A, not after it.

No code has been written.
