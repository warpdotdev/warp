# Spec: GitHub webhook — clone the originating repo and authorize the agent token for it (warp client/worker side)

Cross-repo feature. This spec owns the **warp** (client/worker) side only. The
sibling **warp-server** spec owns populating, persisting, and returning the new
`additional_source_repos` field on the agent run's config snapshot, plus the
named-agent token scoping/validation. That server spec is already committed at
<https://github.com/warpdotdev/warp-server/pull/13180>
(file `.agents/specs/REV-1759-github-webhook-clone-origin-repo.md`). Treat the
server behavior as this spec's **input contract/dependency** and do not spec
warp-server internals here.

Linear: <https://linear.app/warpdotdev/issue/REV-1759/github-webhook-clone-the-originating-repo-and-authorize-the-agent>
Originating thread: <https://slack.com/archives/C0BDQDW8V5E/p1784754450281239>

All file references are pinned to warp commit `47234cc6d8dcd29e06a06162c95ad38fddfa9298`
(base branch `master`).

## Summary

When a GitHub webhook event triggers a Factory automation run, the server
(warp-server spec PR #13180) now records the originating repo on the task's
`AgentConfigSnapshot` as an `additional_source_repos` list (provider-neutral
`SourceRepo` shape: `code_forge`, `owner`, `repo`) and returns it via
`GET /agent/runs/{id}` inside `agent_config` (serde alias `agent_config` on
`AmbientAgentTask`). The warp worker currently clones only
`environment.effective_repos()` and is unaware of the origin repo. This spec
makes the worker consume the server-provided `additional_source_repos`, merge
and **de-duplicate** them with the environment's effective repos, and use the
resulting union for **both** repo cloning and codebase indexing, while keeping
the persisted `AmbientAgentEnvironment`/GSO **immutable** (the merge is a
runtime computation, never written back to the cloud object). Credentials for
the union are seeded server-side and delivered via the existing
`taskGitCredentials` path — the client mints no tokens — so this spec also pins
the expected behavior when a repo in the cloned union lacks working credentials.

## Key design choices

1. **Carry the origin repos on the config snapshot, not on the environment.**
   The server already persists `additional_source_repos` on the per-task
   `AgentConfigSnapshot`. The client reads it off the fetched
   `AmbientAgentTask.agent_config_snapshot` and threads it into the driver as a
   runtime value, then merges it with `environment.effective_repos()` at
   prepare-environment time. The persisted `AmbientAgentEnvironment` GSO is
   **never mutated** — no origin repo is written back to the environment's
   `source_repos`/`github_repos`. (Mutating the GSO would leak per-run state
   into a shared, revision-tracked cloud object and break environment reuse.)
2. **De-dupe by `owner/repo`, case-insensitive, forge-aware.** The dedupe key
   is `(code_forge, lower(owner), lower(repo))`. An origin repo already present
   in the environment (same forge, same lower-cased owner/repo) is **not**
   cloned twice. Different forges are distinct keys even with the same
   owner/repo (defensive; the server enforces single-forge consistency). Repo
   name collisions across distinct owners are NOT deduped (e.g. `a/widget` and
   `b/widget` both clone, into sibling dirs named by `repo`).
3. **One merge, used by clone AND index.** The union slice is computed once and
   feeds `clone_repos`, `register_cloned_repo`, and codebase indexing, so the
   agent never sees a repo that wasn't cloned and never clones a repo that
   won't be indexed. The "single repo auto-cd" behavior keys off the union
   length, not the environment repo count.
4. **Immutability via a passed slice, not a new GSO field.** Rather than adding
   a mutable `effective_repos` override on `AmbientAgentEnvironment`, the merge
   result is passed into `prepare_environment` (or an
   `effective_repos_with_additional` helper) as an owned `Vec<SourceRepo>`.
   `AmbientAgentEnvironment::effective_repos()` stays as-is and remains the
   source of truth for the environment's own repos. This keeps the GSO type
   pure and avoids a hidden per-run mutation seam.
5. **Credentials are the server's contract; the client surfaces errors, never
   partial clones.** The worker does not mint or widen tokens. It must treat
   the cloned union as the exact set the server-issued `taskGitCredentials`
   cover, and if a clone in the union fails (e.g. the origin repo's credentials
   weren't actually scoped, or the installation doesn't cover it), surface a
   clear, actionable error rather than silently dropping that repo and
   continuing with a partial environment.

## PRODUCT

Behavior invariants (consumer view — the "consumer" is the Factory automation
worker that fetches the task, prepares the environment, and runs the agent):

1. **Default / happy path.** A GitHub-webhook Factory automation run is fetched
   via `get_ambient_agent_task`. Its `agent_config_snapshot.additional_source_repos`
   contains exactly the originating repo (e.g.
   `{ code_forge: GITHUB, owner: "warpdotdev", repo: "warp" }`). The worker
   clones that repo **in addition to** every repo in the run's selected
   environment, and the agent's terminal session can read/clone it.
2. **De-duplication.** If the origin repo is already one of the environment's
   effective repos (same forge, same lower-cased `owner/repo`), it is cloned
   exactly once — never twice, never into a colliding directory. The merged
   union contains no duplicate `(forge, lower(owner), lower(repo))` keys.
3. **Immutability of the persisted environment.** Preparing an environment with
   additional repos does **not** mutate the resolved `AmbientAgentEnvironment`
   model or write any change back to Warp Drive. Re-reading the environment
   after the run returns the original `source_repos`/`github_repos` unchanged.
4. **Union used for indexing.** The merged union is the set registered with
   `DetectedRepositories` and indexed by the codebase index manager (Oz harness
   only). The agent's codebase context covers the origin repo too. A repo
   present in the union is indexed iff it would be indexed for an equivalent
   environment repo (non-sandbox, Oz harness).
5. **Single-repo auto-cd uses the union.** When the merged union contains
   exactly one repo, the worker auto-cd's into it (matching today's
   single-environment-repo behavior). With zero or 2+ repos in the union, no
   auto-cd occurs.
6. **Origin-only (no environment) runs.** When the task has
   `additional_source_repos` set but no `environment_id` (the server permits
   origin-only runs under its own gating), the worker still clones the union
   (here: just the origin repo) and prepares the environment off that union.
   The prepare path must not require a non-empty `environment.effective_repos()`.
   (Server-side rejection of origin-only *named-agent* runs is the server
   spec's concern; the client does not second-guess it.)
7. **Non-GitHub-webhook runs unaffected.** A run whose `agent_config_snapshot`
   has no `additional_source_repos` (or the field is absent/`None`) behaves
   exactly as today: the cloned/indexed set equals
   `environment.effective_repos()`.
8. **Credential/clone failure is surfaced, not silent.** If a clone in the
   union fails (credentials missing/insufficient for that repo, network, etc.),
   the run fails environment preparation with a clear error naming the repo, and
   does **not** proceed with a partial clone set. (The error type already
   exists: `PrepareEnvironmentError::CloneRepo`.)

## TECH

### Context (how it works today)

- The runtime agent config snapshot is `cloud_object_models::AgentConfigSnapshot`
  (`crates/cloud_object_models/src/scheduled_ambient_agent.rs:18` @ `47234cc`).
  It is re-exported as `crate::ai::ambient_agents::AgentConfigSnapshot`
  (`app/src/ai/ambient_agents/task.rs:7`) and surfaced to the server-API layer
  (`app/src/server/server_api/ai.rs:129`).
- `AmbientAgentTask` carries the snapshot as
  `agent_config_snapshot: Option<AgentConfigSnapshot>` with serde alias
  `agent_config` (`app/src/ai/ambient_agents/task.rs:168`), populated from
  `GET /agent/runs/{id}` via `get_ambient_agent_task`
  (`app/src/server/server_api/ai.rs:2185`).
- `SourceRepo` (provider-neutral: `code_forge: Option<CodeForge>`, `owner`,
  `repo`) already exists in `cloud_object_models::cloud_environment.rs:62` and
  is re-exported via `app/src/ai/cloud_environments/mod.rs:6`.
- `AmbientAgentEnvironment::effective_repos()` (`cloud_environment.rs:242`)
  resolves the environment's authoritative `source_repos` (or legacy
  `github_repos`) into a `Vec<SourceRepo>` with each repo's effective forge
  filled in via `with_default_code_forge`.
- The driver options hold the resolved environment:
  `AgentDriverOptions.environment: Option<AmbientAgentEnvironment>`
  (`app/src/ai/agent_sdk/driver.rs:301`), set by
  `AgentDriverRunner::resolve_environment` (`mod.rs:1425`).
- For `--task-id` runs, `AgentDriverRunner::fetch_secrets_and_attachments`
  (`mod.rs:1169`) fetches the `AmbientAgentTask` and currently reads only
  `agent_config_snapshot.harness` off it (`mod.rs:1295`–`:1310`); it does not
  read any repo field. `build_merged_config_and_task` (`mod.rs:356`) and
  `build_server_side_task` (`mod.rs:474`) construct the *local* CLI-side
  `AgentConfigSnapshot` and do not carry task-level additional repos either.
- Environment preparation lives in
  `app/src/ai/agent_sdk/driver/environment.rs`. `prepare_environment`
  (`environment.rs:57`) takes the `AmbientAgentEnvironment`, computes
  `environment.effective_repos()` (`environment.rs:67`), and feeds that slice
  to `prepare_environment_impl` (`environment.rs:105`) which calls
  `clone_repos` (`environment.rs:346`), `register_cloned_repo`
  (`environment.rs:454`), and `index_repo_codebase` (`environment.rs:559`).
  `clone_repos`/`build_parallel_clone_command` clone each repo into
  `working_dir/{repo.repo}` (`environment.rs:414`, `:304`).
- The single-repo auto-cd uses `single_repo_name(source_repos)`
  (`environment.rs:229`, `:659`).
- File-based MCP discovery and skill repo resolution both derive expected repo
  paths from `environment.effective_repos()` in `driver.rs:2271`–`:2295` and
  `:2375`. They consume the *environment* repos today; the union must be
  consistent with what actually got cloned so file-based MCP discovery and
  skill loading see the origin repo too.
- Git credentials for cloud runs are fetched via `get_task_git_credentials`
  (`mod.rs:846`) → `AIClient::get_task_git_credentials`
  (`app/src/server/server_api/ai.rs:1390`, GraphQL `taskGitCredentials`) and
  written by `configure_git_credentials` (`driver/git_credentials.rs`). The
  returned `GitCredential`s are host-scoped (`ai.rs:677`–`:685`), not
  per-repo, and are written once up front
  (`bootstrap_git_credentials_for_task`, `mod.rs:860`) plus refreshed
  periodically. The client does not choose which repos a credential covers —
  that's the server's job.

### Design alternatives

- **Where to merge: in `prepare_environment` vs. earlier in
  `build_driver_options_and_task`.** Merging inside `prepare_environment` keeps
  the union computation co-located with cloning/indexing and the single-repo
  auto-cd, and avoids threading a new field through `AgentDriverOptions`.
  Merging earlier (in `build_driver_options_and_task`) would let the union be
  visible to `file_based_mcp_discovery` and skill resolution without a second
  pass, but those already re-derive `environment.effective_repos()` from the
  stored environment. **Chosen: compute the union in
  `prepare_environment`** (or a thin helper it calls) and, to keep MCP/skill
  discovery consistent, also pass the additional repos into the driver options
  so the union is derived once and reused (see Proposed changes #2/#3). The
  environment GSO itself stays immutable.
- **API shape: new `prepare_environment` param vs.
  `effective_repos_with_additional` helper.** A new `additional_source_repos:
  Vec<SourceRepo>` parameter on `prepare_environment` is the most direct.
  Alternatively, add a free function
  `fn effective_repos_with_additional(env, additional) -> Vec<SourceRepo>` and
  have callers compute and pass the union. **Chosen: new parameter on
  `prepare_environment`** (owned `Vec<SourceRepo>`, defaulting to empty at
  call sites), plus a small `merge_repos_deduped` helper for the dedupe logic
  so it is unit-testable in isolation. This keeps the merge logic in one place
  and makes the "environment is immutable" guarantee obvious (the env is taken
  by value/cloned, not mutated).
- **Dedupe key: `(forge, owner, repo)` vs. `repo` only.** Deduping on `repo`
  alone would wrongly collapse `a/widget` and `b/widget` and cause a directory
  collision (`working_dir/widget`). Deduping on `(forge, lower(owner),
  lower(repo))` is correct and matches how the clone target is keyed
  (`working_dir/{repo.repo}`), while still collapsing the same repo across
  case variants (GitHub owners/repos are case-insensitive). **Chosen:
  `(code_forge, lower(owner), lower(repo))`.** Because the clone target dir is
  just `{repo.repo}` (not owner-qualified), the implementer must additionally
  assert/guard that the union contains no two repos with the same `repo` name
  but different owners — see Open questions resolved.
- **Carrying additional repos on `AgentDriverOptions` vs. re-reading the task.
  `AgentDriverOptions` already carries the resolved `environment`; adding an
  `additional_source_repos: Vec<SourceRepo>` field is symmetric and keeps the
  data flow explicit (populated once in `fetch_secrets_and_attachments`,
  consumed in `execute_run`/`prepare_environment`). Re-reading the task at
  prepare time would add a second server roundtrip and re-couple prepare to the
  API client. **Chosen: add the field to `AgentDriverOptions`.**
- **Local (non-`--task-id`) runs.** Local `warp agent run` invocations have no
  server task and therefore no server-provided `additional_source_repos`. The
  field defaults to empty there. **Chosen: empty default for local runs; no CLI
  flag to set it** (it is a server-populated, per-task field, not a user-facing
  knob — matching the server spec's "server-populated only" rule).

### Proposed changes

1. **Add the field to `AgentConfigSnapshot`.** In
   `crates/cloud_object_models/src/scheduled_ambient_agent.rs`, add to
   `AgentConfigSnapshot`:
   ```rust
   /// Extra repositories the worker should clone in addition to the
   /// environment's repos. Server-populated only (e.g. a GitHub webhook's
   /// originating repo); a user-supplied run_config value is ignored by the
   /// server. Persisted in ai_tasks.agent_config_snapshot and returned by
   /// /agent/runs/{id}.
   #[serde(default, skip_serializing_if = "Option::is_none")]
   pub additional_source_repos: Option<Vec<SourceRepo>>,
   ```
   (import `SourceRepo` from `crate::cloud_environment::SourceRepo`). Update
   `is_empty()` (`scheduled_ambient_agent.rs:133`) to include the new field.
   Because the field is `Option` with `skip_serializing_if = "Option::is_none"`,
   existing snapshots (and local configs) without it deserialize as `None` — no
   migration, no breakage. Update every struct-literal construction site that
   currently omits it to add `additional_source_repos: None` (or rely on
   `..Default::default()` where already used). Known literal sites:
   `app/src/ai/agent_sdk/mod.rs:411` (`build_merged_config_and_task`),
   `app/src/ai/agent_sdk/mod.rs:512` (`build_server_side_task`),
   `app/src/ai/agent_sdk/ambient.rs:439`,
   `app/src/ai/agent_sdk/config_file.rs:161` (`merge_with_precedence` — note
   this returns a `AgentConfigSnapshot` built from a `AgentConfigSnapshotFile`;
   set `None` since config files don't carry additional repos),
   `app/src/ai/agent_sdk/integration.rs:103`/`:417`,
   `app/src/ai/agent_sdk/schedule.rs:113`,
   `app/src/ai/agent_sdk/mcp_config_tests.rs:276`, and
   `app/src/ai/ambient_agents/task_tests.rs:11` (`make_task` uses
   `..Default::default()`, so no change needed there). The
   `AgentConfigSnapshotFile` (`config_file.rs:17`) is a strict,
   `deny_unknown_fields` file representation and intentionally does **not** get
   this field (additional repos are server-only).

2. **Thread the field into `AgentDriverOptions`.** In
   `app/src/ai/agent_sdk/driver.rs`, add to `AgentDriverOptions` (`:281`):
   ```rust
   /// Additional per-task repositories (e.g. a GitHub webhook's originating
   /// repo) to clone alongside the environment's repos. Server-populated via
   /// the task's agent config snapshot; empty for local runs.
   pub additional_source_repos: Vec<SourceRepo>,
   ```
   Destructure it in `AgentDriver::new` (`:630`) and store it on `Self` (new
   field `additional_source_repos: Vec<SourceRepo>`), mirroring `environment`.
   Update `new_for_test` (`:789`) to set it to `Vec::new()`.

3. **Populate the field from the fetched task.** In
   `AgentDriverRunner::fetch_secrets_and_attachments` (`mod.rs:1169`), when
   building the `(parent_run_id, conversation_id, harness, ...)` tuple from
   `task_metadata` (`mod.rs:1291`–`:1316`), also extract
   `task_metadata.agent_config_snapshot.as_ref().and_then(|c| c.additional_source_repos.clone()).unwrap_or_default()`
   and assign it to `driver_options.additional_source_repos`. For the
   new-task branch (`initialize_new_task`, `mod.rs:1123`), leave
   `additional_source_repos` empty (the field is set on `AgentDriverOptions`
   at construction in `build_driver_options_and_task` `:1038` to
   `Vec::new()`). For local runs (no task id) it stays empty.

4. **Merge + dedupe, then prepare the union.** Add a helper in
   `app/src/ai/agent_sdk/driver/environment.rs`:
   ```rust
   /// Merge environment repos with additional (task-level) repos, de-duplicating
   /// by `(code_forge, lower(owner), lower(repo))`. The environment is not
   /// mutated; the returned vec is the runtime union used for cloning/indexing.
   pub(super) fn merge_repos_deduped(
       environment_repos: Vec<SourceRepo>,
       additional_repos: Vec<SourceRepo>,
   ) -> Vec<SourceRepo> { ... }
   ```
   Dedupe key: `(code_forge, owner.to_lowercase(), repo.to_lowercase())`. First
   occurrence wins (environment repos first, then additionals not already
   present), preserving deterministic ordering. Add a debug-assert / soft guard
   that the resulting union has no two entries with the same `repo` (case
   sensitive) but different `(code_forge, owner)` — that would collide on the
   `working_dir/{repo}` clone target; log a warning and keep both (clone order
   is non-deterministic for the colliding dir, but this case is not produced by
   the server's single-owner enforcement). Then change `prepare_environment`
   (`environment.rs:57`) to accept an `additional_source_repos: Vec<SourceRepo>`
   parameter and compute `let source_repos = merge_repos_deduped(environment.effective_repos(), additional_source_repos);`
   before passing `source_repos` into `prepare_environment_impl` (unchanged
   signature). Update the single call site in `driver.rs:2303` to pass
   `me.additional_source_repos.clone()` (the field stored on `Self` in step 2).
   Because `prepare_environment_impl` already takes `&[SourceRepo]` and uses it
   for clone, register, index, and `single_repo_name`, the union automatically
   flows to all four.

5. **Keep file-based MCP discovery and skill loading consistent.** In
   `driver.rs:2271`–`:2295` (`environment_source_repos`) and `:2375`
   (`load_environment_skills`), derive the repo list from the same merged union
   used by `prepare_environment` rather than re-calling
   `environment.effective_repos()`. Concretely: compute the union once
   (`merge_repos_deduped(environment.effective_repos(), self.additional_source_repos.clone())`)
   before calling `prepare_environment`, pass the additionals into
   `prepare_environment`, and use the precomputed union for the
   `expected_repo_paths` passed to `setup_file_based_mcp_discovery` and for
   `environment_skill_repos`. This ensures MCP discovery and skill loading see
   the origin repo exactly when it was cloned.

6. **No credential changes on the client.** The client continues to call
   `get_task_git_credentials` and `configure_git_credentials` exactly as today.
   The server spec guarantees the issued credentials cover the deduped union
   (scoped token for the union). The client must not attempt to mint, widen, or
   per-repo-select credentials. The only client-side credential behavior change
   is an explicit guarantee in `prepare_environment`'s error path: a clone
   failure for any union repo returns `PrepareEnvironmentError::CloneRepo`
   (`environment.rs:36`) naming the repo(s), and the run aborts — no silent
   partial clone. (This is already the current behavior for environment repos;
   the spec makes it explicit for the union.)

### Open questions resolved

- **Dedupe key casing.** Resolved to `(code_forge, lower(owner), lower(repo))`.
  GitHub owner/repo names are case-insensitive and the server normalizes to the
  canonical casing, but the client dedupe must be case-insensitive to avoid a
  double-clone when the environment lists `Warpdotdev/Warp` and the webhook
  reports `warpdotdev/warp`. **Assumption to confirm:** GitLab full namespace
  paths are case-sensitive in practice; for GitLab the dedupe still lower-cases
  (acceptable because the server's single-forge + same-owner enforcement means
  a mixed GitHub/GitLab union does not occur in this feature's scope). If a
  future GitLab origin-repo feature needs case-sensitive dedupe, the helper can
  branch on `code_forge`.
- **Repo-name directory collision across distinct owners.** `clone_repos`
  clones into `working_dir/{repo.repo}` (not owner-qualified), so two union
  repos with the same `repo` name but different owners would collide on disk.
  The server spec enforces same-owner for the named-agent union, so this
  collision cannot arise from a GitHub-webhook run. Resolved to: the
  `merge_repos_deduped` helper logs a warning if it detects a same-`repo`
  collision across distinct `(forge, owner)` and keeps both (the second clone
  no-ops because the directory already exists — see `clone_repo`'s
  `dir_exists` skip at `environment.rs:422`). This is a defensive guard, not an
  expected path. **Assumption to confirm:** if directory-collision semantics
  ever need to be strict (fail the run), the implementer/reviewer should flag
  it and the spec will be reworked to owner-qualify the clone target.
- **Origin-only (no environment) runs.** Resolved to: the client prepares the
  environment off the union even when `environment` is `None`. Today
  `prepare_environment` is only called when `driver_options.environment` is
  `Some` (`driver.rs:2269`). For an origin-only run, the environment is `None`
  and `additional_source_repos` is non-empty. The client must still clone the
  origin repo. Concretely: when `environment` is `None` but
   `additional_source_repos` is non-empty, synthesize an environment prep using
   the additionals as the full repo set (no setup commands, no providers).
   **Assumption to confirm:** the server's current origin-only rejection (server
   spec §6) means this path is not exercised for named-agent GitHub-webhook
   runs yet, but the client must not crash if a future server change allows
   origin-only runs. If the server never ships origin-only support, this branch
   stays defensive and untested-by-integration but covered by a unit test.
- **Should the client validate the forge of `additional_source_repos`?**
  Resolved to **no client-side forge validation.** The server spec rejects
  forge-mismatched runs at creation (server spec §4). The client clones
  whatever the server sent using `SourceRepo::https_clone_url()`
  (`cloud_environment.rs:89`), which honors `code_forge` (defaulting to GitHub).
  Re-validating on the client would duplicate server logic and could diverge.
- **`is_empty()` and config-file merge.** Resolved: `additional_source_repos`
  participates in `AgentConfigSnapshot::is_empty()` (a snapshot with only
  additionals is non-empty), and `merge_with_precedence` always sets it to
  `None` (config files don't carry it; it's server-only). This means a local
  `--file` run never accidentally carries additional repos.

## Validation & verification criteria (must ALL pass before merge)

1. **Field added and round-trips.** `AgentConfigSnapshot` has
   `additional_source_repos: Option<Vec<SourceRepo>>` with json tag
   `additional_source_repos` (or matching snake_case) and
   `skip_serializing_if = "Option::is_none"`. A unit test marshals a snapshot
   with one origin repo and asserts the JSON contains
   `"additional_source_repos":[{"code_forge":"GITHUB","owner":"warpdotdev","repo":"warp"}]`,
   and that a snapshot without the field deserializes with the field as `None`.
   — `cargo nextest run -p cloud_object_models` (or `cargo test -p cloud_object_models`).
2. **`is_empty()` updated.** A unit test asserts a snapshot with only
   `additional_source_repos: Some(vec![..])` is **not** empty, and one with
   `None` (and all other fields `None`) is empty. — same package tests.
3. **Dedupe helper.** A unit test on `merge_repos_deduped` covers:
   (a) environment-only (additionals empty) → equals environment repos;
   (b) additionals-only (environment empty) → equals additionals;
   (c) origin repo already in environment (same forge, same owner/repo, mixed
   case) → appears once in the union;
   (d) origin repo not in environment → union has both, environment repo first;
   (e) two additionals with same lower(owner/repo) but different case → one
   entry;
   (f) distinct owners, same `repo` name → both kept (collision guard logs).
   — `cargo nextest run -p app --test …` or the `environment_tests.rs` module
   (`app/src/ai/agent_sdk/driver/environment_tests.rs`).
4. **Immutability of the environment.** A unit/integration test constructs an
   `AmbientAgentEnvironment` with `source_repos = Some(vec![a])`, calls the
   merge+prepare path with `additional_source_repos = vec![b]`, and after the
   call asserts the environment's `source_repos` is still `Some(vec![a])` and
   `effective_repos()` still returns `[a]` only. — `environment_tests.rs` (or a
   new test that drives `prepare_environment` with a stubbed terminal driver).
5. **Union is cloned.** A unit test (or an integration test using the
   `crates/integration` harness if practical) asserts that when the union is
   `[a, b]`, `clone_repos`/`build_parallel_clone_command` emits clone commands
   for both `a` and `b`, and when the origin equals an environment repo the
   command set contains that repo exactly once. The existing
   `parallel_clone_command_runs_repos_in_background_and_waits` test
   (`environment_tests.rs:39`) is extended with a union-derived repo list.
   — `cargo nextest run -p app --test …` / `environment_tests.rs`.
6. **Single-repo auto-cd uses the union.** A unit test asserts
   `single_repo_name` over the merged union returns `Some(repo)` when the union
   has exactly one repo (whether that repo came from the environment or the
   additionals) and `None` for 0 or 2+. — `environment_tests.rs`.
7. **`fetch_secrets_and_attachments` populates the field.** A unit test using a
   stub `AIClient` (the codebase already has stub `AIClient` impls in
   `app/src/ai/ambient_agents/spawn_tests.rs` and
   `app/src/ai/agent_sdk/driver/harness/claude_code_tests.rs` — follow that
   pattern) returns an `AmbientAgentTask` whose
   `agent_config_snapshot.additional_source_repos` is `Some(vec![origin])`, and
   asserts `driver_options.additional_source_repos == vec![origin]` after
   `fetch_secrets_and_attachments` completes. A second case with
   `additional_source_repos == None` asserts the field is empty. —
   `app/src/ai/agent_sdk/mod_tests.rs` or a new test file, using the existing
   test-support HTTP client helper (`test_support.rs`).
8. **Local runs default to empty.** A unit test on `build_merged_config_and_task`
   and `build_server_side_task` (or `AgentDriverOptions` construction in
   `build_driver_options_and_task`) asserts `additional_source_repos` is empty
   for a local run with no task id. — `mod_tests.rs`.
9. **Origin-only (no environment) prepares.** A unit test asserts that when
   `environment` is `None` and `additional_source_repos` is non-empty, the
   driver still attempts to clone the additionals (i.e. the prepare path is
   entered with the additionals as the repo set), and does not panic or silently
   skip. — `environment_tests.rs` or a driver-level test with a stubbed
   terminal driver.
10. **Credential/clone failure surfaces, not silent.** A unit/integration test
    asserts that when a clone in the union fails, `prepare_environment` returns
    `PrepareEnvironmentError::CloneRepo` naming the failing repo(s), and the
    run aborts (no partial-clone continuation). — `environment_tests.rs` (using
    a terminal driver stub that makes `git clone` exit non-zero).
11. **File-based MCP discovery and skill repos see the union.** A unit test (or
    an assertion in an existing integration test) verifies the
    `expected_repo_paths` passed to `setup_file_based_mcp_discovery` and the
    `environment_skill_repos` passed to `load_environment_skills` are derived
    from the merged union, not bare `environment.effective_repos()`, when
    `additional_source_repos` is non-empty. — `driver_tests.rs` or
    `environment_tests.rs`.
12. **No regression for environment-only runs.** A unit test asserts that when
    `additional_source_repos` is empty, `merge_repos_deduped(env_repos, []) ==
    env_repos` and `prepare_environment` behaves exactly as today (same clone
    set, same auto-cd, same indexing). — `environment_tests.rs`.
13. **No `warp-proto-apis` change.** Confirm no file under `warp-proto-apis` is
    modified by this PR (JSON/JSONB + REST, not protobuf). — PR diff review.
14. **Repository checks pass.** `./script/format`, `cargo clippy --workspace
    --all-targets --all-features --tests -- -D warnings`, and the relevant
    test suites pass:
    `cargo nextest run -p cloud_object_models`,
    `cargo nextest run -p app --test …` (the agent_sdk / driver environment
    tests), and `cargo nextest run --no-fail-fast --workspace --exclude command-signatures-v2`.
    The implementer runs `./script/presubmit` before readying the PR. —
    `./script/presubmit` (or the individual commands).

## Out of scope

- Any warp-server change (snapshot field population, validation, token
  scoping, forge-mismatch/origin-only rejection, feature flag) — owned by the
  sibling warp-server spec (PR #13180).
- Multiple GitHub App installations covering the union (server-side credential
  redesign).
- A dedicated source-repo token path for origin-only named-agent runs
  (server-side follow-up).
- Owner-qualifying clone target directories (`working_dir/{owner}/{repo}`) to
  support same-`repo`-name collisions across distinct owners in a single run.
  The server's same-owner enforcement makes this unnecessary today.
- A CLI flag or config-file key for `additional_source_repos` (it is
  server-populated only, per the server spec).
- Changes to `warp-proto-apis` (none expected; JSON/JSONB + REST only).
- Any change to the user-principal (non-named-agent) token path
  (`task_utils.go:247` server-side); GitHub-webhook Factory automations execute
  as the named agent's service-account principal.
