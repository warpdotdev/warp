# Agent hook fixtures

These files back the end-to-end hook tests in `crates/ai/src/hooks/e2e_tests.rs` and
`app/src/ai/blocklist/hooks/e2e_tests.rs`. The tests copy them into a temporary directory and make
the scripts executable before running anything, so nothing here is ever written to.

## `sample_project/`

A repository with an unchanged Cursor `.cursor` directory: `.cursor/hooks.json` and four POSIX
`sh` scripts in `.cursor/scripts/`. A factory selects it with `repo` and `path: .cursor`, so its
commands run from the repository root, exactly as Cursor runs them, and name their scripts as
`.cursor/scripts/...`.

| Event | Script | What it does |
| --- | --- | --- |
| `sessionStart` | `session_env.sh` | Sets `SAMPLE_HOOK_SESSION=started` for every later hook in the conversation. |
| `beforeSubmitPrompt` | `log_prompt.sh` | Appends `prompt_length=<n> session=<value>` to `.hook-prompts.log`, then lets the prompt through. |
| `beforeShellExecution` | `deny_dangerous.sh` | Denies `rm -rf` and `git push ... --force` with a reason the agent can read; allows everything else. Writes its latest input to `.hook-last-shell-input.json`. |
| `afterShellExecution` | `audit.sh` | Appends its whole input, one JSON object per line, to `.hook-audit.log`. |
| `stop` | `true` | Asks for no follow-up. |

Every file the scripts write lands in `$WARP_PROJECT_DIR`, which Warp sets to the repository root
for a source selected from a repository.
The scripts expect one JSON object on one line on stdin, which is what Warp sends; they match
fields with `grep` and `sed`, so they tolerate spaces after a colon but not a pretty-printed
object spread over several lines.

## `factory_layout/`

A factory definition repository, `acme-factory/`, that keeps hook directories beside its own
files. The tests check it out next to a copy of `sample_project/` named `api`, the way a run
clones each repository into its working directory, and build the run's hook list by hand from the
entries in `factory.yaml` and `agents/reviewer/agent.md`; nothing parses those two files.

| Directory | Selected by | What it does |
| --- | --- | --- |
| `hooks/` | `factory.yaml`, `strict: true` | `record_shell.sh` appends each shell command's input to `.factory-shell.log` and allows it. |
| `agents/reviewer/hooks/` | `agent.md`, `onAsk: fail_run` | `reviewer_policy.sh` denies commands starting with `deploy`, asks before ones starting with `publish`, and allows the rest. |
| `api/.cursor` (the copied sample) | `agent.md` | The sample's own hooks, unchanged. |

Commands in both `acme-factory` directories run from the repository root, so they name their
scripts from there (`hooks/record_shell.sh`, `agents/reviewer/hooks/reviewer_policy.sh`).

## `malformed/`

A `.cursor/hooks.json` with one definition whose `matcher` is not a valid regular expression and
one event name Cursor does not define. The file still loads: the bad definition and the unknown
event are skipped with warnings, and the other definition stays active.
