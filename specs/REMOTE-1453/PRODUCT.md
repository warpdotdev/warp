# REMOTE-1453: Resolve service-account global skills at agent driver startup
## Summary
Service accounts can have a configured set of global skill specs that should be available in every Oz agent run. The client resolves those specs at agent-driver startup, clones any org-qualified source repos that are not already on disk, and loads skills before the first query runs.
Global skills are intentionally not treated as permission to load every skill in their source repos. Repos that are part of the run's environment still use normal environment behavior and autodiscover all skills. Repos cloned only because they contain global skills load only the explicitly requested global skills.
## Problem
Before this work, an `oz agent run` only saw skills that were already present on disk or explicitly requested with `--skill <SPEC>`. Service-account global skills exposed by `User.globalSkills` were not made available automatically, and a repo cloned only to satisfy one global skill could accidentally expose unrelated skills in that repo.
## Goals
- For service-account-authenticated Oz agent runs, make every explicitly requested global skill available before the agent processes its first query.
- Autodiscover all skills from repositories that are part of the configured environment.
- For repositories cloned solely as global-skill sources, load only the global skills explicitly listed in `User.globalSkills`.
- If the same `(owner, repo)` appears both in the environment and in `User.globalSkills`, environment behavior wins and all skills from that repo are autodiscovered.
- Reuse existing skill parsing, repository cloning, skill watching, and skill manager flows where possible.
- Keep behavior unchanged for user principals and runs with no global skills.
## Non-goals
- A general plugin or skill registry mechanism beyond startup-time resolution.
- A Warp-managed cache directory for cloned skill repositories.
- Periodic or background refresh of the global skill list during a run.
- Changing CLI `--skill` resolution semantics.
- Adding global-skill loading to non-Oz harnesses in this iteration.
## Behavior
1. When an Oz agent driver starts, it reads the cached `User.globalSkills` list. The list uses the same skill spec format accepted by `--skill`: `skill_name`, `repo:skill_name`, `org/repo:skill_name`, or full-path variants.
2. If `OzPlatformSkills` is disabled, the client treats `globalSkills` as empty and bypasses the flow.
3. Each raw global skill string is parsed with the existing `SkillSpec` parser. Parse failures are logged at warn level and skipped; the rest of the list still resolves.
4. Specs without an org-qualified repository are not cloneable, so they do not add any repository to the global-skill clone set. They may still resolve through normal on-disk skill discovery if the skill is already present.
5. Specs with an `org/repo` qualifier contribute `(org, repo)` to the global-skill repo set. Multiple specs for the same `(org, repo)` result in one clone attempt and retain the explicit specs for later filtering.
6. Global-skill repos are cloned best-effort into `working_dir/<repo>` using the same environment clone helper and partial-clone behavior as environment repos. If the target repo already exists, it is reused. If a clone fails, the run logs a warning and continues without those global-only skills.
7. Environment preparation still clones the environment's `github_repos`, runs setup commands, and performs its normal repo registration. If a global-skill repo and an environment repo share the same `(owner, repo)`, whichever clone path reaches disk first is reused by the other path.
8. Skill loading is classified by source:
   - Environment repos: scan and load all discovered `SKILL.md` files.
   - Global-only repos: scan the repo tree, then keep only skills whose parsed names or explicit paths match the global skill specs for that repo.
   - Overlapping repos: treat the repo as an environment repo, so all skills are loaded.
9. For simple global skill names such as `org/repo:deploy`, filtering matches parsed skill names and mirrors normal provider precedence when multiple providers define the same name. For full-path specs, filtering matches the exact path relative to the repo root.
10. Once loaded, the selected skills participate in the existing `SkillManager` flow. There is no separate global-skill registry or global-skill badge.
11. The flow is Oz-only for now. Non-Oz harnesses keep their existing behavior.
12. Historical conversations are unaffected; they retain whatever skills were available when they ran.
## Edge cases
1. Duplicate global skill specs: cloning is deduped by `(org, repo)` and skill filtering dedupes by selected skill path.
2. Multiple global skills in one global-only repo: only those explicit skills load; unrelated skills in the same repo do not.
3. A repo appears both in the environment and in `globalSkills`: all skills from that repo load because the environment classification wins.
4. No environment configured: cloneable global-skill repos still clone into `working_dir`, but only explicitly requested global skills load from those repos.
5. A global skill references a missing or inaccessible repo: the clone warning is logged, and the run continues without that repo's global-only skills.
6. A non-cloneable global skill spec is present: no repo is cloned for it, and it only resolves if the skill is already discoverable on disk.
7. `working_dir` does not exist: the existing agent-driver working-directory check fails before this flow changes anything.
