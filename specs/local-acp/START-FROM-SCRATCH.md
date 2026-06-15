# Local ACP — Start From Scratch (new context entry point)

Use this doc when opening a **new Cursor chat** to rebuild Local ACP correctly. You do **not** need to reset the git work tree unless you want a clean upstream baseline — but the recommended path is **full scratch on upstream**, using this branch only as **reference**, not as a patch to apply blindly.

---

## Two ways to restart

| Approach | When to use | Risk |
|----------|-------------|------|
| **A. Full scratch (recommended)** | You want correctness over speed; avoid carrying bad wiring from pass 1 | Lowest regression risk |
| **B. Keep new files, revert hooks** | You trust `acp_client` + `app/src/ai/acp/*` and only want to re-wire upstream | Medium — pass 1 logic may embed flag checks |

**You chose A.** The new context should implement wiring phase-by-phase with compile/test gates, not merge the 37-file diff at once.

---

## What to preserve before resetting (optional)

If you reset the work tree, **save reference material** first. You do not need to keep modified upstream files.

### Must keep (documentation — copy or leave in repo)

```
specs/local-acp/
  START-FROM-SCRATCH.md    ← this file (give to new context)
  RESTART-GUIDE.md         ← per-file justifications, regressions, cloud removal
  TECH.md                  ← architecture
  MINIMAL-INTEGRATION.md   ← file tier index
  TASKS.md                 ← feature checklist
```

### Optional reference (pass 1 implementation — read-only, do not copy wholesale)

Use pass 1 code as **examples**, rewrite cleanly in v2:

```
crates/acp_client/
app/src/ai/acp/
app/src/ai/agent_sdk/driver/harness/acp.rs
app/src/ai/blocklist/local_acp_stream.rs
app/src/terminal/view/ambient_agent/harness_selection.rs
```

To archive before reset (optional):

```bash
# From repo root — saves pass-1 new files to /tmp for diffing while you work on clean tree
tar czf /tmp/warp-local-acp-pass1-new-files.tgz \
  crates/acp_client \
  app/src/ai/acp \
  app/src/ai/agent_sdk/driver/harness/acp.rs \
  app/src/ai/blocklist/local_acp_stream.rs \
  app/src/terminal/view/ambient_agent/harness_selection.rs \
  specs/local-acp
```

### Do not preserve (revert / discard)

All **modified** upstream files from pass 1 — especially:

- `app/src/terminal/input.rs`, `view.rs` (large, easy to get wrong)
- `profile_model_selector.rs` (should never have been touched)
- Runtime flag plumbing in `features.rs`, `warp_features`

---

## Resetting to upstream (optional git steps)

Only if you want a clean tree. **Not required** — you can also work on a new branch and revert files selectively.

```bash
# Option 1: New branch from main, keep specs
git fetch origin
git checkout -b local-acp-v2 origin/main   # or your main branch
# Copy specs/local-acp/ from pass-1 branch if needed

# Option 2: Hard reset working tree on current branch (destructive)
git checkout HEAD -- .                   # revert all tracked modifications
# Untracked new dirs (acp_client, app/src/ai/acp) remain until you delete them:
rm -rf crates/acp_client app/src/ai/acp
rm -f app/src/ai/agent_sdk/driver/harness/acp.rs
rm -f app/src/ai/blocklist/local_acp_stream.rs
rm -f app/src/terminal/view/ambient_agent/harness_selection.rs
```

After reset: upstream Warp builds as before. Local ACP does not exist until the new context implements it phase by phase.

---

## Prompt for the new context (paste this)

```
Implement Local ACP as a full replacement for cloud agent in Warp — no runtime feature flag, local ACP only.

Read in order:
1. specs/local-acp/START-FROM-SCRATCH.md
2. specs/local-acp/RESTART-GUIDE.md
3. specs/local-acp/TECH.md

Rules:
- Product: agent pane submits to local ACP subprocesses (Claude, Codex, Gemini, Cursor, Devin). No cloud ambient agent, no server Oz, no FeatureFlag::LocalAcp runtime toggle.
- Put all logic in NEW files under crates/acp_client/ and app/src/ai/acp/.
- Touch existing upstream files ONLY at documented choke points (~18 files). Do NOT touch profile_model_selector.rs.
- Never create AmbientAgentViewModel on local panes. Use LocalAcpHarnessModel for harness/model state only.
- Never use Status::Composing to show the harness picker on local panes.
- Update inline_menu/positioning.rs in the same step as the harness row UI.
- Implement phase-by-phase; cargo check after each phase. Test submit routing before harness UI.
- Delete cloud agent paths; do not guard them with flags.

Optional reference (pass 1, read-only): /tmp/warp-local-acp-pass1-new-files.tgz or the pass-1 branch — use for API shapes, not for copying wiring verbatim.
```

---

## Implementation phases (new context follows this order)

Each phase ends with `cargo check --bin warp-oss --features gui` (0 errors).

### Phase 1 — Transport & module tree (no terminal changes)

- [ ] Add `crates/acp_client/` (AcpProcess, AcpConnection, AcpSession, tests)
- [ ] Workspace + `app/Cargo.toml` dep (`local_acp` in default features, no runtime flag)
- [ ] Add `app/src/ai/acp/*` (registry, path_search, models, submit_model skeleton)
- [ ] `app/src/ai/mod.rs`, `app/src/lib.rs` singleton registration

**Gate:** compiles; no `input.rs` / `view.rs` changes yet.

### Phase 2 — Harness type & catalog

- [ ] `Harness::Cursor`, `Harness::Devin` in `crates/warp_cli/src/agent.rs`
- [ ] `harness_display.rs` for new variants
- [ ] `harness_availability.rs`: local catalog, `models_for_picker`, PATH refresh
- [ ] `local_harness_setup.rs`: ACP binary detection via `path_search`
- [ ] Fix exhaustive `match Harness` in compile-required files only (see RESTART-GUIDE §C)

**Gate:** compiles; harness catalog unit tests pass.

### Phase 3 — Driver & blocklist streaming

- [ ] `app/src/ai/agent_sdk/driver/harness/acp.rs` + wire in `mod.rs`
- [ ] `local_acp_stream.rs`, `ResponseStreamId::new_local()`, quota bypass in `prompt_alert.rs`

**Gate:** compiles; mock ACP driver test if available.

### Phase 4 — Submit path (before any harness UI)

- [ ] `app/src/terminal/view.rs`: `LocalAcpHarnessModel` creation, `ExecuteLocalAcpQuery` handler only
- [ ] `app/src/terminal/input.rs`: event + `submit_ai_query` routes to ACP; **no Oz fallback** (replacement product)
- [ ] `submit.rs` / `submit_model.rs` complete

**Gate:** manual test — agent pane submit spawns ACP process (harness can be hard-coded briefly).

### Phase 5 — Harness UI (decoupled from cloud)

- [ ] `harness_picker.rs` (`LocalAcpHarnessModel`)
- [ ] `harness_selection.rs` backend enum
- [ ] `harness_selector.rs`, `model_selector.rs` — LocalAcp backend only (v2: drop Cloud backend if cloud removed)
- [ ] `input/agent.rs` harness row + `shows_local_acp_harness_row()`
- [ ] `inline_menu/positioning.rs` — **same commit** as harness row

**Gate:** regular terminal tab shell works; agent pane shows picker; tooltips positioned correctly.

### Phase 6 — Remove cloud agent (not guard)

- [ ] Remove/stub: `enter_cloud_agent_view`, `start_spawn_stream`, Oz submit paths, handoff, cloud slash commands
- [ ] Do **not** add `FeatureFlag::LocalAcp` or `cloud_agent_disabled()` — delete paths instead

**Gate:** grep confirms no live `send_user_query_in_new_conversation` from agent submit.

### Phase 7 — Polish

- [ ] Permissions, MCP, slash forwarding, telemetry, session resume
- [ ] Hide zero-state cloud agent copy

**Gate:** full testing matrix in RESTART-GUIDE.md.

---

## What pass 1 got wrong (do not repeat)

1. Created `AmbientAgentViewModel` on all local panes → broke shell input.
2. Used `Status::Composing` for harness picker → Enter hijacked to cloud spawn.
3. Added harness row without positioning fix → broken tooltips.
4. Used runtime `FeatureFlag::LocalAcp` everywhere → wrong model for a replacement product.
5. Guarded cloud paths instead of removing them → fragile, easy to miss a path.
6. Touched `profile_model_selector.rs` → unnecessary regression surface.

---

## Success criteria

- [ ] No runtime flag to enable Local ACP
- [ ] No cloud agent spawn or Oz server query from agent pane
- [ ] Regular terminal tabs unchanged (shell only)
- [ ] Agent pane: harness + model pickers → ACP submit → blocklist streaming
- [ ] ~25 new files, ~18 upstream files touched surgically (see RESTART-GUIDE §B)
- [ ] `profile_model_selector.rs` untouched

---

## Doc map

| Doc | Role |
|-----|------|
| **START-FROM-SCRATCH.md** (this file) | New context entry point, phases, git optional reset |
| **RESTART-GUIDE.md** | Why each file exists, regressions, cloud removal audit |
| **TECH.md** | ACP protocol architecture |
| **MINIMAL-INTEGRATION.md** | Short file-tier index |
| **TASKS.md** | Feature checklist (P0–P3) |
