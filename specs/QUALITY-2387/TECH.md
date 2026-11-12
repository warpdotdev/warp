# Cwd-scoped cloud skill listings

Status: implemented; validation results and limitations are recorded in the draft PR.
Tracking: [QUALITY-2387](https://linear.app/warpdotdev/issue/QUALITY-2387).
Source: [request and decisions](https://warpdev.slack.com/archives/C0BDQDW8V5E/p1794263012687819).

## Context

This is Solution 1a. Solution 1b in `warp-server/specs/QUALITY-2387/TECH.md` stores
skill snapshots and deltas. Deliver one draft PR per repository. Implement on the
same branches as these specs after approval. Land the server change first.

Current source, inspected at `8246c1e1b6c3d37a374fe51f4881d2c5d1dfc58d`:

- [`skill_manager.rs:39-203`](https://github.com/warpdotdev/warp/blob/8246c1e1b6c3d37a374fe51f4881d2c5d1dfc58d/app/src/ai/skills/skill_manager.rs#L39-L203):
  cloud mode bypasses cwd filtering. Local mode requires an ancestor skill directory
  within the detected repository, when a repository exists.
- [`agent_setup.rs:338-527`](https://github.com/warpdotdev/warp/blob/8246c1e1b6c3d37a374fe51f4881d2c5d1dfc58d/app/src/ai/agent_sdk/driver/agent_setup.rs#L338-L527):
  environment and global loading both enable the cloud bypass. Global repository
  checkouts are unregistered siblings; only spec-selected skills are loaded.
- [`agent_setup.rs:150-267`](https://github.com/warpdotdev/warp/blob/8246c1e1b6c3d37a374fe51f4881d2c5d1dfc58d/app/src/ai/agent_sdk/driver/agent_setup.rs#L150-L267):
  prepared workspace supplies the harness cwd before skill loading.
- [`skill_references.rs:205-280`](https://github.com/warpdotdev/warp/blob/8246c1e1b6c3d37a374fe51f4881d2c5d1dfc58d/app/src/ai/agent_sdk/driver/skill_references.rs#L205-L280)
  and [`516-600`](https://github.com/warpdotdev/warp/blob/8246c1e1b6c3d37a374fe51f4881d2c5d1dfc58d/app/src/ai/agent_sdk/driver/skill_references.rs#L516-L600):
  Factory references produce selected symlinks under
  `$TMPDIR/oz-skill-sources/<task_id>/skills/<index>/<name>`.
  The generated folders extend `me.skill_dirs` before loading.
- [`agent_setup.rs:529-600`](https://github.com/warpdotdev/warp/blob/8246c1e1b6c3d37a374fe51f4881d2c5d1dfc58d/app/src/ai/agent_sdk/driver/agent_setup.rs#L529-L600):
  configured/generated skill folders and the deferred-repository helper use the HOME tier.

## Behavior

1. With `CwdScopedCloudSkills` enabled, environment-repository skills use the existing
   local ancestor/repository-root listing policy.
2. Use the live session cwd when present. Otherwise use the prepared harness cwd.
   A single-repository run initially lists that repository's root skills.
   A multi-repository workspace root lists zero environment-repository skills.
3. Entering a repository or nested directory updates the listing through the existing
   conversation-level `list_skills_if_changed` path. Leaving that scope removes its
   listing entries. Do not add request fields or force a resend every turn.
4. Keep the full index. `read_skill` and explicit skill-spec resolution can still access
   indexed skills absent from the current listing.
5. Selected global skills remain listed from every cwd, without making unrelated
   environment or nested skills globally visible. They are reported at HOME scope, which
   the server already ranks above project skills and caps with the HOME budget.
6. HOME skills, `WARP_SKILL_DIRS`, generated Factory reference folders, the deferred-repo
   helper, bundled activation, deduplication, and icon overrides retain their behavior.
   Host-origin checks must prevent the local fallback cwd or local home-tier skills leaking
   into SSH listings.
7. Flag-off behavior remains the existing cloud/global listing behavior. Ordinary local
   sessions without cloud/global setup retain their current filtering in either mode.

## Proposed changes

- Add `FeatureFlag::CwdScopedCloudSkills` in `crates/warp_features/src/lib.rs`.
  Enable only in `DOGFOOD_FLAGS`; do not add it to preview/release defaults.
  Stable-channel cloud workers do not pick up this behavior merely because the server flag is on.
- Gate at the loaders in `agent_setup.rs`, not in `SkillManager`. With the flag on,
  `load_environment_skills` records `AgentDriver.harness_working_dir` as the listing fallback
  and `load_global_skills` registers the selection at HOME precedence; with the flag off, both
  keep enabling the legacy unscoped cloud listing. `SkillManager` only reacts to the state it
  was given, so flag removal deletes the legacy branch without touching listing logic.
- Add `set_cloud_working_dir` to `SkillManager`. The fallback applies only to local-origin
  listings and only when no live cwd is supplied; it feeds the existing ancestor/repository
  filter unchanged.
- Set the fallback during environment skill setup, including startup paths whose repository
  list is empty. Cover cold and prewarmed setup. Do not infer the harness cwd from the first
  repository in a multi-repo environment.
- In `load_global_skills`, register only the skills that survive `filter_skills_by_spec`,
  with `SkillScope::Home`, through the same home-tier path used by `WARP_SKILL_DIRS`,
  generated Factory reference folders, and the deferred-repo helper (`add_home_tier_skills`,
  formerly `add_skills_dirs_skills`). No separate pin set. An environment/global repository
  overlap must not widen the repository's other skills. Local `warp agent run` with global
  skills must not enable all-project listing in enabled mode.
- Because an overlapping selection is reachable from both the home bucket and its repository
  bucket, list each skill file once before the provider deduplication pass.
- Deleting a home-tier skill's path must drop it from the index even when the file lives
  outside the home directory, so no stale descriptor survives through the home bucket.
- Leave `load_skills_dirs` and reference materialization unchanged. Generated symlink folders
  remain HOME-tier; do not register their source repositories.
- Change `dummy_skill_manager.rs` only if compilation exposes a caller requiring the new API.

## Decisions and exclusions

- The requester selected the recommended zero-repo-skills workspace-root behavior over
  one-level descendant inclusion. The server spec adds a static repository-discovery nudge.
- Selected global skills reuse the existing HOME tier rather than a new pin mechanism: it
  already has cwd-independent lifetime and host-origin isolation, and explicit selections
  behave like personal skills. The server ranks HOME above project and applies a separate
  255-entry HOME budget, which an explicit selection will not approach.
- Explicit skill-spec resolution (`resolve_skill_spec`) runs while building driver options,
  before any global skill is loaded, so the home-first precedence it applies to the index is
  unaffected; its only later caller resolves repo-qualified specs, which bypass that path.
- A dogfood flag provides rollback; an unguarded branch-only change was rejected.
- Factory reference source resolution, collision rules, and server path-suffix ranking are
  unchanged. Temporary generated paths can already rank as HOME rather than Factory.
- No GUI/TUI surface redesign, Solution 2, benchmark execution, or production rollout.

## Testing and validation

Add or update behavior-focused cases in `skill_manager_tests.rs`,
`agent_sdk/driver_tests.rs`, and `driver/agent_setup_tests.rs`:

- Behaviors 1-3: single-repo missing-cwd fallback; multi-repo root; repo and nested `cd`;
  sibling-repo exclusion; existing local filtering remains green.
- Behaviors 4-5: unlisted nested skill remains readable; selected global skill survives
  unrelated cwd and missing cwd; environment/global overlap lists the selection once and
  does not widen unselected skills.
- Behavior 6: load a generated Factory-reference-style symlink folder through the actual
  configured-directory loading path. Check its HOME descriptor remains identical across
  workspace, repository, and nested cwd changes. Source-only unselected skills remain unindexed.
  Keep existing configured-directory, origin-isolation, deduplication, and bundled tests green.
- Behavior 7: existing flag-off cloud listing test; cold/prewarmed setup uses the harness cwd.
  Keep only cases that catch distinct defects; do not duplicate every path across all fixtures.

After implementation and self-review, run from the repository root, in order:

1. `cargo nextest run -p warp --lib -E 'test(skills::skill_manager::tests) | test(agent_sdk::driver::tests) | test(agent_setup::tests) | test(read_skill)'`
   Include the new cases in this selection; correct module filters if nextest discovery differs.
2. `cargo clippy -p warp --all-targets --tests -- -D warnings`.
3. `./script/format`.

Use non-GUI checks only, as requested. Cross-repository validation must inspect the
client's send-on-change output and the server's rendered snapshot/delta sequence for
workspace → repo → nested → sibling, unlisted reads, attached skills, and compaction.
Use a headless local client/server path where available; report any unavailable runtime
dependency instead of substituting visual proof. Do not claim benchmark or model-quality results.
Benchmarks are deferred for draft PR delivery.

## Approval

After receiving both spec PRs, the requester asked: "could you make the corresponding
code change of warp and warp server in the same PR". This authorizes implementation of
the presented companion specs on their existing branches. The earlier non-GUI-only and
deferred-benchmark choices remain in force.
