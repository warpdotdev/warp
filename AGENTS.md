## 沟通与命令

- 优先用中文交流；代码、命令、路径、API 名和英文专有名词保持原文。
- 执行 shell 命令统一加 `rtk` 前缀，例如 `rtk git status --short --branch`。
- 除非用户明确要求，不使用 git worktree。
- 涉及文件变更时遵循本仓库 `.gitignore`，不要提交被忽略文件或工具副作用文件。

## 本分支目标

当前分支目标是把 Warp 改造成只保留第三方 agent CLI 支持的客户端。

保留范围：

- 普通 terminal/session/workspace 能力。
- 第三方 CLI agent 识别与展示，例如 `codex`、`claude`/Claude Code、`gemini`、`opencode`。
- 第三方 CLI agent 的 tab/icon/status 展示。运行中状态必须独立于 tab 名称保存和渲染，用户重命名 tab 后仍要显示运行状态。
- MCP servers 和第三方 CLI agent 相关设置页。

清理范围：

- Warp 自带 AI assistant、Ask Warp AI、AI command search、AI 搜索/推荐入口。
- Warp 自带 agent harness/Oz harness，不能作为默认、本地可选项或 UI fallback。
- cloud agent、Oz/cloud 编排平台、agent management、run/task/schedule/handoff/computer use 等 Warp 内建 AI 工具入口。
- bundled Oz platform skill、Oz launch/changelog/onboarding、cloud handoff 和 computer-use 用户入口。

兼容边界：

- 服务端 schema 或历史数据中的 `AIAgentHarness::Oz` 可以作为只读兼容值保留，但不能重新映射成本地可选 harness。
- 对缺失、旧版或未知 harness，使用 `Unknown`/普通 terminal fallback，不回退到 Warp/Oz。

## 从官方上游 master 合并最新内容

本仓库 remote 约定：

- `upstream`: 官方 `git@github.com:warpdotdev/Warp.git`
- `origin`: 当前 fork

合并步骤：

```bash
rtk git status --short --branch
rtk git remote -v
rtk git fetch upstream master
rtk git merge --no-ff upstream/master
```

如有冲突，按本分支目标处理：

- 上游新增 Warp AI/Oz/cloud agent/computer use/handoff 入口时，保留上游非 AI 基础改动，但不要恢复这些入口。
- 上游改动第三方 CLI agent 支持时，优先保留并适配到本分支的第三方 CLI-only 目标。
- `Harness::Oz`、`oz-platform`、`OpenWarpAI`、`WarpAIDataSource`、`OpenOzLaunchModal` 等符号不要重新引入为可运行入口。
- tab title/name 与 agent 运行态必须分离；冲突中不要把运行状态拼回 tab name。

合并后至少执行：

```bash
rtk cargo fmt --all --check
rtk cargo test -p warp_cli harness_
rtk cargo test -p warp summary_title_override
rtk rg -n "Ask Warp AI|AI command search|WarpAIDataSource|OpenWarpAI|TranslateUsingWarpAI|toggle_ai_assistant|toggle_natural_language_command_search" app/src crates/onboarding
rtk rg -n "Harness::Oz|oz-platform|OpenOzLaunchModal|ResetOzLaunchModalState|oz_changelog|oz_launch" app/src crates resources
```

提交或创建 MR 前再跑与改动范围匹配的 Rust 测试，并执行 `.gitignore` 闸门：

```bash
rtk git status --short
rtk git ls-files -ci --exclude-standard
```
