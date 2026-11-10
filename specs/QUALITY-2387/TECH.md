# Cwd-scoped cloud skill listings

Status: proposed; implementation requires human approval.
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
   environment or nested skills globally visible.
6. HOME skills, `WARP_SKILL_DIRS`, generated Factory reference folders, the deferred-repo
   helper, bundled activation, deduplication, and icon overrides retain their behavior.
   Host-origin checks must prevent local or other-host pinned skills leaking into SSH listings.
7. Flag-off behavior remains the existing cloud/global listing behavior. Ordinary local
   sessions without cloud/global setup retain their current filtering in either mode.

## Proposed changes

- Add `FeatureFlag::CwdScopedCloudSkills` in `crates/warp_features/src/lib.rs`.
  Enable only in `DOGFOOD_FLAGS`; do not add it to preview/release defaults.
  Stable-channel cloud workers do not pick up this behavior merely because the server flag is on.
- Replace the cloud boolean's unconditional role with `cloud_working_dir` and
  `pinned_skill_paths`. Retain a flag-off representation of legacy cloud listing mode
  until rollout cleanup; do not delete rollback behavior while introducing the gate.
- Add `set_cloud_working_dir` and `pin_skill_paths` APIs to `SkillManager`.
  Resolve the effective cwd before project filtering. Match pinned paths against
  `SkillPathOrigin` independently of whether the session cwd is present.
- In enabled mode, collect HOME paths, scoped project paths, and pinned selected-global
  paths before the existing shared deduplication/icon/bundled pass.
- Set the fallback from `AgentDriver.harness_working_dir` during environment skill setup,
  including startup paths whose repository list is empty. Cover cold and prewarmed setup.
  Do not infer the harness cwd from the first repository in a multi-repo environment.
- In `load_global_skills`, pin only the paths that survive `filter_skills_by_spec`.
  An environment/global repository overlap must not pin the repository's other skills.
  Local `warp agent run` with global skills must not enable all-project listing in enabled mode.
- Leave `load_skills_dirs`, reference materialization, and `add_skills_dirs_skills` unchanged.
  Generated symlink folders remain HOME-tier; do not pin or register their source repositories.
- Keep pin/index lifecycle coherent: deleted or replaced skills cannot leave stale descriptors
  visible through the pin set. Resolve pins through the live index.
- Change `dummy_skill_manager.rs` only if compilation exposes a caller requiring the new API.

## Decisions and exclusions

- The requester selected the recommended zero-repo-skills workspace-root behavior over
  one-level descendant inclusion. The server spec adds a static repository-discovery nudge.
- Pinning preserves explicit global selection without broadening repository scope.
  HOME-tier Factory reference folders already have the desired lifetime and need no new mechanism.
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
  unrelated cwd and missing cwd; environment/global overlap does not pin unselected skills.
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

This spec records the latest attachment's Factory-reference additions and the requester's
resolved choices. Approval of this spec and the companion server spec is required before
implementation. Neither this document nor opening its draft PR constitutes that approval.
