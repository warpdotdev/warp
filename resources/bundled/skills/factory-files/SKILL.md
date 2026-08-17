---
name: factory-files
description: Create and edit file-based Warp software factory definitions, in a repository tree rooted at a factory.yaml. Use when authoring or changing that factory.yaml, Agent, Automation, Scorer, or Runner files under that root, or its factory and agent skill trees, and when fixing Factory file diagnostics. Do not use for agent-definition Markdown that belongs to another tool, for a tree with no factory.yaml, or to operate a live factory or hand work to one through Factory MCP.
---

# Factory Files

A software factory can be defined by files in a repository. This skill covers
authoring and editing those files, and validating them before you open a pull
request.

warp-server owns the format. It publishes the schema for each version it
supports and validates a tree with the same parser the apply path uses. This
skill carries no copy of the format: a copy ships inside a Warp release, goes
stale against the server, and then reports confident, wrong diagnostics. When
the server cannot be reached, the answer is that the tree was not checked.

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

It selects the tree's resource files and submits them to the server, which runs
the real parser. Add `--json` for machine-readable output and `--server-root
<url>`, or `WARP_SERVER_ROOT`, to point at a local, staging, or self-hosted
server. No credential is required; `WARP_API_KEY` is forwarded when the
environment already carries one, as an agent sandbox does.

The exit code distinguishes three outcomes, and so must you:

- `0` the server checked the tree and found no problem.
- `1` the server checked the tree and reported diagnostics. Fix every one and
  re-run until it is clean.
- `2` the tree was **not** checked. This is not a pass and not a failure; it
  says nothing about the files at all.

### Never imply a check that did not happen
On exit `2`, say plainly that validation did not run and why. Do not describe
the files as valid, correct, or ready, and do not substitute your own reading
of the schema for a verdict. If you cannot reach a server and the change
matters, say so and let the user decide.

On exit `0`, repeat the sentence the validator prints rather than paraphrasing
it into something stronger. A pass means the parser and the state-independent
checks agreed; it does not mean the tree will apply.

Validation resolves no server state. Model IDs, environment IDs, secret names,
runner names, Scorer model IDs, MCP server IDs, integration availability, and
the values of Linear and Slack name aliases are all checked when the plan is
applied. The response lists what it did not check, including any deferred name
aliases; report that distinction rather than claiming a tree is fully verified.

If no Python 3 interpreter is available, do not install one or claim the tree
was validated without the user's approval. Check the changed document against
the fetched schema by hand and report that automated validation was
unavailable.

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

## Do not add a local copy of the format
It is tempting to bundle the schema, or to reimplement a few checks here so
authoring works offline. Both have been tried and removed. A copy inside a Warp
release is routinely older than the server it is used against, and a stale copy
does not fail quietly: it reports a valid field as unknown, and an agent trying
to get to a clean run deletes working configuration to satisfy it. That has
already happened once, to Linear and Slack trigger aliases the server accepts.

Reporting that a tree was not checked costs a little. Reporting the wrong
answer costs correct configuration. Fetch the format when you need it; say
nothing when you cannot.
