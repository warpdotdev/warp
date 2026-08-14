---
name: factory-files
description: Create and edit file-based Warp software factory definitions. Use when authoring or changing factory.yaml, agent or automation Markdown, runner YAML, or factory and agent skill trees, and when fixing Factory file diagnostics. Do not use to operate a live factory or hand work to a factory through Factory MCP.
---

# Factory Files

A software factory can be defined by files in a repository. This skill covers
authoring and editing those files, and validating them before you open a pull
request.

Use this skill for repository files. It is not the skill for operating a live
factory: use `factory-mcp` to send work to a factory, inspect task status, or
pull a task down locally. Playbooks under a factory's own `skills/` directories
tell that factory's agents how to do their job; editing one is a prompt change,
not a schema change, so this skill's rules do not apply to their contents.

## Locate the Factory root
Every Factory tree is rooted at the directory containing `factory.yaml`. All
paths below are relative to that root. A repository may register a
subdirectory as the root, so find `factory.yaml` rather than assuming the
repository root.

```
factory.yaml                        required, exactly one
agents/<name>/agent.md              at least one; exactly one must be MAIN
agents/<name>/skills/**             skills only that agent can use
automations/<name>/automation.md    optional
runners/<name>.yaml                 optional
skills/**                           skills every agent in the factory can use
```

Resource names come from the path, never from a field inside the file. Renaming
an agent means moving its directory.

`automations/<name>.md` is a legacy flat form the parser still accepts. Create
the directory form; when editing an existing flat file, leave it where it is
unless the user asks you to normalize the tree.

## Before you edit
1. Read the files you are about to change, plus `factory.yaml`, so you can see
   what is inherited and what is overridden.
2. Preserve fields and Markdown bodies you were not asked to change. The body
   after an agent's or automation's closing `---` fence is the prompt; never
   fold it into frontmatter.
3. Prefer the smallest edit that satisfies the request.

## Author against the schema
The parser rejects unknown fields, duplicate keys, YAML anchors, aliases,
explicit tags, and multiple documents per file. Do not invent fields.

The bundled JSON Schemas are the machine-readable contract:

```
schemas/factory.schema.json      factory.yaml
schemas/agent.schema.json        agents/<name>/agent.md frontmatter
schemas/automation.schema.json   automations/<name>/automation.md frontmatter
schemas/runner.schema.json       runners/<name>.yaml
schemas/common.schema.json       shared definitions referenced by the above
```

Read `references/schema.md` for the field-by-field reference, defaults, and
inheritance rules. Read `references/triggers.md` before writing or changing an
automation trigger: filter keys are specific to each provider and event, and
the parser does not catch a wrong one. Read `references/examples.md` for
worked examples of each resource.

## Validate before opening a pull request
Run the bundled validator. It needs nothing beyond Python 3.

```bash
python3 {{skill_dir}}/scripts/validate_factory_files.py <factory-root>
```

Add `--json` for machine-readable output. A non-zero exit means at least one
problem; fix every reported problem and re-run until it is clean.

The validator checks structure, field names, enums, mutual exclusions, trigger
filter keys, cron syntax, runner platform rules, and the tree-level rules
(exactly one MAIN agent, automation agent references, duplicate resource
names). It does not resolve server state: model IDs, environment IDs, secret
names, runner names, MCP server IDs, and integration availability are all
validated when the plan is applied. Report that distinction rather than
claiming a tree is fully verified.

If the repository is checked out alongside `warp-server`, you can additionally
run the authoritative parser:

```bash
go test ./logic/factoryfile
```

When the Factory is already registered, a server plan is the strongest
available check. See `references/validation.md` for diagnostic codes and how to
read them.

## Rules that are easy to get wrong
- Exactly one agent declares `agentType: MAIN` (or `FOREMAN`, its canonical
  spelling). Zero or two is an error.
- `model` and `harness` are mutually exclusive everywhere. `model: <id>` is
  shorthand for the Oz harness.
- `agentDefaults` must declare one of them; agents and automations may declare
  neither and inherit.
- Declaring `secrets` or `mcpServers` at agent or automation level replaces the
  inherited value; it does not merge.
- An automation needs at least one trigger, and every trigger needs `provider`
  and `event`.
- A `schedule.cron_fired` trigger needs either an inline `schedule.cron` or a
  non-empty `filter.schedule_ids`, and never both.
- Linux runners require `platform.linux.dockerImage`. A runner with no
  `platform` section defaults to Linux and will fail for that reason.

## Schema drift
The format is `v1alpha1` and still changing. `logic/factoryfile` in
`warp-server` is the authority; these schemas mirror it. If the server reports
a field the schemas reject, or accepts one they do not know, trust the server,
tell the user the schemas are stale, and do not work around the validator by
silently skipping it.
