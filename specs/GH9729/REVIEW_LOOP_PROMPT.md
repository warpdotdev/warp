# GH9729 review loop — Ralph prompt

This file documents the prompt used to drive the GH9729 review Ralph loop.
It is *also* the source of truth for the loop body — keep it in sync if you
ever need to re-launch.

## What the loop does each iteration

1. Open `specs/GH9729/IMPLEMENTATION_TODO.md` and locate the **Reviews**
   table at the bottom.
2. Find the **first row** where either `R1` or `R2` is still `[ ]`.
   - If every row has both columns `[x]`, output
     `<promise>ALL ITEMS REVIEWED</promise>` and exit.
3. For that single row, look up the commit SHA(s) in the table.
4. Run `git show --stat <sha>` and `git show <sha>` to see the full diff.
   For multi-commit rows (`FINAL`), do this for each SHA.
5. Identify which `tech.md` / `product.md` section the bullet references
   (the section anchor is in the matching bullet in the **Steps** list
   above the Reviews table). Read **only** that section, not the whole
   spec. Use the `Explore` subagent if you need broader codebase context.
6. **Spawn two reviewer agents in parallel** (single tool-use block, two
   `Agent` calls) using `subagent_type=general-purpose`:

   - **R1 — Correctness reviewer.** Lens: spec-fidelity, edge cases,
     error paths, security / DoS / resource caps. For decode/cache work
     specifically: decode bombs, SVG XXE / external entities / scripts,
     size caps, allocator limits, non-regular-file handling, async
     cancellation, error-string truncation. Output goes to
     `specs/GH9729/reviews/<item>-r1.md`.
   - **R2 — Quality reviewer.** Lens: idiomatic Rust, naming, structure,
     test rigor (are the bullet's stated test cases all present? negatives?
     determinism? brittle string-matching?), dead code, comment quality,
     module boundaries. Output goes to
     `specs/GH9729/reviews/<item>-r2.md`.

   Each agent must:
   - Read the diff, the referenced spec section, and the surrounding
     code as needed (Explore is fine).
   - Write its review file with this exact frontmatter:

     ```markdown
     ---
     item: <e.g. 1a>
     commit: <sha or comma-separated shas>
     reviewer: R1-correctness   # or R2-quality
     spec_ref: tech.md §<section>
     verdict: pass | pass-with-nits | concerns | blocking
     ---

     # Findings
     <bullet list of findings, each with severity tag: [nit] [minor] [major] [blocking]>

     # What I checked
     <bullet list of the things the reviewer actively verified — even if
     verdict is "pass", list them so the sign-off is auditable>

     # Suggestions
     <optional, actionable follow-ups>
     ```

   - Return a one-paragraph summary including the verdict.

7. Once **both** review files exist on disk and contain real content:
   - Tick the `R1` and `R2` boxes for that row in
     `specs/GH9729/IMPLEMENTATION_TODO.md`.
   - `git add specs/GH9729/IMPLEMENTATION_TODO.md specs/GH9729/reviews/`
   - Commit:
     `GH9729(review): <item> — R1 <verdict>, R2 <verdict>`
   - Stop. Do **not** start the next item.

## Hard rules

- One iteration = one row = one commit. Never batch multiple items.
- Touch only the review tracker and the two new review files. Do **not**
  modify implementation code, `tech.md`, `product.md`, or the **Steps**
  list above.
- Use the `Explore` subagent for codebase lookups; never grep from the
  main context window.
- Both reviewer agents are spawned in a **single** assistant turn (one
  `<function_calls>` block with two `Agent` invocations) so they run
  concurrently.
- If a reviewer comes back with a `blocking` verdict, still record it,
  still tick the box, still commit — Ralph's job is to *surface*
  findings, not to fix them. Fixes come later, off-loop.
- If `specs/GH9729/reviews/` does not yet exist, create it in the same
  commit as the first review.

## Launch command

```
/ralph-loop --max-iterations 20 --completion-promise 'ALL ITEMS REVIEWED' \
  Drive the GH9729 review loop per specs/GH9729/REVIEW_LOOP_PROMPT.md. \
  Read that file, do exactly one iteration as it describes, commit, stop.
```

(17 items × 1 iteration each = 17 iterations; cap of 20 is a safety net.)
