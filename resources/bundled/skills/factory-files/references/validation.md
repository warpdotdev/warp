# Validating and reading diagnostics

## Layers of validation
1. **The bundled validator** (`scripts/validate_factory_files.py`) checks the
   files themselves against the JSON Schemas plus the tree-level rules. It runs
   offline and needs nothing but Python 3. Run it before every pull request.
2. **The parser** (`logic/factoryfile` in `warp-server`) is the authority for
   everything the validator checks. Its diagnostics carry `FF_*` codes and a
   file path and line.
3. **Resolution and apply** validate everything that needs server state: model
   IDs, environment IDs, secret names, runner names, MCP server IDs,
   integration providers, harness model catalogues, worker-host entitlement,
   and runner platform and instance-shape rules.

A clean validator run means the files are structurally correct. It does not
mean the plan will apply. Say so rather than overstating what was checked.

## Running the validator
The script lives at `scripts/validate_factory_files.py` inside this skill's
directory; `SKILL.md` shows its resolved path.

```bash
python3 <skill-dir>/scripts/validate_factory_files.py <factory-root>
python3 <skill-dir>/scripts/validate_factory_files.py <factory-root> --json
```

Exit code 0 means no problems. Each problem reports the file, the field path,
and what is wrong. Fix them all and re-run; do not stop at the first one, since
one wrong field often produces several messages.

The schemas are ordinary JSON Schema 2020-12 documents, so any standard
validator works too if the tree is already converted to JSON.

## Diagnostic codes
The server reports these when a plan is run against a registered Factory.

- `FF_MISSING_FACTORY` — no `factory.yaml` at the Factory root.
- `FF_UNSUPPORTED_VERSION` — `schemaVersion` is not `v1alpha1`.
- `FF_UNSUPPORTED_PATH` — a file under `agents/`, `automations/`, or `runners/`
  is not at a canonical resource path.
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
  automation naming an agent the tree does not declare, or an unknown
  `agentType` or `credentialStrategy`.
- `FF_INVALID_MCP` — an MCP entry is not exactly a non-empty `warpId`.
- `FF_INVALID_TRIGGER` — a trigger is structurally wrong, such as an inline
  schedule on a non-schedule trigger, or a `schedule.cron_fired` trigger that
  declares both or neither of `schedule.cron` and `filter.schedule_ids`.
- `FF_INVALID_EVENT`, `FF_INVALID_FILTER` — the event is unknown, or a filter
  value is outside its valid domain.

## Fixing a diagnostic
Change the file the diagnostic names, at the field it names. Do not silence a
diagnostic by deleting the resource, loosening the schema, or moving a file to
a path the parser ignores.

If a diagnostic contradicts these references, the server is right. Say that the
bundled schemas look stale and, where you can, point at what changed in
`logic/factoryfile`.

## Checking against the parser directly
When `warp-server` is checked out locally, its parser tests are the closest
thing to ground truth. Run them from that checkout, not from the Factory
repository:

```bash
go test ./logic/factoryfile
```

Fixtures under `logic/factoryfile/testdata` show accepted and rejected trees.
