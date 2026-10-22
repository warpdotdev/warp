# Tech spec: OSC 7501 receive path and terminal status indicators

Issue: https://github.com/warpdotdev/warp/issues/16407

Status: proposed; no implementation or runtime validation is claimed by this spec PR.

## Context

Source references below describe `master` at `d0d50955df8c4efcfb91091b150ef4c9a7eaa138`;
implementation should recheck them after spec approval.

- `crates/warp_terminal/src/model/ansi/mod.rs:794`: `osc_dispatch` handles OSC 9 notifications,
  OSC 133 prompt markers, and OSC 777 notifications, but has no OSC 7501 branch.
- `crates/warp_terminal/src/model/ansi/handler.rs:25`: `Handler` is the parser/model boundary;
  `pluggable_notification` at line 413 handles one-shot notification payloads.
- `app/src/terminal/model/terminal_model.rs:2852`: `reset_state` implements terminal reset.
  `prompt_marker` at line 2981 handles prompt boundaries; Warp precmd handling at line 3036 is
  another shell-prompt entry point. `pluggable_notification` at line 3719 emits notification events.
- `app/src/terminal/model_events.rs:289`: model events forward pluggable notifications to consumers;
  exit events are forwarded at lines 246–247, and prompt-start events at line 154.
- `app/src/terminal/cli_agent_sessions/mod.rs:35`: `CLIAgentSessionStatus` models existing CLI-agent
  progress, success, failure, blocked, and cancellation. Its conversation-status conversion starts
  at line 54. Session data includes agent-specific context and rich-input behavior.
- `app/src/terminal/cli_agent_sessions/listener/mod.rs:47`: listeners are restricted to recognized
  agents, and create per-agent handlers. This cannot be the generic protocol's admission gate.
- `app/src/tab.rs:1309`: the horizontal tab's `agent_indicator` currently considers the focused
  terminal's conversation. It does not aggregate protocol records from all panes.
- `app/src/pane_group/pane/view/header_content.rs:78`: `StandardHeader` supports elements beside
  the title, suitable for an existing status icon with a tooltip.

The protocol defines independently replaceable hierarchical records, not agent lifecycle events.
Keep its storage separate from `CLIAgentSessionsModel`; a generic `app` value must never create a
fake agent session, change agent identity, or enable agent controls.

## Proposed changes

### Parser and bounded record model

Add `model/program_status.rs` and `model/program_status_tests.rs` in `warp_terminal`. Define typed
states, blocked kinds, reports, records, and a `ProgramStatusRecords` store. The parser returns a
query, validated report, or ignored input; it never partially mutates storage. The store owns an
update counter for eviction and display ordering, and exposes apply, clear, prompt/exit cleanup,
full reset, result acknowledgement, and highest-priority-summary operations.

Add an OSC 7501 branch and a dedicated `Handler` callback. Parse the original body without
silently concatenating extra semicolon-separated parameters into valid values. Accept BEL/ST and
fragmented delivery through the existing ANSI processor. Verify that the underlying VTE parser's
parameter/sequence bounds cover a maximum legal report; if it truncates oversized input, reject
the complete sequence instead of accepting a valid-looking prefix. Bound accumulation before
allocation, including arbitrarily many malformed or unknown pairs.

Use the protocol's ASCII pair grammar, trimming surrounding whitespace before validation. Validate
all pair limits, including repeated and unknown keys, before selecting the last valid value for a
key. Enforce a 4096-byte sequence cap conservatively including OSC introducer and ST terminator;
16-byte keys; 2732 encoded/2048 decoded bytes for `msg`; 256 encoded/192 decoded bytes for `title`;
32-byte `app`; and 128-byte IDs with at most eight nonempty segments of at most 32 bytes each.
Validate ID/app character sets separately. Base64 accepts standard padded or unpadded encoding;
decoded text must be UTF-8 without C0, DEL, or C1 controls. Invalid text or limits discard the entire
report, even if the offending pair is later superseded. Numeric progress accepts only ASCII decimal
integers in 0–100 and is retained only for working/blocked. Blocked kind is retained only for blocked.

Use a bounded map of at most 256 records. Clearing an ID matches that ID or its slash-delimited
descendants. Resolve app inheritance at read time so changing or clearing a parent updates child
presentation without modifying child records. A report replaces all metadata; preserve no stale
message, progress, kind, or app. Compute a summary with the priority and recency rules in product
invariant 6. Keep record storage independent of primary/alternate grids.

### Query reply, ownership, and events

Handle the capability query through the ANSI processor's existing PTY writer, emitting only:

```text
ESC ] 7501 ; ? : states=idle,working,done,blocked,error : kinds=permission,question,auth ESC \
```

The spaces above are protocol-legal; implementation may omit them. Replies are constant and never
interpolate record content. Avoid introducing a new writer or a model lock from a handler callback.

Own the record store in each `TerminalModel`. Add a dedicated program-status-changed event to the
terminal event plumbing and `ModelEventDispatcher`. Emit it when storage changes, including
removal/eviction; UI consumers observe a summary rather than interpreting raw OSC strings. Do not
route reports through `PluggableNotification` or the per-agent listener.

Clean transient records on OSC 133 initial prompt markers, Warp's actual precmd application, and
PTY exit. Ensure both prompt paths call the same idempotent helper; right prompts must not expire
records. Clear all records on RIS and model destruction, preserving them across DECSTR and screen
switches. Distinguish RIS from other callers of `reset_state` before attaching cleanup there. Shell
command completion alone must not erase terminal results. Acknowledge done/error records at the
terminal's keyboard-input dispatch point, only when input goes to that terminal, not when a palette
or modal receives it. Do not serialize the store or add it to shared-session messages.

### Pane and tab presentation

Provide one summary adapter used by terminal pane headers, horizontal tabs in `app/src/tab.rs`,
and vertical tabs in `app/src/workspace/view/vertical_tabs.rs`. Enumerate all terminal panes in a tab
through the pane group's existing traversal; apply the same priority across pane summaries. Tie
breaks within a pane use record update order; cross-pane ties use a UI-side monotonic observation
order, with stable pane identity as a final deterministic fallback. Do not compare counters belonging
to different terminal stores as though they were a global clock.

Use the existing conversation status icon vocabulary without storing generic records in conversation
history. An idle protocol summary suppresses that pane's legacy badge but contributes no tab
attention candidate. For panes without protocol records, preserve their existing status candidates.
Observe summary changes and pane membership so background-pane updates and closure invalidate
both tab orientations. Pass an already available model reference or event snapshot into render
helpers; audit callers before adding any `TerminalModel` lock.

Own tooltip mouse state on the relevant view rather than creating it during render. Compose an
accessible label and plain tooltip text from state, terminal identity, inherited app, title, message,
and progress. Strip bidi overrides/isolates and invisible formatting controls using an existing
sanitizer if available, otherwise a narrowly scoped display helper. Do not modify stored text or
use report text as markup, navigation, notification content, or a terminal title. Existing rich-input
and notification subscriptions continue receiving only their original event types.

## Testing and validation

| Product invariants | Coverage |
| --- | --- |
| 1–2 | ANSI processor tests feed raw BEL/ST reports and queries across every split point; assert a single constant query reply and no reply for reports, malformed reports, or unknown OSCs. Include a legal maximum-size report and oversized/truncated-sequence rejection. |
| 3–4 | Store tests cover root plus children, complete metadata replacement, nearest-ancestor app inheritance, clearing a missing parent with children, clearing the root, and sibling-prefix preservation. |
| 5, 8 | Model/event tests exercise both prompt mechanisms, right prompts, PTY exit, keyboard acknowledgement, RIS, DECSTR, and primary/alternate transitions; done/error survive prompt and exit until acknowledgement. |
| 6–9 | Summary and pane-group tests cover every priority pair, recency ties, idle precedence, legacy fallback, background panes, pane closure, and both tab layouts. Integration tests assert visible state, tooltip text, and owning pane identity. |
| 10–11 | Parser/display tests cover malformed pairs, whitespace, duplicates, unknown keys/states, invalid IDs/apps/progress/kinds, padded/unpadded base64, invalid UTF-8, controls, bidi formatting, and every size boundary. Assert rejected reports leave records and eviction order unchanged and create no notification/rich-input events. |
| 12–13 | Store tests fill capacity, update an existing record, insert a new record, and verify exact eviction. Open/restore/share tests verify no status leakage or persistence. |

After implementation, run targeted `warp_terminal` tests and relevant app/model/UI integration tests,
then targeted Clippy. Finish with the applicable formatter according to `AGENTS.md`; do not run the
full presubmit unless requested.

Manual acceptance: run the development app with two tabs and split panes. Use a small PTY fixture
to query capabilities and emit working, blocked (each kind), done, error, idle, hierarchical clear,
and malformed reports. Record a narrated demonstration covering background-pane tab indicators,
both tab orientations, tooltips, acknowledgement, prompt/exit cleanup, and screen/reset behavior.
Include screenshots or a recording in the implementation PR. Unit tests alone are insufficient to
claim that the tab/pane feature works.
