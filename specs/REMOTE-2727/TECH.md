# Bundled skill for authoring Factory files
## Summary
[REMOTE-2727](https://linear.app/warpdotdev/issue/REMOTE-2727/bundled-skill-for-authoringediting-file-based-factory-definitions) adds one bundled skill that teaches agents to create and edit file-based software factory definitions. The skill ships in Warp for the native GUI, TUI, and every Oz cloud agent. Copies ship in the Claude Code and Codex Oz platform plugins because third-party harnesses cannot resolve a Warp `bundled_skill_id`.

The skill carries machine-readable JSON Schemas for the format and a dependency-free validator that runs them, so an agent can check its own edits before opening a pull request. That requirement came from the Factory bug bash, where the recurring failures were malformed frontmatter metadata that the parser accepts and the apply step later rejects.

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

  > Create and edit file-based Warp software factory definitions, in a repository tree rooted at a factory.yaml. Use when authoring or changing that factory.yaml, the agent.md, automation.md, or runner YAML files beside it, or its factory and agent skill trees, and when fixing Factory file diagnostics. Do not use for agent-definition Markdown that belongs to another tool, for a tree with no factory.yaml, or to operate a live factory or hand work to one through Factory MCP.

`factory-files` names the artifact instead of the deployment model. `factory-as-code` is broader and can imply sync, deployment, or live operations.

The description leads with the `factory.yaml` root rather than the file names, because `agents/<name>/agent.md` on its own also describes agent-definition files belonging to other tooling.

The negative trigger boundary is required:

- `factory-files` authors and edits repository files under a `factory.yaml` root.
- `factory-mcp` operates a factory and transfers work through MCP.
- Skills under `factory-dev/**/skills` define factory-agent playbooks. They do not define the file format.
- Agent-definition Markdown belonging to another tool is out of scope entirely.

### Rollout
Ship the skill in the stable, always-bundled directory from the first release. Do not add a feature flag or channel gate, and do not add a match arm to `activation_for_bundled_skill`; the default `Always` activation is intentional.

Gating was considered and rejected. The case for it is that Factory is not released: `FactoryMcp` is dogfood-only and `PREVIEW_FLAGS` is empty, so an always-on skill documents an alpha format to every stable user. Three things outweigh that.

The skill is self-gating by context. It does nothing unless the tree has a `factory.yaml`, so the population it can affect is Factory users, which is the same population a flag would select. The cost to everyone else is the name and trigger description in the skill list; full content and the schemas load only when the skill is read.

Gating would misfire across surfaces. Whether Oz cloud agents running factory agents have `FactoryMcp` enabled depends on prod experiment state rather than anything in this repository, and a skill that fails to activate is invisible — the primary requested surface could lose it with no signal. The Claude Code and Codex plugin mirrors load filesystem skills and cannot read a Warp flag at all, so gating would leave third-party harnesses enabled while disabling Warp's own cloud agents.

A flag would also couple authoring guidance to an unrelated kill switch. `FactoryMcp` attaches an MCP server for operating a live factory, which is exactly the boundary this skill excludes; turning it off during an MCP incident would remove file-authoring help for no reason. A dedicated flag avoids that but adds a second flag to flip in lockstep for no additional safety.

The residual risk is trigger collision, not exposure: `agents/<name>/agent.md` also describes agent-definition files belonging to other tooling. That is handled in the trigger rather than by a flag. The description anchors the skill to a tree rooted at `factory.yaml`, names the other-tool case as an explicit exclusion, and `SKILL.md` instructs the agent to stop when no `factory.yaml` is present. The bundled-skill test pins the anchor so it cannot be loosened silently.

### Distribution and version skew
Bundled skills ship inside the artifact; nothing delivers them from the server. Three release artifacts carry the bundle, which covers three of the four requested surfaces:

- The macOS app bundle resolves `Contents/Resources`, and Linux and Windows resolve a `resources` directory beside the executable.
- The TUI builds with the `standalone` feature so it resolves that same sibling directory, and its release job packages `resources` alongside the binary.
- The `oz` CLI is packaged the same way, and `oz-agent-worker` executes exactly that binary (`internal/worker/direct.go`), so cloud Warp agents load the bundle without extra work.

The fourth surface, third-party harness agents, is not covered by any of this and needs the plugin mirrors in sequence step 3.

Because the schemas travel with the artifact, their freshness is bounded by that artifact's version. Cloud runs track releases closely. An installed desktop client can be far behind, and the server can be newer than both. A field added to the format after a client shipped will be reported by that client as an unknown field.

The harmful direction is deletion, not rejection: an agent that trusts an unknown-field report on a file it did not write could remove working configuration. `SKILL.md` and `references/validation.md` therefore separate the two cases — an unknown field the agent just wrote is a mistake to fix, while an unknown field already in the file may be newer than the schemas and must be left alone and reported. Neither surface can be closed by a validator, so it is handled as instruction.

### Oz scope
Make the skill available to every Oz agent.

All Oz agents already receive the shared Warp bundled-resource catalog. No factory-only bundled activation exists. Adding an environment or agent-type gate would increase implementation and testing scope for no material safety benefit, and would reintroduce the invisible-failure risk described under Rollout. Skill metadata is small, and full content is loaded only when the agent reads the skill.

### Skill structure
Use one skill for both creation and editing. Both workflows use the same paths, inheritance rules, field constraints, and diagnostics. Splitting them would duplicate schema context and create competing triggers.

Use progressive disclosure:

- `SKILL.md`
  - Establish the authoring boundary against `factory-mcp` and factory-agent playbooks.
  - Locate the Factory root, inspect existing files, and preserve unrelated fields and prompt bodies.
  - Require canonical paths for new resources.
  - Require a validator run before opening a pull request, and state what the validator does not cover.
  - Point to the smallest relevant reference.
- `schemas/*.schema.json`
  - JSON Schema 2020-12 documents for `factory.yaml` and for agent, automation, and runner documents, plus a shared `common.schema.json` the others reference by relative `$ref`.
- `references/schema.md`
  - Path rules, field reference, enums, defaults, and inheritance, split by which layer enforces each rule.
- `references/triggers.md`
  - The provider, event, and filter-key catalogue, and inline schedule rules.
- `references/examples.md`
  - Curated minimal root, full harness, agent, automation, inline schedule, runner, and scoped-skill examples.
  - Examples use placeholder IDs and secrets. They must not copy production credentials or identifiers from `factory-dev`.
- `references/validation.md`
  - Diagnostic codes and a correction workflow, and the boundary between offline and server-side validation.
- `scripts/validate_factory_files.py`
  - The validator described below.

Keep `SKILL.md` workflow-oriented. Do not repeat complete field tables in it.

`SKILL.md` uses the `{{skill_dir}}` handlebars variable for the validator path. Warp renders it; plugin mirrors must substitute a harness-appropriate path, so the mirror step is a templated copy rather than a byte copy of that one line.

### JSON Schema and validation helper
Ship machine-readable JSON Schemas and a validator that runs them. This reverses the original recommendation, at the requester's direction and on bug-bash evidence: the recurring failures are frontmatter metadata mistakes, and prose alone does not stop an agent from inventing a field.

The validator is `scripts/validate_factory_files.py` and depends on nothing but Python 3. Neither PyYAML nor `jsonschema` is reliably present in agent sandboxes, so the script carries two small readers of its own:

- A restricted YAML reader accepting exactly the subset the Factory file parser accepts. Anchors, aliases, explicit tags, merge keys, duplicate keys, and multiple documents are rejected rather than interpreted, which matches the parser instead of limiting the tool. Anything it cannot read confidently is reported, never guessed. Those constructs are recognized only where a YAML node begins, and a block scalar's body is treated as opaque text, so ordinary prose such as `It's a thing`, `A & B`, or `*emphasis*` inside a description is not mistaken for syntax.
- A JSON Schema evaluator covering only the keywords the bundled schemas use. The schemas remain ordinary JSON Schema 2020-12 documents, so `check-jsonschema`, `ajv`, or `jsonschema` also work when available. A keyword the evaluator does not implement is reported rather than skipped, so adding one to a schema fails loudly instead of quietly under-validating.

The schemas encode what the schema language can express. Three tree-level rules cannot be expressed in a single-document schema and are implemented in the script: exactly one `MAIN`/`FOREMAN` agent, automation `agent` references resolving to a declared agent, and duplicate resource names across the flat and directory automation forms.

Two rules are deliberately not checked, because the server accepts them and a check would produce false failures: a `runner` name may resolve to an existing team runner the tree does not declare, and every server-resolved value (model IDs, environment IDs, secret names, MCP IDs) needs state the validator does not have. `SKILL.md` and `references/validation.md` both state this boundary so the agent does not overstate what a clean run proves.

The schemas cover the trigger filter catalogue, which is the highest-value addition. The parser accepts any mapping as a `filter` and defers key validation to apply time, so a wrong filter key currently survives review. The `warp-server` fixture at `logic/factoryfile/testdata/valid/automations/triage/automation.md` contains exactly this defect today: it uses `teams`, `projects`, `states`, `issues`, `baseBranches`, `channels`, `users`, and `itemUsers`, none of which `triggers.CanonicalizeFilter` accepts. That fixture should be corrected separately.

A parser-backed CLI or an API that validates an arbitrary source tree remains a follow-up. Do not add either endpoint in this work item.

### Schema ownership and drift
Eng-Platform FA owns the Factory file format, so it owns the bundled schemas. A change author who modifies `logic/factoryfile` owns the companion update to `warp/resources/bundled/skills/factory-files/schemas/`.

The schemas are hand-derived from `logic/factoryfile`, `model/types/triggers`, `model/types/runner.go`, `logic/factoryalias`, and `logic/cronspec`. They were verified against the real Go implementation: every accepted and rejected construct in the parity corpus produced the same verdict from the validator and from `factoryfile.ParseTree`, and the filter catalogue was checked against `triggers.ValidateFilter`.

Nothing enforces that agreement continuously, which is the main residual risk, and it is not theoretical. Between first drafting these schemas and self-reviewing them days later, `warp-server` had added two trigger providers (`gitlab` with `merge_request` and `bot_mentioned`, and `factory` with `work_item_stage_changed`). The schemas would have rejected valid automations until they were refreshed by re-dumping the catalogue from `triggers.EventsForProvider`.

Generating the schemas from the Go packages is not possible in one repository: the schemas live in `warp`, the authority lives in `warp-server`. Two follow-ups close the gap, in preference order:

1. Emit the JSON Schemas from `logic/factoryfile` in `warp-server` behind a golden test, and vendor the generated output into `warp` through a companion PR. This makes drift a build failure on the side that owns the format.
2. Failing that, add a scheduled job that runs the bundled validator against a corpus of trees whose expected verdicts are recorded in `warp-server`, and fails when the two disagree.

Until one exists, the interim mitigation is a set of notes in `warp-server` at every source the schemas mirror: the `logic/factoryfile` package doc and its field sets, `logic/factoryalias`, `logic/cronspec`, `model/types/triggers`, and the runner platform rules in `model/types/runner.go`. Each says the schemas are hand-derived, that nothing fails when they fall behind, and what to update. Every schema-changing `warp-server` PR must link a companion `warp` PR that refreshes the schemas, or state why the change has no authoring-visible effect. This belongs in the Factory file code-owner checklist too, not only in comments.

The skill instructs the agent to trust the server and report a stale schema rather than work around the validator, so drift degrades to a stale warning instead of a wrong edit.

### Canonical copy and mirror drift
The canonical authored skill is:

`warp/resources/bundled/skills/factory-files/`

Mirror that directory to:

- `claude-code-warp/plugins/oz-harness-support/skills/factory-files/`
- `codex-warp/plugins/orchestration/skills/factory-files/`

Every file mirrors byte-for-byte except the one `SKILL.md` line carrying the `{{skill_dir}}` validator path, which the mirror step substitutes. Keeping the Warp copy templated is deliberate: an absolute rendered path is the reliable form for a shell command in the surface that serves most users, and the substitution is a single well-known line.

Add `sync-manifest.json` inside the canonical skill directory. It records SHA-256 hashes for `SKILL.md` and every file under `references/`, `schemas/`, and `scripts/`. It does not hash itself. `SKILL.md` is hashed after substitution so a mirror and its source compare equal.

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

1. Add the canonical skill, its schemas, its validator, trigger tests, and packaging tests to the Warp spec PR. **Done.**
2. Add the manifest and sync script to the same branch.
3. Copy the approved Warp skill tree into Claude Code and Codex plugin branches, substituting the templated validator path. These two plugin changes can proceed in parallel.
4. Merge and release both plugin patch versions.
5. Update the Warp platform minimum version constants to the released versions.
6. Run cross-repository drift checks and final skill evaluations.
7. Merge the Warp PR only after the plugin versions named by its minimums are available.

This order prevents Warp from requiring unpublished plugin versions. It also lets plugin copies pin the exact pushed Warp source commit before the Warp PR merges.

The schema-generation follow-up in `warp-server` is tracked separately; it is not a prerequisite for this PR.

## Testing and validation
### Schema parity
The schemas and validator were checked against the authoritative Go implementation:

- The canonical `logic/factoryfile/testdata/schema_contract` tree validates clean.
- Every tree under `logic/factoryfile/testdata/invalid` is rejected, each with a message naming the same defect the parser reports.
- A 73-case positive/negative corpus covering harness blocks, auth sources, inline schedules, runner platforms, alias characters, integrations, filter keys, and author prose produced the expected verdict in every case. It is committed as `script/test_factory_files_skill.py` and runs in `script/presubmit`.
- Every example in `references/examples.md` is assembled into a tree and validated by that same corpus, so a reference that teaches invalid configuration fails the build.
- Every field set, enum, path helper, and trigger filter key in the schemas was diffed mechanically against a contract dumped from `develop`, covering 78 comparisons with no mismatch. That comparator is the prototype for follow-up 1 below.
- For every parser-level case in that corpus, `factoryfile.ParseTree` produced the matching verdict.
- Filter keys and matcher shapes were checked directly against `triggers.ValidateFilter`, including the `schedule_ids` in-only restriction.

Re-run after any format change:

- `./script/test_factory_files_skill.py` in `warp`
- `go test ./logic/factoryfile` in `warp-server`

### Warp bundle
Focused tests under the existing bundled-skill test module:

- `factory-files` parses with name `factory-files`.
- Its trigger description contains create/edit intents and the Factory MCP exclusion.
- `activation_for_bundled_skill("factory-files", ...)` returns `Always`.
- The trigger description anchors the skill to a `factory.yaml` root, so it cannot be loosened into matching unrelated `agents/<name>/agent.md` files.
- Every reference and script `SKILL.md` names exists on disk.
- Every schema file parses as JSON, declares `$id`, and `factory.schema.json` keeps its required-field set and closed property set.
- `sync-manifest.json` matches all canonical skill files. Pending; lands with the manifest in sequence step 2.

Run:

- `cargo test -p warp --lib ai::skills::bundled`
- `SKIP_SETTINGS_SCHEMA=1 NO_LICENSES=1 script/prepare_bundled_resources <temp-dir> stable`
- Assert `<temp-dir>/bundled/skills/factory-files/` contains `SKILL.md`, all references, all schemas, and the validator, and that the packaged validator resolves its schemas from the copied location.

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
- Confirm all three repository copies of the skill directory agree under the sync manifest, which accounts for the substituted validator path.

No visual recording is required. This feature changes agent context and generated files, not a rendered UI workflow.

## Findings worth acting on separately
- The `alias` rule is not what the bug-bash notes assumed. `factoryalias.Normalize` accepts Unicode letters, digits, spaces, `-`, `_`, and `.` up to 60 runes, and preserves case; uniqueness folds case in the comparison key only. There is no lowercase or hyphen-separated requirement. The schemas and reference encode the implemented rule. If the intended product rule really is lowercase-and-hyphenated, that is a server change, not a schema change.
- `logic/factoryfile/testdata/valid/automations/triage/automation.md` uses filter keys the apply step rejects. It passes today only because `ParseTree` does not validate filter keys. Worth fixing in `warp-server` so the fixture stops teaching the wrong spelling.
- Filter keys being parser-accepted and apply-rejected is the underlying gap. Validating filters during parse, or at least during plan, would move the error to where the author can see it.

## Risks and mitigations
- **The bundled schemas can drift from `logic/factoryfile`.** They are hand-derived and nothing enforces agreement continuously. Mitigate with the code-owner checklist now and the generation follow-up next; the skill tells the agent to trust the server and report staleness rather than route around the validator.
- **An older client validates against older schemas.** The bundle ships with the artifact, so a newer server can accept fields an installed client rejects. Mitigate by instructing the agent never to remove a field solely because the validator calls it unknown, and by keeping the server authoritative whenever it is reachable.
- **A bundled validator can be wrong in either direction.** A false rejection blocks a correct edit; a false acceptance gives unearned confidence. Mitigate with the parity corpus, by declining to check anything needing server state, and by having the reader report what it cannot parse instead of guessing.
- **A plugin mirror can remain pinned to an old valid commit.** Mitigate with the daily comparison against Warp `master` and an owning team for failures.
- **Stable rollout increases skill-list size for unrelated agents.** The metadata cost is limited to the name and trigger description. Full content remains progressively disclosed.
- **The trigger can collide with Factory MCP, or with another tool's agent files.** `agents/<name>/agent.md` is not a Factory-specific path. Mitigate by anchoring the description to a `factory.yaml` root, naming both exclusions explicitly, instructing the agent to stop when no `factory.yaml` is present, and pinning the anchor in a test. Cover both collisions in the trigger evals.
- **Examples can leak environment-specific values.** Use placeholders. Do not copy production IDs, secrets, account numbers, or images from live factories.
- **Version ordering can strand existing plugin installations.** Release plugins before merging the Warp minimum-version bump.

## Out of scope
- Changing the `v1alpha1` parser or adding a new schema version.
- Editor completion, a form-based Factory editor, or GUI/TUI authoring UI.
- A parser-backed validator binary or an arbitrary-tree diagnostics API.
- Generating the JSON Schemas from `logic/factoryfile`; tracked as the follow-up above.
- Fixing the CI validation gate for GitHub-linked Factory repositories, which is owned separately.
- Operating, creating, or dispatching work to live factories through Factory MCP.
- Rewriting factory-agent playbooks under `factory-dev/**/skills`.
- Migrating existing flat automation files or normalizing existing Factory trees without a user request.
- Supporting third-party harness plugins other than Claude Code and Codex.
- Automatically publishing plugin releases or automatically merging schema companion PRs.

## Assumptions
- The requester delegated the seven open engineering decisions to this spec and asked for decisive recommendations instead of an additional interview round.
- `v1alpha1` remains the only registered Factory tree adapter for the first release.
- The plugin repositories and the canonical Warp skill remain readable to their CI jobs so cross-repository manifest checks can fetch pinned commits.
