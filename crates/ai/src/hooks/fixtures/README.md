# Agent hook fixtures

These files back the end-to-end hook tests in `crates/ai/src/hooks/e2e_tests.rs` and
`app/src/ai/blocklist/hooks/e2e_tests.rs`. The tests copy them into a temporary directory and
make the scripts executable before running anything, so nothing here is ever written to.

## `sample_project/`

A project with a Cursor-format `.cursor/hooks.json` and four POSIX `sh` scripts in
`.cursor/scripts/`. The scripts read JSON from stdin with `grep` and `sed` only (no `jq`), and
every command is written relative to the project root, as Cursor's project hooks are.

| Event | Script | What it does |
| --- | --- | --- |
| `sessionStart` | `session_env.sh` | Sets `SAMPLE_HOOK_SESSION=started` for every later hook in the conversation. |
| `beforeSubmitPrompt` | `log_prompt.sh` | Appends `prompt_length=<n> session=<value>` to `.hook-prompts.log`, then lets the prompt through. |
| `beforeShellExecution` | `deny_dangerous.sh` | Denies `rm -rf` and `git push ... --force` with a reason the agent can read; allows everything else. Writes its latest input to `.hook-last-shell-input.json`. |
| `afterShellExecution` | `audit.sh` | Appends its whole input, one JSON object per line, to `.hook-audit.log`. |
| `stop` | `true` | Asks for no follow-up. |

Every file the scripts write lands in `$WARP_PROJECT_DIR`, which Warp sets to the project root.
The scripts expect one JSON object on one line on stdin, which is what Warp sends; they match
fields with `grep` and `sed`, so they tolerate spaces after a colon but not a pretty-printed
object spread over several lines.

### Trying it by hand

1. Make a scratch repository and copy the sample into it:

   ```sh
   fixture=$(mktemp -d)/sample && mkdir -p "$fixture" && cd "$fixture" && git init -q
   cp -R <warp checkout>/crates/ai/src/hooks/fixtures/sample_project/. .
   chmod +x .cursor/scripts/*.sh
   ```

2. Run `warp agent run` with the feature flag on, from that directory, with a profile whose
   "execute commands" setting is "always allow". A project's own hook file only takes effect on a
   command-line or Oz run whose profile already lets the agent run commands without asking.

3. Ask the agent to run `rm -rf ./scratch` and then list files. The `rm -rf` is denied with the
   hook's reason, `.hook-audit.log` gains one line per command that ran, and
   `.hook-prompts.log` gains one line per prompt.

`sample_project/run_hooks/hooks.json` binds the same scripts for use as a source delivered with
the run (`--hook-dir` or `WARP_HOOK_DIRS`). Such a file runs its commands from the directory that
holds it, not from the project root, so its commands start with `../.cursor/scripts/`. Pointing
`--hook-dir` at the sample's `.cursor` directory instead would run `.cursor/scripts/...` from
inside `.cursor`, which does not exist.

## `malformed/`

A `.cursor/hooks.json` with one definition whose `matcher` is not a valid regular expression and
one event name Cursor does not define. The file still loads: the bad definition and the unknown
event are skipped with warnings, and the other definition stays active.
