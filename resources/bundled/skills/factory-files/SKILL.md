---
name: factory-files
description: Create and edit file-based Warp software factory definitions, in a repository tree rooted at a factory.yaml. Use when authoring or changing that factory.yaml, Agent, Automation, Scorer, or Runner files under that root, or its factory and agent skill trees, and when fixing Factory file diagnostics. Do not use for agent-definition Markdown that belongs to another tool, for a tree with no factory.yaml, or to operate a live factory or hand work to one through Factory MCP.
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

If there is no `factory.yaml`, this is not a Factory tree and nothing here
applies. `agents/<name>/agent.md` and similar paths are also used by other
agent tooling; stop and say so rather than imposing this schema on them.

```
factory.yaml                        required, exactly one
agents/<name>/agent.md              at least one; exactly one must be MAIN
agents/<name>/skills/**             skills only that agent can use
automations/<name>/automation.md    optional
runners/<name>.yaml                 optional
scorers/<name>/scorer.md            optional; Markdown body is the rubric
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
   after an Agent's or Automation's closing `---` fence is its prompt; a
   Scorer's body is its rubric. Never fold either into frontmatter.
3. Prefer the smallest edit that satisfies the request.

## Author against the schema
The current server parser rejects unknown fields, but bundled schemas can be
older than the server. They therefore validate known fields while preserving
unknown properties and newer catalogue values. Do not invent a field when a
documented one exists, and do not delete an existing unknown field. Duplicate
keys, YAML anchors, aliases, explicit tags, and multiple documents remain
invalid.

The bundled JSON Schemas are the machine-readable contract:

```
schemas/factory.schema.json      factory.yaml
schemas/agent.schema.json        agents/<name>/agent.md frontmatter
schemas/automation.schema.json   automations/<name>/automation.md frontmatter
schemas/runner.schema.json       runners/<name>.yaml
schemas/scorer.schema.json       scorers/<name>/scorer.md frontmatter
schemas/common.schema.json       shared definitions referenced by the above
```

Read `references/schema.md` for the field-by-field reference, defaults, and
inheritance rules. Read `references/triggers.md` before writing or changing an
automation trigger: filter keys are specific to each provider and event, and
the parser does not catch a wrong one. Read `references/scorers.md` before
writing or changing a Scorer. Read `references/examples.md` for worked
examples of each resource.

## Validate before opening a pull request
Run the bundled validator with Python 3.8 or newer, using the host's command (`python3`,
`python`, or `py -3`). Quote both paths because an app-bundle path can contain
spaces.

```bash
python3 "{{skill_dir}}/scripts/validate_factory_files.py" "<factory-root>"
```

Add `--json` for machine-readable output. A non-zero exit means at least one
problem; fix every reported problem and re-run until it is clean.

If no Python 3 interpreter is available, do not install one or claim the tree
was validated without the user's approval. Check the changed document against
the corresponding JSON Schema manually and report that automated validation
was unavailable.

The validator checks known field structure, mutual exclusions, trigger and
Scorer semantics, cron syntax, runner platform rules, and tree-level rules
(exactly one MAIN agent, Agent references, duplicate resource names). Unknown
properties and newer catalogue values pass through for version skew. It does
not resolve server state: model IDs, environment IDs, secret names, runner
names, Scorer model IDs, MCP server IDs, and integration availability are all
validated when the plan is applied. Report that distinction rather than
claiming a tree is fully verified.

If a `warp-server` checkout is available, its parser tests are the authority.
Run them from that checkout, not from the Factory repository:

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

## Schema drift and version skew
The format is `v1alpha1` and still changing. `logic/factoryfile` in
`warp-server` is the authority; these schemas only mirror it.

They also ship inside your Warp version rather than coming from the server, so
they can be older than the server the Factory syncs against. A field the server
added after your version was built will be reported here as unknown.

Because of that:

- Never delete, rename, or rewrite a field only because the validator calls it
  unknown. On a file you did not author, that is at least as likely to be a
  newer field as a mistake. Leave it, and say the schemas may be behind.
- Treat unknown-field reports on your own new edits as real. You are the one
  who just introduced the field.
- If the server and these schemas disagree, the server is right. Say the
  bundled schemas look stale rather than working around the validator by
  skipping it.
- If the validator reports that it does not describe the tree's
  `schemaVersion`, it stopped instead of applying `v1alpha1` rules to a format
  it does not know. Validate with the server; never downgrade `schemaVersion`
  to make the local run pass.

If you are editing the bundled schemas themselves rather than a Factory tree,
their openness is deliberate and load-bearing. Read the "If you are changing
these schemas" section of `references/validation.md` before tightening
anything.
