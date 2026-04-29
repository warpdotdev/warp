## Description
<!-- Please remember to add your design buddy onto the PR for review, if it contains any UI changes! -->

## Testing
<!--
How did you test this change?  What automated tests did you add?  If you didn't add any new tests, what's your justification for not adding any?

If you're not sure whether you should add a test, check our testing policy: https://www.notion.so/warpdev/How-We-Code-at-Warp-257fe43d556e4b3c8dfd42f70004cc72#1f97825450504baa9c5fd87a737daa09
-->

## Server API dependencies
<!-- You may remove this section if your PR does not have any server dependencies. -->
- [ ] Is this change necessary to make the client compatible with a desired [server API breaking change](https://www.notion.so/warpdev/How-to-safely-introduce-server-API-breaking-changes-0aa805ff5d5d41fd8834f3c95caba0b4?pvs=4#d55ecf8aea3449949d3c33b0e67f6800)?
- [ ] Does this change rely on a [new server API](https://www.notion.so/warpdev/How-to-add-a-new-full-stack-feature-8412cede405a4ec194b32bdd4b951035?pvs=4#04da1e6a493542d68b3e998c7d339640)?
  - [ ] If so, is the use of this API restricted to client channels that rely on the staging server (e.g. WarpDev)?
- [ ] Is this change enabling the use of a server API on client channels that rely on the production server (e.g. WarpStable)?
  - [ ] If so, has the new server API been stable on production for at least one server release cycle? See [here](https://www.notion.so/warpdev/How-to-add-a-new-full-stack-feature-8412cede405a4ec194b32bdd4b951035?pvs=4#73b202f939834b97ab1fbdf7fc82cd53) for more details.

## Agent Mode
- [ ] Warp Agent Mode - This PR was created via Warp's AI Agent Mode

## Changelog Entries for Stable
<!--
The entries below will be used when constructing a soft-copy of the stable release changelog. Leave blank or remove the lines if no entry in the stable changelog is needed. Entries should be on the same line, without the `{{` `}}` brackets. You can use multiple lines, even of the same type. The valid suffixes are:

* NEW-FEATURE: for new, relatively sizable features. Features listed here will likely have docs / social media posts / marketing launches associated with them, so use sparingly.
* IMPROVEMENT: for new functionality of existing features.
* BUG-FIX: for fixes related to known bugs or regressions.
* IMAGE: the image specified by the URL (hosted on GCP) will be added to Dev & Preview releases. For Stable releases, see the pinned doc in the #release Slack channel.
* OZ: Oz-related updates. Use `CHANGELOG-OZ`. At most 4 Oz updates are shown in-app per release.
-->

CHANGELOG-NEW-FEATURE: {{text goes here...}}
CHANGELOG-IMPROVEMENT: {{text goes here...}}
CHANGELOG-BUG-FIX: {{text goes here...}}
CHANGELOG-BUG-FIX: {{more text goes here...}}
CHANGELOG-IMAGE: {{GCP-hosted URL goes here...}}
CHANGELOG-OZ: {{text goes here...}}
