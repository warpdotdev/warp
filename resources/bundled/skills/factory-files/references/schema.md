# v1alpha1 field reference
Mirrors `logic/factoryfile` in `warp-server`. The JSON Schemas under
`schemas/` are the machine-readable form of everything here.

Two layers enforce these rules. The **parser** reads the tree and produces
`FF_*` diagnostics; it validates shape, field names, and enums. **Resolution
and apply** validate everything that needs server state, plus a few rules the
parser leaves alone (integration provider slugs, runner platform, instance
shapes, harness model catalogues). A file can parse cleanly and still be
rejected when the plan is applied.

## factory.yaml
Required: `schemaVersion`, `name`, `repositories`, `agentDefaults`.

- `schemaVersion` — `v1alpha1`, the only version these schemas describe. A tree
  declaring a different version is reported as unvalidatable rather than
  checked against v1alpha1 rules; never downgrade the value to silence that.
- `name` — non-empty string.
- `description` — free text.
- `alias` — display handle, used as the factory's @-mention name on integrated
  platforms. Letters, digits, spaces, `-`, `_`, `.`; at most 60 characters.
  The server trims surrounding whitespace before counting and storing it.
  Case is preserved; uniqueness is compared case-insensitively across the
  workspace. There is no lowercase or hyphenation requirement.
- `credentialStrategy` — `EXECUTOR` or `CREATOR`. Omitting it leaves the value
  already stored on the server untouched; it does not reset to a default.
  Explicit null has the same undeclared meaning.
- `repositories` — at least one `{owner, name}` pair, no other keys, no
  duplicates.
- `secrets` — list of managed secret names. Duplicates are rejected here.
- `mcpServers` — map of server name to `{warpId}`. `warpId` is the only key an
  entry may carry.
- `cloudProviders.gcp` — `projectNumber`, `workloadIdentityFederationPoolId`, and
  `workloadIdentityFederationProviderId` are all required;
  `serviceAccountEmail` is optional. Quote `projectNumber` so YAML keeps it a
  string.
- `cloudProviders.aws` — `roleArn` required.
- `providers` — legacy read-only alias for `cloudProviders`. New files should
  use `cloudProviders`; when both exist, the server uses `cloudProviders`.
- `integrations` — list of `{type}`. Current known types are `jira`, `linear`,
  and `slack`; preserve newer provider slugs.
  The current server rejects `github` because repository access comes from
  `repositories`; an older bundled schema leaves the final catalogue decision
  to a server plan. An empty list explicitly detaches every provider; omitting
  the section leaves the server-owned set alone.
- `agentDefaults` — execution defaults every agent inherits. Accepts `model`
  XOR `harness` (one is required), plus `runner`, `environmentId`, `secrets`,
  `mcpServers`, and `workerHost`. When `harness` is used here, both
  `harness.type` and `harness.model` are required.

## `agents/<name>/agent.md`
The directory name is the agent name. All frontmatter fields are optional; a
file with empty frontmatter is valid. The Markdown body is the agent's prompt.

- `description`
- `agentType` — `CUSTOM`, `MAIN`, `FOREMAN`, `TRIAGE`, `SPEC`, `IMPLEMENT`,
  `REVIEW`, `VERIFY`. `MAIN` is an authoring alias for `FOREMAN`. Omitting it
  resolves to `CUSTOM`. Exactly one agent in the tree must be `MAIN`/`FOREMAN`.
- `credentialStrategy` — as above.
- `model` / `harness` — mutually exclusive override. Null `model` inherits.
- `runner` — a runner name. It may name a runner declared under `runners/`, or
  an existing team runner that the tree does not declare. Null inherits.
- `environmentId` — null inherits.
- `secrets` — replaces the inherited list.
- `mcpServers` — replaces the inherited map.
- `workerHost` — self-hosted worker host. Null or empty clears an inherited
  host and defers to the workspace default.

## `automations/<name>/automation.md`
The directory name is the automation name. `triggers` is required and must
have at least one entry. The Markdown body is the run prompt.

- `enabled` — boolean, defaults to true.
- `agent` — name of a declared agent. Defaults to the MAIN agent.
- `model` / `harness`, `runner`, `environmentId`, `secrets`, `mcpServers`,
  `workerHost` — same semantics as on an agent.
- `triggers` — see `triggers.md`.

## `runners/<name>.yaml`
The file name is the runner name. All fields are optional to the parser, but
apply-time platform validation makes some effectively required.

- `description`
- `setupCommands` — list of shell commands.
- `instanceShape` — when present, both `vcpus` and `memoryGb` are required.
  Linux requires both to be positive powers of two, with no format-level upper
  bound. macOS accepts only `4/7`, `6/14`, `8/14`, `12/28`, and `12/56`.
- `platform.os` — `linux` or `macos`, defaulting to `linux`.
- `platform.arch` — `x86_64` or `aarch64`, defaulting to `x86_64` on Linux and
  `aarch64` on macOS. Supported pairs are `linux/x86_64`, `linux/aarch64`, and
  `macos/aarch64`.
- `platform.linux.dockerImage` — required for every Linux runner, including one
  that only defaults to Linux by omitting `platform.os`.
- `platform.mac.version` — `14`, `15`, `26`, or `27`. The whole `mac` section
  may be omitted, which defaults the version to `26`; if the section is
  present, `version` is required. Quote it so YAML keeps it a string. Only
  valid on macOS.

## `scorers/<name>/scorer.md`
The directory name is the Scorer name. The Markdown body is its required
rubric. Read `scorers.md` for the full contract and a worked example.

## The harness block
`model: <id>` is shorthand for `harness: {type: oz, model: <id>}`. Declaring
both `model` and `harness` is an error at every level.

- `harness.type` — `oz`, `claude` (`claude-code` is also accepted), `codex`,
  or `gemini`.
- `harness.model` — a model ID valid for that harness. On an override, null
  inherits; an empty string is invalid.
- `harness.reasoningLevel` — rejected by the current server on the `oz`
  harness. Per-harness capabilities change, so the bundled validator leaves
  that call to the server.
- `harness.auth` — rejected by the current server on the `oz` harness, on the
  same server-owned basis as `reasoningLevel`. Null explicitly clears inherited
  auth.
  - `auth.source: managedSecret` requires `auth.secretName`.
  - `auth.source: workerEnvironment` forbids `auth.secretName` and requires a
    self-hosted `workerHost`.

As an override on an agent or automation, `harness` must declare at least one
of `type`, `model`, `reasoningLevel`, or `auth`; an empty block is an error.

## Inheritance
Values flow `factory.agentDefaults` → agent → automation. Each layer overrides
the one above it for the fields it declares.

`secrets` and `mcpServers` **replace** rather than merge: an agent that
declares `secrets: []` gets no factory secrets. Additional secrets and MCP
servers the server requires (for example those backing a declared integration)
are merged in on top during resolution.

`workerHost` and `harness.reasoningLevel` are three-state: omitted inherits,
null or empty clears, and a non-empty value overrides. `harness.model`,
top-level `model`, `runner`, and `environmentId` inherit when omitted or null;
their empty-string form is invalid.

## YAML restrictions
Each file is a single YAML document. The parser rejects anchors (`&name`),
aliases (`*name`), explicit tags (`!!str`), merge keys (`<<`), duplicate
mapping keys, and non-scalar mapping keys. Markdown resources must open with a
`---` fence and close it before the body.
