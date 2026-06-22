# Common Skills Installation — Product Spec
## Summary
Warp development setup should automatically make the shared `warpdotdev/common-skills` agent skills available to local agents, without requiring developers to remember manual restore commands. `script/bootstrap` and `script/run` should install or update common skills from the repository lock, support both project-local and global installs, work whether the common-skills repo is checked out locally or not, and prevent duplicate project/global common-skill definitions.
## Problem
Common skills are consumed by agents across Warp development workflows. If they are missing, stale, installed in two places, or restored from an unexpected source, local agents can behave differently across developer machines and cloud-like local test scenarios.
## Goals / Non-goals
1. Common skills should be present and match the Warp checkout's expected versions after successful setup flows.
2. Developers should explicitly choose project-local or global installation through a flag, environment variable, or interactive prompt.
3. Developers should be able to test the same behavior with a local common-skills checkout, a common-skills worktree, or the remote `warpdotdev/common-skills` repo.
4. Installing common skills must not duplicate the same common skills in both project and global locations.
5. Normal setup should follow the Warp checkout's lock file; adopting newer upstream common skills is an explicit developer-approved update to that lock, not an unreviewed floating dependency update during every run.
## Behavior
1. `./script/bootstrap` includes common-skills setup as part of the normal local platform bootstrap flow. A developer who runs bootstrap without opting out should end the setup with common skills installed in exactly one supported target and matching the Warp checkout's common-skills lock.

2. `./script/bootstrap --skip-common-skills` skips all common-skills installation and verification behavior. Platform bootstrap behavior continues as usual, and the command does not create, remove, update, or verify common skills.

3. `WARP_SKIP_COMMON_SKILLS_INSTALL=1` skips common-skills installation and verification even when bootstrap or run would otherwise install common skills. This environment override is respected consistently by bootstrap and run.

4. `./script/run` checks common skills before launching the local Warp build. When common skills are missing or stale, the check attempts to restore them before the app launches. When common skills are already current, the check is quiet and does not distract the developer from the run flow.

5. `./script/run` is strict for common-skills setup when common-skills installation is enabled. If run cannot install or verify common skills, it reports the error and fails before launching Warp so the developer does not start a local build with missing, stale, duplicated, or mismatched common skills.

6. `./script/bootstrap` is equally strict for common-skills setup when common-skills installation is enabled. If bootstrap cannot install or verify common skills, bootstrap reports the error and fails so the developer knows setup is incomplete.

7. `./script/run --install-common-skills` forces a common-skills restore attempt before launching Warp, even when the current install appears up to date. This gives developers a simple recovery path when they suspect local skill contents are damaged or stale.

8. If a developer explicitly selects project installation with `./script/bootstrap --install-common-skills-in-repo`, `WARP_COMMON_SKILLS_INSTALL_TARGET=project`, or the installer `--project` option, common skills are installed into the Warp checkout's project-local skill directory as local developer state.

9. Project-local common-skill installs are ignored by Git and must not be checked into source control. The ignore behavior applies only to the locked common-skill paths installed from `warpdotdev/common-skills`; unrelated project-local skills in `.agents/skills` must remain visible in normal `git status`. A developer who installs common skills into the project directory should not see the installed common-skill directories as untracked or modified files and should not need to manually avoid committing them.

10. If a developer explicitly selects global installation with `./script/bootstrap --install-common-skills-globally`, `WARP_COMMON_SKILLS_INSTALL_TARGET=global`, or the installer `--global` option, common skills are installed into the user's global agent skills directory.

11. Common-skills installation target selection is always explicit. The installer must not infer the target from an existing project-local or global install. A target is explicit only when the developer provides a project/global flag, sets the install-target environment variable, or answers an interactive prompt for the current command.

12. When no project/global target is provided in an interactive command, bootstrap and run ask the developer where common skills should be installed. The prompt presents project-local and global options, recommends global for normal local development, and uses global only when the developer accepts the default for that prompt.

13. When no project/global target is provided and no interactive prompt is available, the installer fails with an actionable error asking the developer or automation to choose project or global explicitly. The command should not hang in CI, automation, cloud setup, or non-interactive shell contexts.

14. Project-local installs and global installs are mutually exclusive for common skills. If common skills are detected in both the Warp checkout and the user's global skill directory, install and verification flows fail with an actionable error telling the developer to remove one copy before continuing. The error should suggest using `remove_common_skills --repo-root <warp-checkout>` to remove the project-local copy or `remove_common_skills --repo-root <warp-checkout> --global` to remove the global copy.

15. Duplicate detection applies only to the locked common skills, not every skill the developer has installed. Developers may keep unrelated global skills such as personal utilities, local-only workflows, or other agent skills alongside global common skills.

16. Installing common skills globally must not remove or overwrite unrelated global skills. Only the common skills listed by the Warp checkout's common-skills lock are installed, updated, verified, or removed by these flows.

17. Installing common skills project-locally must not remove or overwrite unrelated project-local skills. Only the common skills listed by the Warp checkout's common-skills lock are installed, updated, verified, or removed by these flows.

18. Successful install, update, and skip paths verify that every locked common skill exists in the selected target and matches the content expected by the Warp checkout. A setup flow is not considered successful merely because an install command exited successfully; the installed skill contents must also match the lock.

19. Verification failures are specific and actionable. Missing skills, duplicated project/global installs, missing lock files, invalid target choices, and content mismatches each produce errors that tell the developer what is wrong and, when possible, how to recover.

20. `--verify-only` verifies the current common-skills state without installing, updating, creating a lock, or prompting for an install target. If the lock file is missing, it fails immediately with a missing-lock error. If the locked skills are missing, duplicated, or mismatched, it reports those verification errors without changing local skill state.

21. If the Warp checkout has no common-skills lock during a normal install flow, the installer creates one from `warpdotdev/common-skills` and installs the corresponding common skills into the selected target. If `WARP_COMMON_SKILLS_REF=<git-ref>` is set, the missing lock is created from that branch, tag, or commit instead. This supports first-time setup of a checkout that has not yet adopted a common-skills lock and lets developers test a common-skills branch end to end.

22. If the Warp checkout already has a common-skills lock, install flows treat that lock as the manifest for the selected target: the lock determines which common skills should exist and what their file contents should be. Restoring from the lock means the selected project-local target is brought back to that manifest, or the selected global target is installed or verified when it is empty or already matches the same lock. If the global target is already pinned to a different common-skills lock, the flow fails with a version-mismatch error instead of overwriting it. Normal bootstrap and run flows do not silently float to newer common-skills contents that were pushed upstream after the lock was written.

23. In an interactive normal install flow where the Warp checkout already has a common-skills lock, the installer checks whether `warpdotdev/common-skills` would produce a different lock before asking the developer to choose project-local vs. global installation. If `WARP_COMMON_SKILLS_REF=<git-ref>` is set, the check uses that branch, tag, or commit as the candidate source. If the candidate common-skills version differs, the installer tells the developer that common skills have been updated and asks whether to update the checkout's `skills-lock.json` and reinstall common skills from the updated lock.

24. If the developer accepts the interactive upstream update prompt, the installer updates the checkout's `skills-lock.json`, then continues to the explicit project/global target choice when no target was already provided. After the target is selected, it reinstalls common skills from the updated lock. The resulting lock diff is a tracked source change that the developer can review and commit intentionally.

25. If the developer declines the interactive upstream update prompt, the installer leaves `skills-lock.json` unchanged and continues normal setup from the existing lock. It may still prompt for the project/global target and restore missing or stale local skill contents to match the existing lock.

26. Explicit non-interactive flows never prompt to update `skills-lock.json` from upstream. CI, cloud setup, and direct installer invocations with `--non-interactive` use the current checkout lock as the source of truth and fail rather than hanging if required choices were not provided. Local `script/run` behaves like bootstrap: it may prompt for upstream lock updates and project/global target selection when run from an interactive terminal.

27. When new common-skill versions are pushed to `warpdotdev/common-skills`, a developer can also adopt them by manually updating the Warp checkout's common-skills lock. After the lock changes, the next successful `script/run`, `script/bootstrap`, or direct installer invocation updates project-local common skills or an unpinned/matching global install to match the new lock.

28. If the common-skills lock changes on a branch, `script/run` detects that the locally installed common skills no longer match the checkout's expected versions before launching Warp. For a project-local install, or for a global install that is not pinned to a conflicting version, run restores the selected target automatically. For a global install pinned to a different version, run fails with the same actionable version-mismatch error used by setup.

29. If the common-skills lock has not changed and installed common skills already match it, `script/run` and `script/bootstrap` skip reinstalling. Skipping should be fast, quiet in normal run flows, and must not modify tracked files.

30. The installer supports a direct remove flow for developers who intentionally want to clear common skills from one target. Removing project-local common skills removes only the locked common skills from the project target. Removing global common skills removes only the locked common skills from the global target.

31. `remove_common_skills --clear-lock` is destructive by design: in addition to removing locked common skills from the selected target, it removes the checkout's common-skills lock. After this, `--verify-only` should fail until the lock is restored or a normal install flow recreates it.

32. If common skills are removed from the selected target but the lock remains, the next normal install flow restores the missing common skills from the lock. If both skills and lock are removed, the next normal install flow treats the checkout as missing a lock and creates a new one from `warpdotdev/common-skills`.

33. The default script source for common-skills installation is the remote `warpdotdev/common-skills` repository. Bootstrap and run must not silently discover or execute scripts from local sibling checkouts or worktrees.

34. `WARP_COMMON_SKILLS_SCRIPTS_DIR` lets a developer explicitly test scripts from a local common-skills checkout or worktree. Local script execution is allowed only through this explicit override; it should never happen merely because a checkout or worktree exists nearby.

35. When no explicit local scripts directory is set, bootstrap and run fetch and execute the remote common-skills script. A developer on a machine that has only the Warp checkout should be able to bootstrap or run without first cloning `warpdotdev/common-skills`, and a developer who does have a local common-skills checkout should still get the remote default unless they opt into local script execution.

36. `WARP_COMMON_SKILLS_REF=<git-ref>` selects which remote common-skills branch, tag, or commit is used for both the remote script path and the common-skills source used by missing-lock creation and interactive upstream lock update checks. This lets developers test an unpublished or not-yet-main common-skills script and skill-content branch against a local Warp checkout without relying on a local common-skills checkout.

37. Local and remote script sources should produce the same install behavior for a given script version. Differences in behavior should be attributable to the selected script version, not to whether the script came from a local path or a remote URL.

38. If remote script fetching fails because the network is unavailable, the branch does not exist, the script is missing, or the remote host returns an error, the command reports the remote fetch failure. Bootstrap and run both treat that as common-skills setup failure when common-skills installation is enabled.

39. Global common-skills installs are shared per user and may be used by multiple client repositories. If multiple client repos depend on the same locked common-skills version and all select global installation, each repo's setup flow should verify the existing global install and succeed without duplicating or unnecessarily reinstalling the same skills.

40. If a client repo selects global installation but the existing global common skills match a different common-skills lock than that repo expects, setup should fail with an actionable version-mismatch error rather than silently replacing the global install. The error should explain that another checkout may be pinned to a different common-skills version and that the developer must explicitly reconcile, update, or remove/reinstall the global common skills. When the current interactive flow has just asked the developer to adopt an upstream common-skills update and the developer accepted, reinstalling the selected global target from the newly updated lock is an explicit reconciliation and may proceed.

41. User-facing output should make state transitions understandable without being noisy. Installs, updates, target prompts, duplicate errors, and verification errors should be visible. No-op checks during normal `script/run` should be quiet when everything is already correct.

42. Common-skills setup should be safe to rerun. Repeating bootstrap, run, forced install, verify-only, or remove commands should either converge to the requested state or report a clear error; repeated commands should not accumulate duplicate skill copies.

43. A developer can recover from most bad local states with a small set of understandable actions:
   - Run normal install to restore missing skills from an existing lock.
   - Force install to repair damaged skill contents.
   - Remove one target when duplicate project/global common skills are reported.
   - Reconcile, update, or remove/reinstall global common skills when multiple checkouts are pinned to different common-skills versions.
   - Restore or recreate the lock when `--clear-lock` or manual file deletion removed it.

44. The checked-in common-skills lock is the source of truth for code review. When common skills are intentionally updated, reviewers should be able to see the lock change in the branch rather than having local bootstrap/run flows silently adopt unreviewed upstream content.
