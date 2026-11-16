# REMOTE-1453: Tech Spec - Resolve service-account global skills at agent driver startup
## Context
The product behavior is defined in `specs/REMOTE-1453/PRODUCT.md`. This spec covers the client-side runtime implementation for resolving service-account `User.globalSkills` during Oz agent startup.
Relevant existing pieces:
- `crates/warp_cli/src/skill.rs`: `SkillSpec`, `SkillSpec::from_str`, `SkillSpec::is_full_path`.
- `app/src/ai/skills/file_watchers/skill_watcher.rs`: scans repository trees for `SKILL.md` files.
- `app/src/ai/skills/skill_manager.rs`: stores the loaded skills for the active run.
- `app/src/ai/agent_sdk/driver/environment.rs`: environment repository clone helper used by environment prep.
- `app/src/ai/cloud_environments/mod.rs`: `GithubRepo { owner, repo }` and `AmbientAgentEnvironment.github_repos`.
- `app/src/ai/agent_sdk/driver.rs`: agent startup sequence, environment prep, and Oz-only repo skill loading.
## Implemented approach
### 1. Global skill parsing helpers
`app/src/ai/skills/global_skills.rs` owns the reusable parsing and filtering helpers and is re-exported through `app/src/ai/skills/mod.rs`.
- `resolve_skill_repos(raw_specs: &[String]) -> (Vec<SkillSpec>, Vec<GithubRepo>)` parses raw server strings with the existing CLI parser, logs warn-level parse failures, skips invalid entries, and extracts org-qualified `(org, repo)` pairs from the parsed specs while preserving first-seen order.
- `filter_skills_by_spec(repo_path, skills, specs)` takes the full scan result for one global-only repo and returns only the parsed skills explicitly requested for that repo.
Filtering behavior:
- Full-path specs match `repo_path.join(spec.skill_identifier)` exactly.
- Simple-name specs match parsed `ParsedSkill::name` values and pick the first match by provider-directory precedence.
- Duplicate selected paths are collapsed.
### 2. Driver source classification
`app/src/ai/agent_sdk/driver.rs` separates global skill resolution from repository skill loading using two explicit async methods rather than a unified load-request type.
- `GlobalSkillResolution` carries parsed global specs and cloneable global skill repos.
- `load_environment_skills` handles all repos that are part of the configured environment: it waits for `RepoMetadataModel` indexing to complete, then scans with `SkillWatcher::read_skills_for_repos` and loads all discovered skills.
- `load_global_skills` handles global-only repos: it reads directly from the known provider directories on disk via `read_skills_from_directories` (no `RepoMetadataModel` dependency), then applies `filter_skills_by_spec` to keep only the explicitly requested skills.
Because global skill repos are cloned before environment prep and are not registered with `DetectedRepositories`, they are not picked up by `SkillWatcher`'s `RepositoryMetadataEvent` path; `load_global_skills` reads them directly from disk instead. If a repo appears in both the environment and the global skill set, `load_environment_skills` runs over it and loads all skills; `load_global_skills` may also process it and add the filtered subset, but `SkillManager` deduplicates by path so the effective result is all skills from that repo.
### 3. Driver startup sequence
For Oz harnesses, `AgentDriver::run_internal` now does the following after the session is shared and before environment prep:
1. Read cached `AuthStateProvider::get().global_skills()`.
2. Parse specs and resolve cloneable repos with `resolve_skill_repos`.
3. Clone those repos best-effort with `environment::clone_repo` (which clones the repo but does NOT register it with `DetectedRepositories`, so only the explicitly requested global skills are loaded from it).
4. Prepare the configured environment, if any, preserving the existing file-based MCP discovery and setup-command sequencing. Environment repos are cloned via `ensure_repo_cloned`, which both clones and registers with `DetectedRepositories`.
5. Call `load_environment_skills` with the environment's `github_repos`, then `load_global_skills` with the global specs and repos.
Global-skill cloning is performed for all harnesses (before the Oz-only guard) so that skills are on disk, but skill loading into `SkillManager` remains Oz-only because third-party harnesses have separate skill systems.
### 4. Repo skill loading
Skill loading is split into two separate functions instead of a single unified `load_skills_from_repos`:
- `load_environment_skills(foreground, repos)`: waits for `RepoMetadataModel` indexing on each repo, then scans with `SkillWatcher::read_skills_for_repos` and loads all discovered skills into `SkillManager`.
- `load_global_skills(foreground, specs, repos)`: reads skills directly from provider directories via `read_skills_from_directories` (using `SKILL_PROVIDER_DEFINITIONS`), applies `filter_skills_by_spec` to keep only explicitly requested skills, and loads the result into `SkillManager`. This path does not depend on `RepoMetadataModel` because global-skill repos are not registered with `DetectedRepositories`.
Both functions call `SkillManager::set_cloud_environment(true)` and `handle_skills_added`, preserving the existing in-scope behavior for repo-loaded skills.
## Behavior guarantees
- Environment repos continue to autodiscover all skills.
- Global-only repos do not expose unrelated skills from the same repo.
- Environment/global overlap is resolved in favor of environment autodiscovery.
- Clone failures for global-skill repos are warn-and-continue.
- Invalid global skill specs are warn-and-skip.
- Non-cloneable specs do not trigger clone attempts.
## Testing and validation
Unit coverage added or expected:
- `resolve_skill_repos` skips invalid specs and handles empty input, unqualified specs, org-qualified specs, and duplicate repos.
- `filter_skills_by_spec` loads only requested simple-name skills, respects provider precedence, and matches full-path specs.
Validation commands:
- `cargo fmt`
- Targeted `cargo nextest` for the `global_skills` and `driver` tests touched by this change.
- A targeted `cargo check` or package build if test compilation exposes broader type issues.
## Follow-ups
- Add integration coverage around a real service-account run once the test harness can inject `User.globalSkills` and cloned repo contents cheaply.
- Consider refreshing `User.globalSkills` at run start if cached user state can become stale for long-lived clients.
- Consider global-skill provenance in skill listing UI or logs if users need to debug why a skill was available.
- Revisit third-party harness behavior if those harnesses adopt the same runtime skill loading path.
