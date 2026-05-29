# Repeatable `oz agent run --skill` Technical Spec
## Problem
`RunAgentArgs` currently stores `--skill` as `Option<SkillSpec>`, and the local agent setup resolves at most one skill before building the task. The implementation must preserve that first skill as the invoked skill while still resolving and registering every repeated skill so the Oz harness sees them in the normal `SkillsManager` context.
## Relevant code
- `crates/warp_cli/src/agent.rs:260` - `RunAgentArgs.skill` defines the local `oz agent run --skill` CLI field.
- `crates/warp_cli/src/lib_tests.rs:397` - parser tests lock in accepted prompt/skill combinations.
- `app/src/ai/agent_sdk/mod.rs:230` - feature-gate handling rejects `--skill` when Oz platform skills are disabled.
- `app/src/ai/agent_sdk/mod.rs:329` - `build_merged_config_and_task` consumes the resolved invoked skill.
- `app/src/ai/agent_sdk/mod.rs:741` - `AgentDriverRunner::resolve_skill` resolves the single CLI skill.
- `app/src/ai/agent_sdk/mod.rs:811` - `build_driver_options_and_task` resolves skill before task/config construction.
- `app/src/ai/skills/resolve_skill_spec.rs:132` - existing resolver implements skill spec lookup behavior.
- `app/src/ai/skills/skill_manager.rs:372` - `SkillManager::handle_skills_added` registers parsed skills by path and name.
- `app/src/ai/agent_sdk/driver_tests.rs:580` - existing skill-loading tests verify `SkillsManager` contents.
## Current state
The parser accepts only one local `--skill` because `RunAgentArgs.skill` is an `Option<SkillSpec>`. Agent setup clones sandboxed fully qualified skill repos only for that one spec, resolves it with `resolve_skill_spec`, and passes the resulting `Option<ResolvedSkill>` into config/task construction. Environment and global skills are loaded later in `AgentDriver::run_internal`, but the explicitly invoked skill is not explicitly added to `SkillsManager` by the setup path.
## Proposed changes
1. Change only local `RunAgentArgs.skill` to `Vec<SkillSpec>` with append semantics. Leave `RunCloudArgs.skill` as `Option<SkillSpec>` because repeatable behavior is scoped to `oz agent run`.
2. Update local prompt-group, feature-gate, and prompt-source checks from `is_some()` to `!is_empty()`.
3. Add a helper on `RunAgentArgs` such as `invoked_skill(&self) -> Option<&SkillSpec>` to centralize first-skill selection. Use it anywhere the code needs the historical single-skill behavior, including task creation prompt text.
4. Replace `AgentDriverRunner::resolve_skill` with a multi-skill resolver that:
   - returns `Vec<ResolvedSkill>`,
   - iterates `args.skill` in CLI order,
   - applies the same sandboxed `org/repo` clone rule before resolving each skill,
   - calls `resolve_skill_spec` for each spec with the same working directory and error formatting.
5. Add all resolved skills to `SkillsManager` after resolution and before task construction. This should call `SkillManager::handle_skills_added` with the resolved skills' `parsed_skill` values. The manager's path/name maps already deduplicate by path, so repeated registration is safe.
6. Continue passing only `resolved_skills.first().cloned()` into `build_merged_config_and_task` and `build_server_side_task`. Those functions should not concatenate multiple skill instruction bodies.
7. Update CLI tests and add focused app-unit coverage. Prefer small helper tests over end-to-end UI tests where possible.
## End-to-end flow
1. Clap parses repeated `--skill` values into `RunAgentArgs.skill` in command-line order.
2. Local agent setup canonicalizes the working directory.
3. Setup resolves each skill spec in order, cloning qualified sandboxed repos when needed.
4. Setup registers every resolved parsed skill with `SkillsManager`.
5. Setup selects the first resolved skill as `invoked_skill`.
6. Existing config/task construction uses `invoked_skill` exactly like the old single resolved skill.
7. `AgentDriver::run_internal` starts the Oz harness with the invoked skill prompt and a `SkillsManager` that also includes any additional resolved skills.
## Risks and mitigations
- Risk: secondary skills accidentally change the prompt or run name.
  - Mitigation: keep existing config/task functions single-skill and pass only the first resolved skill.
- Risk: feature-gate checks accidentally allow hidden `--skill` usage.
  - Mitigation: update all local checks to use `!args.skill.is_empty()`.
- Risk: resolving the same repo-qualified skill multiple times clones redundantly.
  - Mitigation: existing `clone_repo_for_skill` skips targets that already exist as git repos; no new clone cache is needed.
- Risk: third-party harness behavior changes unexpectedly.
  - Mitigation: this change only impacts local `RunAgentArgs`; `run-cloud` remains unchanged, and existing Oz-only skill loading gates stay in place.
## Testing and validation
- `crates/warp_cli/src/lib_tests.rs`: assert repeated `--skill` parses in order.
- `app/src/ai/agent_sdk/mod.rs` or adjacent tests: verify helper behavior selects only the first skill for invoked-skill code paths.
- `app/src/ai/agent_sdk/driver_tests.rs`: verify registering resolved CLI skills inserts all parsed skills into `SkillsManager`.
- Run:
  - `cargo nextest run -p warp_cli agent_run_accepts_multiple_skills`
  - `cargo nextest run -p warp <targeted skill/agent SDK tests>`
  - `cargo fmt`
## Follow-ups
- Consider whether `oz agent run-cloud --skill` should become repeatable separately if cloud task creation needs the same primary-plus-supporting-skill behavior.
## Parallelization
Parallel sub-agents are not proposed. The parser, resolver, config, and tests are tightly coupled and fit in a small local change; splitting the work would add merge overhead without reducing meaningful wall-clock time.
