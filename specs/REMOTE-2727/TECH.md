# Bundled skill for authoring Factory files
## Summary
[REMOTE-2727](https://linear.app/warpdotdev/issue/REMOTE-2727/bundled-skill-for-authoringediting-file-based-factory-definitions) adds one bundled skill that teaches agents to create and edit file-based software factory definitions. The skill ships in Warp for the native GUI, TUI, and every Oz cloud agent. Copies ship in the Claude Code and Codex Oz platform plugins because third-party harnesses cannot resolve a Warp `bundled_skill_id`.

The skill carries machine-readable JSON Schemas for the format and a dependency-free validator that runs them, so an agent can check its own edits before opening a pull request. That requirement came from the Factory bug bash, where the recurring failures were malformed frontmatter metadata that the parser accepts and the apply step later rejects.

This work needs only a technical spec. The user workflow is already defined, and the remaining decisions concern packaging, rollout, validation, and schema maintenance.

The implementation uses `warpdotdev/warp` as its primary repository. Warp owns the canonical skill copy and the shared distribution path for three of the four requested surfaces. `warp-server` remains authoritative for the Factory file schema. The Claude Code and Codex plugin repositories contain downstream mirrors.

The schema-ownership, drift, trigger-filter validation, and follow-up design in this document is superseded by [`warpdotdev/warp-server/specs/REMOTE-2868/TECH.md`](https://github.com/warpdotdev/warp-server/blob/develop/specs/REMOTE-2868/TECH.md). REMOTE-2868 moves schema generation and parser-backed validation to `warp-server` while retaining this skill's permissive offline fallback. This document remains authoritative for the skill trigger, authoring workflow, canonical paths, symlink policy, and boundary against `factory-mcp`.

## Context
The Factory file parser accepts a versioned, path-derived tree:

- `factory.yaml`
- `agents/<name>/agent.md`
- `automations/<name>/automation.md`
- `runners/<name>.yaml`
- `scorers/<name>/scorer.md`
- `skills/**`
- `agents/<name>/skills/**`

The parser also accepts the legacy flat path `automations/<name>.md`, but rendering always emits the directory form. The skill must create the canonical directory form and may edit an existing flat file without moving it unless the user asks for normalization.

The current schema is `v1alpha1`. The parser rejects unknown fields, duplicate keys, YAML anchors, aliases, explicit tags, malformed frontmatter, invalid paths, invalid references, and invalid field values. It also requires exactly one `MAIN` or `FOREMAN` agent. The following sources are authoritative:

- [`logic/factoryfile/v1alpha1.go @ 182c274c6d`](https://github.com/warpdotdev/warp-server/blob/182c274c6d/logic/factoryfile/v1alpha1.go) defines the schema version, allowed field sets, document decoding, and tree-level rules.
- [`logic/factoryfile/path.go @ 182c274c6d`](https://github.com/warpdotdev/warp-server/blob/182c274c6d/logic/factoryfile/path.go) defines canonical resource and skill paths.
- [`logic/factoryfile/agenttype.go @ 182c274c6d`](https://github.com/warpdotdev/warp-server/blob/182c274c6d/logic/factoryfile/agenttype.go) defines accepted agent types and the `MAIN` alias.
- [`logic/factoryfile/credentialstrategy.go @ 182c274c6d`](https://github.com/warpdotdev/warp-server/blob/182c274c6d/logic/factoryfile/credentialstrategy.go) defines credential strategies.
- [`logic/factoryfile/parsetree.go @ 182c274c6d`](https://github.com/warpdotdev/warp-server/blob/182c274c6d/logic/factoryfile/parsetree.go) selects the adapter and classifies resource paths.
- [`logic/factoryfile/v1alpha1_scoredefinition.go @ 182c274c6d`](https://github.com/warpdotdev/warp-server/blob/182c274c6d/logic/factoryfile/v1alpha1_scoredefinition.go) defines file-managed Scorer frontmatter, rubric, sampling, and label constraints.
- [`model/types/triggers @ 182c274c6d`](https://github.com/warpdotdev/warp-server/tree/182c274c6d/model/types/triggers) defines providers, events, filter fields, matcher forms, and canonical overlap checks.
- [`model/types/runner.go @ 182c274c6d`](https://github.com/warpdotdev/warp-server/blob/182c274c6d/model/types/runner.go) defines runner platforms, macOS versions, defaults, and instance-shape validation.
- [`logic/factoryalias/alias.go @ 182c274c6d`](https://github.com/warpdotdev/warp-server/blob/182c274c6d/logic/factoryalias/alias.go) and [`logic/cronspec/cronspec.go @ 182c274c6d`](https://github.com/warpdotdev/warp-server/blob/182c274c6d/logic/cronspec/cronspec.go) define alias and inline schedule syntax.

The schema is still changing. During this PR, `cloudProviders` replaced the legacy read-only `providers` key and file-managed Scorers landed. A manually copied closed-world schema will become stale.

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

  > Create and edit file-based Warp software factory definitions, in a repository tree rooted at a factory.yaml. Use when authoring or changing that factory.yaml, Agent, Automation, Scorer, or Runner files under that root, or its factory and agent skill trees, and when fixing Factory file diagnostics. Do not use for agent-definition Markdown that belongs to another tool, for a tree with no factory.yaml, or to operate a live factory or hand work to one through Factory MCP.

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
  - JSON Schema 2020-12 documents for `factory.yaml` and Agent, Automation, Runner, and Scorer documents, plus a shared `common.schema.json` the others reference by relative `$ref`.
- `references/schema.md`
  - Path rules, field reference, enums, defaults, and inheritance, split by which layer enforces each rule.
- `references/triggers.md`
  - The provider, event, and filter-key catalogue, and inline schedule rules.
- `references/scorers.md`
  - Scorer fields, classification labels, thresholds, sampling, Agent references, and rubric rules.
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

The validator is `scripts/validate_factory_files.py` and depends on nothing but Python 3.8 or newer. Neither PyYAML nor `jsonschema` is reliably present in agent sandboxes, so the script carries two small readers of its own:

- A restricted YAML reader for the canonical forms the skill emits. Anchors, aliases, explicit tags, merge keys, duplicate keys, and multiple documents are rejected rather than interpreted, which matches the parser. It is deliberately not described as a complete YAML implementation: anything outside the canonical subset is reported rather than guessed, and the skill tells the agent not to normalize an existing server-accepted file merely for this reader. YAML syntax constructs are recognized only where a node begins, and a block scalar's body is treated as opaque text, so ordinary prose such as `It's a thing`, `A & B`, or `*emphasis*` inside a description is not mistaken for syntax.
- A JSON Schema evaluator covering only the keywords the bundled schemas use. A keyword the evaluator does not implement is reported rather than skipped, so adding one to a schema fails loudly instead of quietly under-validating. The schemas remain ordinary JSON Schema 2020-12 documents, so `check-jsonschema`, `ajv`, or `jsonschema` can read them, but those tools check the schema layer only and silently skip everything listed below.

The schemas validate known fields while allowing additional properties and newer catalogue values. This is deliberate: a schema bundled in an older client must not reject or erase source accepted by a newer server.

The dividing line is whether a plausible server change could make the input valid. Anything a release could add or retune is recorded as an `x-warp-known-values` or `x-warp-known-max-items` annotation and left to the server: unknown properties everywhere, agent types, credential strategies, harness types and their per-harness capabilities (`reasoningLevel` and `auth` on `oz`), integration slugs, trigger providers and events, runner operating systems, architectures and macOS versions, Scorer output forms, and the Scorer label cap. A tree whose `schemaVersion` these schemas do not describe is reported once and not validated further, rather than being buried under bogus `v1alpha1` unknown-field reports.

What stays rejected is what remains wrong under any of those changes: malformed YAML and frontmatter, missing required fields, wrong types, exactly one `MAIN`/`FOREMAN` agent, Automation and Scorer Agent references, duplicate resource names, non-empty Scorer rubrics, mixed Scorer outcomes, trimmed alias length, normalized repository/secret/Scorer duplicates, Linux power-of-two shapes, full cron grammar, duplicate inline schedule identities, and matcher `in`/`not_in` conflicts. The last several are owned by the script rather than the schema language.

This trades false acceptance for false rejection on purpose. The server revalidates every tree at apply time, so a tolerated-but-wrong field costs one clear server diagnostic, whereas a false rejection blocks correct work and invites an agent to "fix" valid configuration.

The openness is easy to mistake for an unfinished edge and quietly undo, so it is enforced rather than merely documented. `assert_schemas_stay_forward_compatible` in `script/test_factory_files_skill.py` walks every bundled schema and fails presubmit if `enum`, `const`, `maxItems`, or `additionalProperties: false` appears outside the two scoped exceptions, naming the offending pointer and pointing back here. It runs before the corpus so a tightening reports that explanation instead of a downstream rejection. The same warning is repeated where someone would be standing when they make the change: a `$comment` at the root of all six schemas, the validator's module docstring, the corpus docstring and an inline banner over the tolerance cases, the Rust assertions in `bundled_tests.rs`, and an "If you are changing these schemas" section in `references/validation.md` that `SKILL.md` points at.

Two rules are deliberately not checked, because the server accepts them and a check would produce false failures: a `runner` name may resolve to an existing team runner the tree does not declare, and every server-resolved value (model IDs, environment IDs, secret names, MCP IDs) needs state the validator does not have. `SKILL.md` and `references/validation.md` both state this boundary so the agent does not overstate what a clean run proves.

The trigger filter catalogue is the one catalogue still enforced, and it is the highest-value check here: the parser accepts any mapping as a `filter` and defers key validation to apply time, so a wrong filter key currently survives review. It is kept drift-safe by scoping it, because every filter rule fires only when both the provider and the event match values these schemas know. A newer provider, or a newer event on a known provider, matches no rule and leaves its filter unconstrained, so the check cannot reject a tree built for a newer server. The residual gap is a new filter key added to an existing provider/event pair, which the server would still catch at apply time.

Correction: `teams`, `projects`, `states`, `issues`, `baseBranches`, `channels`, `users`, and `itemUsers` in `logic/factoryfile/testdata/valid/automations/triage/automation.md` are supported authoring aliases, not a defective fixture. Apply rewrites the GitHub aliases locally and resolves the Linear and Slack name aliases through provider snapshots before `triggers.CanonicalizeFilter`. The validator must not pass the authored keys directly to canonical validation. REMOTE-2868 defines the key-by-key policy.

A parser-backed CLI or an API that validates an arbitrary source tree remains a follow-up. Do not add either endpoint in this work item.

### Schema ownership and drift
Eng-Platform FA owns the Factory file format, so it owns the bundled schemas. A change author who modifies `logic/factoryfile` owns the companion update to `warp/resources/bundled/skills/factory-files/schemas/`.

The schemas are hand-derived from `logic/factoryfile`, `logic/factoryfile/v1alpha1_scoredefinition.go`, `model/types/triggers`, `model/types/runner.go`, `logic/factoryalias`, and `logic/cronspec`. They intentionally accept some inputs the current server rejects when those inputs are plausible future extensions; the server remains authoritative.

Nothing enforces that agreement continuously, which is the main residual risk, and it is not theoretical. During this PR, `warp-server` added trigger providers, renamed `providers` to `cloudProviders`, and added file-managed Scorers. Permissive unknown-property/catalogue handling limits breakage, while explicit schemas and references are refreshed for new resource kinds and known fields.

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

The Warp skill and the plugin mirrors ship independently. The Warp PR does not change either `MINIMUM_PLATFORM_PLUGIN_VERSION`, so it cannot require an unpublished plugin version and does not need to wait for one.

1. Add the canonical skill, its schemas, its validator, trigger tests, and packaging tests to the Warp spec PR. **Done.**
2. Merge the Warp PR. The native client, TUI, and Warp cloud agents pick the skill up from the shared bundle at that point.
3. Add the manifest and sync script.
4. Copy the merged Warp skill tree into Claude Code and Codex plugin branches, substituting the templated validator path. These two plugin changes can proceed in parallel.
5. Merge and release both plugin patch versions.
6. In a separate Warp PR, raise the platform minimum version constants to the released versions. That PR, not this one, is the one that must not merge before the named releases exist.
7. Run cross-repository drift checks and final skill evaluations.

The real constraint is only that Warp must never require a plugin version that is not published, which binds step 6 alone. Landing the skill first also gives the plugin copies a merged commit to pin instead of a moving branch.

The schema-generation follow-up in `warp-server` is tracked separately; it is not a prerequisite for this PR.

## Testing and validation
### Schema parity
The schemas and validator were checked against the authoritative Go implementation:

- The canonical `logic/factoryfile/testdata/schema_contract` tree validates clean.
- All three Factory roots currently in `warpdotdev/factory-dev` (`v1`, `frank`, and `dan-factory`) pass both the bundled validator and `factoryfile.ParseTree`.
- The regression corpus distinguishes required structural/semantic failures from deliberately tolerated forward-compatible fields and catalogue values.
- Every example in `references/examples.md` is assembled into a tree and validated by that same corpus, so a reference that teaches invalid configuration fails the build.
- Current `develop` paths and known fields, including `cloudProviders` and `scorers/<name>/scorer.md`, are represented; future additions are tolerated.

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
- Every schema file parses as JSON, uses a relative `$id`, and every local `$ref` resolves to a packaged file and JSON pointer.
- `script/test_factory_files_skill.py` runs `prepare_bundled_resources` and verifies the packaged `factory-files` tree has the same files and bytes as the canonical source.

Run:

- `cargo test -p warp --lib ai::skills::bundled`
- `./script/test_factory_files_skill.py`

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
Before the Warp skill PR merges:

- Confirm GUI, TUI, and remote-server bundles contain the same skill tree, which `script/test_factory_files_skill.py` checks by packaging it.

Before the minimum-version bump in step 6 merges:

- Confirm the Claude and Codex released versions are greater than or equal to the updated platform minimums.
- Confirm all three repository copies of the skill directory agree under the sync manifest, which accounts for the substituted validator path.

No visual recording is required. This feature changes agent context and generated files, not a rendered UI workflow.

## Findings worth acting on separately
- The `alias` rule is not what the bug-bash notes assumed. `factoryalias.Normalize` accepts Unicode letters, digits, spaces, `-`, `_`, and `.` up to 60 runes, and preserves case; uniqueness folds case in the comparison key only. There is no lowercase or hyphen-separated requirement. The schemas and reference encode the implemented rule. If the intended product rule really is lowercase-and-hyphenated, that is a server change, not a schema change.
- `logic/factoryfile/testdata/valid/automations/triage/automation.md` uses supported authoring aliases. The earlier claim that apply rejects them was incorrect; apply rewrites or resolves them before canonical validation.
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
