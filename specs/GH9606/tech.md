# Tech Spec: Built-in `/review` slash command

**Issue:** [warpdotdev/warp#9606](https://github.com/warpdotdev/warp/issues/9606)

## Context

Warp's static slash commands are declared as `StaticCommand` constants/lazies in [`app/src/search/slash_command_menu/static_commands/commands.rs`](https://github.com/warpdotdev/warp/blob/master/app/src/search/slash_command_menu/static_commands/commands.rs) and routed through the dispatch logic in `app/src/terminal/input/slash_commands/mod.rs`. The pattern for an agent-facing command (one that submits a turn rather than opening a panel) is well established by `/init`, `/index`, and `/fork`.

This spec adds a single new entry to that registry plus a focused diff-gathering helper. No new infrastructure, no new modules.

### Relevant code

| Path | Role |
|---|---|
| `app/src/search/slash_command_menu/static_commands/commands.rs` | The `StaticCommand` registry. The new `REVIEW` constant goes here, alongside `OPEN_CODE_REVIEW` and `INIT`. |
| `app/src/search/slash_command_menu/static_commands/mod.rs` | `Availability` bitflags (`REPOSITORY`, `AI_ENABLED`, etc.) and the `COMMAND_REGISTRY` aggregator. The new constant is added to the registry. |
| `app/src/terminal/input/slash_commands/mod.rs` | Dispatch site for selected slash commands. The new arm builds the agent prompt and calls into the existing agent-turn submission path. |
| `app/src/code_review/` | Existing code-review surface that knows how to read the working tree's diff against `HEAD`. The diff-gathering helper introduced here reuses any helpers it exposes; if none are public, a small `git diff` shellout lives alongside the dispatch. |
| `app/src/settings/ai.rs` | The `AISettings` group. The new `ReviewCommandMaxDiffBytes` setting lands here. |
| `app/src/server/telemetry/events.rs` | Telemetry. `/review` is recorded via the existing slash-command telemetry path; no new event type needed. |

### Related closed PRs and issues

- `/open-code-review` and `/pr-comments` are the closest existing analogs. Neither submits an agent turn; both open panels. `/review` is closer in shape to `/init`, which constructs an agent prompt and submits it.
- No closed PRs interact with this surface that are relevant context.

## Crate boundaries

The new code lives entirely in `app/`. Diff gathering happens in the same process via either an existing `code_review` helper or a `tokio::process::Command` shell out to `git`. No new crate, no new shared type, no cross-crate boundary changes.

## Proposed changes

### 1. New `StaticCommand` constant

**File:** `app/src/search/slash_command_menu/static_commands/commands.rs`.

```rust
pub const REVIEW: StaticCommand = StaticCommand {
    name: "/review",
    description: "AI review of local uncommitted changes",
    icon_path: "bundled/svg/diff.svg",
    availability: Availability::REPOSITORY
        .union(Availability::AI_ENABLED)
        .union(Availability::HAS_UNCOMMITTED_CHANGES),
    auto_enter_ai_mode: true,
    argument: None,
};
```

`HAS_UNCOMMITTED_CHANGES` is a new flag; see Section 3.

`auto_enter_ai_mode: true` matches `/agent` and `/fork` — selecting the command should land the user in an agent turn, not the terminal-input default.

The constant is exported from `mod.rs`'s `COMMAND_REGISTRY` (find the existing aggregator macro/list and append).

### 2. Dispatch arm

**File:** `app/src/terminal/input/slash_commands/mod.rs`.

The dispatch path that consumes `SlashCommandId` already has a switch over the registry's identifiers. Add an arm:

```rust
SlashCommandId::Review => {
    let workspace_root = ctx.current_session_cwd()
        .and_then(|p| find_git_root(&p)); // existing helper

    let Some(repo_root) = workspace_root else {
        // Defensive — Availability::REPOSITORY should have prevented this
        ctx.show_toast("Run /review from a Git repository.");
        return;
    };

    let max_bytes = AISettings::handle()
        .review_command_max_diff_bytes()
        .clamp(1024, 1_048_576);

    let diff = match gather_uncommitted_diff(&repo_root, max_bytes) {
        Ok(d) => d,
        Err(e) => {
            ctx.show_toast(format!("/review failed to gather diff: {e}"));
            return;
        }
    };

    if diff.payload.is_empty() {
        ctx.show_toast("/review found no uncommitted changes.");
        return;
    }

    let prompt = build_review_prompt(&diff);
    submit_agent_turn(ctx, prompt);  // existing entry point used by /init etc.
    send_telemetry_from_ctx(
        ctx,
        TelemetryEvent::SlashCommandAccepted(SlashCommandAcceptedDetails {
            command_name: "/review".to_owned(),
            ..
        }),
    );
}
```

### 3. New `Availability` flag: `HAS_UNCOMMITTED_CHANGES`

**File:** `app/src/search/slash_command_menu/static_commands/mod.rs`.

The existing `Availability` bitflags struct gets one more flag. The condition is true when the active session's CWD is inside a Git repo *and* `git status --porcelain` returns at least one entry. Existing `Availability` flags are checked at palette-render time via cached `WorkspaceState` flags (find the cache via `grep -rn "Availability::REPOSITORY" app/src/`). The new flag rides the same cache; the cache is invalidated when the working tree changes (which the file-tree observer already tracks for the existing Code Review panel).

If the cache infrastructure already tracks "has uncommitted changes" — likely, since `/open-code-review` is enabled per-repo regardless of state but the panel itself shows a "no changes" empty state — V1 reuses that signal directly. If not, the cache acquires one new boolean populated by the same file-tree observer.

### 4. Diff gathering helper

**File:** new `app/src/terminal/input/slash_commands/review.rs`.

```rust
pub struct ReviewDiff {
    pub payload: String,           // The diff text inlined into the prompt
    pub truncated: Option<TruncationNote>,
}

pub struct TruncationNote {
    pub original_files: usize,
    pub original_bytes: usize,
    pub kept_files: usize,
    pub kept_bytes: usize,
}

pub fn gather_uncommitted_diff(repo_root: &Path, max_bytes: usize) -> anyhow::Result<ReviewDiff> {
    // 1. Run `git diff HEAD --no-color` for tracked-file changes (staged + unstaged).
    let tracked = run_git(repo_root, &["diff", "HEAD", "--no-color"])?;

    // 2. Detect untracked files via `git ls-files --others --exclude-standard`
    //    and synthesize a /dev/null → file diff for each so they appear in the
    //    review just like new-file diffs would after `git add`. Bounded by max_bytes
    //    along with the rest.
    let untracked_paths = run_git(
        repo_root,
        &["ls-files", "--others", "--exclude-standard"],
    )?;
    let untracked_diff = synthesize_new_file_diffs(repo_root, &untracked_paths)?;

    let combined = format!("{tracked}\n{untracked_diff}");

    // 3. If under the cap, return as-is. Otherwise split into per-file diffs,
    //    sort shortest-first, keep adding until we'd exceed the cap, and report.
    if combined.len() <= max_bytes {
        return Ok(ReviewDiff { payload: combined, truncated: None });
    }
    Ok(truncate_diff(combined, max_bytes))
}

fn build_review_prompt(diff: &ReviewDiff) -> String {
    // Fixed V1 template per product.md. Diff content inlined between
    // [BEGIN DIFF] / [END DIFF] markers exactly once.
    ...
}
```

`run_git` is a thin `tokio::process::Command` wrapper. The existing `code_review` module may already expose an equivalent helper (find via `grep -rn "fn.*git.*diff\|fn.*diff.*git" app/src/code_review/`). Prefer reusing if present, to keep diff-gathering behavior consistent across the two entry points.

`truncate_diff` splits on the per-file `diff --git a/... b/...` headers (well-known stable format), sorts files ascending by per-file diff length, accumulates until adding the next file would exceed the cap, and returns the kept set with a `TruncationNote`. The note is interpolated into the prompt template's truncation line.

### 5. Settings entry

**File:** `app/src/settings/ai.rs`.

Add via the existing `define_settings_group!` macro:

```rust
review_command_max_diff_bytes: ReviewCommandMaxDiffBytes {
    type: u32,
    default: 51200,
    supported_platforms: SupportedPlatforms::ALL,
    sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
    private: false,
    toml_path: "ai.review_command_max_diff_bytes",
    description: "Maximum diff size (bytes) the /review command sends to the agent. Larger diffs are truncated longest-files-first.",
},
```

The dispatch arm clamps the read value to `[1024, 1_048_576]` defensively; the macro itself doesn't currently support range constraints (verify at implementation time).

### 6. Documentation

The static command's `description` field (`"AI review of local uncommitted changes"`) is what surfaces in the palette and is the only user-facing text that ships in the command itself. A docs page at `docs/slash-commands.md` (or wherever the existing `/init` and `/index` are documented) gets a short paragraph. Out of scope for the spec's core gate; recommend shipping in the same release.

## Testing and validation

Each invariant from `product.md` maps to a test at this layer:

| Invariant | Test layer | File |
|---|---|---|
| 1, 2 (palette presence/absence) | unit | `app/src/search/slash_command_menu/static_commands/mod_test.rs` extension — for each combination of (in-repo, AI-enabled, has-changes), assert presence/absence/disabled state. |
| 3 (gather diff + submit turn) | unit | `app/src/terminal/input/slash_commands/review_tests.rs` (new) — mock `run_git` to return a small diff, assert the constructed prompt contains the diff and the marker text, assert `submit_agent_turn` is invoked exactly once. |
| 4 (truncation when over cap) | unit | review_tests — feed a diff with 5 files of varying lengths totaling 100k bytes, cap at 30k, assert kept-files-set is the shortest N that fit and the prompt contains "This diff was truncated". |
| 5 (BEGIN/END markers exactly once) | unit | review_tests — assert prompt body contains `[BEGIN DIFF]` exactly once and `[END DIFF]` exactly once. |
| 6 (Availability flags) | unit | command_tests — assert `REVIEW.availability` includes `REPOSITORY`, `AI_ENABLED`, `HAS_UNCOMMITTED_CHANGES`. |
| 7 (untracked files included) | unit | review_tests — set up a temp repo with one untracked new file, assert the gathered diff includes a `/dev/null → b/<path>` synthetic diff for it. |
| 8 (telemetry event) | unit | review_tests — assert `send_telemetry_from_ctx` is called with `command_name = "/review"`. |
| 9 (follow-up turns have diff context) | integration | UI integration test — invoke `/review`, agent responds; user submits a follow-up "focus on auth/"; assert the agent's prompt context includes the original diff. (This falls out of the existing turn flow; the test is a regression guard.) |
| 10 (no auto re-invoke on follow-ups) | unit | review_tests — assert no second call to `gather_uncommitted_diff` is triggered when a follow-up turn is submitted. |

### Cross-platform constraints

- `git diff HEAD --no-color` is portable across Git versions Warp supports today.
- `git ls-files --others --exclude-standard` is also portable.
- The `tokio::process::Command` wrapper handles Windows path quoting via the same convention used elsewhere for git invocations.
- The diff is UTF-8; binary-file diffs are coerced via `--no-color` (binary diffs collapse to a "Binary files differ" line, which is fine for review purposes — the agent can ask follow-up questions if it needs the binary itself).

## End-to-end flow

```
User types `/review` in Agent input
  └─> [slash_command_palette]                                (existing)
        └─> filter by Availability flags
              ├─> REPOSITORY: cached at session-cwd-change
              ├─> AI_ENABLED: cached at AI-settings-change
              └─> HAS_UNCOMMITTED_CHANGES: cached at file-tree-change
        └─> render `/review` if all flags set; disabled-with-tooltip if HAS_UNCOMMITTED_CHANGES is false
        └─> on selection → SlashCommandId::Review

User accepts the command
  └─> [slash_commands::dispatch::Review arm]                 (new)
        ├─> resolve repo_root from session cwd
        ├─> read max_bytes from AISettings (clamped)
        ├─> [gather_uncommitted_diff]                        (new helper)
        │     ├─> run `git diff HEAD --no-color`             (tracked)
        │     ├─> run `git ls-files --others --exclude-standard` (untracked names)
        │     ├─> synthesize new-file diffs for untracked
        │     ├─> if combined ≤ max_bytes → return as-is
        │     └─> else truncate_diff (shortest-files-first, return TruncationNote)
        ├─> [build_review_prompt]                            (V1 fixed template)
        │     └─> interpolate diff between [BEGIN DIFF] / [END DIFF]
        ├─> [submit_agent_turn]                              (existing entry point)
        │     └─> agent processes prompt, streams response into conversation
        └─> [send_telemetry_from_ctx]                        (existing)
              └─> SlashCommandAccepted { command_name: "/review", ... }

Agent response renders inline in the conversation
  └─> User submits a follow-up turn (normal agent flow)
        └─> conversation context includes the original `/review` prompt + diff;
            no special re-invocation of the slash command.
```

## Risks

- **Diff payload as a security surface.** The diff is sent to the agent provider verbatim, including any secrets a developer accidentally committed locally (`.env` files, tokens in fixtures, etc.). **Mitigation:** the V1 prompt is silent about this; the existing AI privacy posture applies. Long-term, the existing redaction infrastructure (used for command output) could be applied to diffs — tracked as a follow-up. Document the risk in the docs page.
- **Truncation hides important changes.** Sorting shortest-files-first preserves the most files but may drop a large, security-critical change. **Mitigation:** the `[truncated]` marker in the prompt makes the truncation visible to the user, who can re-invoke with a higher `max_diff_bytes` or re-narrow scope manually. A follow-up could let the user opt into a "longest files first" mode or per-file selection.
- **Untracked-file inclusion may surprise users.** A developer who runs `/review` after `git status` showing only modified files might not expect the untracked files in their workspace to be reviewed too. **Mitigation:** the prompt's diff content visibly contains the new-file diffs; the user can see what was included. If user feedback suggests this is too aggressive, V2 can gate untracked inclusion behind a setting.
- **`Availability::HAS_UNCOMMITTED_CHANGES` cache staleness.** If the file-tree observer is debounced, the palette might briefly show `/review` enabled when it should be disabled (or vice versa). **Mitigation:** the dispatch arm re-checks `gather_uncommitted_diff` and shows a graceful toast if there are no changes — invariant 4's "found no uncommitted changes" path handles the race.
- **Large repos with many untracked vendored files.** A user who hasn't `git ignore`d their `node_modules` or `target/` would see a 1GB "diff" gathered. **Mitigation:** `git ls-files --others --exclude-standard` already respects `.gitignore`; the `--exclude-standard` flag excludes default ignores. The truncation cap is the second line of defense.

## Follow-ups (out of this spec)

- Commit-range review: `/review HEAD~3..HEAD`, `/review main..feature`. Argument syntax + Availability flag updates.
- User-customizable review prompt template (per-repo `.warp/review-template.md`).
- Per-file scope: `/review src/auth/`.
- Streaming summary→deep-dive (multi-turn agent dialogue rather than one big turn).
- Redaction layer for diff payloads sent to the agent provider, mirroring the existing command-output redaction for shell output.
- "Re-review" affordance in the conversation header that re-runs `/review` with the *current* working tree state without retyping.
