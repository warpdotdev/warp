# Spec: Default factory via a local `~/.warp/factory` config

Linear: [SAL-70](https://linear.app/warpdotdev/issue/SAL-70/default-factory-local-warpfactory-config-readwritten-by-factory-skills) · Estimate: L · Type: feature
Originating thread: https://warpdev.slack.com/archives/C0BDQDW8V5E/p1786026333909699
Code references in this spec are commit-pinned to `warpdotdev/warp@7a6044bd5377d708ab1d3767ece505a49d232aed` unless a reference explicitly points at the open PR #14793 branch.

## Why this spec exists

`~/.warp/factory` is a **net-new, durable, user-facing on-disk contract** that at
least three independently-shipped consumers must implement **identically**:

1. the Warp native (GUI) app,
2. the Warp headless TUI, and
3. each third-party harness plugin repo (`warpdotdev/claude-code-warp`,
   `warpdotdev/codex-warp`, and the gemini/opencode equivalents), which ship
   through the `oz-harness-support` platform plugin.

They ship on separate cadences from separate repos. If the file path, format,
schema, precedence, and read/write semantics are not pinned **before** they
build, they will diverge and a user's default will behave differently depending
on which surface they are in. The point of this spec is to pin that contract
precisely and checkably. It is a shared-contract spec first and an
implementation plan for `warpdotdev/warp` second.

## Problem

Today every Factory MCP workflow starts by resolving a factory: the `factory-mcp`
bundled skill instructs the agent to run `list_factories → pick the factory →
use its uid as factory_uid`, and *every* task-level tool needs that `factory_uid`
(factory-mcp `SKILL.md` lines 49–53, 116, 222–223, on the open PR #14793 branch
`factory/factory-mcp-bundled-skill`). For a user who almost always uses one
factory, that discovery round-trip is wasted turns on every fresh session — "the
agent sometimes takes a few turns to find the right factory."

The fix is a **default factory**, in the spirit of `gcloud config set project`:
persist the user's chosen factory locally and skip the discovery turn when it is
set.

## Scope and non-goals

In scope (this spec / the `warpdotdev/warp` implementation it drives):
- Define the `~/.warp/factory` on-disk contract (path, format, schema,
  precedence, read/absent/malformed/stale behavior, write semantics) as a
  **consumer-agnostic** contract.
- Implement that contract in `warpdotdev/warp`: a channel-aware path helper, a
  bundled-skill template variable exposing it, and edits to the `factory-mcp`
  bundled skill so the native app **and** the TUI read the default (skipping
  `list_factories`) and write it (confirm-first).

Explicitly **not** in scope:
- **Server-side injection** of the default into agent context, or any
  cloud-backed user setting. The requester proposed this himself and withdrew it
  ("Don't do the serverside injection, just do what zach bai is suggesting").
  Do not scope, design, or recommend it.
- **The third-party harness implementation.** Those changes land in the separate
  marketplace plugin repos (`claude-code-warp`, `codex-warp`, …) via the
  `oz-harness-support` plugin; those repos are not in this workspace and are a
  **follow-on**. This spec nonetheless defines the contract they must honor and
  calls out the 3p obligations explicitly (see "The shared on-disk contract" and
  "Third-party harness obligations").
- **Re-specifying the `factory-mcp` skill itself** — that skill is PR #14793
  (APP-5217). This work extends it.
- A general Warp-factory **preferences directory**. Zach Bai's broader
  "preferences dir" framing is a future extension; this ticket ships only the
  default factory, but the schema is designed (via the preserve-unknown-keys
  rule below) so that growth is not a breaking migration.

## Key design choices

> **Update — Option A approved (requester: "Do A").** Decision 3 below originally
> kept all file I/O in skill prose with only a Rust path helper. On review, the
> requester approved reversing it so the read/write/resolve logic is real,
> unit-tested Rust invoked in production, because the whole file contract was
> otherwise unverifiable by a deterministic test. The current design ships a
> `warp_core::factory_config` module plus a `oz factory default get|set|clear`
> CLI subcommand that the skill (and, later, 3p harnesses) invoke. The six
> behavior invariants below are unchanged and remain the normative contract.

1. **One JSON file, minimal schema, forward-compatible.** `~/.warp*/factory/config.json`
   holding a single default today, with a **normative preserve-unknown-keys rule**
   so it can grow into a preferences directory later without a breaking change.
2. **Home-based, channel-aware, GUI+TUI-shared path** via `warp_home_config_dir()`
   — **not** `config_local_dir()`. This is a correction of the triage pointer and
   is load-bearing: only the home-based helper yields the *same* file for the GUI,
   the TUI, and an external 3p harness on every platform (see "Design
   alternatives → Path helper").
3. **A canonical Rust implementation, invoked via a CLI (revised, Option A).**
   Warp ships the read/resolve/write logic as a real `warp_core::factory_config`
   module and exposes it through a `oz factory default get|set|clear` CLI
   subcommand. The `factory-mcp` skill invokes that CLI instead of hand-rolling
   file I/O, so native, TUI, and 3p harnesses (which already shell out to the
   Oz CLI via `harness-support`) all go through one canonical, tested path. This
   supersedes the original "agent does the I/O via skill prose; a Rust helper
   would be dead code" decision: the module is live production code with a real
   caller, which is what makes the file contract deterministically testable. 3p
   harnesses that cannot invoke the CLI still honor the same file contract
   directly.
4. **Explicit request always wins; the default only fills the gap; management
   flows ignore it.**
5. **Never pin a default silently** — writes are confirm-first only.

## == PRODUCT ==

### Summary

Let a user set a default factory once, stored locally in
`~/.warp*/factory/config.json`, so subsequent Factory MCP workflows use it and
skip the interactive `list_factories` pick. When no default is set, behavior is
unchanged. The same on-disk contract is shared by the native app, the TUI, and
(via a follow-on) third-party harnesses.

### Behavior (numbered, testable invariants)

From the user's / consumer's point of view:

1. **Default set → discovery skipped (happy path).** When
   `~/.warp*/factory/config.json` exists and contains a valid
   `default_factory_uid`, a Factory MCP workflow that needs a factory uses that
   uid directly as `factory_uid` and does **not** call `list_factories` first.
2. **No default set → unchanged (backward compatible).** When the file is
   **absent**, behavior is exactly as today: the skill falls back to
   `list_factories → pick → use uid`, with no new message and no error.
3. **Explicit factory overrides the default.** When the user's request names a
   specific factory, that explicit choice is always used, regardless of any
   stored default. The default only supplies `factory_uid` when the request does
   not name one.
4. **Management / discovery flows ignore the default.** Workflows whose purpose
   is to enumerate or change factories — listing all factories, switching the
   default, or setting/clearing the default — always operate on the real
   `list_factories` result and never short-circuit on the stored default.
5. **Stale default (deleted or not visible) → surface once, then fall back.**
   When a stored `default_factory_uid` no longer resolves to a factory visible to
   the current account, the agent tells the user once that the saved default is no
   longer available, falls back to `list_factories` discovery, and offers to
   update or clear the saved default. It does **not** silently re-run discovery.
6. **Malformed file → warn, preserve, fall back.** When the file exists but is
   unreadable (invalid JSON, or present but missing/empty `default_factory_uid`,
   or wrong shape), the agent warns the user that the config is unreadable, falls
   back to `list_factories`, and does **not** overwrite or delete the file
   (never destroy a hand-edited file behind the user's back).
7. **Writing the default is confirm-first — never silent.** The default is
   written only when the user **explicitly** asks to set/remember a factory, or
   after the user picks a factory via `list_factories` and answers **yes** to an
   optional "want me to remember this as your default?" prompt. A default is
   **never** auto-pinned from first use.
8. **Set/clear are user-visible and idempotent.** Setting a default writes
   `default_factory_uid` (and the optional advisory name); clearing removes the
   default (behavior reverts to invariant 2). Re-setting the same default is a
   successful no-op.
9. **Unknown keys survive a write.** Any keys in the file the current consumer
   does not recognize are ignored on read and **preserved** on write, so a
   newer consumer's additional preferences are never dropped by an older one.
10. **Native + TUI parity.** Invariants 1–9 hold identically in the native app
    and the TUI, reading and writing the **same** file. (The TUI is not
    read-only.)
11. **Advisory name never drives resolution.** `default_factory_uid` is the only
    field used to resolve the factory. A stored human-readable name is advisory —
    used only to tell the user "using your default factory X" — and may be stale
    if the factory was renamed; it is never used to look up or match a factory.

## == TECH ==

### Context: how this area works today

- **Factory MCP is a built-in MCP server**, `warp-factory`
  (`app/src/ai/mcp/builtin.rs:41` @ `7a6044b`), attached automatically for
  logged-in users and gated by `FeatureFlag::FactoryMcp`
  (`crates/warp_features/src/lib.rs:937` @ `7a6044b`, currently in
  `DOGFOOD_FLAGS`). Interactive GUI/TUI clients attach it via
  `TemplatableMCPServerManager::sync_builtin_servers`; CLI/cloud runs attach it
  per-run in `AgentDriver::builtin_factory_mcp_for_run`
  (`app/src/ai/agent_sdk/driver.rs:1308` @ `7a6044b`). The app never chooses a
  `factory_uid`; the **agent** does, at tool-call time, following the skill.
- **The `factory-mcp` bundled skill** drives the workflows. It is added by the
  **open** PR [#14793](https://github.com/warpdotdev/warp/pull/14793) on branch
  `factory/factory-mcp-bundled-skill` as
  `resources/bundled/skills/factory-mcp/SKILL.md` (+ `references/factory-mcp-tools.md`),
  registered in `app/src/ai/skills/bundled.rs`. Its factory-resolution step is
  the `list_factories → pick → use uid as factory_uid` flow (SKILL.md lines
  49–53, 116, 222–223 on that branch). **This PR is not yet merged** — see
  "Sequencing dependency & risks".
- **Bundled skills are rendered with handlebars template variables** at load
  time: `read_bundled_skills` calls `handlebars::render_template(&skill.content, &context)`
  (`app/src/ai/skills/bundled.rs:436` @ `7a6044b`), where `context` comes from
  `build_bundled_skill_context` (`bundled.rs:465` @ `7a6044b`). That map already
  exposes concrete config paths to skills, e.g. `{{gui_mcp_config_file_path}}`,
  `{{tui_mcp_config_file_path}}`, `{{settings_file_path}}`. This is the
  established mechanism to give the skill an exact, channel-aware path.
- **Path helpers live in `crates/warp_core/src/paths.rs`** (@ `7a6044b`):
  - `config_local_dir()` (`paths.rs:155`) — the **GUI's** non-portable config
    dir. On macOS it is `~/.warp*`; on Linux/Windows it is the platform
    XDG/AppData project dir (**not** `~/.warp`). This is what triage pointed at.
  - `tui_config_local_dir()` (`paths.rs:226`) — the **TUI's** separate dir
    (`~/.warp_cli*` on macOS; `config_local_dir()/cli` elsewhere). Deliberately
    distinct from the GUI's so the two never clobber each other.
  - `warp_home_config_dir()` (`paths.rs:66`) — a **home-based, channel-aware**
    `~/.warp*` dir on **all** platforms, "Warp-authored, user-facing config …
    in the home directory". It is what already backs `~/.warp*/skills`
    (`warp_home_skills_dir`, `paths.rs:70`) and `~/.warp*/.mcp.json`
    (`warp_home_mcp_config_file_path`, `paths.rs:74`). Crucially it is a function
    of channel + data-profile only, so the **GUI and TUI resolve it to the same
    directory**.
- **The channel suffix** comes from `warp_home_config_dir_name()`
  (`paths.rs:51`): `.warp` for Stable/Preview, `.warp-dev`, `.warp-local`,
  `.warp-oss`, `.warp-integration`, plus an optional `-{data_profile}` suffix.

### The shared on-disk contract (the deliverable all consumers honor)

- **Location.** `<warp home config dir>/factory/config.json`, where
  `<warp home config dir>` is the channel-aware home directory
  (`~/.warp` for stable, `~/.warp-dev`, `~/.warp-local`, … otherwise) — the same
  `.warp*` root that already holds `skills/` and `.mcp.json`. This directory is
  identical for the native app and the TUI.
- **Format.** UTF-8 **JSON**, one object at the top level.
- **Schema (v1).**
  ```json
  {
    "default_factory_uid": "fac_abc123",
    "default_factory_name": "Acme Backend Factory"
  }
  ```
  - `default_factory_uid` (string, **required for a usable default**) — the
    authoritative value, used directly as `factory_uid`. Its absence, emptiness,
    or a non-string value means "no usable default" and is treated as the
    malformed/absent path per invariants 2/6.
  - `default_factory_name` (string, **optional, advisory**) — a human-readable
    label so a consumer can say "using your default factory X" without a lookup.
    **Never** used to resolve or match a factory; may be stale after a rename.
  - **Unknown/extra keys** — ignored on read, and **preserved on write**
    (invariant 9). This preserve-on-write rule is normative, not an aside: it is
    what lets the directory grow into a preferences store later without a breaking
    migration. A consumer writing the file must read-modify-write, retaining every
    key it does not own.
- **Read semantics.** Absent file → no default, unchanged behavior, no message
  (invariant 2). Present + valid → use `default_factory_uid` (invariant 1).
  Present + unreadable/invalid shape → warn + preserve + fall back (invariant 6).
  Valid uid that does not resolve to a visible factory → surface once + fall back
  + offer to update/clear (invariant 5).
- **Write semantics.** Confirm-first only (invariant 7): explicit "set/remember"
  request, or an affirmative answer to the post-pick prompt. Writes create the
  `factory/` directory if missing, are atomic enough to avoid a torn file
  (write-temp-then-rename is the recommended pattern), preserve unknown keys, and
  are idempotent for an unchanged value (invariant 8).
- **Precedence.** Explicit factory in the request > stored default > interactive
  discovery (invariants 3, 1, 2). Management/discovery flows bypass the default
  entirely (invariant 4).

### Proposed changes in `warpdotdev/warp`

Ordered, each independently testable:

1. **Path helpers (`crates/warp_core/src/paths.rs`).** Add
   `factory_config_dir()` → `warp_home_config_dir().map(|d| d.join("factory"))`
   and `factory_config_file_path()` → `factory_config_dir().map(|d| d.join("config.json"))`,
   mirroring the existing `warp_home_skills_dir` / `warp_home_mcp_config_file_path`
   helpers. These are `Option<PathBuf>` (home dir may be unresolvable) and are
   channel-aware and GUI/TUI-shared by construction.
2. **Template variable (`app/src/ai/skills/bundled.rs`).** Add a
   `{{factory_config_file_path}}` entry to `build_bundled_skill_context`
   (`bundled.rs:465`), rendered from `factory_config_file_path()` via the existing
   `display_optional_path` helper, and document it in the doc-comment list at
   `bundled.rs:449`. This gives the `factory-mcp` skill body the exact
   channel-aware path in both the GUI and TUI processes (which both run
   `read_bundled_skills`).
3. **Canonical config module (`crates/warp_core/src/factory_config.rs`, Option A).**
   A `FactoryConfig` serde struct with a `#[serde(flatten)]` extra-keys map
   (preserve-unknown-keys), plus `resolve_at`/`set_default_at`/`clear_default_at`
   (path-injectable, for tests) and `resolve_default`/`set_default`/`clear_default`
   (over the real `factory_config_file_path()`). Reads return absent/parsed/
   malformed; writes are atomic (temp-then-rename), create the `factory/` dir,
   preserve unknown keys, and refuse to clobber a malformed file; resolution is
   uid-authoritative with the name advisory only. Unit-tested against a real
   filesystem (all six invariants).
4. **CLI subcommand (`crates/warp_cli/src/factory.rs` + `app/src/ai/agent_sdk/factory.rs`).**
   `oz factory default get|set|clear`, gated on `FeatureFlag::FactoryMcp`, is the
   live caller of the module. `get` prints the resolved default as JSON (`{}` when
   unset/malformed, with a stderr warning on malformed); `set`/`clear` write
   confirm-first (the skill decides when to invoke them). This is the one
   canonical path native, TUI, and 3p harnesses (via `harness-support`) share.
5. **`factory-mcp` skill edits (`resources/bundled/skills/factory-mcp/SKILL.md`,
   on top of PR #14793).** Extend the factory-resolution guidance so the skill,
   before `list_factories`, runs `oz factory default get` and honors the result
   (invariant 1), unless the request names a factory (invariant 3) or the
   workflow is a management/discovery flow (invariant 4); handles absent
   (invariant 2), malformed (invariant 6), and stale (invariant 5) exactly per
   the behavior invariants; and adds a confirm-first "set / remember / clear"
   flow via `oz factory default set|clear` (invariants 7–9). The skill invokes
   the CLI rather than hand-rolling file I/O. The `{{factory_config_file_path}}`
   template variable is retained to document the on-disk location for 3p.
6. **No feature-flag change** beyond the existing `FeatureFlag::FactoryMcp` gate
   that already governs the skill, MCP server, and the new CLI subcommand.

### Design alternatives

- **Path helper — `config_local_dir()` (triage's pointer) vs
  `warp_home_config_dir()` (chosen).** `config_local_dir()` is the GUI's dir and
  resolves to a *different* location than the TUI's, and on Linux/Windows it is an
  XDG/AppData path, not `~/.warp`. Using it would make the "one shared file"
  contract false across frontends and platforms, defeating the whole point.
  `warp_home_config_dir()` is home-based, channel-aware, and identical for GUI and
  TUI on every platform, and it matches the existing `~/.warp*/skills` and
  `~/.warp*/.mcp.json` conventions — so a 3p harness can target the same literal
  path. **Chosen: `warp_home_config_dir()`.**
- **How the native/TUI skill obtains the path — handlebars template variable
  (chosen) vs hardcoded literal in prose.** A template variable
  (`{{factory_config_file_path}}`) reuses the exact mechanism already used for
  `{{gui_mcp_config_file_path}}` etc., so channel/profile suffixes and platform
  differences are resolved by Rust rather than re-derived in prose. A hardcoded
  literal would be wrong on non-stable channels and non-macOS platforms. **Chosen:
  template variable** for warp's own skill; the **literal path contract** is what
  3p harnesses honor (they have no handlebars).
- **File shape — single minimal `config.json` (chosen) vs a namespaced
  preferences object now.** A namespaced/general-preferences schema up front
  over-designs for keys we cannot yet name. The minimal single-purpose file plus
  the normative preserve-unknown-keys rule gives the same forward-compatibility
  without the speculative structure. **Chosen: minimal + preserve-unknown-keys.**
- **Format — JSON (chosen) vs TOML.** Warp local config elsewhere uses TOML, but
  JSON is the lowest-friction cross-consumer contract for the 3p plugin repos to
  read/write identically and matches the `.mcp.json` neighbor. **Chosen: JSON.**
- **Who does the file I/O — agent via skill prose vs a Rust module behind a CLI
  (revised to the latter, Option A).** The original choice kept I/O in skill
  prose with only a Rust path helper, reasoning a read/write helper would be
  dead code. That left the file contract (round-trip, preserve-unknown-keys,
  malformed non-destruction, absent no-op) unverifiable by a deterministic test.
  On review the requester approved **Option A**: ship a real `warp_core::factory_config`
  module and a `oz factory default` CLI that the skill invokes, giving the module
  a real production caller and one canonical path across native, TUI, and 3p
  (which shell out to the Oz CLI via `harness-support`). **Chosen: Rust module +
  CLI** (unit-tested; live). 3p harnesses that cannot invoke the CLI still honor
  the same file contract directly.
- **Write trigger — confirm-first, explicit-or-prompt (chosen) vs auto-pin first
  used factory.** Auto-pinning is explicitly ruled out by the requester
  ("do not get a default silently pinned behind their back"). **Chosen:
  confirm-first** (explicit set, or affirmative post-pick prompt).
- **Stale default — surface-once + fall back (chosen) vs silent fall back.**
  Silent fall back leaves the user confused about why their default "stopped
  working" and silently re-burns the discovery turns the feature exists to
  remove. **Chosen: surface once, fall back, offer to update/clear.**
- **Malformed file — warn + preserve + fall back (chosen) vs warn + rewrite.**
  Rewriting risks clobbering a file the user hand-edited with a typo. **Chosen:
  warn + preserve.**

### Third-party harness obligations (follow-on repos — contract only)

The 3p plugin repos (`claude-code-warp`, `codex-warp`, gemini/opencode
equivalents), shipping via `oz-harness-support`, implement the *same* contract in
their own skills. To stay identical they must honor:
- the **path**: `<channel-aware ~/.warp* home dir>/factory/config.json` (they
  cannot call `warp_home_config_dir()`, so they use the literal channel-aware
  path);
- the **format** (JSON) and **schema** (v1 above), with `default_factory_uid`
  authoritative and `default_factory_name` advisory only;
- the **precedence** (explicit > default > discovery) and management-flow bypass;
- **absent / malformed / stale** handling per invariants 2, 6, 5;
- **confirm-first write** per invariant 7 and **preserve-unknown-keys** per
  invariant 9.

Open item flagged for the 3p follow-on (out of scope here, recorded so it is not
lost): a child harness needs to know which channel-suffixed `~/.warp*` dir to
read. The cleanest cross-consumer approach is for Warp to **expose the resolved
factory config path to child harnesses** (e.g. an environment variable set when
launching the harness) so 3p plugins do not re-derive channel logic. The 3p spec
should decide this; this spec only defines the on-disk contract.

### Sequencing dependency & risks

- **PR #14793 is open, not merged.** The `factory-mcp` skill this work extends is
  not on `master` yet. The implementation must **base its branch on / land after
  #14793** (extend the same skill file), not fork a second copy. If #14793's skill
  structure changes before merge, re-read it from its branch (GitHub is the source
  of truth) before editing. This is the top risk: mitigate by rebasing on the
  merged skill.
- **Contract drift across the three consumers** is the risk this spec exists to
  prevent. Mitigation: the schema + behavior invariants above are the single
  source both warp and the 3p repos cite.
- **Path/channel mistakes** (writing to the GUI-only or XDG path, or the wrong
  channel suffix) would silently split the file per surface. Mitigation: the
  `warp_home_config_dir()`-based helper + its unit tests (below).
- **Blast radius is low**: no default set → byte-for-byte unchanged behavior
  (invariant 2); the change is additive and flag-gated by the existing
  `FactoryMcp` flag.

### Open questions resolved

- Directory scope → **minimal single `config.json`** now + normative
  preserve-unknown-keys (requester, q1).
- Format & identity → **JSON**; `default_factory_uid` authoritative,
  `default_factory_name` optional/advisory, never used for resolution
  (requester, q2).
- Precedence → **explicit request wins; default fills the gap; management flows
  ignore it** (requester, q3).
- Stale default → **surface once + fall back + offer update/clear** (requester,
  q4).
- Absent vs malformed → **absent = silent unchanged; malformed = warn + preserve
  + fall back** (requester, q5).
- Write semantics & scope → **confirm-first (explicit set or affirmative
  post-pick prompt), never auto-pinned; both native and TUI can write**
  (requester, q6).
- Path helper (`config_local_dir()` vs `warp_home_config_dir()`) → **assumption**:
  resolved to `warp_home_config_dir()` on the codebase evidence that only it is
  GUI/TUI-shared and home-based on all platforms; this corrects the triage
  pointer. Implementer/reviewer should confirm this is acceptable (it changes the
  helper named in the ticket, not the user-facing `~/.warp/factory` outcome).
- Whether a Rust read/write helper is needed → **yes, revised to Option A**
  (requester: "Do A"). A real `warp_core::factory_config` module + a
  `oz factory default` CLI now own read/write/resolve so the file contract is
  deterministically testable; the module is live via the CLI caller.

## Validation & verification criteria (must ALL pass before merge)

Backend/config-path portion (deterministic, `warpdotdev/warp`):

1. **Path resolution unit tests** (`crates/warp_core/src/paths.rs` tests): new
   `factory_config_dir()` / `factory_config_file_path()` resolve under the
   channel-aware `~/.warp*` home dir (e.g. `.warp/factory/config.json` for
   stable, `.warp-dev/factory/config.json` for dev), and are **equal for the GUI
   and TUI resolution** (same value regardless of frontend). A test asserts the
   path sits under `warp_home_config_dir()`, not `config_local_dir()` or
   `tui_config_local_dir()`. Fails before the helper is added, passes after.
2. **Template-variable test** (`app/src/ai/skills/bundled.rs` tests, alongside the
   existing `build_bundled_skill_context` coverage): the context map contains
   `factory_config_file_path` and its value equals `factory_config_file_path()`
   rendered via `display_optional_path`; a skill body containing
   `{{factory_config_file_path}}` renders to that path. This is the regression
   test for the new variable.
3. **Config-module file-contract tests** (`crates/warp_core/src/factory_config_tests.rs`,
   real filesystem): round-trip (uid persisted verbatim), preserve-unknown-keys
   across `set` and `clear`, malformed file surfaced and never overwritten/
   deleted, absent file is a silent no-op (no file created), advisory name never
   participates in resolution, and the real `factory_config_file_path()` location
   round-trips under a temp `$HOME`. These cover invariants 5–9 deterministically
   (the Option A caller made this possible).
4. **Skill-doc change is exempt from a test.** The edits to
   `resources/bundled/skills/factory-mcp/SKILL.md` are `skill-doc-only` per
   `factory-verification` (a test would assert a sentence exists and break on
   rewording). No test is added for the prose; the config module (crit. 3), path
   helper (crit. 1), and template variable (crit. 2) it relies on carry the
   regression coverage. Record this skip category in the PR body.
4. **Repository checks gate (scope-proportional).** From the repo root, the
   documented checks pass over every touched package (`crates/warp_core`,
   `app`): `./script/format` (or `./script/presubmit`), `cargo clippy --workspace
   --all-targets --all-features --tests -- -D warnings`, and
   `cargo nextest run -p warp_core` plus the touched `app` skills module tests.
   The PR's CI is the full-suite backstop; a red required check outranks a local
   pass.

Behavioral criteria (verify the invariants; checked by the skill's exercised
behavior — see crit. 8 for how):

5. **Skip-on-default (invariant 1) & unchanged-on-absent (invariant 2).** With a
   valid `config.json` present, a factory workflow proceeds without a
   `list_factories` call and uses the stored `default_factory_uid`; with the file
   absent, the same workflow calls `list_factories` exactly as before and emits no
   new message.
6. **Precedence & management bypass (invariants 3, 4).** A request naming a
   specific factory uses that factory despite a stored default; a
   list/switch/set/clear-default workflow calls `list_factories` and does not
   short-circuit.
7. **Failure modes (invariants 5, 6, 9).** Stale uid → exactly one user-facing
   "default no longer available" message, then discovery, plus an offer to
   update/clear, and the file is left intact. Malformed file → one "config
   unreadable" warning, discovery, and the file is **not** modified. A write that
   updates the default **preserves** a pre-existing unknown key in the file.
8. **Confirm-first write (invariant 7, 8).** No workflow writes `config.json`
   without an explicit set request or an affirmative answer to the post-pick
   prompt; first-use never auto-pins. Setting then clearing round-trips
   (set writes the uid; clear removes it; re-set of the same value is a no-op).
9. **Native + TUI parity (invariant 10).** Invariants 1–9 are exercised in both
   the native app and the TUI against the same file location, confirming reading
   **and** writing parity (the TUI is not read-only).
10. **User-facing visual proof (mandatory, `factory-verification`).** Because the
    discovery-turn-skip and the confirm-first prompts are observable in the running
    agent, capture computer-use visual proof (video by default) showing: (a) with a
    default set, a factory workflow proceeds without a `list_factories` pick turn;
    (b) setting a default via the confirm-first flow; and (c) the stale/malformed
    message path. Attach the proof to SAL-70 and to the PR body. Missing or
    spec-mismatched proof is a blocking reject at review.
