# Rail search: type-to-filter over projects, tasks, and transcript content

**Status:** v1 — spec merged from three designer proposals 2026-08-05; five open questions for Sam
in §10. Not implemented.
**Grounding:** three investigations (Orbit's `session-search.ts` mechanism report; Warp's existing
search infrastructure; rail plumbing + measured corpus), plus tech-lead spot-checks of the
load-bearing claims against this checkout (see §9 — two claims corrected, one doc bug found).
**Related:** [`../resume-project-task/plan.md`](../resume-project-task/plan.md) (the rail),
[`../rail-triage/plan.md`](../rail-triage/plan.md) (colors, chips, ordering contract),
[`../lazy-shell-startup/plan.md`](../lazy-shell-startup/plan.md).

---

## 1. Problem

The rail lists every project and every task, and the list keeps growing. Sam wants one field that
answers three questions at three different costs:

1. **Which project was that?** — match project names (and paths). Free: already in memory.
2. **Which task was that?** — match task/conversation names. Free: already in memory.
3. **Which session was the one where we talked about X?** — match the *content* of coding-agent
   transcripts. Not free: the corpus is ~1.5 GB of rail-relevant JSONL, and a whole-tree `rg` is
   ~1.45 s warm.

The rail already has a filter idiom (`rail_shells.rs`), an ordering contract that forbids
reordering, and three composition hazards a second filter can trip. This spec picks one design,
resolves the three real disagreements between the proposals, and phases the work so (1) and (2)
ship alone and (3) never blocks them.

## 2. Judgment: which design is the spine

**LOWEST-RISK is the spine.** Not for caution — because it is the only proposal whose read of
`crates/warp_ripgrep` survives contact with the code.

`search_streaming(patterns, paths, ignore_case, multiline)` (`crates/warp_ripgrep/src/search.rs:170-178`)
takes no glob parameter and no heap-limit parameter. `SEARCHER_LINE_HEAP_LIMIT = 64 * 1024`
(`search.rs:16`) is a private const applied inside the searcher build (`search.rs:83`), and
`WalkBuilder::new(...)` is used with defaults (`search.rs:66-70`) — which skip hidden directories,
and the corpus lives in `~/.claude`. SHIP-SMALL's "three mandatory fixes to `warp_ripgrep`" and
MOST-CAPABLE's phase-4 equivalent therefore mean **editing a crate that `global_search` ships on,
to serve a rail feature**. That is the largest blast radius of the three designs, and neither
prices it. LOWEST-RISK's refusal — enumerate the files ourselves, read them ourselves, keep the
blast radius inside the rail — is the correct reading.

Its second advantage is a mechanism neither sibling has: a digest cache keyed
`(PathBuf, len, SystemTime)`, where the `len` half makes growth **append-only** — a live transcript
that grew re-reads only `[old_len, EOF)`, a seek-and-read that `read_tail`
(`transcript_naming.rs:103-112`) already proves as an idiom in this corpus. That is free
incrementality on the one corpus that is genuinely append-only.

**MOST-CAPABLE is the runner-up**, and loses on one self-admitted point: it persists the digest to
disk, reintroducing the exact staleness failure mode Orbit refused on purpose
(`session-search.ts:9-15`). Nothing in the measurements justifies the cost — a per-project digest
is ≤4 MB and sub-second to rebuild.

**SHIP-SMALL is third**, but not wrong: its phasing instinct is the one all three converge on, and
its Enter-triggered escape hatch is grafted below.

### Grafted from the runners-up

| Graft | From | Why |
|---|---|---|
| Digest covers **user turns + assistant prose**, not prompts-only | MOST-CAPABLE | LOWEST-RISK's prompts-only cannot find "the error the agent printed", which is a large share of real queries. Excluding tool-result bodies and pasted blobs still drops the bulk of the bytes. |
| Incompleteness readout — `indexed 180/281 sessions` | MOST-CAPABLE | The section must never imply completeness while the digest is building. |
| Enter-triggered full-fidelity "search all transcripts in \<project\>" | SHIP-SMALL | The escape hatch for the digest's deliberate lossiness. This — and only this — is where reopening the `warp_ripgrep` question is justified, because it is one-shot and user-invoked. |
| Honesty labels in the section header ("this device only", "16 most recent per project") | SHIP-SMALL + Orbit `:373` | Transcripts are per-host and the scan is capped; a silent partial result reads as a bug. |

### Rejected outright

- **MOST-CAPABLE's "Past sessions" section as a second surface.** Its own weakness list concedes it
  is strictly more UI code, none of it reusable, and it duplicates row activation and re-litigates
  triage's color precedence. Content hits attach to existing rows instead (§4).
- **MOST-CAPABLE's persisted digest.** See above.
- **Orbit's anchor planner + coverage floor, for v2.** Orbit's own single-token short-circuit
  (`session-search.ts:103`) means `POA-2236` behaves identically under a literal
  `memchr::memmem` scan. The planner buys nothing until pasted blobs are in scope — defer it to the
  escape-hatch phase alongside `history.jsonl`.
- **The subagent→parent fold, for v2.** If we enumerate only top-level `<uuid>.jsonl` ourselves,
  subagent files are never read, so there is nothing to fold. It becomes necessary only when a
  recursive walk enters the picture (P4).
- **`SearchMixer` / `QueryFilter::Transcripts` registration.** Real leverage — it would give
  transcript search to Cmd-P, filter atoms, `priority_tier` grouping and the result renderer at
  once (`mixer.rs:143,169`; `data_source.rs:139-244`; `async_snapshot_data_source.rs:23-51`, all
  verified to exist). Deliberately not taken: that machinery renders a palette dropdown, and the
  thing a rail is *for* is in-place spatial filtering. This is a trade of leverage for fit, and it
  is the single most reversible decision here — a P4 mixer source can reuse the digest verbatim.

## 3. What already exists

| Piece | Where | State |
|---|---|---|
| Rail pure-filter idiom (plain data, no `AppContext`, tests beside it) | `workspace/rail_shells.rs:78-107`, tests via `#[path]` at `:120-122` | exists — the template to copy |
| The "hidden N" subtraction that a second filter must not corrupt | `rail_shells.rs:104` — `hidden_shells: rows.len() - visible.len()` | exists, **verified**; see §5 |
| Exemptions: active tab never hidden; selected project keeps a fallback row | `rail_shells.rs:88`, `:95-101` | exists |
| Header filter button routed through the Appearance page's own toggle | `workspace/view.rs:23864-23886`, gated by `if *tab_settings.rail_show_tasks` | exists — **the `rail_show_tasks` gate is undocumented in all three designs**; see §10 |
| Sidebar query-string precedent (`String` field, `Edited→notify`, `Escape→clear+focus_active_tab`) | `vertical_tabs_search_input` `view.rs:1516-1540`; state `vertical_tabs.rs:716` | exists, **verified verbatim** |
| Ordering contract — rank is a pure function of `(projects, priorities)`, never of agent events | `project_priorities.rs:151-187`; rationale `view.rs:24097-24100` | exists — forbids relevance reordering |
| Dormant row cap | `view.rs:24064` `MAX_DORMANT_TASK_ROWS = 5`, applied `:24428` | exists |
| Header chips fed from the **unfiltered** task set | `view.rs:23789-23805`; `rail_triage.rs:214-232,253-264,273-288` | exists — must stay unfiltered |
| Triage owns label text color and outranks active/inactive shading | `view.rs:24337-24346`, palette `:23755-23763` | exists — highlight must use another channel |
| Fuzzy matcher returning **char** indices | `fuzzy_match/src/lib.rs:88`, authority is `match_internal` `:128-136` | exists; fn-level doc is wrong — see §9 |
| Highlight sink taking char indices verbatim | `warpui_core/src/elements/gui/text.rs:340` ("Note that indices are char indices") | exists |
| Transcript bounded-read helpers + parse rules | `transcript_naming.rs:41` (256 KiB head), `:46` (64 KiB tail), `:103-112` seek-tail, `:224-234` / `:238-248` prompt extraction | exists — **the two prompt fns are private**, see §9 |
| Scan model shape: main-thread snapshot → `ctx.spawn` → replace bucket → `emit`+`notify` | `session_scan.rs:220-278`; rail observes at `view.rs:3079-3083` | exists — the template for the digest model |
| Per-directory memo keyed `(path, mtime)`, returned complete | `session_scan.rs:97-109` | exists — extend with `len` |
| Scan caps and throttle | `session_scan.rs:84` `MAX_SCANNED_SESSIONS_PER_DIR = 16`; `:93` `RESCAN_INTERVAL = 20s`; truncation `:341` | exists |
| Anchored session-id filename filter (keeps `<uuid>/subagents/**` out of rows) | `session_scan.rs:140-167` | exists |
| Dormant row payload a content hit can construct directly | `project_layout.rs:69-80` `DormantTask { agent, session_id, label, cwd, origin }`; built at `:205-215` | exists — see §6 |
| Debounce + generation-tagged, abortable search | `warp_core/src/async/debounce.rs:74`; `global_search/view.rs:69,662-667`; `global_search/model.rs:67-69,104-120` | exists — patterns to copy |
| In-process ripgrep | `warp_ripgrep/src/search.rs:170-178` (subprocess re-exec `:180-214`), `:16` heap limit, `:66-70` default walk | exists — **not used before P4**, see §2 |

## 4. The design in one paragraph

A magnifier button joins the status chips, funnel and `+` in the rail header. Clicking it (or the
palette entry) reveals a one-line field under the header and focuses it — it never auto-focuses.
Typing filters the rail **in place**: non-matching task rows and non-matching projects disappear,
matching characters get a background wash, and order never changes. That tier is synchronous,
disk-free and instant. Separately, after a 300 ms pause, a second tier scans an in-memory digest of
the transcripts belonging to the projects the rail already knows, and surfaces matching sessions as
**ordinary rows carrying a one-line snippet subline** — including sessions past the 16-per-directory
scan cap, which the match itself promotes into a row. When the digest is not enough, one explicit
action — "Search all transcripts in \<project\> ⏎" — runs a full-fidelity pass. Escape clears the
query and returns focus to the active tab. The empty-query path is byte-identical to today's rail.

## 5. Composition: search is the last filter, and only ever removes rows

Order, stated once so nobody re-derives it:

1. `rail_project_rows(projects, priorities)` — **unchanged** (`project_priorities.rs:151-187`).
2. `ProjectLayout::compute_with_handles` — unchanged.
3. `rail_triage` over the **unfiltered** task set; header chips keep reading it
   (`view.rs:23789-23805`). A query must never shrink a chip count, or a blocked agent becomes
   unreachable mid-type.
4. `visible_live_rows(rows, RailShellFilter)` — unchanged (`rail_shells.rs:78`).
5. **New:** `rail_search::visible_rows(...)`, consuming step 4's output.
6. Dormant rows filtered **before** `take(MAX_DORMANT_TASK_ROWS)` (`view.rs:24064,24428`).

Three consequences, each a decided rule:

- **No relevance reordering, ever.** The ordering contract is explicit that a project must not move
  because one of its agents changed state (`view.rs:24097-24100`); a query is even weaker grounds.
  Search filters; it does not rank the rail. (Content *hits* may be internally ranked by recency
  when choosing which snippet to show on a row — that is a within-row choice, not a row order.)
- **Search does not extend `RailShellFilter`.** Folding a query into it turns
  `hidden_shells = rows.len() - visible.len()` (`rail_shells.rs:104`, verified) into a lie that
  reports search-excluded *agent* rows as hidden *shells*. Search carries its own
  `hidden_by_query` counter and renders its own dim summary row.
- **Exemptions are inherited, with one open question.** The active tab's row is never hidden
  (`rail_shells.rs:88`) — decided, because the rail must not disagree with the terminal on screen.
  Whether the selected project keeps its `fallback_row` (`rail_shells.rs:95-101`) is a taste call
  the designers split on → §10.

**Highlight channel:** background wash + font weight + a leading marker. **Never text color** —
triage owns label color and deliberately outranks active/inactive shading (`view.rs:24337-24346`).
Indices come from `fuzzy_match::match_indices_case_insensitive` and feed
`Text::with_single_highlight` with no conversion (both are char indices; see §9).

## 6. Architecture

### Tier 1 — names and fragments (synchronous, no disk)

New pure module **`app/src/workspace/rail_search.rs`**, beside `rail_shells.rs` / `rail_triage.rs`,
same style — plain data, no `AppContext`, tests in `rail_search_tests.rs` via the `#[path]`
convention:

```
RailSearchQuery   { text, lowered }
RailRowFragments  { project, task, cwd, session_id, snippet: Option<Snippet> }
RailSearchView    { visible, hidden_by_query, matched_projects }
fn visible_rows(rows, query, exemptions) -> RailSearchView
```

Empty query short-circuits before any work (`rail_shells.rs:79-84` pattern).

**Match a fragment set, not the rendered label.** New `tab_title::rail_task_fragments(pane_group, ctx)`
returns the cheap tiers — agent title, latest user prompt, custom rename, cwd basename, branch —
and omits the shell-title tier. The **primary** reason is correctness, not cost: `rail_task_label`
routes through `TabSettings::rail_task_info` (`tab_title.rs:143`), so matching the rendered string
alone would make results depend on a display setting — a tab configured to show its branch would
not match its own agent title. (The terminal-model-lock argument is real but secondary and weaker
than the designs claimed; see §9.) Modelled on `pane_search_text_fragments`
(`vertical_tabs.rs:3893-3905`).

Projects match on `display_name` **and** the full path via `ProjectKey::to_storage_key`
(`project_key.rs:101`) — free, already in memory. Dormant/scanned rows match on `label` + `cwd` +
`session_id` prefix (`project_layout.rs:69-80`).

Fragments are gathered **only while the query is non-empty** — the `has_agent` precedent, "a rail
that is not filtering must not pay for them" (`view.rs:24246-24255`).

### Tier 2 — the digest (in-memory, append-only, off-thread)

**`app/src/workspace/rail_transcript_digest.rs`** — a model in the exact shape of
`ClaudeSessionScanModel::refresh` (`session_scan.rs:220-278`): compute the due set on the main
thread, move an owned snapshot into `ctx.spawn`, do all `std::fs` there, then replace buckets,
`ctx.emit`, `ctx.notify`. The rail observes it exactly as it observes the scan model
(`view.rs:3079-3083`).

- **Content:** user turns **and** assistant prose. Excluded: tool-result bodies, pasted file
  contents. This is the MOST-CAPABLE graft: prompts-only cannot find the error the agent printed.
  Role attribution follows Orbit's rule done structurally via serde, not regex — an envelope
  `"type":"user"` **without** `tool_use_id` is something Sam typed; with it, it is a tool result
  (Orbit `:259-267`). Extraction reuses `real_user_prompt_text` / `user_prompt_text`
  (`transcript_naming.rs:224-248`), which requires a `pub(crate)` visibility change (§9).
- **Cache key `(PathBuf, len, SystemTime)`.** The mtime half is the `session_scan.rs:97-99` idiom
  that makes staleness structurally impossible. The `len` half makes growth append-only: a grown
  transcript re-reads only `[old_len, EOF)`, seek-and-read as at `transcript_naming.rs:103-112`.
  **Errors are never memoized** — a transient I/O failure must retry, not be cached as empty.
- **Not persisted to disk.** Rebuild is ≤4 MB and sub-second per project; persistence would buy
  that back at the price of a staleness mode Orbit refused on purpose.
- **Search over it** is a case-insensitive literal substring scan (`memchr::memmem`, already in the
  lock file) — literal, never fuzzy, so `POA-2236` stays one token.
- **Per-line `serde_json` in try/continue**, lossy UTF-8 decode: a live transcript's final line is
  routinely torn.

### Tier 3 — the escape hatch (explicit, one-shot, full fidelity)

"Search all transcripts in \<project\> ⏎" runs a full pass over that one project's directory, no
digest, no role filter. This is the only place the `warp_ripgrep` question is reopened, and it is
reopened on its own merits at P4 — one-shot and user-invoked amortizes both the subprocess re-exec
(`search.rs:180-214`) and the argument for parameterizing `SEARCHER_LINE_HEAP_LIMIT` (`search.rs:16`)
and the `WalkBuilder` defaults (`search.rs:66-70`). If that crate change is judged too invasive
even then, the same pass runs through our own reader at lower speed. Either way it is not on the
critical path for P1–P3.

### Content hits become rows — including past the cap

A content hit already yields everything `DormantTask { agent, session_id, label, cwd, origin }`
needs (`project_layout.rs:69-80`, constructed at `:205-215`). So:

- A hit whose session already has a row (live tab, handle-backed dormant, or scanned) **keeps that
  row visible** and attaches a dim one-line snippet subline.
- A hit whose session is **past `MAX_SCANNED_SESSIONS_PER_DIR = 16`** (`session_scan.rs:341`) is
  promoted into a `DormantTask { origin: Scanned }` row. This is the one place I override
  LOWEST-RISK, which answered this case with a footnote: the measurement shows real projects at 72,
  36, 24 and 17 top-level transcripts, so the cap is not a corner case. "Filter before the cap"
  therefore also means "expand past the cap for matches."
- A hit that cannot be mapped to a resumable session id — `session_id_from_transcript_file_name`
  (`session_scan.rs:140-167`) stays between any file list and the rail — is **not** fabricated into
  a row. A scanned session has no pane and must never invent one.

### State and gating

Transient `RailSearchState { query: String, input: ViewHandle<EditorView> }` on `Workspace`,
alongside `rail_wait_age_refresh` (`view.rs:1131-1141`) and `project_rail_resizable_state`
(`view.rs:1165`) — **never** a `TabSettings` entry. Input built once in `Workspace::new`, following
`vertical_tabs_search_input` (`view.rs:1516-1540`) verbatim: `SingleLineEditorOptions`, no
`select_all_on_focus` (that belongs to transient dropdowns, `view.rs:1482-1483`),
`EditorEvent::Edited` → copy `buffer_text` → `ctx.notify()`, `EditorEvent::Escape` → clear +
`focus_active_tab`. Its `MouseStateHandle` is a constructed-once field like `util.rs:46,48`, never
an inline `default()`.

Only the **content-tier-enabled** bool is persisted, in the `rail_hide_shells_without_agents` mold
(`tab_settings.rs:721-730`), gated on `FeatureFlag::Projects`, with the matching Command Palette
entry per AGENTS.md.

Debounce: **tier 1 is not debounced** (`keybindings.rs:537` and `vertical_tabs.rs:1775-1800` both
filter synchronously per keystroke). **Tier 2 is**, at 300 ms via `warp_core::r#async::debounce`
into `ctx.spawn_stream_local`, exactly as `global_search/view.rs:69,662-667`, with a monotonic
`search_id` so superseded results are dropped (`global_search/model.rs:67-69,104-120`).

## 7. Performance budget

Corpus re-measured on this machine **2026-08-05**: `~/.claude/projects` holds 73 directories and
**5,939** top-level `*.jsonl` (9,944 including subagents). **5,658 of those 5,939 live in one
`/private/var/folders/…` temp directory** that can never be a rail project — so the rail-relevant
corpus is **281 files**, and per-project ≤16 after the scan cap. The largest real project
directories hold 72 / 36 / 24 / 17 top-level transcripts.

| Path | Budget | Basis |
|---|---|---|
| Keystroke → tier-1 repaint | **< 2 ms, no async** | ~10 projects + ~49 tasks × ~5 fragments ≈ 250 `match_indices` calls |
| Zero-query cost | **zero** | fragments gathered only while the query is non-empty (`view.rs:24246-24255` precedent) |
| First digest build, one project | **~0.3–0.6 s off-thread** | Warp's own dir: ~5 files / ~50 MB; rail shows "indexing…" while tier-1 results are already on screen |
| Steady-state tier-2 scan | **~1 ms** | ≤16 files × bounded digest ≈ ≤4 MB per project, `memchr::memmem` |
| Incremental refresh | **`[old_len, EOF)` only** | `(path, len, mtime)` key; transcripts are append-only |
| Tier 3, one project | **< 1.5 s** | whole-tree warm `rg -l` measured at 1.45 s over a corpus 95% irrelevant; one project is far less |

Hard caps: per-file read cap (a 92 MB transcript is scanned partially and flagged `partial`),
per-refresh byte cap, an LRU bound on the digest store, ≤5 content rows per project and ≤50 total
with a visible "+N more" (`global_search/view.rs:65,815-820` precedent).

## 8. Risks

| Risk | Note |
|---|---|
| Digest is lossy by construction | Tool-result bodies and pasted blobs are excluded. This is the deliberate 30× latency win, and it **must be said in the UI** — the section header carries "conversation text only" and the tier-3 action is the escape hatch. A search that silently half-answers is worse than one that says so. |
| `hidden_shells` corruption | The single easiest mistake here. Search composes outside `visible_live_rows` and owns its own counter — asserted by a unit test that filters agent rows and checks the shells count is unchanged. |
| Chip counts shrinking mid-query | Chips read the unfiltered `task_triage` (`view.rs:23789-23805`). A blocked agent must stay reachable while typing; test it. |
| Label resolution inverting the render order | Tier 1 matches fragments, not `rail_task_label`, so the four-tier resolution (and its conditional shell-title lock) is never forced for a row that will be hidden. |
| Match highlight colliding with triage | Highlight uses background/weight/marker only. A recolored label would fight `view.rs:24337-24346`, which deliberately wins. |
| Focus stealing | The field never auto-focuses; the rail is persistent and keystrokes belong to the terminal. Escape clears **and** returns focus (`view.rs:1533-1536`). |
| Reach gap: projects with no open tab and no handle | `refresh_claude_session_scan` (`view.rs:5989-6023`) defines the reachable set, and the directory slug is never reverse-mapped because the encoding is lossy (`session_scan.rs:53-56`). Such projects are invisible to every tier. The known escape hatch — enumerate `~/.claude/projects` and trust each transcript's own `cwd` head field (verified present at line 6 of a real transcript, well inside the 256 KiB head pass) — is feasible and deliberately deferred to P4. |
| Self-referential corpus | Content search will match the agent's own chatter about the query, because these transcripts record this work. A relevance annoyance, not a correctness bug — but it will be visible on day one. |
| Ctrl-Tab divergence | `activate_prev_tab`/`activate_next_tab` branch away from `search_query` in project mode (`view.rs:12712-12716,12739-12744`), so a rail query does **not** scope tab cycling unless wired. Wiring it means cycling can land on a tab whose row is not drawn. → §10 |
| `warp_ripgrep` change at P4 | Parameterizing `SEARCHER_LINE_HEAP_LIMIT` and the `WalkBuilder` defaults touches a crate `global_search` ships on. Gate it behind its own review; the fallback is our own reader. |
| New RAM | A digest store the rail did not have. Bounded and non-persisted, but non-zero. |

## 9. Claims verified, and three corrections

Spot-checked against this checkout (branch `samithaj/project-tabs-layout`, 2026-08-05):

**Confirmed as cited:** `rail_shells.rs:104` subtraction and the two exemptions at `:88`/`:95-101`;
`SEARCHER_LINE_HEAP_LIMIT = 64 * 1024` at `search.rs:16` with `WalkBuilder` defaults at `:66-70`
and the `current_exe()` re-exec at `:180-214`; `vertical_tabs_search_input` at `view.rs:1516-1540`
behaving exactly as described (`Edited`→`notify`, `Escape`→clear+`focus_active_tab`);
`MAX_SCANNED_SESSIONS_PER_DIR = 16` (`session_scan.rs:84`, truncated `:341`), `RESCAN_INTERVAL = 20s`
(`:93`), the `(path, mtime)` memo (`:97-109`); `HEAD_READ_BYTES = 256 KiB` / `TAIL_READ_BYTES = 64 KiB`
(`transcript_naming.rs:41,46`); `MAX_DORMANT_TASK_ROWS = 5` (`view.rs:24064`);
`Text::with_single_highlight` documented as char indices (`text.rs:340`);
`AsyncSnapshotDataSource::new` (`async_snapshot_data_source.rs:23-51`) and
`warp_core/src/async/debounce.rs:74` both exist as described. Corpus re-measured (§7).

**Corrected:**

1. **The terminal-model-lock argument is weaker than all three designs claim.**
   `resolve_rail_task_label` (`tab_title.rs:171-183`) takes its sources after the first as
   `FnOnce`, and the shell-title tier runs **only if the first three all blank**. "Every row pays a
   lock every frame" is not accurate. The fragment-set decision stands, but its load-bearing
   justification is the **correctness** one — results must not depend on `TabSettings::rail_task_info`
   — with the lock as a secondary note. Corrected in §6.
2. **`fuzzy_match`'s function-level doc comment says "byte indices"; it is wrong.** The struct field
   doc (`lib.rs:59-62`) and `match_internal`'s inline comment (`lib.rs:128-136`, "The fuzzy_indices
   API returns char indices, so we don't need to manually convert") are the authority, and skim's
   `fuzzy_indices` does return char indices. The designs' claim that indices feed
   `with_single_highlight` with no conversion **holds** — but cite `match_internal`, not the fn doc,
   or a reader greps the wrong line and files a bug.
3. **`real_user_prompt_text` / `user_prompt_text` are private** (`transcript_naming.rs:224,238` —
   bare `fn`). All three designs say "reuse". That is a `pub(crate)` visibility change, listed as
   planned work in §11, not a free reuse.

**Found, undocumented by any design:** the rail's header filter button is wrapped in
`if *tab_settings.rail_show_tasks` (`view.rs:23863`). Whether the search field sits inside or
outside that gate is a real question → §10.

## 10. Open questions for Sam

1. **Does the search field follow `rail_show_tasks`?** The funnel button is hidden when task rows
   are off (`view.rs:23863`) — "with nothing listed there is nothing to filter". But search also
   matches **projects**, which are listed regardless. Hide it with the tasks, or keep it as a
   project-name filter?
2. **Does search keep the selected project's `fallback_row` exemption?** (`rail_shells.rs:95-101`.)
   Keeping it means the project you are standing in never collapses to a bare header, even when
   nothing matches. Dropping it means a query cleanly empties every non-matching project —
   arguably the point of searching. The designers split on this.
3. **Should a query scope Ctrl-Tab in project mode?** Today it does not: `activate_prev_tab` /
   `activate_next_tab` return early through `project_visible_indices` and never consult a search
   query (`view.rs:12712-12716,12739-12744`). Wiring it makes cycling agree with what is drawn;
   not wiring it means cycling can land on a hidden row.
4. **Content tier default on or off?** Off means the feature is name-only until discovered; on
   means every rail user starts paying a background digest build.
5. **Ship the keybinding bound or unbound?** Global search ships bound (`cmd-shift-F`,
   `workspace/mod.rs:861-870`); an unbound `EditableBinding` plus a palette entry is the
   lower-collision option for a sidebar field.

## 11. Phases

| Phase | Scope | Depends on |
|---|---|---|
| 0 | `pub(crate)` on `real_user_prompt_text` / `user_prompt_text` (`transcript_naming.rs:224,238`); `tab_title::rail_task_fragments` | — |
| 1 | `rail_search.rs` (pure module + tests), header magnifier, field, Escape, `hidden_by_query` summary row, background-wash highlight, palette entry. **Names + fragments only, no disk.** Ships alone; revert = delete one module and one render branch | 0 |
| 2 | `rail_transcript_digest.rs` over the already-scanned directories, **selected project only**; snippet sublines on existing rows; promotion of past-the-cap matches into `DormantTask` rows; honesty labels + `indexed N/M` readout. Setting default per §10.4 | 1 |
| 3 | Widen to all rail projects; background prewarm on rail open; "+N more"; result caps | 2 |
| 4 | Tier-3 escape hatch (full-fidelity per-project pass — and the `warp_ripgrep` parameterization decision on its own merits); enumerate `~/.claude/projects` by each transcript's own `cwd` head field to reach handle-less projects; Orbit's anchor planner + coverage floor for pasted blobs; `history.jsonl` (~29% of pastes live only there) with the minute→±2min→hour probe; Codex rollouts. Keep a `HitSource`-shaped enum from P2 so none of this reshapes results | 3 |
| 5 | (optional, separate spec) Register the digest as a `QueryFilter::Transcripts` mixer source so Cmd-P gets transcript search too | 2 |
