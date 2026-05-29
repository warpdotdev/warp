# Repeatable `oz agent run --skill` Product Spec
## Summary
`oz agent run --skill <SPEC>` should accept the flag multiple times. Each provided skill should be resolved with the same behavior as the existing single-skill flag, and every resolved skill should be available to the running Oz agent through the normal skill system. Only the first skill should be treated as the invoked skill for the run.
## Problem
Today, users can provide one `--skill` to set the initial skill instructions for an Oz agent run. Some workflows need one primary invoked skill plus additional skills available for the agent to read later. Users currently cannot express that directly in a single command without relying on environment-wide skill discovery or modifying prompts manually.
## Goals
- Allow multiple `--skill <SPEC>` flags on `oz agent run`.
- Resolve every provided skill using the same lookup, qualification, cloning, and error behavior that the single flag already uses.
- Make every resolved skill available in the run's `SkillsManager`.
- Submit only the first resolved skill as the invoked skill.
- Preserve existing behavior for commands that provide zero or one `--skill`.
## Non-goals
- Changing `oz agent run-cloud --skill` behavior.
- Changing skill resolution precedence or accepted skill spec formats.
- Combining multiple skill instruction bodies into the initial prompt.
- Changing the skill file format or the displayed skill-invoked output.
## Figma / design references
Figma: none provided.
## User Experience
1. A user can run `oz agent run --skill primary --skill helper --prompt "Do the task"`.
2. The command parser accepts repeated `--skill` flags and preserves their order.
3. The first skill in the command is the invoked skill. Its instructions are used exactly where the previous single `--skill` instructions were used:
   - as the base prompt for a local prompt run,
   - as the skill-only prompt for a skill-only run,
   - as the server-side attached skill for a `--task-id` run.
4. Additional skills are not treated as invoked skills. They do not replace the run name, base prompt, task creation prompt, server-side attached skill, or skill-invoked output.
5. Every provided skill spec is resolved before the run starts. If any skill fails to resolve, the command fails with the same style of error the single-skill path already reports.
6. Each resolved skill is registered with the run's `SkillsManager` so it is available through the agent's normal skill-listing and read-skill behavior.
7. Skill resolution order is deterministic and follows the CLI order. The first successfully resolved skill remains the invoked skill even when later skills have different names, providers, or qualified repo specs.
8. Existing commands with a single `--skill` keep their current behavior and output.
9. Existing commands without `--skill` keep their current prompt validation behavior.
## Success Criteria
1. `oz agent run --skill a --skill b --prompt "..."` parses successfully.
2. All provided skill specs are resolved using the existing resolver.
3. All resolved skills are inserted into `SkillsManager`.
4. Only the first resolved skill is passed into the existing invoked-skill path.
5. Single-skill and no-skill command behavior remains unchanged.
6. Tests cover parser order, multi-skill resolution, `SkillsManager` population, and first-skill-only invoked behavior.
## Validation
- Add CLI parsing coverage for repeated `--skill` preserving order.
- Add Rust unit coverage around resolving multiple skills from disk and registering all of them with `SkillsManager`.
- Add or update task/config tests so only the first skill contributes invoked-skill configuration.
- Run targeted Rust tests for `warp_cli` and the touched agent SDK modules.
## Open questions
- None.
