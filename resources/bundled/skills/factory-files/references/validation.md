# Validating and reading diagnostics

## Layers of validation
1. **The server's parser** (`logic/factoryfile` in `warp-server`) is the only
   authority. `POST /api/v1/factory-files/validate` runs it over a tree you
   submit as paths and content, and adds the state-independent rules the apply
   path enforces next: runner platforms and instance shapes, and trigger filter
   keys and matchers. Its diagnostics carry `FF_*` codes with a path, line, and
   column.
2. **Resolution and apply** validate everything that needs server state: model
   IDs, environment IDs, secret names, runner names, MCP server IDs,
   integration providers, harness model catalogues, worker-host entitlement,
   and the values of Linear and Slack name aliases.

There is no third layer, and deliberately no local one. A clean result from the
endpoint means the files pass the structural and state-independent semantic
checks. It does not mean the plan will apply. The response lists the checks it
did not run; say so rather than overstating what was checked.

## Running the validator
The script lives at `scripts/validate_factory_files.py` inside this skill's
directory; `SKILL.md` shows its resolved path. It does not parse the format. It
selects the tree's resource files by path, refuses symlinks, submits the bytes,
and relays what comes back.

```bash
python3 "<skill-dir>/scripts/validate_factory_files.py" "<factory-root>"
python3 "<skill-dir>/scripts/validate_factory_files.py" "<factory-root>" --json
```

`--server-root <url>`, `WARP_SERVER_ROOT`, or `WARP_SERVER_ROOT_URL` selects a
local, staging, or self-hosted server, in that order of precedence. An Oz
sandbox exports its own server root under the last of those names, so a
sandbox validates against the server it belongs to rather than production.

The endpoint needs no credential; `WARP_API_KEY` is forwarded when the
environment already has one, which makes the request attributable inside an
agent sandbox. If the server answers a forwarded key with 401 or 403, the
request is retried once without it and the drop is noted on stderr, so a stale
or wrong-environment key cannot turn a validatable tree into exit `2`.

Use Python 3.8 or newer via the host's command (`python3`, `python`, or
`py -3`). If none is available, do not install an interpreter or claim
automated validation without the user's approval; inspect the changed document
against the fetched schema and report the gap.

## The three outcomes
- `0` — the server checked the tree and found no problem.
- `1` — the server checked the tree and reported diagnostics.
- `2` — the tree was **not** checked.

Exit `2` is not a pass and not a failure. It happens when the server is
unreachable, answers with an error or a malformed body, the directory is not a
Factory root, or the tree is larger than the endpoint accepts. In every case
the correct report is that validation did not run, with the reason. Saying
anything about whether the files are correct would be inventing a verdict.

With `--json`, `validated` distinguishes the cases: a run that reached no
verdict carries `validated: false` and no `valid` key at all, so there is
nothing to misread.

Each problem reports the file, the field path, and what is wrong. Fix them all
and re-run; do not stop at the first one, since one wrong field often produces
several messages.

A resource file that is a symlink is reported and never uploaded. The server
parses the repository tree, where a symlink is stored as its target path rather
than its target's content, so it never follows one either; a Factory resource
has to be a real file. Reading the target locally would also let a repository
aim a resource at any readable path on the machine.

## Deferred resolutions
A response can carry `deferred_resolutions` alongside its diagnostics. A
deferred entry is not a problem: it names an authored value the endpoint
deliberately did not prove, because proving it needs provider state. Linear and
Slack name aliases are the current case — the endpoint checks that `teams` or
`channels` is a list of non-empty names applicable to that event, and leaves
whether those names exist to apply time.

Report deferred entries. They are the difference between "this parses" and
"this will work".

## Diagnostic codes
The server reports these from the validation endpoint and when a plan is run
against a registered Factory.

- `FF_MISSING_FACTORY` — no `factory.yaml` at the Factory root.
- `FF_UNSUPPORTED_VERSION` — `schemaVersion` names no registered tree adapter.
  The server stops rather than applying another version's rules. Correct the
  version; never lower it to make a check pass.
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
  the fetched schema.
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
  key or value is outside its valid domain.

## Fetching the schema
The schema endpoints are unauthenticated and cacheable:

```bash
curl -s https://app.warp.dev/api/v1/factory-files/schemas
curl -s https://app.warp.dev/api/v1/factory-files/schemas/v1alpha1
```

They are ordinary JSON Schema 2020-12 documents, exact for the version they
describe, so any standard validator works if a tree is already converted to
JSON. `x-warp-*` annotations carry constraints JSON Schema cannot express
portably, such as trimmed Unicode alias rules and the power-of-two Linux
compute sizes; only a Warp validator enforces those.

Fetching a schema is not validation. A successful fetch says the server is
reachable, nothing more.

## Why there is no offline mode
This skill used to bundle the schemas and a local validator so authoring worked
without a server. That was removed, and should not be reintroduced.

The copy shipped inside a Warp release, so it was routinely older than the
server a Factory syncs against. A stale copy does not degrade gracefully: it
reports a field the server accepts as unknown, and an agent trying to reach a
clean run resolves that by deleting working configuration. It happened — an
earlier revision rejected the Linear and Slack trigger aliases (`teams`,
`projects`, `states`, `issues`, `channels`, `users`, `itemUsers`) that the
apply path rewrites and accepts, on a tree taken from the server's own
`testdata/valid`.

The trade is deliberate: never checking is recoverable, and the report says so.
Checking wrongly costs correct configuration and is not obviously wrong to the
agent acting on it.

## Fixing a diagnostic
Change the file the diagnostic names, at the field it names. Do not silence a
diagnostic by deleting the resource or moving a file to a path the parser
ignores.

## Checking against the parser directly
When `warp-server` is checked out locally, its parser tests are the closest
thing to ground truth. Run them from that checkout, not from the Factory
repository:

```bash
go test ./logic/factoryfile/...
```

Fixtures under `logic/factoryfile/testdata` show accepted and rejected trees.
