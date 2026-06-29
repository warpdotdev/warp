---
name: upstream-change-analysis
description: Audits upstream Warp changes for Warper porting decisions with hard evidence gates. Use when comparing this fork with upstream, deciding which upstream commits to port, or writing upstream-porting specs.
---

# Upstream Change Analysis

Use this skill when analyzing upstream `warpdotdev/warp` commits for possible Warper ports.

This skill exists because a previous pass produced fake analysis: dependency claims without checking the dependency graph, vague "why" text that did not answer why, and local-agent/control-plane enthusiasm that ignored what Warper is trying to become. Do not repeat that failure. Warper ports painkillers, not lip gloss, recreational drugs, make-work refactors, or whatever upstream product experiments fell out of a large VC-funded team. The job is not to make upstream sound useful. The job is to reject everything that is not proven necessary for Warper.

## Hard Line

- Apply XP scope control: do not recommend implementation unless Warper will fail its current local-first purpose without it. "Would be nice", "upstream fixed it", generic safety talk, team convenience, and future-facing compatibility are not enough. A `Port` must stop an unsafe-to-run app, local data corruption, command execution exposure in a retained path, credential/local-file exposure, a crash that breaks normal terminal use, or a current build/release path from failing.
- Security labels, advisory IDs, scanner output, and upstream private PR subjects do not justify a port. A security port must prove the vulnerable dependency or code path exists in Warper, prove the path is reachable from a retained local workflow, explain the concrete attacker-controlled input or corruption mechanism, and state what user harm occurs without the port. If any of those points cannot be proven from code and primary evidence, the decision is `Skip` or `Defer`.
- Do not write BS. If a human can ask "did you actually check that?" and the answer is no, the analysis is invalid.
- Do not write conditional relevance when the repo can answer the condition. Check the code.
- Do not write generic rationale. "Security", "correctness", "compatibility", "ergonomics", "rendering", and team-efficiency labels are not reasons.
- Do not treat local features as automatically good. A local control API, local agent plugin, local OAuth flow, local watcher, local credential read, or local automation surface can still be exactly the startup/product bloat Warper is removing.
- Do not create a spec file whose content is mostly "do not port X". That is not a spec. Put rejection in the audit and move on. If a previous pass created a deferred-record spec, delete it instead of preserving it as a tombstone.
- Do not port startup/background work unless it directly supports `WARPER-001`, `WARPER-002`, or `WARPER-005`, and the code evidence proves it.
- Do not use upstream VC-product logic as a rationale. New agent surfaces, plugin managers, control planes, queues, telemetry-adjacent plumbing, or "platform" abstractions are rejected unless Warper's own specs require them.
- Do not launder upstream's reason into Warper's reason. "Why upstream did it" and "why Warper needs it" are separate answers. Upstream can have a real reason that is still useless or harmful for Warper.
- Do not sanitize make-work. If the evidence says upstream moved code around because a large team needed a project, call it churn. Do not rename it "architecture", "platform maturity", or team enablement.

## Regression Accountability Gate

Before any `Port` or `Port manually` implementation starts, write the retained Warper workflow that can regress. For terminal/session changes, this must include tab creation, split-pane creation, multi-pane launch or tab config creation, cross-tab sequencing, and shell startup directory behavior when the touched code can affect those paths. For persistence changes, include restore, migration, and new-object creation. For security changes, include the exploit or corruption path plus at least one benign retained workflow that must not change.

Any upstream diff touching process spawn, terminal server IPC, pane/session creation, working directories, launch configs, tab configs, restore, persistence, file opening, command execution, shell bootstrap, or dependency resolution must satisfy all of these before implementation:

- Identify the exact state or protocol invariant the port relies on.
- Add or update a failing local test for that invariant before production code changes.
- Add a regression test or manual smoke matrix for adjacent user workflows, not only the narrow upstream bug.
- Prefer a smaller local fix when upstream's patch mixes the needed invariant with unrelated refactors, product surface, or speculative hardening.
- Stop and reclassify to `Defer` if the required validation cannot be run in the current toolchain and no reviewer can run it before merge.

## Failure Examples

These are examples of analysis failure, not wording preferences:

- `Relevant only i[f] Warper still carries affected Diesel`: invalid because `Cargo.toml`, `Cargo.lock`, and Diesel usage can be checked.
- `Local terminal rendering correctnes[s]`: invalid because it does not say whether Warper renders that protocol, what the protocol is, what local path handles it, or why the fix matters.
- `Upstream fixed inline images, so local rendering correctness`: invalid because it does not answer whether Warper renders startup inline images, which protocol is involved, whether iTerm/Kitty paths exist, and what user pain this fixes.
- `Port local agent interop`: invalid unless current Warper requirements prove that specific agent path is needed and the code does not add startup/control/plugin/credential surface.
- `Better agent ecosystem support`: invalid because Warper is removing failed startup product surface, not importing upstream's agent experiments by default.
- A `PRODUCT.md` that is 90 percent "do not do upstream things": invalid. That belongs in an audit decision, not a product spec.

## Required Inputs

Read these before deciding anything:

- `specs/WARPER-001/PRODUCT.md`
- `specs/WARPER-002/PRODUCT.md`
- `specs/WARPER-003/PRODUCT.md`
- `specs/WARPER-004/PRODUCT.md`
- `specs/WARPER-005/PRODUCT.md`
- `specs/WARPER-005/TECH.md` if local agent/tool behavior is involved
- Current `AGENTS.md` instructions if present

Then establish the Git facts:

```bash
git remote -v
git merge-base HEAD upstream/master
git log --oneline --decorate --left-right HEAD...upstream/master
git show --stat --oneline --no-renames <commit>
git show --name-only --format=fuller <commit>
```

If `upstream/master` is missing or stale, fetch upstream before analysis. If network access is blocked, say that clearly and base the analysis only on available refs.

## PR And Issue Grounding

For every upstream commit with a PR number in the subject, fetch public metadata when available:

```bash
gh pr view <number> --repo warpdotdev/warp --json number,title,body,url,closingIssuesReferences
gh issue view <number> --repo warpdotdev/warp --json number,title,body,url
```

If the PR or issue is private, deleted, or not publicly resolvable, record that. Do not invent motivation from the title. Use the commit diff, tests, and current Warper code as the evidence.

## Upstream Why Comes First

Before deciding Warper relevance, answer why upstream made the change. Use the PR body, linked issue, release notes, commit message, tests, and diff context. A real upstream why names the pressure behind the change: crash, user-visible bug, data loss, command execution bug, dependency vulnerability, build breakage, platform compatibility failure, product expansion, metrics-driven growth work, internal refactor, cleanup, or team-process work.

Do not turn upstream's why into Warper's why. If upstream changed something for customer growth, onboarding polish, settings discovery, hosted workflows, agent expansion, internal platform work, or broad refactoring, write that plainly. That may be a true explanation for upstream. It is not a porting rationale for Warper.

Classify the upstream motive before the port decision:

- `Painkiller`: fixes proven current pain in a path Warper keeps, such as a normal-use crash, local data loss, command execution bug, local file corruption, credential exposure, build/release blockage, or a dependency vulnerability that is present and reachable.
- `Lip gloss`: improves appearance, copy, discoverability, onboarding, preferences, or nice-to-have compatibility without fixing current Warper pain.
- `Recreational drug`: adds agent orchestration, plugin/control APIs, queues, watchers, background jobs, credential reads, OAuth, cloud-shaped local plumbing, or new automation surface that Warper did not ask for.
- `Churn`: refactors, renames, restructures, test-process changes, abstraction reshuffles, or internal platform work without a concrete retained Warper behavior fix.

Warper ports only painkillers that clear the XP necessity bar. Warper skips lip gloss, recreational drugs, churn, and speculative hardening unless the current code and `WARPER-001` through `WARPER-005` prove that Warper will fail its local-first purpose without the change.

## Current-Code Proof

For every claimed relevance, prove the current Warper path exists. Use `rg`, `Cargo.toml`, `Cargo.lock`, direct source reads, and line-numbered citations.

Required checks by category:

- Dependency bump: exact package versions in `Cargo.toml` and `Cargo.lock`; usage paths if the crate is not obviously build-wide.
- Terminal protocol fix: feature flag, escape parser, model handler, view/event side effect, and whether it affects startup reliability or only visual compatibility.
- MCP change: startup registration, config path, log path, OAuth path, current tool exposure, and whether OpenRouter exposes MCP tools now.
- CLI-agent change: current session model/listener registration, exact supported agents, plugin manager path, and whether adding another agent is a Warper requirement or product bloat.
- Repo/file tool change: OpenRouter tool schema, local executor, file/repo model, and validation for bad inputs.
- UI/UX feature: current Warper product objective, not merely retained UI.
- Platform fix: proof that the platform is a current Warper target. Windows-only changes are skipped unless a Warper spec explicitly adds Windows support.
- Startup-affecting change: models registered at launch, filesystem watchers, secure storage reads, network clients, timers, background tasks, and logs.

Do not summarize a category as "present". Show the exact files and lines.

## Decision Labels

Use exactly these decisions:

- `Port`: current Warper has the affected path, the change clears the XP necessity bar, and the diff does not add hosted/startup/product surface.
- `Port manually`: the local fix clears the XP necessity bar, but upstream mixed it with hosted, telemetry, remote, branding, platform, or broad refactor work.
- `Defer`: the change may fit later, but it is optional, product-significant, startup-sensitive, or not on the current Warper critical path.
- `Skip`: the dependency/path is absent, the target platform is out of scope, or the change conflicts with Warper's local-only requirements.

Do not use a maybe-label unless the user explicitly asks for brainstorming. A porting audit must make a resolution.

## Ruthless Relevance Gate

Answer these questions for each commit:

1. Why did upstream make this change? Cite PR, issue, release note, commit message, test, or diff evidence. If public metadata is unavailable, say exactly what is unavailable and infer only from the diff.
2. Is upstream's motive a `Painkiller`, `Lip gloss`, `Recreational drug`, or `Churn`? Do not skip this classification.
3. What exactly did upstream change?
4. What current Warper code path does it touch? Cite files and lines.
5. What breaks, becomes unsafe, corrupts local state, or blocks current build/release work if Warper does not port it now?
6. Does it add startup work, credential reads, watchers, OAuth, plugin management, local HTTP/control APIs, or new agent orchestration?
7. Does it reintroduce hosted Warp, Oz, cloud tasks, server transcripts, telemetry, billing, Drive, account state, remote session sharing, or upstream branding?
8. Can the useful part be manually ported without the bad part?
9. What validation would prove the port did not regress Warper's local-only baseline?

If any required answer is missing, the decision is `Defer` or `Skip`.

## Spec Creation Gate

Create `specs/WARPER-00X/PRODUCT.md` only when all are true:

- At least one commit is `Port` or `Port manually`.
- The change is product-significant enough that implementation needs a spec.
- The spec says what Warper should build, not mostly what it should avoid.
- The rationale cites current Warper code and WARPER requirements.
- Hosted/Oz/cloud/telemetry/remote/platform noise is excluded from the scope.

Do not create a spec for skipped work, local-agent experiments, broad UI polish, process preferences, or "maybe useful later" ideas. A negative decision belongs in the audit table.

## Output Format

For each commit, produce a row with:

- Commit hash and upstream title.
- PR/issue evidence, or "not publicly resolvable".
- Upstream why, with cited evidence or a clear unavailable-metadata note.
- Upstream motive classification: `Painkiller`, `Lip gloss`, `Recreational drug`, or `Churn`.
- What upstream changed in concrete terms.
- Current Warper evidence with files and line numbers.
- Decision.
- Why Warper should or should not care, separately from upstream's why.
- Validation required if ported.

Keep separate sections for:

- `Port first`
- `Port manually`
- `Defer`
- `Skip`
- `Specs created`
- `Specs not created`

## Validation

Before finishing:

```bash
rumdl check <changed markdown files>
rg -n "Relevant only i[f]|if Warper stil[l]|Local terminal rendering correctnes[s]|Port i[f]|[Cc]onside[r]|PowerShell test[s]|security hardenin[g]|startup-cleanu[p]|startup cleanu[p]" <changed markdown files>
git diff --check
git status --short
```

If a requested preamble intentionally violates `MD041`, run `rumdl check ... --disable MD041` and say why.

Do not claim the analysis is complete unless markdown validation passes, the fake-rationale grep is clean, and every `Port` row has current-code evidence.
