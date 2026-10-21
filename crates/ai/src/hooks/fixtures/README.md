# Agent hook fixtures

These files back the end-to-end hook tests in `crates/ai/src/hooks/e2e_tests.rs`. The tests copy
them into a temporary directory and make the scripts executable before running anything, so
nothing here is ever written to.

## `sample_project/`

A Cursor-format sample with `.cursor/hooks.json` and four POSIX `sh` scripts in
`.cursor/scripts/`. The parser tests use that file directly. Runtime tests use
`run_hooks/hooks.json`, which names the same scripts from an explicitly configured source
directory.

| Event | Script | What it does |
| --- | --- | --- |
| `sessionStart` | `session_env.sh` | Sets `SAMPLE_HOOK_SESSION=started` for every later hook in the conversation. |
| `beforeSubmitPrompt` | `log_prompt.sh` | Appends `prompt_length=<n> session=<value>` to `.hook-prompts.log`, then lets the prompt through. |
| `beforeShellExecution` | `deny_dangerous.sh` | Denies `rm -rf` and `git push ... --force` with a reason the agent can read; allows everything else. Writes its latest input to `.hook-last-shell-input.json`. |
| `afterShellExecution` | `audit.sh` | Appends its whole input, one JSON object per line, to `.hook-audit.log`. |
| `stop` | `true` | Asks for no follow-up. |

Every file the scripts write lands in `$WARP_PROJECT_DIR`, which Warp sets from the run's
workspace roots.

`sample_project/run_hooks/hooks.json` binds the same scripts for use as a source delivered with
the run (`--hook-dir`, `WARP_HOOK_DIRS`, or resolved agent configuration). A source runs commands
from the directory that holds it, so these commands start with `../.cursor/scripts/`.

## `malformed/`

A `.cursor/hooks.json` with one definition whose `matcher` is not a valid regular expression and
one event name Cursor does not define. The file still loads: the bad definition and the unknown
event are skipped with warnings, and the other definition stays active.
