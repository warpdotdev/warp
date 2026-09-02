<a href="https://www.warp.dev">
    <img width="1024" alt="Warp Agentic Development Environment product preview" src="https://github.com/user-attachments/assets/9976b2da-2edd-4604-a36c-8fd53719c6d4" />
</a>
&nbsp;
<p align="center">
  <a href="https://www.warp.dev"><img height="20" alt="Built with Warp" src="https://raw.githubusercontent.com/warpdotdev/brand-assets/main/Github/Built-With-Warp-Export@2x.png" /></a>
  &nbsp;
  <a href="https://oz.warp.dev"><img height="20" alt="Powered by Oz" src="https://raw.githubusercontent.com/warpdotdev/brand-assets/main/Github/Powered-By-Oz-Export@2x.png" /></a>
</p>

<p align="center">
  <a href="https://www.warp.dev">Website</a>
  ·
  <a href="https://www.warp.dev/code">Code</a>
  ·
  <a href="https://www.warp.dev/agents">Agents</a>
  ·
  <a href="https://www.warp.dev/terminal">Terminal</a>
  ·
  <a href="https://www.warp.dev/drive">Drive</a>
  ·
  <a href="https://docs.warp.dev">Docs</a>
  ·
  <a href="https://www.warp.dev/blog/how-warp-works">How Warp Works</a>
</p>

> [!NOTE]
> OpenAI be the founding sponsor o' this here new, open-source Warp treasure chest, and the new agentic management workflows be powered by GPT models. Aye!

<h1></h1>

## About

[Warp](https://www.warp.dev) be an agentic development environment, born o' the terminal like a kraken from the deep. Use Warp's built-in coding agent, or bring yer own CLI agent (Claude Code, Codex, Gemini CLI, and other scurvy companions).

## Installation

Ye can [download Warp](https://www.warp.dev/download) and [read our docs](https://docs.warp.dev/) for platform-specific instructions, ye landlubber.

## Warp Contributions Overview Dashboard

Set sail for [build.warp.dev](https://build.warp.dev) to:
- Watch thousands o' Oz agents triage issues, write specs, implement changes, and review PRs — a proper crew at work
- Spy the top contributors and in-flight features
- Track yer own issues with GitHub sign-in
- Click into active agent sessions in a web-compiled Warp terminal

## Oz for OSS

Keepin' a popular open-source project afloat? [Apply for Oz credits](https://tally.so/r/LZWxqG) to explore [Oz for OSS](https://github.com/warpdotdev/oz-for-oss).

Oz for OSS be our partner program for bringin' the same agentic open-source management workflows used in this here repository to select partner ships. We work directly with maintainers to implement workflows for issue triage, PR review, community management, and contributor coordination in a way that fits each voyage.

## Licensing

Warp's UI framework (the `warpui_core` and `warpui` crates) be licensed under the [MIT license](LICENSE-MIT).

The rest o' the code in this treasure chest be licensed under the [AGPL v3](LICENSE-AGPL).

## Open Source & Contributing

Warp's client codebase be open source and lives in this here repository. We welcome community contributions and have designed a lightweight workflow to help new hands get started. For the full contribution flow, read our [CONTRIBUTING.md](CONTRIBUTING.md) guide, ye scurvy dog.

> [!TIP]
> **Chat with contributors and the Warp crew** in the [`#oss-contributors`](https://warpcommunity.slack.com/archives/C0B0LM8N4DB) Slack channel — a fine tavern for ad-hoc questions, design discussion, and pairing with maintainers. New aboard? [Join the Warp Slack community](https://go.warp.dev/join-preview) first, then jump into `#oss-contributors`.

### Issue to PR

Before filin', [search existing issues](https://github.com/warpdotdev/warp/issues?q=is%3Aissue+is%3Aopen+sort%3Areactions-%2B1-desc) for yer bug or feature request. If nothin' exists, [file an issue](https://github.com/warpdotdev/warp/issues/new/choose) usin' our templates. Security vulnerabilities should be reported privately as described in [CONTRIBUTING.md](CONTRIBUTING.md#reporting-security-issues) — no shoutin' secrets across the deck.

Once filed, a Warp maintainer reviews the issue and may apply a readiness label: [`ready-to-spec`](https://github.com/warpdotdev/warp/issues?q=is%3Aissue+is%3Aopen+label%3Aready-to-spec) signals the design be open for contributors to spec out, and [`ready-to-implement`](https://github.com/warpdotdev/warp/issues?q=is%3Aissue+is%3Aopen+label%3Aready-to-implement) signals the design be settled and code PRs be welcome. Anyone can pick up a labeled issue — mention **@oss-maintainers** on an issue if ye'd like it considered for a readiness label.

### Building the Repo Locally

To build and run Warp from source, weigh anchor:

```bash
./script/bootstrap   # platform-specific setup
./script/run         # build and run Warp
./script/presubmit   # fmt, clippy, and tests
```

See [AGENTS.md](AGENTS.md) for the full engineering guide, includin' coding style, testing, and platform-specific notes.

## Joining the Team

Fancy joinin' the crew? See our [open roles](https://www.warp.dev/careers).

## Support and Questions

1. See our [docs](https://docs.warp.dev/) for a comprehensive guide to Warp's features, ye curious sailor.
2. Join our [Slack Community](https://go.warp.dev/join-preview) to connect with other users and get help from the Warp crew — contributors hang out in [`#oss-contributors`](https://warpcommunity.slack.com/archives/C0B0LM8N4DB).
3. Try our [Preview build](https://www.warp.dev/download-preview) to test the latest experimental features.
4. Mention **@oss-maintainers** on any issue to escalate to the crew — for example, if ye encounter problems with the automated agents.

## Code of Conduct

We ask everyone to be respectful and empathetic. No keelhaulin' o' crewmates. Warp follows the [Code of Conduct](CODE_OF_CONDUCT.md). To report violations, email warp-coc at warp.dev.

## Open Source Dependencies

We'd like to call out a few o' the [open source dependencies](https://docs.warp.dev/help/licenses) that have helped Warp get off the ground and sail the seven seas:

- [Tokio](https://github.com/tokio-rs/tokio)
- [NuShell](https://github.com/nushell/nushell)
- [Fig Completion Specs](https://github.com/withfig/autocomplete)
- [Warp Server Framework](https://github.com/seanmonstar/warp)
- [Alacritty](https://github.com/alacritty/alacritty)
- [Hyper HTTP library](https://github.com/hyperium/hyper)
- [FontKit](https://github.com/servo/font-kit)
- [Core-foundation](https://github.com/servo/core-foundation-rs)
- [Smol](https://github.com/smol-rs/smol)
