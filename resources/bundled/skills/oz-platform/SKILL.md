---
name: oz-platform
description: Use Warp's REST API and command line to run, configure, and inspect Oz cloud agents
---

# oz-platform

Use the Oz REST API and CLI to:
* Spawn cloud agents
* Get the status of a cloud agent
* Schedule cloud agents to run repeatedly
* Create and manage the environments in which cloud agents run
* Provide secrets for cloud agents to use

## Command Line

The Oz CLI is installed as `{{warp_cli_binary_name}}`. To get help output, use `{{warp_cli_binary_name}} help` or `{{warp_cli_binary_name}} help <subcommand>`.
Prefer `--output-format text` to review the response, or `--output-format json` to parse fields with `jq`.
You can find more information at https://docs.warp.dev/reference/cli.

The most important commands are:
* `{{warp_cli_binary_name}} agent run-cloud`: Spawn a new cloud agent. You can configure the prompt, model, environment, and other settings.
* `{{warp_cli_binary_name}} run list` and `{{warp_cli_binary_name}} run get <run-id>`: List all cloud agent runs, and get details about a particular run.
* `{{warp_cli_binary_name}} environment list` and `{{warp_cli_binary_name}} environment get`: List available environments, and get more information about a particular environment.
* `{{warp_cli_binary_name}} schedule list` and `{{warp_cli_binary_name}} schedule get`: List scheduled tasks with most recent runs, and get more information about a particular scheduled run.

Most subcommands support the `--output-format json` flag to produce JSON output, which you can pipe into `jq` or other commands.

### Examples

Start a cloud agent, and then monitor its status:

```sh
$ {{warp_cli_binary_name}} agent run-cloud --prompt "Update the login error to be more specific" --environment UA17BXYZ
# ...
Spawned agent with run ID: 5972cca4-a410-42af-930a-e56bc23e07ac
```

```sh
$ {{warp_cli_binary_name}} run get 5972cca4-a410-42af-930a-e56bc23e07ac
# ...
```

Schedule an agent to summarize feedback every day at 8am UTC:

```sh
$ {{warp_cli_binary_name}} schedule create --cron "0 8 * * *" \
    --name "GitHub issue summary" \
    --prompt "Collect all feedback from new GitHub issues and provide a summary report" \
    --environment UA17BXYZ
```

List and inspect scheduled agents:

```sh
$ {{warp_cli_binary_name}} schedule list
$ {{warp_cli_binary_name}} schedule get <schedule-id>
```

Create a secret for cloud agents to use:

```sh
$ {{warp_cli_binary_name}} secret create JIRA_API_KEY --team --value-file jira_key.txt --description "API key to access Jira"
```

## REST API

Oz has a REST API for starting and inspecting cloud agents.

All API requests require authentication using an API key. The user can generate API keys in their Warp settings, on the `Platform` page (accessible via `{{warp_url_scheme}}://settings/platform`).

You can find the full OpenAPI specification here: https://docs.warp.dev/reference/api-and-sdk

### TypeScript / JavaScript SDK

The TypeScript SDK is available via NPM. It is fully async, and works with Node, Bun, and Deno.

* Package link: https://www.npmjs.com/package/oz-agent-sdk
* Source Code: https://github.com/warpdotdev/oz-sdk-typescript
* API reference: https://raw.githubusercontent.com/warpdotdev/oz-sdk-typescript/HEAD/api.md

### Python SDK

The Python SDK is available from PyPi. It can be used synchronously or asynchronously.

* Package link: https://pypi.org/project/oz-agent-sdk/
* Source Code: https://github.com/warpdotdev/oz-sdk-python
* API reference: https://raw.githubusercontent.com/warpdotdev/oz-sdk-python/refs/heads/main/api.md

### API Examples

```sh
curl -L -X POST {{warp_server_url}}/api/v1/agent/run \
    --header 'Authorization: Bearer YOUR_API_KEY' \
    --header 'Content-Type: application/json' \
    --data '{
        "prompt": "Update the login error to be more specific",
        "config": {
            "environment_id": "UA17BXYZ"
        }
    }'
```

```sh
curl -L -X GET {{warp_server_url}}/api/v1/agent/runs/5972cca4-a410-42af-930a-e56bc23e07ac \
    --header 'Authorization: Bearer YOUR_API_KEY' \
    --header 'Content-Type: application/json'
```

## GitHub Actions Integration

You can trigger Oz cloud agents from GitHub Actions workflows. This enables automation like:
* Triaging issues when they're created or labeled
* Running checks on pull requests
* Responding to CI events or deployment triggers

Use GitHub Actions when the trigger itself lives in GitHub: An event like an issue being opened, a PR being labeled, a push, or a CI workflow completing.

For periodic/recurring work, prefer `{{warp_cli_binary_name}} schedule create` to enhance scheduled run tracking with the Oz platform.

The agent will have access to the `gh` CLI to communicate back to the repository. Prefer prompting the agent to use `gh` vs. requiring the agent to respond with structured output for the GitHub workflow to parse.

### Action Setup

Use `warpdotdev/oz-agent-action@main` in your workflow. Required inputs:
* `prompt`: The task description for the agent
* `warp_api_key`: API key (store in GitHub secrets, e.g., `${{ secrets.WARP_API_KEY }}`)
* `profile`: Optional agent profile identifier (can use repo variable, e.g., `${{ vars.WARP_AGENT_PROFILE || '' }}`)

The action outputs `agent_output` with the agent's response.

### Minimal Workflow Example

```yaml
name: Run Oz Agent
on:
  issues:
    types: [opened, labeled]

jobs:
  agent:
    runs-on: ubuntu-latest
    permissions:
      contents: write
      issues: write
      pull-requests: write
    steps:
      - uses: actions/checkout@v6
      - uses: warpdotdev/oz-agent-action@main
        id: agent
        with:
          prompt: |
            Analyze the GitHub issue and provide a summary.
            Issue: ${{ github.event.issue.title }}
            ${{ github.event.issue.body }}

            Respond to the issue with a comment containing your summary using the `gh` CLI.
          warp_api_key: ${{ secrets.WARP_API_KEY }}
          profile: ${{ vars.WARP_AGENT_PROFILE || '' }}
      - name: Use Agent Output
        run: echo "${{ steps.agent.outputs.agent_output }}"
```

### Common Patterns

**Conditional steps**: Use `if: steps.agent.outputs.agent_output` to branch on agent results.

**Templating**: Use `actions/github-script@v7` to construct dynamic prompts from issue templates, repo context, or code.

**Error handling**: Check action success with `if: success()` or `if: failure()`.

**Git operations**: The action runs with checked-out code and Git credentials, so agents can commit and push changes.


## Environments

All cloud agents run in an environment. The environment defines:
* Which programs are preinstalled for the agent (based on a Docker image)
* The Git repositories to check out before the agent starts
* Setup commands to run, such as `npm install` or `cargo fetch`

You should almost always run cloud agents in an environment. Otherwise, they may not have the necessary code or tools available.

Cloud agents run in a sandbox, so they _can_ install additional programs into their environment. They also have Git credentials to create PRs and push branches.

Cloud environments DO NOT store secret values, like API keys. Use the `{{warp_cli_binary_name}} secret` commands instead.

## Using Third-Party Coding CLIs

Oz environments support running third-party coding agent CLIs such as Claude Code, Codex, Gemini CLI, Amp, Copilot CLI, and OpenCode. The `-agents` tagged variants of prebuilt Oz Docker images (e.g. `warpdotdev/dev-rust:1.85-agents`) come with the most popular CLIs preinstalled. Base tags (without `-agents`) do not include coding agent CLIs.

For detailed per-CLI documentation (installation, authentication, non-interactive flags, and artifact reporting), see [references/third-party-clis.md](./references/third-party-clis.md).

### For Interactive Agents: Launching Cloud Agents with Third-Party CLIs

When you are an interactive agent launching a cloud agent to use a third-party CLI:

1. **Environment Selection**: First, ask the user which environment to use. Present the public `-agents` image options from [warpdotdev/oz-dev-environments](https://github.com/warpdotdev/oz-dev-environments):
   - `warpdotdev/dev-base:latest-agents`
   - `warpdotdev/dev-go:1.23-agents`
   - `warpdotdev/dev-rust:1.83-agents`
   - `warpdotdev/dev-rust:1.85-agents`
   - `warpdotdev/dev-java:21-agents`
   - `warpdotdev/dev-dotnet:8.0-agents`
   - `warpdotdev/dev-ruby:3.3-agents`
   - `warpdotdev/dev-web:latest-agents`
   - `warpdotdev/dev-full:latest-agents`

   Also ask if they want to use an existing environment instead. To list their environments:
   ```sh
   {{warp_cli_binary_name}} environment list --output-format text
   ```

   If they choose a public image without an existing environment, create one with `{{warp_cli_binary_name}} environment create ...`

2. **Prompt Construction**: Construct a simple prompt that delegates CLI invocation to the cloud agent:
   ```sh
   {{warp_cli_binary_name}} agent run-cloud \
       --environment <ENV_ID> \
       --prompt 'Read the oz-platform skill for instructions on using [CLI name] to solve: <task description>'
   ```

   **Do not** include the CLI command syntax in the prompt. The cloud agent will read the oz-platform skill and follow those instructions.

**Example**:
```sh
$ {{warp_cli_binary_name}} agent run-cloud \
    --environment <ENV_ID> \
    --prompt 'Read the oz-platform skill for instructions on using Claude Code to solve: Summarize the architecture of this project'
```

### For Cloud Agents: Invoking Third-Party CLIs

When you are a cloud agent instructed to use a third-party CLI:

1. **Environment**: You are already running in an environment with the CLI preinstalled (if it's in an `-agents` image).

2. **Authentication**: API keys are available as environment variables (e.g. `ANTHROPIC_API_KEY`, `OPENAI_API_KEY`, `GEMINI_API_KEY`). These were configured as Oz secrets by the user.

3. **Task Delegation (IMPORTANT)**: The user's task should be completed **entirely by the third-party CLI**. Do NOT use Warp's built-in tools to complete the task yourself:
   - Do NOT use `edit_files`, `read_files`, `grep`, `codebase_semantic_search`, or other Warp coding tools to perform the user's task
   - The third-party CLI should do all the coding, file editing, searching, and analysis work
   - Your role is to:
     - Set up the CLI (e.g., authenticate if needed)
     - Construct the prompt for the CLI with the user's task
     - Run the CLI and monitor its execution
     - Debug any issues with the CLI itself
     - Report artifacts back to Warp (see below)

4. **CLI Invocation**: Read [references/third-party-clis.md](./references/third-party-clis.md) for detailed instructions on:
   - Non-interactive mode flags for each CLI (e.g. `claude -p`, `codex exec`, `gemini -p`)
   - Authentication setup steps if needed (e.g. Codex requires `printenv OPENAI_API_KEY | codex login --with-api-key`)
   - Useful flags and options
   - Example commands

5. **Artifact Reporting**: When the third-party CLI creates a PR, parse its output for the PR URL and branch name, then call `report_pr` to register the artifact in the Warp UI.

**Example workflow**:
```sh
# 1. Read this skill and references/third-party-clis.md to understand CLI usage

# 2. Set up authentication if needed (e.g., for Codex)
# For Claude Code, ANTHROPIC_API_KEY is already available

# 3. Run the CLI with the user's task - let it do ALL the work
$ claude -p "Summarize the architecture of this project"

# 4. If a PR was created, parse the CLI output and report the artifact
# Example: report_pr(pr_url="https://github.com/...", branch="feature-branch")
```

**What NOT to do**:
```sh
# ❌ Don't read files yourself to help the CLI
$ read_files ...

# ❌ Don't search the codebase yourself
$ grep ...

# ❌ Don't edit files yourself
$ edit_files ...

# ✅ Instead, let the third-party CLI handle everything
$ claude -p "Complete the entire task: <user's task>"
```
