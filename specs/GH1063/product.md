# PRODUCT.md — Rename Oz to Warp Agent in settings and onboarding

Issue: https://github.com/warpdotdev/warp-external/issues/1063

## Summary

The in-app agent is being renamed from "Oz" to "Warp Agent" in the settings page
and the onboarding agent slide. "Oz" is reserved for the cloud agent
orchestration platform and must not be used for the in-app agent's user-facing
strings in these surfaces. This change updates only user-visible copy; the cloud
agent orchestration product continues to be surfaced as "Oz" wherever it
currently is.

Figma: none provided.

## Goals / Non-goals

In-scope surfaces (user-facing strings change):

- The settings sidebar entry for the in-app agent under the "Agents" umbrella.
- The primary heading on the AI settings page that labels the global AI enable
  toggle.
- The onboarding agent slide header and the "disable" checkbox label on that
  slide.

Out of scope (must continue to say "Oz"):

- Anything that refers to the cloud agent orchestration platform, including the
  "Oz Cloud API Keys" settings subpage, the "Oz" harness in the harness
  selector, and any zero-state or blocklist strings that mention the cloud Oz
  agent.
- Internal identifiers (enum variants, field names, action names, telemetry
  keys, settings keys, URL fragments) may keep the name `Oz`. This spec
  constrains user-visible strings only.
- Other surfaces that mention "Oz" (agent view zero state, tab titles,
  documentation, etc.) are not covered by this issue and are not changed here.

## Behavior

1. On the settings sidebar, under the "Agents" umbrella, the first subpage entry
   is labeled "Warp Agent" (previously "Oz"). The ordering of subpages, the
   umbrella name ("Agents"), and all sibling subpages ("Profiles", "MCP
   servers", "Knowledge", "Third party CLI agents") are unchanged.

2. Opening that "Warp Agent" subpage renders the same settings content it did
   before this rename. No widgets, toggles, options, defaults, or telemetry
   change behavior as a result of this rename.

3. On the "Warp Agent" subpage, the primary page heading that sits above the
   global AI enable toggle reads "Warp Agent" (previously "Oz"). Typography,
   color, size, weight, and layout of the heading are unchanged; only the text
   differs.

4. The global AI enable toggle itself (its on/off state, its behavior when
   toggled, and the remote-session organization-policy warning shown next to it)
   is unchanged. The Sign-up CTA shown to anonymous users on this page is
   unchanged.

5. On the onboarding agent slide (the third onboarding step), the title reads
   "Customize your Warp Agent" (previously "Customize your Agent, Oz"). The
   subtitle "Select your in-app agent's defaults." is unchanged.

6. When the "new settings modes" feature flag is enabled and the slide renders
   the disable checkbox row, the checkbox label reads "Disable Warp Agent"
   (previously "Disable Oz"). Checkbox visual state, hit target, and click
   behavior are unchanged — toggling it still enables/disables the in-app agent
   for onboarding and still dims the upper sections of the slide while checked.

7. Search within settings still finds the in-app agent subpage when the user
   types any previously-matching search term (for example, terms already covered
   by the AI page's global widget search terms such as "ai", "agent", "next
   command", "api keys"). Additionally, typing "warp agent" finds this subpage.
   Typing "oz" is acceptable to continue matching this subpage so existing
   muscle memory is not broken, but is not required by this spec; search for
   "Oz Cloud API Keys" must continue to find the cloud platform subpage
   regardless.

8. Deep links and external callers that previously navigated to this subpage by
   section identifier continue to resolve to the same subpage. In particular,
   navigation requests that reference the legacy "Oz" section name still land on
   the renamed "Warp Agent" subpage. Navigation requests referencing "Warp
   Agent" also resolve to the same subpage.

9. The "Oz Cloud API Keys" subpage under the "Cloud platform" umbrella is
   unchanged — same label, same location, same behavior. No cloud-agent surface
   is renamed by this change.

10. Onboarding progress (step index, total step count, back/next navigation, the
    "disable agent" effect on subsequent onboarding state) is unchanged. Nothing
    about the Autonomy section, Default model section, upgrade banner, or
    plan-activated toast changes.

11. Accessibility: the settings sidebar item, the page heading, and the
    onboarding checkbox label expose the new visible text ("Warp Agent",
    "Disable Warp Agent") to assistive technologies. No separate aria-only label
    still announces "Oz" for the renamed surfaces.

12. The rename applies consistently across themes, appearances, and the new
    settings modes feature flag on/off. The only user-visible difference between
    flag states remains whether the "Disable Warp Agent" row renders at all; its
    label text does not depend on the flag.
