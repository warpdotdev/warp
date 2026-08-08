# Spec: Correct per-sender user avatars in shared agent-mode sessions and restored transcripts (client / `warpdotdev/warp`)

Linear: [APP-3164](https://linear.app/warpdotdev/issue/APP-3164/display-correct-user-avatars-in-shared-agent-mode-sessions-and-ambient)
Originating thread: https://warpdev.slack.com/archives/C0BJCFR6GUF/p1785769987907299
Repo: `warpdotdev/warp` (client). Estimate: L. Code references pinned at `warp @ 02c0420632504b39d5b424bb7a5cd06b8f1c4b33`.

> **Multi-repo task — this spec covers the CLIENT half only.** A sibling spec
> covers `warpdotdev/warp-server` (durable per-message author persistence, the
> `AIConversation` GraphQL exposure) and `warpdotdev/warp-proto-apis` (the
> `UserQuery` author field). This spec **consumes** that contract and states the
> exact shape the client needs from it (see *Server / proto contract the client
> depends on*). The client change is not shippable until that contract lands;
> the dependency is called out explicitly below.

## Problem statement
When user A starts a cloud/shared agent-mode conversation and users B, C… send
queries into it via a Warp shared session, every user query renders with a
single avatar. In a **live** shared session the sharer sees the correct
per-sender avatar (resolved against live presence), but as soon as the
conversation is viewed as a **restored transcript or a shared link** (or by a
different viewer), every query collapses to one identity — the conversation
owner's or, worse, the current viewer's own avatar. It is impossible to tell
who actually sent each message.

Root cause (client side): per-message author identity is **ephemeral**. The only
durable author is the single conversation `creator`. Live attribution uses a
session-sharing `ParticipantId` resolved against the in-memory `PresenceManager`;
it is dropped on restore and there is no durable author to fall back to, so the
render path falls back to the local user.

## Current state (how it works today)
- Each `AIAgentExchange` carries `response_initiator: Option<ParticipantId>`
  (`app/src/ai/agent/mod.rs:3200-3202 @ 02c0420`). `ParticipantId` is a
  session-sharing id, only meaningful against a **live** `PresenceManager`. There
  is **no** durable author field on the exchange.
- **Live submit path:** `AIConversation::update_for_new_request_input`
  (`app/src/ai/agent/conversation.rs:2016-2033 @ 02c0420`) sets
  `response_initiator: shared_session_response_initiator` from
  `RequestInput.shared_session_response_initiator`
  (`app/src/ai/blocklist/controller.rs:204 @ 02c0420`). For a viewer's query
  received on the sharer, `execute_agent_prompt_for_shared_session(participant_id)`
  (`app/src/ai/blocklist/controller/shared_session.rs:657-663 @ 02c0420`) supplies
  the participant; for the sharer's own queries it is `None`.
- **Restore path:** restored exchanges are rebuilt from persisted proto
  messages (`api::Task.messages` → `into_exchanges()` →
  `create_exchange_from_messages`,
  `app/src/ai/agent/api/convert_conversation.rs:1881-2036 @ 02c0420`). Line
  **2034** hardcodes `response_initiator: None` — the persisted `UserQuery`
  proto has no author, so nothing can be rehydrated.
- **Avatar render (native + WASM share this):**
  `AIBlock::render` (`app/src/ai/blocklist/block/view_impl.rs:961-993 @ 02c0420`)
  resolves `self.model.response_initiator(app)` against the live
  `PresenceManager` (`get_participant`) to get `(display_name, photo_url, color)`
  and **`.unwrap_or((self.user_display_name, self.profile_image_path, None))`** —
  i.e. the **current local user** whenever there is no live presence (every
  transcript / re-opened view). This local-user fallback is the reported symptom.
  The tuple is passed to `query::maybe_render` as `user_display_name` /
  `profile_image_path` / `avatar_color`.
- **Profile lookups already available client-side** (no new infra needed to
  render a resolved author):
  - `UserProfiles` singleton
    (`app/src/workspaces/user_profiles.rs:35-97 @ 02c0420`):
    `profile_for_uid(UserUid) -> Option<&UserProfileData{display_name, email, photo_url}>`.
  - `UserProfileWithUID{firebase_uid, display_name, email, photo_url}`
    (`crates/cloud_object_models/src/user_profile.rs:6-34 @ 02c0420`), convertible
    from `warp_graphql::user::PublicUserProfile` and from
    `session_sharing_protocol::common::ProfileData`.
  - `AIConversation::server_metadata().creator: Option<UserProfileWithUID>`
    (the single creator added by REMOTE-1782 / warp-server#11468).
  - The live presence `ProfileData` carries `firebase_uid` — the bridge from a
    live `ParticipantId` to a **durable uid** at submit time.
  - The details panel already implements the resolve-then-fallback pattern
    (`app/src/ai/conversation_details_panel.rs:190-206,279-300 @ 02c0420`):
    creator profile → `PrincipalInfo{display_name, photo_url, uid}`; else
    `UserProfiles::profile_for_uid`; else a uid/initial fallback.

## Prior-work sweep (what already exists; what this builds on)
The requester explicitly asked not to repeat work already done or in flight.
Findings, and exactly where this spec builds on vs. avoids each:

- **REMOTE-196** (Done): introduced live-only ephemeral attribution — the
  `response_initiator`/`ParticipantId` machinery
  (`set_current_response_initiator`, live `get_participant` avatar resolution).
  This is the foundation of the **live** path this spec keeps. It never
  persisted anything. No open warp PR.
- **REMOTE-1782** (In Review; assignee Roland Huang) — **this is the "original
  ticket" the requester's follow-up refers to** (Roland Huang is APP-3164's
  creator/assignee). It added the single `AIConversation.creator`
  (warp-server#11468) and departed/absent-participant avatar handling for
  **live** sessions (`get_participant_attribution` in `PresenceManager`). Its
  client PR **#12585** (`ian/shared-session-absent-initiator-avatar`) is
  currently **CLOSED** (not merged as of this SHA), so its client changes may
  have moved to another branch or not yet landed — the implementer MUST
  re-check what actually merged before touching `presence_manager.rs` /
  `view_impl.rs` (see *Open questions resolved*). REMOTE-1782 is
  **owner/creator-level + live departed-participant**, NOT durable per-message
  multi-user attribution in transcripts — that gap is exactly what APP-3164
  fixes.
- **REMOTE-2361** (In Review, `blocked`; PR **#14426**,
  `factory/remote-2361-absent-viewer-color-retention`): client-only. Adds a
  persistent `session_colors` map to `PresenceManager` so per-message avatar
  **colors** survive a browser participant disconnect, and reads avatars via a
  new `get_participant_info_for_avatar` in
  `app/src/ai/blocklist/block/view_impl.rs`. Color/rendering only, **live**
  session; explicitly no server change; does **not** fix restore/transcript
  attribution.
- **REMOTE-2363** (In Review, `blocked`; PR **#14430**,
  `factory/remote-2363-browser-avatar-color`): client-only, WASM/browser. Adds
  attribution to distinguish self vs. others in the browser so per-sender
  colors differ. Touches `view_impl.rs` + `presence_manager.rs`. Color-only,
  live; no persistence.
- **PR #11844** (`oz-agent/unknown-creator-details-panel`, "Show unknown creator
  when profile is missing"): the unknown-author fallback pattern in the details
  panel this spec mirrors for the avatar (see behavior 5).

**Files this spec touches that in-flight PRs also change (do not regress):**
- `app/src/ai/blocklist/block/view_impl.rs` (query-avatar resolution, 961-993) —
  also changed by #14426 and #14430. This spec's restore-time resolution MUST
  build on their `get_participant_info_for_avatar` / self-vs-other attribution,
  adding a durable-profile fallback **after** presence resolution fails, never
  reverting to raw `get_participant` or the local-user fallback.
- `app/src/terminal/shared_session/presence_manager.rs` — changed by #14426,
  #14430, and REMOTE-1782's work. This spec adds no live-presence behavior; it
  only consumes presence and adds an orthogonal durable-author fallback. Must
  not disturb `session_colors` / `get_participant_info_for_avatar` /
  `get_participant_attribution`.
- `app/src/ai/conversation_details_panel.rs` — #11844's unknown-creator path;
  the unknown-author avatar fallback here mirrors it for consistency.

Sequencing recommendation: land after (or rebase onto) #14426/#14430 so this
change extends the current avatar-resolution helper rather than colliding with
it.

## Key design choices
1. **Add a durable author to the exchange, resolve the avatar from it, keep
   presence for live.** Introduce a durable author identity (a `firebase_uid`,
   plus an optional snapshot profile) on `AIAgentExchange`; the render path tries
   live presence first (unchanged live behavior + color), then the durable author
   profile, then a neutral unknown — **never** the current viewer/owner.
2. **Resolve author profiles through the existing `UserProfileWithUID` /
   `UserProfiles` machinery** rather than inventing a new client profile type —
   `PublicUserProfile → UserProfileWithUID` and `UserProfiles::profile_for_uid`
   already exist and already back the details panel.
3. **Consume, don't define, the persistence/API/proto contract.** The client
   requires a per-message/per-exchange author `firebase_uid` (and ideally a
   snapshot `PublicUserProfile`) from the server; the exact storage/proto shape
   is the sibling repos' spec. This spec states the required shape precisely.

## == PRODUCT ==
### Summary
Every user query in a shared agent-mode session or ambient-agent conversation —
during the live session **and** when later viewed as a restored transcript or
shared link, on the native client and in the WASM/browser viewer — renders the
avatar (and name) of the participant who actually sent that query. When the
sender genuinely cannot be resolved, a neutral "unknown" avatar is shown. The
avatar never silently defaults to the conversation owner or the current viewer.

### Behavior (numbered, testable invariants)
1. **Live shared session (unchanged, non-regressed).** In a live shared session
   viewed by the sharer, each user query shows the initiating participant's
   avatar, display name, and per-sender color, resolved from live presence,
   exactly as today (REMOTE-196/1782/2361/2363 behavior preserved).
2. **Restored transcript / re-opened conversation — owner's own queries.** When
   a conversation is restored from persistence (no live presence), the owner's
   own queries show the owner's avatar and name (resolved from the durable
   author, not from "current user happens to be the owner").
3. **Restored transcript / re-opened conversation — other participants'
   queries.** Each query sent by a non-owner participant shows **that
   participant's** avatar and display name, resolved from the durable author
   persisted with the message — not the owner's and not the viewer's.
4. **Viewed by a different user / via a shared link.** When a user other than
   any original participant opens the conversation (or opens a shared link),
   every query still shows its **original** sender's avatar/name. The current
   viewer's identity is never substituted for any message it did not send.
5. **Unresolvable author → neutral unknown (never viewer/owner).** When no
   author can be resolved for a query (no live presence AND no usable durable
   author profile — e.g. a legacy message predating durable authorship, or a uid
   with no resolvable profile), the query renders a **neutral "unknown" avatar**
   (generic silhouette, no borrowed name), consistent with the details panel's
   "Unknown" treatment (PR #11844). It must **never** fall back to the current
   local user's or the owner's avatar. *(Assumed default c1 — see Open questions
   resolved; awaiting requester confirmation.)*
6. **Self-attribution.** A viewer's own queries in a shared session show the
   viewer's own avatar/name, treated as just another resolved author (no special
   self-exclusion). *(Assumed default c4.)*
7. **Name-on-hover.** The resolved sender's display name is available on the
   avatar in restored views via the same avatar affordance used in live sessions
   (the display name already flows into `query::maybe_render`). No new hover
   widget is required; parity with the live session's existing avatar
   name-display is the bar. *(Assumed default c2.)*
8. **Per-sender color on restore (best-effort).** In restored views, a
   per-sender color is applied when it can be derived deterministically from the
   durable author (e.g. from uid), so distinct senders remain visually
   distinguishable; when it cannot be derived it is omitted (avatar + name still
   correct). Live-session color continues to come from `PresenceManager`
   unchanged. *(Assumed default c2 — color is not required for correctness, only
   avatar + name are.)*
9. **WASM/browser parity.** All of the above (correct avatar, unknown fallback,
   name, best-effort color) hold in the WASM/browser transcript/shared-session
   viewer to the same standard as the native client. *(Assumed default c3.)*
10. **Graceful degradation for legacy conversations.** A conversation persisted
    before durable authorship existed (no per-message author) does not crash or
    mis-attribute: unresolved queries follow behavior 5 (neutral unknown); if the
    server chooses to backfill legacy messages to the conversation `creator`
    (a sibling-repo decision), the client renders whatever author the contract
    delivers. The client makes **no** owner/viewer assumption of its own.

## == TECH ==
### Context
See *Current state* for the pinned call sites. The change is concentrated in
three client seams: (a) the durable field on `AIAgentExchange`, (b) populating
it on the submit and restore paths, and (c) resolving it in the avatar render.

### Server / proto contract the client depends on (STATE EXPLICITLY for the sibling spec)
The client cannot fix restore-time attribution without durable author data. The
client requires, and this spec assumes the sibling repos provide:

- **`warp-proto-apis` — `apis/multi_agent/v1/task.proto`, `UserQuery`:** a
  durable author identity on each persisted user query. **Minimum required:** the
  author's Firebase `uid` (string). **Preferred:** also an optional snapshot of
  the author's `ProfileData`/`PublicUserProfile` (display_name, photo_url,
  email) captured at send-time, so avatars render correctly even when the
  viewer's `UserProfiles` cache has never seen that uid (cross-org shared links).
- **`warp-server` — `AIConversation` GraphQL:** the per-message author must be
  retrievable by the client on restore. Two acceptable shapes; **preferred (A)**
  because it matches the client's proto-message restore path:
  - **(A) Author on the persisted message.** The author uid (and optional
    snapshot profile) rides on each persisted `UserQuery` proto message returned
    in `AIConversation` tasks, so `create_exchange_from_messages` can read it
    directly while rebuilding the exchange.
  - **(B) Author map on the conversation.** `AIConversation` exposes a
    per-exchange/per-message `author: PublicUserProfile` (or a
    `participants: [PublicUserProfile]` set + a per-message uid), which the
    client joins by message/exchange id during restore.
- **Profile resolution:** whichever shape, the client resolves a
  `UserProfileWithUID` for the author. A snapshot `PublicUserProfile` (converts
  directly, `crates/cloud_object_models/src/user_profile.rs:25-34`) removes the
  dependency on the viewer's `UserProfiles` cache and is the safest choice for
  shared links; a bare uid works only when `UserProfiles::profile_for_uid`
  already knows that uid.
- **Backfill / legacy:** the client does not decide backfill; it renders the
  author the contract delivers and falls back to neutral-unknown (behavior 5)
  when none is present. Whether legacy messages backfill to `creator` is a
  sibling-repo product decision — the client must handle **both** "no author"
  and "author = creator".

**Reconciliation flag:** if the sibling server/proto spec cannot put the author
on the persisted message (shape A) and instead exposes only a conversation-level
map (shape B), the client restore change grows to thread that map into
`into_exchanges()` / `create_exchange_from_messages` (which today take only
`&api::Task`); note this so the two specs stay consistent.

### Proposed client changes
1. **Durable author on the exchange.** Add a durable author field to
   `AIAgentExchange` (`app/src/ai/agent/mod.rs:3159-3203`), e.g.
   `author: Option<ExchangeAuthor>` where `ExchangeAuthor` carries at least
   `uid: UserUid` and an optional snapshot `UserProfileWithUID`. Keep
   `response_initiator` as-is for live presence/color; the new field is the
   durable complement (name it distinctly to avoid conflation, e.g. `author`).
2. **Populate on submit (live).** When building `RequestInput` /
   `update_for_new_request_input`, capture the submitting user's durable uid:
   - Sharer's own query: the current authenticated user's uid
     (`AuthStateProvider … user_id()`, cf.
     `app/src/ai/agent_conversations_model/entry.rs:332-337`).
   - Viewer's query (`execute_agent_prompt_for_shared_session`): derive the
     durable uid from the participant's live presence `ProfileData.firebase_uid`
     (the participant is already known there via `ParticipantId`).
   Persist this uid to the server on the outbound request so it lands on the
   `UserQuery` proto (server-side; the client just supplies it).
3. **Rehydrate on restore.** In `create_exchange_from_messages`
   (`convert_conversation.rs:1881-2036`), replace the hardcoded author gap
   (`response_initiator: None` stays for presence) by reading the durable author
   from the persisted `UserQuery` message(s) in the exchange and setting the new
   `author` field. Add a model accessor mirroring
   `AIBlockModel::response_initiator` (e.g. `fn author(&self, app) ->
   Option<ExchangeAuthor>`, `app/src/ai/blocklist/block/model/model_impl.rs`
   / `model.rs`).
4. **Avatar resolution (native + WASM).** In `AIBlock::render`
   (`view_impl.rs:961-993`), change the resolution order to:
   1. **Live presence** via the current `get_participant_info_for_avatar` /
      attribution helper (keeps live avatar + color; do not regress #14426/#14430).
   2. **Durable author** — resolve the exchange `author` to a
      `UserProfileWithUID` (snapshot profile if present, else
      `UserProfiles::profile_for_uid(uid)`) → `(display_name, Some(photo_url),
      best-effort color-from-uid)`.
   3. **Neutral unknown** — a generic silhouette avatar with no name (mirror
      `conversation_details_panel` unknown treatment / PR #11844).
   Remove the `.unwrap_or((self.user_display_name, self.profile_image_path,
   None))` local-user fallback for shared/restored blocks. Keep the local-user
   identity **only** for genuinely local, non-shared, non-restored conversations
   (where the current user truly is the sole author).
5. **Unknown avatar affordance.** Add/verify a neutral "unknown participant"
   avatar rendering in the `query` avatar element consistent with the details
   panel's unknown-creator visual, so behavior 5 has a concrete render.
6. **WASM parity.** Because `AIBlock::render` and
   `ConversationDetailsData::from_conversation` are shared by native and WASM
   (`app/src/workspace/view/wasm_view.rs`,
   `app/src/terminal/view/shared_session/*`), verify the resolution runs
   identically under `target_family = "wasm"`; guard any presence-only calls so
   the durable-author path is reached in the browser where live presence is
   absent.

### Design alternatives
- **Durable author storage (chosen: new `author` field on `AIAgentExchange`
  populated from a persisted `UserQuery` author).**
  - *Alt A — overload `response_initiator` to also carry a durable uid.* Rejected:
    `ParticipantId` is a session-sharing concept resolved against live presence;
    conflating durable identity into it muddies the live vs. durable distinction
    and would fight REMOTE-2361/2363, which actively evolve the presence path.
  - *Alt B — resolve author purely from a conversation-level author map at
    render time (no exchange field).* Viable if the server can only expose shape
    (B); costs a per-render lookup and threads the map through many call sites.
    Chosen approach keeps a resolved value on the exchange and prefers shape (A).
- **Author profile source (chosen: snapshot `PublicUserProfile` when available,
  else `UserProfiles::profile_for_uid`).**
  - *Alt — bare uid only, always resolved via `UserProfiles`.* Rejected as the
    sole mechanism: `UserProfiles` is populated from the viewer's teammates/
    persistence and will miss authors on cross-org shared links, degrading to
    "unknown" even when the server knew the profile. Snapshot avoids this.
- **Unresolvable fallback (chosen: neutral unknown).** Alternatives (fall back to
  `creator`, or to viewer) rejected per the ticket's acceptance criteria — see
  Open questions resolved c1.

### Open questions resolved
These were posed to the requester as alignment questions; no answers had arrived
in this run's window, so each is resolved to the **most conservative** reading
and flagged for confirmation at spec approval (this PR is a draft; nothing ships
before approval). If the requester answers differently, revise before
implementation.
- **c1 (unresolvable author render):** Resolve to a **neutral unknown silhouette,
  never viewer/owner** — matches the ticket's explicit acceptance criterion and
  PR #11844's precedent. *Confirm: is "unknown" preferred over falling back to
  the conversation `creator` for truly-unresolvable messages?*
- **c2 (color + name on restore):** Resolve to **correct avatar + name-on-hover
  required; per-sender color best-effort** (derived from uid where possible,
  omitted otherwise). Color is not required for correctness. *Confirm whether
  restored transcripts must reproduce live per-sender colors exactly.*
- **c3 (WASM parity):** Resolve to **full native/WASM parity** for avatar +
  unknown fallback + name; color best-effort in both. *Confirm reduced browser
  affordances are not acceptable.*
- **c4 (self-attribution):** Resolve to **self shows self**, no special-casing.
- **Server contract shape (A vs B):** Assume **(A)** author-on-persisted-message
  (least client friction); implementer reconciles with the sibling server/proto
  spec and adopts (B) with the noted extra threading if (A) is infeasible.
- **REMOTE-1782 client landing:** PR #12585 is CLOSED at this SHA. The
  implementer MUST confirm which presence/avatar helper actually exists on the
  base branch (`get_participant` vs `get_participant_info_for_avatar` vs
  `get_participant_attribution`) after #14426/#14430/REMOTE-1782 resolve, and
  build the new fallback onto whatever is current — do not assume this SHA's
  `get_participant`.
- **Telemetry:** No per-sender attribution telemetry is added by the client
  unless the requester asks; noted as out of scope here (the triage record lists
  it as an open question owned at the product level).

## Reproduction (carry-forward)
A full live multi-user repro is environment-mismatched for an automated runner
(needs Warp desktop + a second participant + a live cloud shared session +
persistence), so reproduction is code-path-confirmed per triage:
`convert_conversation.rs:2034` (author dropped to `None` on restore) →
`view_impl.rs:989-993` (falls back to the local user). The
validation criteria below convert this into an automated regression plus
computer-use visual proof.

## Validation & verification criteria (must ALL pass before merge)
1. **Reproduction fixed (unit, restore path).** New unit test in
   `app/src/ai/agent/api/convert_conversation_tests.rs`: given a persisted task
   whose `UserQuery` messages carry two distinct author uids (per the assumed
   contract), `into_exchanges()` produces exchanges whose new `author` field
   equals the respective persisted uid — **not** `None`. Fails before the change
   (author is always `None`), passes after. *(Verifies behaviors 2–4, 10.)*
2. **Regression test — avatar resolves from durable author, not local user.**
   New unit/view test in `app/src/ai/blocklist/block/view_impl_*tests` (or the
   nearest existing avatar-resolution test): a restored exchange with a durable
   author whose uid differs from the current local user, with **no** live
   presence, resolves the query avatar to the author's profile
   (display_name/photo_url), and specifically **not** to
   `self.user_display_name`/`self.profile_image_path`. Fails before, passes after.
   *(Behaviors 2, 3, 4.)*
3. **Neutral-unknown fallback (unit).** A restored exchange whose author cannot
   be resolved (no snapshot profile and `UserProfiles::profile_for_uid` returns
   `None`) renders the neutral unknown avatar with no borrowed name, and never
   the local user/owner. *(Behavior 5.)*
4. **Self-attribution (unit).** A restored exchange authored by the current user
   resolves to the current user's profile via the author path (not via the
   removed blanket local-user fallback). *(Behavior 6.)*
5. **Live path non-regression (unit).** With live presence present, avatar +
   color still resolve from `PresenceManager` via the current
   `get_participant_info_for_avatar`/attribution helper; assert the durable-author
   path is only consulted when presence resolution yields nothing. Confirm no
   regression to REMOTE-2361 (`session_colors`) / REMOTE-2363 (browser self-vs-
   other) behavior — run their touched tests
   (`presence_manager_tests.rs`, the `view_impl` avatar tests). *(Behavior 1.)*
6. **WASM parity (unit / build).** The avatar resolution and
   `ConversationDetailsData::from_conversation` compile and behave identically
   under `target_family = "wasm"`; add or extend a test exercised in the WASM
   configuration (or assert the shared code path has no native-only presence
   dependency on the durable branch). Build the WASM target per repo docs.
   *(Behavior 9.)*
7. **Repository checks (scope-proportional gate, per `factory-verification`).**
   `./script/format` clean; `cargo clippy --workspace --all-targets
   --all-features --tests -- -D warnings` clean; `cargo nextest run` for the
   touched packages (the `app` crate AI/blocklist modules and
   `convert_conversation`) green, with the PR's CI as the full-suite backstop.
8. **Computer-use visual proof (user-facing — required, video by default).**
   Two flows captured with the computer use tool and attached to the task and PR:
   (a) a **live** shared session with 2+ distinct query authors showing the
   correct per-sender avatars; (b) the **same conversation re-opened as a
   restored transcript / shared link** (ideally by a different viewer) showing
   each query still attributed to its original sender, and an unresolvable author
   rendering the neutral unknown avatar (never the viewer's). *(Behaviors
   1–5, 9.)*
9. **No mis-attribution in a pure-local conversation (regression).** A normal
   local (non-shared, non-restored) single-user conversation still shows the
   current user's avatar on their queries — confirming the narrowed local-user
   fallback did not remove correct behavior for the common case.
10. **Cross-repo integration gate (documented, human-verified).** Because the
    fix spans repos, before merge confirm the client is built against a server +
    proto that provide the agreed author contract (shape A or B). If the server
    contract is not yet available, the client PR stays behind it — noted as a
    merge-order dependency, not a silent assumption.

## Risks / blast radius
- **Shared render path churn.** `view_impl.rs` avatar resolution and
  `presence_manager.rs` are actively changed by #14426/#14430/REMOTE-1782;
  landing order and rebasing matter (see prior-work sweep). Mitigation: extend
  the current helper, add the live-path non-regression test (criterion 5),
  sequence after those PRs.
- **Cross-repo coupling.** Client is inert/incorrect without the server+proto
  contract. Mitigation: explicit contract section + merge-order gate
  (criterion 10); reconciliation flag for shape A vs B.
- **Privacy of exposed identity on public/anonymous shared links.** Owned by the
  sibling server/product spec (what identity, if any, is exposed for anonymous
  participants / public links). The client renders only what the API returns and
  degrades to neutral-unknown; it introduces no new client-side identity
  exposure beyond rendering server-provided authors.
- **Legacy conversations.** Handled by behavior 5/10 (neutral unknown or
  server-chosen `creator` backfill); no crash/mis-attribution.
