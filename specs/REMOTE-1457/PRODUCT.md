# PRODUCT.md
## Summary
Add a Harness filter dropdown to `AgentManagementView` so users can narrow the Runs list to runs driven by a specific execution harness (Warp Agent, Claude Code, Gemini CLI), and surface the harness on each run card's metadata line.
## Figma
Figma: none provided. Visual treatment matches the existing filter dropdowns in `app/src/ai/agent_management/view.rs` (Status, Source, Has artifact, etc.) and the harness icon + brand-color treatment established by the conversation details sidebar in `specs/REMOTE-1455/PRODUCT.md`.
## Behavior
1. A Harness dropdown is present in the filters row of the Runs header on `AgentManagementView`, alongside the existing Status, Source, Created on, Has artifact, Environment, and Created by dropdowns. It is visible under the same conditions as the existing dropdowns (no extra feature-flag gating beyond whatever already gates the view).
2. The dropdown label prefix is `Harness`, so the button reads `Harness: <selected>` after a selection, matching the `Status: …` / `Source: …` treatment from the adjacent dropdowns.
3. The dropdown exposes exactly these options, in this order:
    1. `All` — no harness filtering.
    2. `Warp Agent` — the default/Oz harness.
    3. `Claude Code` — the Claude harness.
    4. `Gemini CLI` — the Gemini harness.
   Display names match `app/src/ai/harness_display.rs::display_name`. If a new harness is added to `warp_cli::agent::Harness` later, it is expected to appear here too, but adding it is out of scope for this feature.
4. Each non-`All` option renders its harness's leading logo icon, tinted with that harness's brand color, matching the treatment used in the conversation details sidebar (REMOTE-1455): Warp/Oz uses the first-party Warp icon + theme foreground, Claude uses `Icon::ClaudeLogo` tinted Claude orange, Gemini uses `Icon::GeminiLogo` tinted Gemini blue. The `All` option has no leading icon, consistent with the existing Status dropdown's `All` row.
5. The selected harness appears in the dropdown's collapsed button label, e.g. `Harness: Claude Code`. The default selection is `All`.
6. Selecting a harness option filters the visible Runs list to items whose resolved harness equals the selected value:
    * A cloud task whose `agent_config_snapshot.harness.harness_type` is set matches the option for that harness.
    * A cloud task whose `agent_config_snapshot` is present but has no `harness` field is treated as `Warp Agent` and matches only the `Warp Agent` option.
    * A cloud task whose `agent_config_snapshot` has not loaded yet (e.g. a stub row) has an unknown harness; it matches only the `All` option and is excluded from every specific-harness filter, including `Warp Agent`. This mirrors the conversation details sidebar behavior, which omits the harness row when the harness is unknown.
    * A local/interactive conversation (no ambient task) is treated as `Warp Agent` and matches only the `Warp Agent` option.
    * No item ever matches more than one specific-harness option.
7. Selecting `All` clears the harness constraint and restores the list to what it would be with no harness filter applied (all other filters still respected).
8. The Harness filter is independent of every other filter. It combines with the existing Owner, Status, Source, Created on, Has artifact, Environment, Creator, and search filters via logical AND — an item appears only if it matches all active filters.
9. When the Harness filter is set to anything other than `All`, `AgentManagementFilters::is_filtering()` reports `true`, so the existing `Clear all` chip appears and, when clicked, resets Harness to `All` along with the other non-owner filters.
10. `Clear filters` from the empty-results view (`render_no_results_view`) also resets Harness to `All`.
11. The selected Harness value persists across app restarts via `PersistedAgentManagementFilters`, matching how Status, Source, Created on, Artifact, and Environment already persist. On first launch after upgrade (no persisted value), the filter defaults to `All`.
12. Changing the Harness filter triggers the same server refresh path that other filter changes do — i.e. the view calls its common "filter changed" handler so the server list fetch is retried with the new filter set and the local list re-renders. If the server does not yet support filtering by harness, the visible list must still honor the selected harness by filtering client-side over tasks already loaded in the model; users must not see runs from harnesses other than the one selected.
13. The Harness filter never changes which tasks are loaded into the underlying model for other views (details panel, transcript panel, deep-link navigation). It only affects which of those already-loaded items are displayed in the Runs list.
14. If the Runs list ends up empty solely because the Harness filter excluded every loaded item, the existing no-results state (`No results matched your filters` + `Clear filters` button) is shown, identical to what other filters already produce.
15. The Owner toggle (`Personal` vs `All`) does not affect Harness semantics: Harness filters the same way regardless of whether the user is viewing personal-only or team-wide runs.
16. The deep-link flow that scopes the view to a specific environment (`apply_environment_filter_from_link`) resets Harness to `All` along with the other non-owner filters, so a deep link never leaves a stale harness constraint in place.
17. Telemetry: changing the Harness dropdown emits the existing `FilterChanged` telemetry event with a new `FilterType::Harness` variant, consistent with how Status, Source, CreatedOn, Owner, and Creator changes are already tracked.
18. Keyboard and focus behavior for the Harness dropdown matches the other filter dropdowns in the same row — it is reachable via Tab navigation from adjacent dropdowns and obeys the same open/close keybindings the existing filter dropdowns already honor. No new global keybinding is introduced for Harness.
19. Every run card in the list renders a `Harness: <display name>` segment on its metadata line, immediately after the `Source: …` segment and before `Run time: …` / `Credits used: …`, using the existing ` • ` separator between segments. Example: `Source: Oz Web • Harness: Claude Code • Run time: 2 minutes`.
20. The `<display name>` on each card uses the same resolution as the Harness filter (invariant 6): explicit, known `harness_type` → that harness's display name; snapshot-present-but-no-harness → `Warp Agent`; local/interactive conversations → `Warp Agent`. Display names match `app/src/ai/harness_display.rs::display_name` (`Warp Agent`, `Claude Code`, `Gemini CLI`). When a card's harness is unknown (per invariant 6, third bullet), the Harness segment is omitted entirely — the card does not guess `Warp Agent`.
21. The Harness segment is included on every card whose harness is known (per invariant 20). It is not gated on any filter, the Source segment's presence, or card hover state. If the Source segment is omitted (e.g. a local conversation with no `AgentSource`), a present Harness segment becomes the row's leading segment.
22. The Harness segment on cards is text-only — no leading icon, no brand color, no click target, no tooltip. It uses the same text color and font as the rest of the metadata row.
23. Selecting a specific harness in the Harness filter hides cards whose resolved harness does not match (including cards with an unknown harness); cards that remain visible continue to show their own Harness segment, which will match the selected filter value.
