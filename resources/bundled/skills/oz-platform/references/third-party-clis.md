# Third-Party Coding Agent CLIs

**Context**: This reference is for cloud agents who need to invoke third-party coding CLIs. If you are an interactive agent launching a cloud agent, see the "For Interactive Agents" section in the main oz-platform skill instead.

This reference documents third-party coding agent CLIs that Oz cloud agents can install and run. Each CLI has its own authentication, non-interactive mode, and nuances.

The `-agents` tagged variants of prebuilt Oz Docker images (e.g. `warpdotdev/dev-base:latest-agents`, `warpdotdev/dev-rust:1.85-agents`) include Claude Code, Codex, and Gemini CLI preinstalled. Base tags (without `-agents`) do not include coding agent CLIs.

> **Note**: Commands below that reference the Warp CLI should use the CLI binary name from the parent skill (the `{{warp_cli_binary_name}}` value).

## Reporting Artifacts

When a third-party CLI creates a PR or branch, Oz does not automatically detect it. You must explicitly register these artifacts so they appear in the Warp UI.

**Pull Requests**: After a third-party CLI creates a PR, call the `report_pr` tool with the `pr_url` and `branch` parameters. Parse the CLI's output to extract the PR URL and branch name.

**How to prompt for this**: When writing prompts that instruct an Oz agent to delegate work to a third-party CLI, always include instructions to:
1. Tell the third-party CLI to print the PR URL and branch name in its output.
2. After the CLI finishes, parse the output and call `report_pr` to register the artifact.

**Plans**: Plan artifacts are registered automatically by the Oz agent when it creates plans using its built-in plan tools. No special handling is needed.

---

## Claude Code

- **Package**: `npm install -g @anthropic-ai/claude-code`
- **Auth**: `ANTHROPIC_API_KEY` environment variable (automatic).
- **Non-interactive mode**: `claude -p "<prompt>"`
- **Useful flags**: `--output-format json`, `--allowedTools "Read,Write,Bash(...)"`, `--append-system-prompt`

### Oz secret

```sh
{{warp_cli_binary_name}} secret create ANTHROPIC_API_KEY --team --value-file key.txt --description "Anthropic API key for Claude Code"
```

### Example prompt

```
Run: claude -p "Analyze the codebase and list potential security issues" --output-format json
```

### Artifact reporting prompt

```
Run Claude Code to implement the feature:
  claude -p "Create a branch, implement the login fix, commit, push, and create a PR using gh CLI. Print the full PR URL and branch name at the end."

After Claude Code finishes, find the PR URL and branch name from its output.
Then call the report_pr tool with those values so the PR appears as an artifact.
```

---

## Codex CLI

- **Package**: `npm install -g @openai/codex`
- **Auth**: Requires an explicit login step before use:
  ```
  printenv OPENAI_API_KEY | codex login --with-api-key
  ```
  Alternatively, `CODEX_API_KEY` can be set directly.
- **Recommended**: Add `printenv OPENAI_API_KEY | codex login --with-api-key` as an environment setup command so authentication happens automatically before the agent starts.
- **Non-interactive mode**: `codex exec "<prompt>"`
- **Regional endpoint**: Set `OPENAI_BASE_URL` if needed (e.g. `https://us.api.openai.com/v1`).
- **Useful flags**: `--full-auto`, `--sandbox workspace-write`, `--json`, `--skip-git-repo-check`

### Oz secret

```sh
{{warp_cli_binary_name}} secret create OPENAI_API_KEY --team --value-file key.txt --description "OpenAI API key for Codex CLI"
```

### Example prompt

```
First authenticate Codex: printenv OPENAI_API_KEY | codex login --with-api-key
Then run: codex exec "Refactor the utils module to reduce duplication"
```

### Artifact reporting prompt

```
Authenticate Codex: printenv OPENAI_API_KEY | codex login --with-api-key
Then run: codex exec --full-auto "Create a branch, fix the bug, commit, push, and create a PR. Print the PR URL and branch name."
After Codex finishes, parse the PR URL and branch name from its output and call report_pr.
```

---

## Gemini CLI

- **Package**: `npm install -g @google/gemini-cli`
- **Auth**: `GEMINI_API_KEY` environment variable (automatic). Obtain from [Google AI Studio](https://aistudio.google.com/apikey).
- **Non-interactive mode**: `gemini -p "<prompt>"` (headless mode)
- **Useful flags**: `--output-format json`, `--yolo` (auto-approve tool actions)

### Oz secret

```sh
{{warp_cli_binary_name}} secret create GEMINI_API_KEY --team --value-file key.txt --description "Gemini API key for Gemini CLI"
```

### Example prompt

```
Run: gemini -p "Review the test suite and suggest missing edge cases" --output-format json
```

### Artifact reporting prompt

```
Run Gemini CLI:
  gemini -p "Create a branch, implement the change, commit, push, and create a PR using gh CLI. Print the full PR URL and branch name at the end." --yolo
After it finishes, parse the PR URL and branch from the output and call report_pr.
```

---

## Amp

- **Package**: `npm install -g @sourcegraph/amp`
- **Auth**: `AMP_API_KEY` environment variable. Obtain from [ampcode.com/settings](https://ampcode.com/settings). In isolated mode, uses `ANTHROPIC_API_KEY` instead.
- **Non-interactive mode**: `amp -x "<prompt>"` (execute mode)
- **Useful flags**: `--dangerously-allow-all` (skip tool approval prompts)

### Oz secret

```sh
{{warp_cli_binary_name}} secret create AMP_API_KEY --team --value-file key.txt --description "Amp API key"
```

### Example prompt

```
Run: amp -x "List all TODO comments in the codebase and group them by priority"
```

### Artifact reporting prompt

```
Run Amp:
  amp --dangerously-allow-all -x "Create a branch, implement the fix, commit, push, and create a PR using gh CLI. Print the full PR URL and branch name."
After Amp finishes, parse the PR URL and branch from the output and call report_pr.
```

---

## Copilot CLI

- **Binary**: `copilot` (standalone, from [github/copilot-cli](https://github.com/github/copilot-cli))
- **Auth**: `GH_TOKEN` or `GITHUB_TOKEN` environment variable with a fine-grained PAT that has the **Copilot Requests** permission. Also supports `COPILOT_GITHUB_TOKEN`.
- **Non-interactive mode**: `copilot -p "<prompt>"`
- **Useful flags**: `--allow-all-tools`
- **Note**: The `gh copilot` extension (distinct from standalone `copilot`) requires OAuth and does **not** work with PATs.
- **Not preinstalled** in Oz images. Install via setup commands or GitHub releases.

### Oz secret

```sh
{{warp_cli_binary_name}} secret create GH_TOKEN --team --value-file token.txt --description "GitHub PAT with Copilot Requests permission"
```

### Example prompt

```
Run: copilot -p "Review the latest changes and suggest improvements" --allow-all-tools
```

### Artifact reporting prompt

```
Run Copilot CLI:
  copilot -p "Create a branch, implement the fix, commit, push, and create a PR using gh CLI. Print the full PR URL and branch name at the end." --allow-all-tools
After Copilot finishes, parse the PR URL and branch from the output and call report_pr.
```

---

## OpenCode

- **Install**: `curl -fsSL https://opencode.ai/install | bash` (or via binary release)
- **Auth**: Uses provider-specific API keys via environment variables (e.g. `ANTHROPIC_API_KEY`, `OPENAI_API_KEY`). Also reads from `.env` files. Run `opencode auth login` to configure interactively.
- **Non-interactive mode**: `opencode run "<prompt>"` or `opencode -p "<prompt>"`
- **Useful flags**: `-f json` (JSON output), `-q` (quiet/no spinner)
- **Not preinstalled** in Oz images. Install via setup commands.

### Example prompt

```
Run: opencode run "Explain the architecture of this project" -q
```

---

## Droid (Factory)

- **Install**: `curl -fsSL https://app.factory.ai/cli | sh`
- **Auth**: Requires a Factory account. Use `/login` in the CLI or generate an API key from Factory Settings. **Headless env-var auth is not yet confirmed** — this CLI may require interactive login.
- **Non-interactive mode**: `droid exec "<prompt>"`
- **Useful flags**: `--auto low|medium|high` (permission tier), `--skip-permissions-unsafe`
- **Status**: **Not currently supported** for headless Oz environments due to unclear non-interactive auth. Excluded from prebuilt images.

---

## Quick Reference

| CLI | Command | Auth Env Var | Non-Interactive Flag | Preinstalled |
|-----|---------|-------------|---------------------|-------------|
| Claude Code | `claude` | `ANTHROPIC_API_KEY` | `-p` | Yes |
| Codex | `codex` | `OPENAI_API_KEY` | `exec` | Yes |
| Gemini CLI | `gemini` | `GEMINI_API_KEY` | `-p` | Yes |
| Amp | `amp` | `AMP_API_KEY` | `-x` | No |
| Copilot CLI | `copilot` | `GH_TOKEN` / `GITHUB_TOKEN` | `-p` | No |
| OpenCode | `opencode` | Provider-specific | `run` / `-p` | No |
| Droid | `droid` | N/A (interactive login) | `exec` | No |
