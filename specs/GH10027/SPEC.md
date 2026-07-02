# Spec: Multiline redaction + matched-output redaction (GH-10027)

## Problem

Secret Redaction works for single-line tokens (API keys, etc.)
that match the existing single-line regex set. SysOps/DevOps users
hit two gaps:

1. **Matched command outputs:** running `kubectl get secret <name>
   -o yaml` outputs YAML containing base64-encoded secrets. The
   *output* of that command should be redacted before reaching the
   agent, even when the individual lines don't match secret regexes.
2. **Multiline secrets:** `BEGIN PRIVATE KEY` blocks span many
   lines; today only the first line might match a pattern, leaving
   subsequent lines unredacted. The same delimiter blocks may also
   appear in pasted/typed input (e.g., a user pastes a private key
   into the agent prompt).

## Goal

Extend the secret-redaction layer with two new mechanisms:
**output-rule redaction** keyed off the command that produced the
output, and **multiline-pattern redaction** for delimiter-bracketed
blocks (e.g., `BEGIN ... END`). Multiline-pattern redaction applies
to both command output streams and pasted/typed input blocks.

## Pattern syntax

All `command_pattern`, `start_pattern`, `end_pattern`, `argv_contains`,
and `argv_excludes` fields are **Rust regex** (the `regex` crate
syntax). They are NOT glob patterns. In particular:

- `*` is a regex quantifier, not "any string". Use `.*` if you mean
  "any string", or use anchored boundaries (`(\s|$)`) to allow
  trailing arguments.
- Patterns SHOULD be anchored at start (`^`) when matching a
  command-line; trailing-arg tolerance is achieved with
  `(\s+\S+)*\s*` or a lookahead `(\s|$)`.
- The `(?i)` inline flag may be used for case-insensitive matching.
- Whitespace inside patterns is literal (no `x` flag); use `\s+` for
  whitespace runs.

Every default pattern below has at least one positive unit test
(must match) and at least one negative unit test (must not match
near-miss commands like `kubectl get secrets-config` for the
`kubectl get secret` rule).

## Rule id naming convention

- **Built-in (default) rules** use the reserved prefix
  `default-<tool>-<purpose>` in kebab-case. Examples:
  `default-kubectl-get-secret`, `default-aws-secretsmanager`,
  `default-vault-kv-get`, `default-pem-private-key`.
- **User rules** use any non-empty string id that does NOT begin
  with `default-`. The `default-` prefix is reserved.
- A user rule whose id begins with `default-` and does NOT correspond
  to a real built-in id is rejected at config-load time with a
  warning surfaced in the Settings UI; the rule is dropped, and all
  other rules (including built-ins) remain active.

## Behavior contract

### B1 — Command-keyed output redaction

A new section in the redaction config:

```toml
[[redaction.command_output_rules]]
id = "default-kubectl-get-secret"
command_pattern = "^kubectl(\\s+\\S+)*\\s+get\\s+secret(\\s|$)"
redact_full_output = true
replacement = "[redacted: kubectl secret output]"
```

When the active block's command (as detected by Warp's existing
command parser) matches `command_pattern`, the entire block output
sent to the agent is replaced with `replacement`. The user still
sees the real output in their terminal — only the agent context
is redacted.

#### B1.1 — Command matching normalization

`command_pattern` does **not** apply to the raw command string
verbatim. Before matching, Warp normalizes the command into a
*subcommand path string* and a separate *argv set*. The
`command_pattern` regex matches ONLY against the subcommand path
string. Trailing resource-name positionals are NOT included in the
string `command_pattern` matches against — they are separated into
the argv set.

1. **Argv parse:** Split using shell-aware tokenization (the same
   parser used elsewhere in Warp). Quoting and escaping are
   respected.
2. **Subcommand path extraction:** For known multi-word CLIs
   (`kubectl`, `aws`, `gcloud`, `vault`, `op`, `gh`, `doppler`,
   `bw`), the normalizer walks the argv from index 0 and consumes
   tokens into the subcommand path **only while the token is a
   non-flag verb-like segment recognized by the per-CLI subcommand
   table**. Resource names, IDs, and other free-form positionals
   are NOT part of the subcommand path; they go into the argv set.

   Concretely:

   - `kubectl get secret foo`
       - subcommand path: `kubectl get secret`
       - argv set: `{ "foo" }`
   - `aws secretsmanager get-secret-value --secret-id my/secret`
       - subcommand path: `aws secretsmanager get-secret-value`
       - argv set: `{ "--secret-id", "my/secret" }`
   - `kubectl -n ns get secret foo`
       - subcommand path: `kubectl get secret`
       - argv set: `{ "-n", "ns", "foo" }`
   - `kubectl get secrets-config foo`
       - subcommand path: `kubectl get secrets-config`
         (because `secrets-config` is NOT in the verb table; the
         normalizer leaves it in the path because it sits where a
         subcommand would, but the per-rule regex anchored on
         `secrets?(\s|$)` rejects it — see negative tests in T10).

   The CLI-aware verb tables that drive the walk live in code, not
   in user config; users do not need to learn the internal verb
   tables to write rules. The default rules in B1.2 are written
   against the same tables.

3. **Argv set:** Everything that is not in the subcommand path —
   flags, flag values, resource names, file paths, anything else —
   ends up here as an unordered set. Rules use `argv_contains` /
   `argv_excludes` to constrain on this set.

4. **Pattern semantics (canonical):** `command_pattern` is a Rust
   regex matched against the **subcommand path string** described
   above. **It is NOT matched against any string that includes the
   trailing resource-name positionals.** A rule that wants to
   require/forbid a resource name uses `argv_contains` /
   `argv_excludes`; the resource-name positional never appears in
   the string `command_pattern` is run against.

   The trailing `(\s|$)` boundary in the V1 default patterns
   (e.g., `^kubectl\s+get\s+secrets?(\s|$)`) exists for one reason
   only: to allow zero-or-more **subcommand-path** tokens to follow
   without admitting near-miss tokens. The end-of-string case
   (`$`) covers `kubectl get secret` with no further subcommand
   tokens; the whitespace case (`\s`) covers a deeper subcommand
   that may extend the path within the verb table (e.g., a future
   `kubectl get secret some-known-verb`). It does NOT exist to
   admit trailing resource names — those are not in the matched
   string at all.

5. **Why this matters.** The round-2 review surfaced an internal
   inconsistency where the prior wording could be read as either
   "subcommand path only" or "subcommand path including trailing
   positionals." The canonical answer is **subcommand path only**.
   This is the single contract; reviewers should reject any spec
   text or code comment that suggests otherwise. T_command_match_contract
   (added in Test plan) asserts the contract on every default
   rule and on a synthetic adversarial input.

6. **Explicit non-contradiction clause (round-N fix).** Any earlier
   mention in this spec of trailing `(\s|$)` "admitting trailing
   positionals" is to be read STRICTLY as admitting trailing
   *subcommand-path tokens drawn from the per-CLI verb table* —
   never as admitting resource-name positionals, file paths, or
   any other free-form argv tokens. Resource names, IDs, file
   paths, and flags are members of the argv set only and are
   invisible to `command_pattern`. Where prior round notes used
   the phrase "trailing positionals," that phrase refers to
   verb-table subcommand tokens; it does NOT refer to user-supplied
   resource positionals. This clause supersedes any earlier
   wording in this document that could be read otherwise.

#### B1.2 — Default command-output rules (regex form)

V1 ships defaults covering the most common secret-bearing CLIs.
Each is argument-order independent per B1.1. Patterns are matched
against the normalized subcommand path string (NOT against any
string that includes resource-name positionals). The `(\s|$)`
trailing boundary admits trailing *verb-table subcommand tokens*
only — not user-supplied resource names — and excludes near-miss
tokens like `secrets-config`. Resource names and other free-form
positionals live in the argv set and are constrained via
`argv_contains` / `argv_excludes`, not via `command_pattern`.

| id                                   | `command_pattern` (Rust regex)                                |
| ------------------------------------ | ------------------------------------------------------------- |
| `default-kubectl-get-secret`         | `^kubectl\s+get\s+secrets?(\s|$)`                             |
| `default-kubectl-describe-secret`    | `^kubectl\s+describe\s+secrets?(\s|$)`                        |
| `default-kubectl-get-sealedsecret`   | `^kubectl\s+get\s+sealedsecrets?(\s|$)`                       |
| `default-aws-secretsmanager`         | `^aws\s+secretsmanager\s+get-secret-value(\s|$)`              |
| `default-aws-ssm-get-parameter`      | `^aws\s+ssm\s+get-parameters?(\s|$)`                          |
| `default-gcloud-secrets-access`      | `^gcloud\s+secrets\s+versions\s+access(\s|$)`                 |
| `default-vault-kv-get`               | `^vault\s+kv\s+get(\s|$)`                                     |
| `default-vault-read`                 | `^vault\s+read(\s|$)`                                         |
| `default-op-item-get`                | `^op\s+item\s+get(\s|$)`                                      |
| `default-bw-get`                     | `^bw\s+get(\s|$)`                                             |
| `default-gh-secret-list`             | `^gh\s+secret\s+list(\s|$)`                                   |
| `default-doppler-secrets-get`        | `^doppler\s+secrets\s+get(\s|$)`                              |

All defaults redact the full output block and use a stable `id`
so users can override or disable them (see B3).

#### B1.3 — Shell prefixes, compound invocations, and command-substitution

The normalizer in B1.1 receives the **effective leaf command** that
actually produced the captured output block, not the raw line the
user typed. This closes the security gap where a secret-bearing
CLI is wrapped in a shell prefix or compound expression and would
otherwise slip past `command_pattern`.

The following invocation forms MUST all normalize to a
subcommand_path that the matching V1 default rule sees and
redacts:

| Form                                          | Effective leaf command captured by the normalizer | Matched by V1 default? |
| --------------------------------------------- | -------------------------------------------------- | ---------------------- |
| `kubectl get secret foo`                      | `kubectl get secret foo`                           | YES (`default-kubectl-get-secret`) |
| `sudo kubectl get secret foo`                 | `kubectl get secret foo`                           | YES — `sudo` is a recognized prefix and is stripped before normalization |
| `sudo -E kubectl get secret foo`              | `kubectl get secret foo`                           | YES — flags to `sudo` are consumed by the prefix table |
| `env FOO=bar kubectl get secret foo`          | `kubectl get secret foo`                           | YES — `env` prefix + assignments stripped |
| `KUBECONFIG=./kc kubectl get secret foo`      | `kubectl get secret foo`                           | YES — leading `VAR=value` shell assignments stripped before resolving the leaf |
| `time kubectl get secret foo`                 | `kubectl get secret foo`                           | YES — `time`, `nice`, `nohup` are recognized prefixes |
| `/usr/local/bin/kubectl get secret foo`       | `kubectl get secret foo`                           | YES — absolute-path argv[0] is basenamed before normalization |
| `xargs kubectl get secret foo`                | `kubectl get secret foo` (one invocation per arg group) | YES — `xargs` is a recognized prefix that defers to its trailing command |
| `aws secretsmanager get-secret-value …`<br>`\| jq .`             | `aws secretsmanager get-secret-value …`            | YES — for `cmd \| filter` (pipeline), the block-level capture is keyed off the **first** command in the pipeline. Output is the post-pipeline stdout, but the rule is keyed off the head command. |
| `vault kv get x && echo done`                 | TWO independent commands captured per block; rule fires on the `vault kv get x` portion | YES — `&&`, `\|\|`, and `;` separate the captured block into segments, each normalized and matched independently. The block's captured output is segmented by separator and redacted per-segment. |
| `( kubectl get secret foo )` / `{ kubectl get secret foo ; }` | `kubectl get secret foo`                           | YES — grouping constructs are unwrapped before normalization |
| `bash -c 'kubectl get secret foo'`            | `kubectl get secret foo`                           | YES — `bash -c`, `sh -c`, `zsh -c` with a single-string argument are recursively re-parsed and the inner leaf is normalized |
| `kubectl get secret $(name)` / `` kubectl get secret `name` `` | `kubectl get secret <substituted>`                 | YES — command substitution in the resource positional does NOT escape the rule because the resource positional is in the argv set, not in `command_pattern`'s matched string |
| `watch -n5 'kubectl get secret foo'`          | `kubectl get secret foo`                           | YES — `watch` with a quoted command argument is treated like `bash -c` |
| `unknown-wrapper kubectl get secret foo`      | NOT matched                                        | NO — only the explicit prefix table is stripped. Unknown wrappers are matched verbatim and intentionally fail closed for the wrapper (the user's terminal still shows real output; the agent context shows the verbatim wrapper command and its full output **unredacted** unless a user rule names the wrapper). See "Compound-invocation fail-open caveat" below. |

**Prefix table (V1).** The stripped-before-normalization prefix
list is exhaustively: `sudo` (with its own flag set),
`env` (with trailing `VAR=value` assignments), `time`, `nice`,
`ionice`, `nohup`, `stdbuf`, `xargs`, `watch`, leading
`VAR=value …` assignments with no command verb, and `bash -c` /
`sh -c` / `zsh -c` / `dash -c` (single-string forms are
recursively re-parsed).

**Compound-invocation fail-open caveat.** A wrapper that is NOT
in the V1 prefix table (e.g., a custom user script
`my-secret-runner kubectl get secret foo`) is intentionally NOT
auto-stripped. This is documented as a known V1 gap, not a silent
fail-open: the spec's threat model assumes the user controls the
prefix table. Users can either (a) define a user rule whose
`command_pattern` includes their wrapper, or (b) request the
wrapper be added to the prefix table in a follow-up. The clippy
lint in B4.3 still enforces that any agent-bound text path runs
through the choke-point — i.e., the wrapper case still falls
under single-line and multiline redaction (e.g., PEM blocks in
the wrapped output are still redacted by the B2 multiline rule
even though the B1 command-keyed rule did not fire).

**Acceptance criteria for B1.3.** All matched rows above MUST
fire the corresponding V1 default rule in T_shell_prefix_contract
(added in Test plan). The "unknown-wrapper" row MUST NOT fire
the V1 default rule but MUST still trigger B2 multiline PEM
redaction if the captured output contains a PEM block — i.e.,
defense in depth across single-line, command-keyed, and multiline
layers.

### B2 — Multiline pattern redaction

```toml
[[redaction.multiline_rules]]
id = "default-pem-private-key"
# IMPORTANT: the algorithm-prefix segment uses `[A-Z ]*` (zero-or-more,
# not one-or-more), so the bare `-----BEGIN PRIVATE KEY-----` form
# (PKCS#8, no algorithm tag) is matched alongside the algorithm-tagged
# variants such as `BEGIN RSA PRIVATE KEY`, `BEGIN EC PRIVATE KEY`,
# `BEGIN OPENSSH PRIVATE KEY`, `BEGIN ENCRYPTED PRIVATE KEY`. This
# closes the round-2 critical gap where `[A-Z ]+` left bare PKCS#8
# private keys unredacted in both output and pasted input. The
# trailing optional space inside `[A-Z ]*` is preserved so a stray
# space before "PRIVATE" still matches.
start_pattern = "-----BEGIN [A-Z ]*PRIVATE KEY-----"
end_pattern = "-----END [A-Z ]*PRIVATE KEY-----"
replacement = "[redacted: PEM key block]"
inclusive = true
applies_to = ["output", "input"]
```

The above pattern matches every form of PEM private-key block that
the OpenSSL / OpenSSH / cryptography ecosystems emit:

| Form                                              | BEGIN/END line                                 | Matched by V1 default? |
| ------------------------------------------------- | ---------------------------------------------- | ---------------------- |
| Bare PKCS#8                                       | `-----BEGIN PRIVATE KEY-----`                  | YES (zero-or-more handles the empty algorithm segment) |
| PKCS#1 RSA                                        | `-----BEGIN RSA PRIVATE KEY-----`              | YES |
| SEC1 EC                                           | `-----BEGIN EC PRIVATE KEY-----`               | YES |
| OpenSSH                                           | `-----BEGIN OPENSSH PRIVATE KEY-----`          | YES |
| Encrypted PKCS#8                                  | `-----BEGIN ENCRYPTED PRIVATE KEY-----`        | YES |
| DSA                                               | `-----BEGIN DSA PRIVATE KEY-----`              | YES |

Acceptance test T16 (added in Test plan) is the regression guard
that fixes this critical gap and asserts the bare PKCS#8 form is
redacted both on the output stream and on the pasted-input path.

The redaction layer walks the stream line-by-line. When
`start_pattern` matches, all subsequent lines are buffered until
`end_pattern` matches; the entire buffered range is replaced with
`replacement`. If `end_pattern` never matches (truncated stream),
the open block is replaced at flush time so no partial PEM body
leaks.

#### B2.1 — `inclusive` field

`inclusive` controls whether the lines that contain the matching
delimiters are themselves redacted:

- `inclusive = true` — the redaction range INCLUDES the line
  containing `start_pattern` and the line containing `end_pattern`.
  The delimiters themselves are replaced. Use for PEM blocks where
  the BEGIN/END markers should not appear in agent context.
- `inclusive = false` — only the content BETWEEN the delimiter
  lines is replaced. The lines containing `start_pattern` and
  `end_pattern` are forwarded verbatim. Use when the delimiter
  itself is a useful structural marker (e.g., a heredoc tag) and
  only the body is sensitive.

**Default**: `false`. Built-in PEM rule sets `inclusive = true`
explicitly.

#### B2.2 — `applies_to` field

`applies_to` is the explicit scope for the rule:

- `"output"` — redact this delimiter block when it appears in
  command output streams flowing to the agent.
- `"input"` — redact this delimiter block when it appears in
  pasted or typed user input flowing to the agent (e.g., the
  prompt box). Terminal display is not affected by input-side
  redaction either; only the agent context is sanitized.

V1 default for the PEM private-key rule is
`applies_to = ["output", "input"]` so a user pasting a private key
is protected by default. Users may scope rules narrower if needed.

### B3 — Default rule set and override semantics

V1 ships defaults for the most common cases:

- The 12 command-output rules listed in B1.2.
- PEM private-key blocks (bare PKCS#8 with no algorithm prefix,
  plus RSA, EC, OPENSSH, ENCRYPTED, DSA — see B2 algorithm-form
  table) → multiline (B2), applied to both output and input
  (`id = "default-pem-private-key"`, `inclusive = true`,
  `start_pattern = "-----BEGIN [A-Z ]*PRIVATE KEY-----"` —
  `[A-Z ]*` is zero-or-more so the bare form matches).
- Generic env-style `\bAKIA[0-9A-Z]{16}\b` (already handled by
  single-line redaction; called out so users don't add a
  duplicate).

#### B3.1 — Override semantics

User-supplied redaction rules merge with defaults by rule `id`
(string). Rule `id` is mandatory for both default and user-supplied
rules.

Rules:

1. **Identity by id.** Two rules with the same `id` are considered
   the same logical rule. User wins.
2. **Replace by id.** A user rule with the same `id` as a default
   rule fully replaces the default's pattern, replacement, and
   scope. The merged rule keeps the user's fields.
3. **Disable by id.** A user rule with `disabled = true` and the
   id of a default rule disables that default rule. No new rule
   is added.

   ```toml
   [[redaction.command_output_rules]]
   id = "default-kubectl-get-secret"
   disabled = true
   ```
4. **Add by new id.** A user rule with an id not matching any
   default is appended to the active rule set.
5. **Precedence (highest first):**
   1. User-explicit (user-supplied rule with non-disabled state)
   2. User-disabled (user-supplied rule with `disabled = true`)
   3. Default built-in
6. **Within the active set, evaluation order is:** command-output
   rules first (block-level, may short-circuit the rest);
   single-line rules next; multiline rules last (because they may
   span lines that single-line rules already redacted).

#### B3.2 — Invalid user-rule handling (per-rule fail-closed)

Each user-supplied rule is validated INDIVIDUALLY at config-load
time. The fail-closed invariant: **invalid rules drop in isolation
and never disable built-in protections**.

| Failure mode                                 | Result                                                                                                               |
| -------------------------------------------- | -------------------------------------------------------------------------------------------------------------------- |
| Invalid regex in any pattern field           | Rule is DROPPED. Other user rules and ALL defaults remain active. Warning logged with rule id and parser error. Surfaced in Settings UI as `Rule '<id>' has invalid pattern: <error>`. |
| Missing required field (`id`, `pattern`, …)  | Rule is DROPPED. Other rules unaffected. Warning surfaced in Settings UI.                                            |
| Duplicate user-supplied id (same id twice in user config) | The first occurrence is kept and participates in normal B3.1 merge semantics against any matching default; subsequent duplicates within the user config are DROPPED with a warning. If the kept first occurrence shares an id with a built-in default, it overrides/disables that default per B3.1 (Replace/Disable). The "duplicate" failure mode applies only to repeated user entries within the user config; it does NOT inhibit the documented user-wins override of a built-in default by the kept first occurrence. |
| User id begins with reserved `default-` prefix but does not match a real built-in | Rule is REJECTED with warning; dropped. Other rules unaffected. |
| Whole config file malformed (TOML parse error)  | The user config is treated as empty. **All built-in defaults remain active.** Loud warning in Settings UI; redaction is never disabled by a config error. |

Net invariant: there is **no failure mode in user configuration
that disables built-in default redaction**. Defaults are always
loaded from a compiled-in source independent of user config
parsing.

### B4 — Centralized agent-context boundary

Redaction MUST run at a single choke-point on every path that
sends text to the agent. Decentralized redaction (each call site
remembering to call the redactor) is the failure mode this section
prevents.

#### B4.1 — `redact_for_agent` choke-point

A single function:

```rust
/// Redact `text` according to the active rule set before it leaves
/// Warp for the agent. Every agent-bound text path MUST call this.
pub fn redact_for_agent(text: &str, scope: AgentBoundScope) -> String;
```

`AgentBoundScope` carries enough context for the redactor to
distinguish output-side vs input-side rules and to associate
output with its originating command (for B1).

#### B4.2 — Enumerated agent-bound paths

Every path below MUST call `redact_for_agent`. This list is
exhaustive for V1:

| Path                                                                  | Scope    | Notes                                                                 |
| --------------------------------------------------------------------- | -------- | --------------------------------------------------------------------- |
| Block output capture (existing single-line redactor's call site)      | Output   | Passes originating command to enable B1 command-keyed rules.          |
| Prompt input field (text typed into the agent prompt)                 | Input    | Runs at submit, not on each keystroke.                                |
| Pasted text into the prompt                                           | Input    | Runs at paste-commit, before submission, so PEM blocks never reach the wire. |
| Tab config `commands` field input (user-configured command shortcut)  | Input    | Same as prompt input — text becomes part of agent context.            |
| Conversation history replay (re-sending prior context)                | Output   | Replays go through the same choke-point with the same rules.          |
| Tool-call inputs returned to the agent (bash tool stdout, edit tool diffs, file-read tool content) | Output   | Each tool's "result back to model" path calls the choke-point.        |
| File-read content forwarded to the agent                              | Output   | Includes editor selection forwarded as context.                       |

#### B4.3 — Lint enforcement

A custom clippy lint (mirroring the pattern used in PR #10222 for
safe-read enforcement) flags any new construction of
agent-bound text that bypasses `redact_for_agent`. The lint is
keyed on the type of the agent-bound message struct: any code
that constructs that struct from a `String` MUST source the string
from `redact_for_agent` or be annotated with a justification
comment that the linter accepts.

CI fails on lint violations. Adding a new agent-bound path is
therefore a deliberate act that requires either calling the
choke-point or explicitly justifying a bypass.

### B5 — Telemetry-safe

Redaction failures (a multiline block opened but never closed
before the buffer cap, an oversized block triggering the cap
behavior in B6, an invalid user rule dropped) are logged with the
rule `id` only — never the buffered or matched content.

### B6 — Performance bound (buffer cap behavior)

Multiline buffering is capped at `WARP_REDACTION_MAX_BUFFER_BYTES`
(default 1 MiB). The cap behavior is designed so that no
post-cap content ever leaks once redaction is active:

1. **State machine:** Each multiline rule has two states:
   `inactive` and `active`. Matching `start_pattern` transitions
   `inactive → active`. Matching `end_pattern` transitions
   `active → inactive` and emits the `replacement`.
2. **Cap reached while `active`:** When the accumulated buffer
   exceeds the cap, the redaction layer:
   1. **Drops** the buffered bytes from memory (no leak through
      logs or telemetry).
   2. **Keeps** the rule in the `active` state — does *not* revert
      to `inactive`.
   3. Emits the placeholder
      `[REDACTED: oversized block, content dropped]` once into the
      agent stream in place of the dropped buffer.
   4. Continues scanning subsequent input lines for `end_pattern`.
      Any line read while `active` is consumed by the redactor and
      not emitted to the agent.
3. **Exit:** State returns to `inactive` only on (a) `end_pattern`
   match or (b) stream flush / EOF. Stream flush in `active` state
   emits `[REDACTED: unterminated block, content dropped]` and
   resets state.
4. **Net guarantee:** Once a multiline rule transitions to
   `active`, no line is forwarded to the agent until the rule
   transitions back to `inactive`, regardless of buffer cap.

## Acceptance criteria

- A1. With default rules: running `kubectl get secret foo -o yaml`
  shows real output in the terminal but the agent context shows
  the replacement string. The same applies to
  `kubectl -n ns get secret foo` and
  `kubectl get -n ns secret foo` (argument-order independence per
  B1.1).
- A2. With default rules: pasting a private-key block (PEM
  delimiters) into the prompt or receiving one in command output
  redacts the entire BEGIN..END range INCLUDING the BEGIN and END
  lines (`inclusive = true`), not just the first line.
- A3. A user rule in TOML with the same `id` as a default rule
  replaces that default. A user rule with `id = "<default-id>"` and
  `disabled = true` disables the corresponding default. A user rule
  with a new id is added. (Precedence per B3.1.)
- A4. A truncated PEM block (no END line) is still fully redacted
  via the buffer-cap fail-safe (B6). No subsequent lines leak into
  the agent context until end-delimiter or stream flush.
- A5. An oversized PEM-style block (greater than 1 MiB) emits the
  oversized-block placeholder, drops the buffered bytes from
  memory, and continues to suppress subsequent lines until the END
  delimiter matches.
- A6. A user config containing one rule with an invalid regex and
  one valid rule loads the valid rule, drops the invalid rule with
  a warning, and keeps ALL built-in defaults active.
- A7. A completely malformed user config (unparseable TOML) keeps
  all built-in defaults active and surfaces a warning in the
  Settings UI.
- A8. Every agent-bound path enumerated in B4.2 routes through
  `redact_for_agent`. Adding a new agent-bound construction site
  without calling the choke-point fails CI via the clippy lint
  (B4.3).
- A9. Every shell-prefix / compound-invocation form enumerated in
  the B1.3 "Matched by V1 default" column fires the corresponding
  V1 default rule. The unknown-wrapper row does NOT fire the V1
  command-keyed rule but DOES still trigger PEM multiline
  redaction if the captured output contains a PEM block (defense
  in depth across B1, single-line, and B2 layers).

## Test plan

- T1–T4 = unit tests for each rule type (single, command-keyed,
  multiline-bracketed, fail-safe).
- T5 = integration test feeding a fixture command output through
  the redaction pipeline and asserting both terminal display and
  agent context.
- T6 = command-matching normalization tests covering argument
  reordering across all 12 default command-output rules.
- T7 = override-semantics tests for the four merge cases in B3.1
  (replace, disable, add, precedence).
- T8 = buffer-cap state-machine test verifying that lines after
  the cap are not leaked until end-delimiter or flush.
- T9 = input-side multiline test: pasted PEM block in the agent
  prompt is redacted before the prompt is sent to the agent.
- T10 = per-rule positive/negative regex test for every default
  command-output rule (must match intended forms; must NOT match
  near-miss commands such as `kubectl get secrets-config`,
  `aws secretsmanager describe-secret`, `vault kv put`).
- T11 = `inclusive` field test: a multiline rule with
  `inclusive = false` retains delimiter lines in the agent stream
  while replacing only the body; `inclusive = true` replaces the
  entire range.
- T12 = invalid-user-rule isolation test: config with a mix of
  valid and invalid rules loads only the valid ones; built-in
  defaults remain active. Includes the duplicate-id case and the
  reserved-prefix case.
- T13 = malformed-config fail-closed test: TOML parse failure
  results in an empty user-rule set with all defaults active.
- T14 = agent-context boundary integration test: each path in
  B4.2 (block output, prompt submit, paste, tab-config command,
  history replay, tool-call result, file-read forwarding) is
  exercised end-to-end with a sentinel secret that MUST be
  redacted.
- T15 = lint test: a deliberately-broken commit constructs the
  agent-bound message struct from raw `String` without going
  through `redact_for_agent` and the clippy lint flags it.
- T16 = **PEM bare-PKCS#8 critical regression test** (closes the
  round-2 critical gap). Six fixtures, each a complete BEGIN..END
  PEM block:
  - `-----BEGIN PRIVATE KEY-----` (bare PKCS#8, NO algorithm tag)
  - `-----BEGIN RSA PRIVATE KEY-----`
  - `-----BEGIN EC PRIVATE KEY-----`
  - `-----BEGIN OPENSSH PRIVATE KEY-----`
  - `-----BEGIN ENCRYPTED PRIVATE KEY-----`
  - `-----BEGIN DSA PRIVATE KEY-----`

  For each fixture:
    - Output path: insert into a captured command output stream;
      assert the entire BEGIN..END range is replaced with
      `[redacted: PEM key block]`, including BEGIN and END lines
      (because `inclusive = true`).
    - Input path: insert into the agent prompt-paste path; assert
      the same redaction before send.

  The bare PKCS#8 case is the explicit regression guard for the
  `[A-Z ]+` → `[A-Z ]*` fix; this case MUST fail under the prior
  pattern and pass under the new one. CI fails if any of the six
  fixtures leaks a single key byte to the agent stream.

- T_command_match_contract = **Command-matching contract test**
  (closes the round-2 important "internally inconsistent" gap on
  whether `command_pattern` sees trailing resource positionals).
  Drive the normalizer with the following inputs and assert the
  documented (subcommand_path, argv_set) split:

  | Input                                       | Expected subcommand_path             | Expected argv_set                          |
  | ------------------------------------------- | ------------------------------------ | ------------------------------------------ |
  | `kubectl get secret foo`                    | `kubectl get secret`                 | `{"foo"}`                                  |
  | `kubectl -n ns get secret foo`              | `kubectl get secret`                 | `{"-n", "ns", "foo"}`                      |
  | `aws secretsmanager get-secret-value --secret-id my/s` | `aws secretsmanager get-secret-value` | `{"--secret-id", "my/s"}`              |
  | `vault kv get kv/data/app/prod`             | `vault kv get`                       | `{"kv/data/app/prod"}`                     |
  | `kubectl get secrets-config foo`            | `kubectl get secrets-config`         | `{"foo"}` (rule rejects via regex anchor)  |

  Then assert that the V1 default `command_pattern` regexes
  (B1.2) are matched ONLY against the subcommand_path string and
  that `match("kubectl get secret", default_kubectl_get_secret)` is
  `Some(_)` while `match("kubectl get secret foo", ...)` is NEVER
  invoked by the matcher (the resource name is in the argv set,
  not in the matched string). A test double for the matcher
  records the exact strings it sees and fails if any contains a
  trailing resource positional. Asserts the canonical "subcommand
  path only" contract end-to-end.

- T_shell_prefix_contract = **Shell-prefix / compound-invocation
  contract test** (closes the round-N security concern that the
  spec did not specify how common shell prefixes and compound
  invocations interact with `command_pattern`). For every row in
  the B1.3 table:

    1. Feed the raw command line into the normalizer.
    2. Assert the resulting effective leaf command equals the
       row's "Effective leaf command" column.
    3. Run the V1 default rule set against the effective leaf
       command and assert it fires when the row says "YES" and
       does NOT fire when the row says "NO".
    4. For the `cmd && cmd2` / `cmd ; cmd2` / `cmd || cmd2` rows,
       assert the captured block is segmented and that the
       redaction fires on the correct segment(s) while the
       non-secret-bearing segment's output is forwarded
       unredacted.
    5. For the `bash -c '...'` / `sh -c '...'` / `watch -n5 '...'`
       rows, assert that the inner quoted command is recursively
       re-parsed and that the inner leaf is what matches.
    6. For the unknown-wrapper row, assert that the V1
       command-keyed rule does NOT fire AND that a PEM block
       embedded in the captured output IS still redacted via the
       B2 multiline rule. This is the defense-in-depth regression
       guard for the documented compound-invocation fail-open
       caveat.

  CI fails if any matched row leaks a single secret byte to the
  agent stream, or if any non-matched row (the unknown wrapper)
  is silently extended to fire the command-keyed rule (which
  would change the documented fail-open behavior without an
  intentional spec change).

## Out of scope (V1)

- ML-based redaction (entropy heuristics, custom-trained models).
- Per-pane redaction overrides.
- User-typed-secret detection beyond explicit delimiter blocks
  (e.g., heuristic detection of password-like tokens in free-form
  input). Targeted for V1.5.
