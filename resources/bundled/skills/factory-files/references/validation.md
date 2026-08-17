# Validating and reading diagnostics

## Layers of validation
1. **The server's parser** (`logic/factoryfile` in `warp-server`) is the
   authority. `POST /api/v1/factory-files/validate` runs it over a tree you
   submit as paths and content, and adds the state-independent rules the apply
   path enforces next: runner platforms and instance shapes, and trigger filter
   keys and matchers. Its diagnostics carry `FF_*` codes with a path, line, and
   column.
2. **The bundled validator** (`scripts/validate_factory_files.py`) is the
   offline floor. It checks the files against the JSON Schemas shipped beside
   it plus the tree-level rules, needs nothing but Python 3, and runs when the
   server cannot be reached. It can be older than the server.
3. **Resolution and apply** validate everything that needs server state: model
   IDs, environment IDs, secret names, runner names, MCP server IDs,
   integration providers, harness model catalogues, worker-host entitlement,
   and the values of Linear and Slack name aliases.

A clean result from either of the first two means the files pass the structural
and state-independent semantic checks. It does not mean the plan will apply.
The server response lists the checks it did not run; say so rather than
overstating what was checked.

## Running the validator
The script lives at `scripts/validate_factory_files.py` inside this skill's
directory; `SKILL.md` shows its resolved path.

```bash
python3 "<skill-dir>/scripts/validate_factory_files.py" "<factory-root>"
python3 "<skill-dir>/scripts/validate_factory_files.py" "<factory-root>" --json
python3 "<skill-dir>/scripts/validate_factory_files.py" "<factory-root>" --offline
```

It reads the tree's `schemaVersion`, confirms the server publishes it, and
submits the tree; only if that fails does it fall back. `--server-root <url>`
or `WARP_SERVER_ROOT` selects a local, staging, or self-hosted server;
`WARP_API_KEY` authenticates the validation endpoint. `--offline` skips the
server deliberately.

Every run ends with a sentence naming the path that ran, and `--json` carries
the same fact in `validated_with`. Repeat it. An offline pass is not a server
verdict, and a successful schema fetch is not validation.

A version the server does not publish stops the run rather than being measured
against another version's rules. Correct the version; never lower it to make a
check pass.

Use Python 3.8 or newer via the host's command (`python3`, `python`, or `py -3`). If none is
available, do not install an interpreter or claim automated validation without
the user's approval; inspect the changed document against its JSON Schema and
report the validation gap.

Exit code 0 means no problems. Each problem reports the file, the field path,
and what is wrong. Fix them all and re-run; do not stop at the first one, since
one wrong field often produces several messages.
The bundled reader handles the canonical YAML forms this skill emits, not every
piece of YAML syntax accepted by `gopkg.in/yaml.v3`. If it cannot read an
existing file that the server accepts, do not normalize or rewrite the file
merely for the reader; report that local validation was unavailable and use a
server plan when possible.

A resource file that is a symlink is reported and not read. The server parses
the repository tree, where a symlink is stored as its target path rather than
its target's content, so it never follows one either; a Factory resource has to
be a real file. Reading the target locally would also let a repository aim a
resource at any readable path on the machine.

The schemas are ordinary JSON Schema 2020-12 documents, so any standard
validator works too if the tree is already converted to JSON. That applies to
the documents the server serves as well; those are exact for the version they
describe. `x-warp-*` annotations carry constraints JSON Schema cannot express
portably, such as trimmed Unicode alias rules and the power-of-two Linux
compute sizes; only a Warp validator enforces those annotations.

## Deferred resolutions
A server response can carry `deferred_resolutions` alongside its diagnostics. A
deferred entry is not a problem: it names an authored value the endpoint
deliberately did not prove, because proving it needs provider state. Linear and
Slack name aliases are the current case — the endpoint checks that `teams` or
`channels` is a list of non-empty names applicable to that event, and leaves
whether those names exist to apply time.

## Diagnostic codes
The server reports these from the validation endpoint and when a plan is run
against a registered Factory.

- `FF_MISSING_FACTORY` — no `factory.yaml` at the Factory root.
- `FF_UNSUPPORTED_VERSION` — `schemaVersion` names no registered tree adapter.
  The bundled validator reports an unrecognized version and stops rather than
  applying v1alpha1 rules to a tree it does not describe.
- `FF_UNSUPPORTED_PATH` — a file that resembles an Agent, Automation, Runner,
  or Scorer resource is at a non-canonical path. Other unrelated files under
  those directories are intentionally ignored.
- `FF_DUPLICATE_PATH` — the same resource name is declared twice, most often an
  automation declared in both the flat and directory forms.
- `FF_INVALID_DOCUMENT` — a file is empty or its root is not a YAML mapping.
- `FF_MALFORMED_FRONTMATTER` — a Markdown resource is missing an opening or
  closing `---` fence.
- `FF_INVALID_YAML` — the YAML could not be parsed.
- `FF_DUPLICATE_KEY` — a mapping repeats a key. Also reported when two agents
  declare `MAIN`/`FOREMAN`, or a secret is listed twice.
- `FF_ANCHOR`, `FF_ALIAS`, `FF_TAG` — YAML anchors, aliases, and explicit tags
  are not permitted.
- `FF_UNKNOWN_FIELD` — a field the schema does not define. Check spelling and
  the field reference; do not add the field to the schema to make it pass.
- `FF_MISSING_REQUIRED` — a required field is absent or empty.
- `FF_TYPE_MISMATCH` — a value has the wrong YAML type.
- `FF_INVALID_VALUE` — a value violates a format or exclusivity rule, such as
  declaring both `model` and `harness`, or an alias with disallowed characters.
- `FF_INVALID_REFERENCE` — a named reference does not resolve, such as an
  Automation or Scorer naming an Agent the tree does not declare, or an
  unknown current `agentType` or `credentialStrategy`.
- `FF_INVALID_MCP` — an MCP entry is not exactly a non-empty `warpId`.
- `FF_INVALID_TRIGGER` — a trigger is structurally wrong, such as an inline
  schedule on a non-schedule trigger, or a `schedule.cron_fired` trigger that
  declares both or neither of `schedule.cron` and `filter.schedule_ids`.
- `FF_INVALID_EVENT`, `FF_INVALID_FILTER` — the event is unknown, or a filter
  value is outside its valid domain.

The bundled schemas do not reproduce every catalogue rejection above. Unknown
properties, agent types, credential strategies, harnesses and their per-harness
capabilities, integration types, trigger providers and events, runner platform
values, Scorer output forms, and server-tunable limits such as the Scorer label
cap are all preserved so an older client does not reject source accepted by a
newer server. A server plan is authoritative.

Filter keys are the one catalogue still checked, because a misspelled key is a
common mistake that otherwise survives until apply. The check applies only when
both the provider and the event are ones these schemas know; a newer provider,
or a newer event on a known provider, leaves its filter unconstrained.

What the bundled validator still refuses is what stays wrong under any of those
changes: malformed YAML and frontmatter, missing required fields, values of the
wrong type, references to Agents the tree does not declare, more or fewer than
one `MAIN`/`FOREMAN` Agent, duplicate resource names and labels, an empty
Scorer rubric, a label set that cannot both pass and fail, and filters that can
never match.

## If you are changing these schemas
The permissiveness above is load-bearing, not an unfinished edge. These files
ship inside a Warp release and are routinely older than the `warp-server` they
run against, so closing them back up would reject configuration a newer server
accepts and push agents to delete working fields.

When the format gains a value, add it to the relevant `x-warp-known-values` or
`x-warp-known-max-items` annotation. Do not turn an annotation back into
`enum`, `const`, `maxItems`, or `additionalProperties: false`. The regression
corpus in `script/test_factory_files_skill.py` asserts several of these
tolerances on purpose; if one starts failing, a schema was tightened.

## Fixing a diagnostic
Change the file the diagnostic names, at the field it names. Do not silence a
diagnostic by deleting the resource, loosening the schema, or moving a file to
a path the parser ignores.

If a diagnostic contradicts these references, the server is right. Say that the
bundled schemas look stale and, where you can, point at what changed in
`logic/factoryfile`.

## Version skew
The schemas ship inside the Warp version running them, not from the server, so
they can lag the server that a Factory actually syncs against. Cloud agent runs
track releases closely; an installed desktop client can be much older.

The asymmetry matters when reading an `unknown field` report:

- On a field you just wrote, it is almost certainly a mistake. Fix it.
- On a field that was already in the file, it may be a newer field your copy of
  the schemas does not know. Leave it alone and report the possibility. Removing
  it would silently drop working configuration.

## Checking against the parser directly
When `warp-server` is checked out locally, its parser tests are the closest
thing to ground truth. Run them from that checkout, not from the Factory
repository:

```bash
go test ./logic/factoryfile
```

Fixtures under `logic/factoryfile/testdata` show accepted and rejected trees.
