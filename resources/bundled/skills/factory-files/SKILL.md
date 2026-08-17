---
name: factory-files
description: Create and edit file-based Warp software factory definitions, in a repository tree rooted at a factory.yaml. Use when authoring or changing that factory.yaml, Agent, Automation, Scorer, or Runner files under that root, or its factory and agent skill trees, and when fixing Factory file diagnostics. Do not use for agent-definition Markdown that belongs to another tool, for a tree with no factory.yaml, or to operate a live factory or hand work to one through Factory MCP.
---

# Factory Files

A software factory can be defined by files in a repository. This skill covers
authoring and editing those files, and validating them before you open a pull
request.

warp-server owns the format. It publishes the schema for each version it
supports and validates a tree with the same parser the apply path uses, so ask
it rather than reasoning from a copy that ships inside a Warp release. This
skill bundles schemas and a validator as an offline floor for when the server
cannot be reached, and always says which of the two ran.

Use this skill for repository files. It is not the skill for operating a live
factory: use `factory-mcp` to send work to a factory, inspect task status, or
pull a task down locally. Playbooks under a factory's own `skills/` directories
tell that factory's agents how to do their job; editing one is a prompt change,
not a schema change, so this skill's rules do not apply to their contents.

## Locate the Factory root
Every Factory tree is rooted at the directory containing `factory.yaml`. All
paths below are relative to that root. A repository may register a
subdirectory as the root, so find `factory.yaml` rather than assuming the
repository root. Do not follow symlinks while looking: the server parses the
repository tree, where a symlink is stored as its target path rather than its
target's content.

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

## Author against the server's schema
Read the tree's `schemaVersion` from `factory.yaml`; a tree that omits it is
`v1alpha1`. Then fetch the schema for that version:

```bash
curl -s https://app.warp.dev/api/v1/factory-files/schemas
curl -s https://app.warp.dev/api/v1/factory-files/schemas/<schemaVersion>
```

The registry lists the versions the server supports. The version endpoint
returns every document describing one version, keyed by file name:
`factory.schema.json` for `factory.yaml`, `agent.schema.json`,
`automation.schema.json`, `runner.schema.json` and `scorer.schema.json` for the
corresponding resources, and `common.schema.json` for the definitions they
share. Both endpoints are unauthenticated. They are exact for the version they
describe: an unknown field is an error, and each enumerated value is one the
server accepts today.

If the server does not publish the declared version, stop. Do not measure the
tree against a version it does not claim to be, and never lower
`schemaVersion` to make a check pass.

Read `references/examples.md` for worked examples of each resource, and
`references/scorers.md` before writing or changing a Scorer. The field-by-field
catalogue is not duplicated here any more; the fetched schema carries it, with
a description on each field.

## Validate before opening a pull request
Run the bundled validator with Python 3.8 or newer, using the host's command
(`python3`, `python`, or `py -3`). Quote both paths because an app-bundle path
can contain spaces.

```bash
python3 "{{skill_dir}}/scripts/validate_factory_files.py" "<factory-root>"
```

It asks the server first and falls back to the bundled copy on its own. Add
`--json` for machine-readable output, `--server-root <url>` to point at a
local, staging, or self-hosted server, and `--offline` to skip the server
deliberately. `WARP_SERVER_ROOT` sets the root too. The validation endpoint is
authenticated and reads `WARP_API_KEY`, which agent sandboxes already carry.

A non-zero exit means at least one problem; fix every reported problem and
re-run until it is clean.

If no Python 3 interpreter is available, do not install one or claim the tree
was validated without the user's approval. Check the changed document against
the fetched schema by hand and report that automated validation was
unavailable.

### Say which validation ran
The validator prints one of two sentences. Repeat it; do not paraphrase it
away.

- Server: the tree went through warp-server's own parser for its declared
  version. State-dependent apply checks still did not run.
- Offline: the server was unreachable, unauthenticated, or answered unusably,
  so the bundled copy ran instead. That copy can be older than the server, so a
  pass is weaker evidence than it looks.

Never present an offline pass as a server verdict, and never treat a successful
schema fetch as validation on its own.

Neither path resolves server state. Model IDs, environment IDs, secret names,
runner names, Scorer model IDs, MCP server IDs, integration availability, and
the values of Linear and Slack name aliases are all checked when the plan is
applied. The server response lists what it did not check; report that
distinction rather than claiming a tree is fully verified.

When the Factory is already registered, a server plan remains the strongest
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
- Trigger filter keys depend on the `(provider, event)` pair. Some fields have
  a friendlier authoring spelling that the server rewrites for you: GitHub
  `baseBranches` and `prNumbers`, Linear `teams`, `projects`, `states` and
  `issues`, and Slack `channels`, `users` and `itemUsers`. Each stands in for
  its canonical key, and declaring both is an error. The Linear and Slack ones
  name objects the server looks up at apply time, so they take a plain list of
  names rather than an `in`/`not_in` matcher.

## When the bundled copy and the server disagree
The server is right. The bundled schemas and validator ship inside your Warp
version, so they can be older than the server the Factory syncs against, and
they are deliberately permissive to avoid rejecting what a newer server
accepts.

- Never delete, rename, or rewrite a field only because the offline validator
  calls it unknown. On a file you did not author, that is at least as likely to
  be a newer field as a mistake. Leave it, and say the bundled copy may be
  behind.
- Treat unknown-field reports on your own new edits as real. You are the one
  who just introduced the field.
- If the offline validator reports that it does not describe the tree's
  `schemaVersion`, it stopped rather than applying `v1alpha1` rules to a format
  it does not know. Validate against the server instead.

If you are editing the bundled schemas themselves rather than a Factory tree,
their openness is deliberate and load-bearing, and it is not the policy the
server's own schemas follow. Read the "If you are changing these schemas"
section of `references/validation.md` before tightening anything.
