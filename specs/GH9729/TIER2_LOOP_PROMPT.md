# GH9729 Tier 2 review/impl loop — Ralph prompt

This file documents the prompt for the **fused** ralph-loop driving Tier 2
(UX polish) of the post-v1 follow-up list. It is also the source of truth
for the loop body — keep it in sync if you re-launch.

## What the loop does each iteration

1. Open `specs/GH9729/TIER2_TODO.md` and locate the **Tracker** table.
2. Find the first row where any of `Impl | R1 | R2` is `[ ]`.
   - If every row has all three columns `[x]` (or `—` for `t2-FINAL`),
     output `<promise>ALL TIER2 ITEMS DONE</promise>` and exit.
3. Resolve which column is unchecked first for that row, and act on
   exactly that column:

### Impl iteration

- Read the bullet for that item under **Steps** in `TIER2_TODO.md` and the
  referenced `tech.md` section. Use the `Explore` subagent for codebase
  context; do not grep from the main window.
- Implement the change. Keep the diff small (≤ ~150 lines where reasonable;
  split if larger by adding `Impl-a / Impl-b` sub-rows to the tracker).
- Run only the narrowest tests relevant to the change.
- Commit: `GH9729(tier2-impl): <item> — <one-line summary>`.
- Tick the `Impl` box and record the commit SHA in the `Impl commit`
  column. Stop.

### R1 iteration (correctness)

- Look up the impl commit SHA in the tracker. Run `git show --stat <sha>`
  and `git show <sha>`.
- Spawn one `general-purpose` reviewer agent with this lens: spec-fidelity
  vs `tech.md` §688-713, error paths, edge cases, security/DoS where
  relevant (decode bombs, allocator caps, async cancellation).
- The agent writes `specs/GH9729/reviews/tier2-<item>-r1.md` with this
  frontmatter:

  ```markdown
  ---
  item: tier2-<item>
  commit: <sha>
  reviewer: R1-correctness
  spec_ref: tech.md §<section>
  verdict: pass | pass-with-nits | concerns | blocking
  ---

  # Findings
  <bullet list, each finding tagged [nit] [minor] [major] [blocking]>

  # What I checked
  <bullet list>

  # Suggestions
  <optional, actionable follow-ups>
  ```

- Commit: `GH9729(tier2-review): <item> R1 — <verdict>`.
- Tick the `R1` box. Stop.

### R2 iteration (quality)

- Identical to R1 but `reviewer: R2-quality`, output suffix `-r2.md`,
  lens: idiomatic Rust, naming, structure, test rigor (negatives,
  determinism, brittle string-matching), dead code, comment quality,
  module boundaries.
- Commit: `GH9729(tier2-review): <item> R2 — <verdict>`.

## Hard rules

- One iteration = one commit. Never batch multiple boxes.
- Touch only the files the current iteration requires.
- Use the `Explore` subagent for codebase lookups; never grep from the
  main context window.
- If a reviewer comes back with a `blocking` verdict, still record it,
  still tick the box, still commit — Ralph's job is to surface findings,
  not to fix them. Fixes come later, off-loop, by adding a follow-up row
  to the tracker.
- If the impl agent hits a design gap that `tech.md` does not resolve,
  mark the row's `Impl` cell `[blocked]` (not `[x]`) with a one-line
  reason in the row, do not commit code, and skip to the next row's
  `Impl` slot.

## Launch command

```bash
/ralph-loop --max-iterations 24 --completion-promise 'ALL TIER2 ITEMS DONE' \
  Drive the GH9729 Tier 2 loop per specs/GH9729/TIER2_LOOP_PROMPT.md. \
  Read that file, do exactly one iteration as it describes, commit, stop.
```

(7 items × 3 boxes = 21 iterations max; cap of 24 is a safety net. The
`t2-FINAL` row only has the `Impl` box.)
