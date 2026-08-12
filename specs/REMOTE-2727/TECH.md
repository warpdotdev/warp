# Bundled skill for authoring Factory files
## Summary
[REMOTE-2727](https://linear.app/warpdotdev/issue/REMOTE-2727/bundled-skill-for-authoringediting-file-based-factory-definitions) adds one bundled skill that teaches agents to create and edit file-based software factory definitions. The skill ships in Warp for the native GUI, TUI, and every Oz cloud agent. Byte-identical copies ship in the Claude Code and Codex Oz platform plugins because third-party harnesses cannot resolve a Warp `bundled_skill_id`.

This work needs only a technical spec. The user workflow is already defined, and the remaining decisions concern packaging, rollout, validation, and schema maintenance.

The implementation uses `warpdotdev/warp` as its primary repository. Warp owns the canonical skill copy and the shared distribution path for three of the four requested surfaces. `warp-server` remains authoritative for the Factory file schema. The Claude Code and Codex plugin repositories contain downstream mirrors.

## Context
The Factory file parser accepts a versioned, path-derived tree:

- `factory.yaml`
- `agents/<name>/agent.md`
- `automations/<name>/automation.md`
- `runners/<name>.yaml`
- `skills/**`
- `agents/<name>/skills/**`

The parser also accepts the legacy flat path `automations/<name>.md`, but rendering always emits the directory form. The skill must create the canonical directory form and may edit an existing flat file without moving it unless the user asks for normalization.

The current schema is `v1alpha1`. The parser rejects unknown fields, duplicate keys, YAML anchors, aliases, explicit tags, malformed frontmatter, invalid paths, invalid references, and invalid field values. It also requires exactly one `MAIN` or `FOREMAN` agent. The following sources are authoritative:

- [`logic/factoryfile/v1alpha1.go (17-149) @ 7cddb925`](https://github.com/warpdotdev/warp-server/blob/7cddb92584ae4164c3fe34b0a88ed5ab5c75a24f/logic/factoryfile/v1alpha1.go#L17-L149) defines the schema version, allowed field sets, whole-tree parsing, and the main-agent rule.
- [`logic/factoryfile/path.go (5-50) @ 7cddb925`](https://github.com/warpdotdev/warp-server/blob/7cddb92584ae4164c3fe34b0a88ed5ab5c75a24f/logic/factoryfile/path.go#L5-L50) defines canonical resource and skill paths.
- [`logic/factoryfile/agenttype.go (10-43) @ 7cddb925`](https://github.com/warpdotdev/warp-server/blob/7cddb92584ae4164c3fe34b0a88ed5ab5c75a24f/logic/factoryfile/agenttype.go#L10-L43) defines accepted agent types and the `MAIN` alias.
- [`logic/factoryfile/credentialstrategy.go (11-31) @ 7cddb925`](https://github.com/warpdotdev/warp-server/blob/7cddb92584ae4164c3fe34b0a88ed5ab5c75a24f/logic/factoryfile/credentialstrategy.go#L11-L31) defines credential strategies.
- [`logic/factoryfile/parsetree.go (26-208) @ 7cddb925`](https://github.com/warpdotdev/warp-server/blob/7cddb92584ae4164c3fe34b0a88ed5ab5c75a24f/logic/factoryfile/parsetree.go#L26-L208) selects the adapter and classifies resource paths.
- [`logic/factoryfile/parsetree_test.go (13-77) @ 7cddb925`](https://github.com/warpdotdev/warp-server/blob/7cddb92584ae4164c3fe34b0a88ed5ab5c75a24f/logic/factoryfile/parsetree_test.go#L13-L77) holds the existing parse/render schema-contract golden test.

The schema is still changing. For example, `integrations` is already accepted by the current `factoryFields` set even though older summaries of the format omit it. A manually copied field table will become stale.

Warp already provides the common distribution mechanism:

- [`app/src/ai/skills/bundled.rs (346-445) @ 352a7fc1`](https://github.com/warpdotdev/warp/blob/352a7fc10707fd6a8ef3341961870746f4a9c557/app/src/ai/skills/bundled.rs#L346-L445) loads directories under `resources/bundled/skills` and uses the directory name as the bundled skill ID.
- [`app/src/ai/skills/bundled.rs (553-572) @ 352a7fc1`](https://github.com/warpdotdev/warp/blob/352a7fc10707fd6a8ef3341961870746f4a9c557/app/src/ai/skills/bundled.rs#L553-L572) defaults unlisted skills to `BundledSkillActivation::Always`.
- [`script/prepare_bundled_resources (60-83) @ 352a7fc1`](https://github.com/warpdotdev/warp/blob/352a7fc10707fd6a8ef3341961870746f4a9c557/script/prepare_bundled_resources#L60-L83) copies the bundled tree into client and remote-server distributions.

The live Factory example at [`factory-dev/v1/factory.yaml @ b17d6b27`](https://github.com/warpdotdev/factory-dev/blob/b17d6b275c40679c42ddc19a71dac36c5adf8d86/v1/factory.yaml) demonstrates the canonical root. Its agent, automation, runner, and skill directories provide realistic examples, but they are not the schema authority.

Third-party harness plugins load filesystem skills instead of Warp bundled IDs:

- Claude Code loads Oz skills from [`plugins/oz-harness-support/skills @ e0e18e16`](https://github.com/warpdotdev/claude-code-warp/tree/e0e18e162aa5ec97a6d4cecdf66b08b75eb1e149/plugins/oz-harness-support/skills).
- Codex loads Oz skills from [`plugins/orchestration/skills @ f11334dc`](https://github.com/warpdotdev/codex-warp/tree/f11334dc715d48f07d133db6046895cb45f4accb/plugins/orchestration/skills).
- Warp enforces minimum platform plugin versions in [`claude.rs (17-26) @ 352a7fc1`](https://github.com/warpdotdev/warp/blob/352a7fc10707fd6a8ef3341961870746f4a9c557/app/src/terminal/cli_agent_sessions/plugin_manager/claude.rs#L17-L26) and [`codex.rs (23-31) @ 352a7fc1`](https://github.com/warpdotdev/warp/blob/352a7fc10707fd6a8ef3341961870746f4a9c557/app/src/terminal/cli_agent_sessions/plugin_manager/codex.rs#L23-L31).

## Decisions
### Skill identity and trigger
Use:

- Bundled skill ID and frontmatter `name`: `factory-files`
- User-facing title: `Factory Files`
- Trigger description:

  > Create and edit file-based Warp software factory definitions. Use when authoring or changing `factory.yaml`, agent or automation Markdown, runner YAML, or factory and agent skill trees, and when fixing Factory file diagnostics. Do not use to operate a live factory or hand work to a factory through Factory MCP.

`factory-files` names the artifact instead of the deployment model. `factory-as-code` is broader and can imply sync, deployment, or live operations.

The negative trigger boundary is required:

- `factory-files` authors and edits repository files.
- `factory-mcp` operates a factory and transfers work through MCP.
- Skills under `factory-dev/**/skills` define factory-agent playbooks. They do not define the file format.

### Rollout
Ship the skill in the stable, always-bundled directory from the first release. Do not add a feature flag or channel gate.

The skill is documentation-only and has no server dependency. A dogfood or preview gate would delay the requested default availability and would not reduce third-party plugin risk. The moving `v1alpha1` format is addressed through generated references and drift checks, not by limiting discovery.

Do not add a match arm to `activation_for_bundled_skill`. The default `Always` activation is intentional.

### Oz scope
Make the skill available to every Oz agent.

All Oz agents already receive the shared Warp bundled-resource catalog. No factory-only bundled activation exists. Adding a new environment or agent-type gate would increase implementation and testing scope for no material safety benefit. Skill metadata is small, and full content is loaded only when the agent reads the skill.

### Skill structure
Use one skill for both creation and editing. Both workflows use the same paths, inheritance rules, field constraints, and diagnostics. Splitting them would duplicate schema context and create competing triggers.

Use progressive disclosure:

- `SKILL.md`
  - Establish the authoring boundary.
  - Locate the Factory root.
  - Inspect existing files before editing.
  - Select create, edit, or diagnose workflow.
  - Require canonical paths for new resources.
  - Preserve unrelated fields and prompt bodies when editing.
  - Point to the smallest relevant reference.
- `references/v1alpha1.md`
  - Generated path rules, field tables, enums, defaults, inheritance rules, and validation constraints.
  - A generated header records the `warp-server` source commit and a digest of the schema inputs.
- `references/examples.md`
  - Curated minimal root, full harness, agent, automation, inline schedule, runner, and scoped-skill examples.
  - Examples use placeholder IDs and secrets. They must not copy production credentials or identifiers from `factory-dev`.
- `references/validation.md`
  - Diagnostic codes and a correction workflow.
  - Existing plan/apply diagnostics are described as optional validation for an already registered Factory, not as a local validator.

Keep `SKILL.md` workflow-oriented. Do not repeat complete field tables in it.

### Validation helper
Do not ship a validation script in the first cut.

The authoritative parser is a pure Go package in `warp-server`, but it is not distributed with the Warp client or harness plugins. The existing `POST /api/v1/factory/:uid/plan` route validates a committed SHA for an authenticated, registered Factory. It cannot validate an arbitrary local tree. A shell-only validator would duplicate only part of the schema and could incorrectly approve files that the server rejects.

The skill should:

- Make conservative edits.
- Use repository tests when the parser source is available.
- Use plan diagnostics when the Factory is registered and the user requests a plan.
- Report when authoritative validation is unavailable.

A standalone parser-backed CLI or an API that accepts an arbitrary source tree is a follow-up. Do not add either endpoint in this work item.

### Schema ownership and generated reference
Eng-Platform FA owns the generated schema reference because it owns `logic/factoryfile`. A change author who modifies the Factory file format owns the companion documentation update.

Add a deterministic generator and golden test in `warp-server`:

1. Generate `logic/factoryfile/testdata/schema_contract/v1alpha1-reference.md` from the in-package field sets, canonical path helpers, `AgentTypeTokens`, and `CredentialStrategyTokens`.
2. Include manually curated rule records for constraints that field sets cannot express, such as required fields, `model` XOR `harness`, defaults, inheritance, main-agent cardinality, schedule restrictions, and harness authentication rules.
3. Embed a SHA-256 digest of these schema inputs:
   - `v1alpha1.go`
   - `path.go`
   - `agenttype.go`
   - `credentialstrategy.go`
4. Add `TestV1Alpha1ReferenceGolden`. The test fails when generated content or the source digest differs from the checked-in reference.
5. Expand `TestParseTree_SchemaContractFixture` so its fixture exercises every documented resource shape and accepted top-level field, including `integrations`, full harness authentication, automations, schedules, and runners.
6. Reuse the existing `-update-golden` convention to regenerate the reference and canonical fixture.

The digest deliberately makes any edit to the schema inputs an explicit documentation event, including a constraint change that does not add a field. The golden test cannot prove prose quality. Eng-Platform FA review and the expanded valid/invalid parser tests remain required.

Every schema-changing `warp-server` PR must link a companion `warp` PR that refreshes `references/v1alpha1.md`, or state why the digest changed without user-facing schema impact. This requirement belongs in the Factory file code-owner checklist. Do not rely on an unowned reminder in this skill.

### Canonical copy and mirror drift
The canonical authored skill is:

`warp/resources/bundled/skills/factory-files/`

Mirror that directory byte-for-byte to:

- `claude-code-warp/plugins/oz-harness-support/skills/factory-files/`
- `codex-warp/plugins/orchestration/skills/factory-files/`

Add `sync-manifest.json` inside the canonical skill directory. It records SHA-256 hashes for `SKILL.md` and every file under `references/`. It does not hash itself.

Add `warp/script/sync_factory_files_skill` with two modes:

- `--write <plugin-skill-dir>` replaces a mirror from the Warp source and writes plugin-local upstream metadata outside the mirrored directory.
- `--check <plugin-skill-dir>` verifies byte equality and all manifest hashes.

Each plugin repository stores the Warp source commit in its test metadata. Plugin CI must:

- Fetch the skill from that exact public Warp commit.
- Run the equivalent byte and manifest comparison.
- Run on pull requests and on a daily schedule.
- On the scheduled run, also compare the pinned manifest to `warpdotdev/warp` `master`. A mismatch fails the job and identifies that the mirror needs a release.

The exact source pin makes plugin releases reproducible. The scheduled comparison makes an old but internally consistent pin visible instead of allowing it to remain stale indefinitely.

Do not maintain independently edited plugin variants. Harness-specific instructions belong outside `factory-files`.

### Plugin releases and Warp minimums
Both platform plugins need patch releases because existing installations do not contain the new filesystem skill.

At the revisions researched for this spec:

- Claude `oz-harness-support` is `1.1.2`; release at least `1.1.3`.
- Codex `orchestration` is `0.4.0`; release at least `0.4.1`.

If another release occurs first, use the next patch version instead. Update both the marketplace manifest and plugin manifest in each repository. Update plugin README skill lists and version text where applicable.

After both plugin releases exist, update only the matching platform minimums in Warp:

- `claude.rs::MINIMUM_PLATFORM_PLUGIN_VERSION`
- `codex.rs::MINIMUM_PLATFORM_PLUGIN_VERSION`

Do not bump `MINIMUM_PLUGIN_VERSION` for the separate local notification plugins.

## Implementation sequence
Use the spec PR branch as the primary Warp implementation branch after approval.

1. Add the schema-reference generator, golden test, expanded fixture, and generated reference in `warp-server`.
2. Add the canonical skill, manifest, sync script, trigger tests, and packaging tests to the Warp spec PR.
3. Copy the approved Warp skill tree into Claude Code and Codex plugin branches. These two plugin changes can proceed in parallel.
4. Merge and release both plugin patch versions.
5. Update the Warp platform minimum version constants to the released versions.
6. Run cross-repository drift checks and final skill evaluations.
7. Merge the Warp PR only after the plugin versions named by its minimums are available.

This order prevents Warp from requiring unpublished plugin versions. It also lets plugin copies pin the exact pushed Warp source commit before the Warp PR merges.

## Testing and validation
### Schema contract
Run in `warp-server`:

- `go test ./logic/factoryfile -run 'TestV1Alpha1ReferenceGolden|TestParseTree_SchemaContractFixture'`
- The generated `references/v1alpha1.md` copy in Warp must have the same schema-input digest as the server golden.

### Warp bundle
Add focused tests under the existing bundled-skill test module:

- `factory-files` parses with name `factory-files`.
- Its trigger description contains create/edit intents and the Factory MCP exclusion.
- `activation_for_bundled_skill("factory-files", ...)` returns `Always`.
- `sync-manifest.json` matches all canonical skill files.

Run:

- The focused bundled-skill Rust tests selected by their final test names.
- `SKIP_SETTINGS_SCHEMA=1 NO_LICENSES=1 script/prepare_bundled_resources <temp-dir> stable`
- Assert `<temp-dir>/bundled/skills/factory-files/SKILL.md` and all references exist.

### Skill behavior
Create deterministic eval prompts and compare the skill-assisted output with a baseline:

1. Create a minimal Factory with one main agent from a repository and model request.
2. Edit an existing Factory to add one runner and one automation while preserving unrelated fields and Markdown bodies.
3. Repair files containing an unknown field, two main agents, and an invalid automation reference from supplied diagnostics.
4. Update an agent-scoped skill without moving it to the Factory-wide skill directory.
5. Ask to send work to a live factory. `factory-files` must not trigger; `factory-mcp` is the correct skill.
6. Ask to change a factory agent's review playbook. The agent must edit the playbook and must not treat it as a Factory file schema change.

Each generated tree must pass `factoryfile.ParseTree` through a small test harness in `warp-server`. Trigger evaluations must cover GUI/Oz bundled metadata and both plugin filesystem metadata.

### Plugin mirrors
In each plugin repository:

- Run the new mirror check against the pinned Warp commit.
- Parse the marketplace and plugin manifests.
- Assert `skills/factory-files/SKILL.md` is packaged.
- Run the repository's existing shell test suite.

After release, verify a fresh Claude Code Oz run and a fresh Codex Oz run list `factory-files` as a filesystem skill and can read its references.

### Release check
Before the Warp PR merges:

- Confirm the Claude and Codex released versions are greater than or equal to the updated platform minimums.
- Confirm GUI, TUI, and remote-server stable bundles contain the same manifest.
- Confirm all three repository copies of the skill directory are byte-identical.

No visual recording is required. This feature changes agent context and generated files, not a rendered UI workflow.

## Risks and mitigations
- **Schema prose can still be wrong.** Generated field sets and digests detect drift but cannot infer every semantic rule. Mitigate with curated rule records, expanded valid and invalid parser tests, and Eng-Platform FA review.
- **A plugin mirror can remain pinned to an old valid commit.** Mitigate with the daily comparison against Warp `master` and an owning team for failures.
- **Stable rollout increases skill-list size for unrelated agents.** The metadata cost is limited to the name and trigger description. Full content remains progressively disclosed.
- **The trigger can collide with Factory MCP.** Include explicit positive and negative trigger evals and keep operation verbs out of the positive description.
- **Examples can leak environment-specific values.** Use placeholders. Do not copy production IDs, secrets, account numbers, or images from live factories.
- **Version ordering can strand existing plugin installations.** Release plugins before merging the Warp minimum-version bump.

## Out of scope
- Changing the `v1alpha1` parser or adding a new schema version.
- Adding JSON Schema, editor completion, a form-based Factory editor, or GUI/TUI authoring UI.
- Adding a local validator binary or an arbitrary-tree diagnostics API.
- Operating, creating, or dispatching work to live factories through Factory MCP.
- Rewriting factory-agent playbooks under `factory-dev/**/skills`.
- Migrating existing flat automation files or normalizing existing Factory trees without a user request.
- Supporting third-party harness plugins other than Claude Code and Codex.
- Automatically publishing plugin releases or automatically merging schema companion PRs.
- Guaranteeing that provider-specific automation filter keys are valid without authoritative server diagnostics.

## Assumptions
- The requester delegated the seven open engineering decisions to this spec and asked for decisive recommendations instead of an additional interview round.
- `v1alpha1` remains the only registered Factory tree adapter for the first release.
- The plugin repositories and the canonical Warp skill remain readable to their CI jobs so cross-repository manifest checks can fetch pinned commits.
