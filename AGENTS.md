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
- 账号体系、Warp Drive、登录/同步/团队协作等依赖 Warp 云端账号能力的用户入口。

兼容边界：

- 服务端 schema 或历史数据中的 `AIAgentHarness::Oz` 可以作为只读兼容值保留，但不能重新映射成本地可选 harness。
- 对缺失、旧版或未知 harness，使用 `Unknown`/普通 terminal fallback，不回退到 Warp/Oz。
- 上游文档或代码若新增 Warp-native/Oz/cloud agent 入口，合并时必须按本目标改写或删除。

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
rtk cargo check -p warp --bin zerp
rtk cargo test -p warp_cli harness_
rtk cargo test -p warp summary_title_override
rtk rg -n "Ask Warp AI|AI command search|WarpAIDataSource|OpenWarpAI|TranslateUsingWarpAI|toggle_ai_assistant|toggle_natural_language_command_search" app/src crates/onboarding
rtk rg -n "Harness::Oz|oz-platform|OpenOzLaunchModal|ResetOzLaunchModalState|oz_changelog|oz_launch" app/src crates resources
```

提交或创建 MR 前再跑与改动范围匹配的 Rust 测试，并执行 `.gitignore` 闸门：

```bash
rtk git status --short
rtk git diff --check
rtk git ls-files -ci --exclude-standard
```

## 开发命令

### 构建与运行

- `rtk cargo run`：本地构建并运行。
- `rtk cargo check -p warp --bin zerp`：检查当前 app binary。
- `rtk cargo bundle --bin warp`：打包主 app。

### 本地 server

连接本地 `warp-server`：

```bash
rtk cargo run --features with_local_server
rtk env SERVER_ROOT_URL=http://localhost:8082 WS_SERVER_URL=ws://localhost:8082/graphql/v2 cargo run --features with_local_server
```

环境变量：

- `SERVER_ROOT_URL`：HTTP endpoint，默认 `http://localhost:8080`。
- `WS_SERVER_URL`：WebSocket endpoint，默认 `ws://localhost:8080/graphql/v2`。

### 测试、格式化、lint

- `rtk cargo fmt --all --check`：检查 Rust 格式。
- `rtk cargo fmt --all`：格式化 Rust。
- `rtk cargo test`：运行标准测试。
- `rtk cargo nextest run --no-fail-fast --workspace --exclude command-signatures-v2`：运行 workspace nextest。
- `rtk cargo test --doc`：运行 doc tests。
- `rtk cargo clippy --workspace --all-targets --all-features --tests -- -D warnings`：运行 clippy。
- `rtk ./script/presubmit`：运行提交前检查。
- `rtk ./script/run-clang-format.py -r --extensions 'c,h,cpp,m' ./crates/warpui/src/ ./app/src/`：格式化 C/C++/Obj-C。
- `rtk find . -name "*.wgsl" -exec wgslfmt --check {} +`：检查 WGSL shader 格式。

### Bootstrap 与 common skills

- `rtk ./script/bootstrap`：平台 setup，并按 `skills-lock.json` 安装/更新 common agent skills。
- `rtk ./script/bootstrap --skip-common-skills`：跳过 common skills。
- `rtk ./script/bootstrap --install-common-skills-in-repo`：安装到本 checkout 的 `.agents/skills`。
- `rtk ./script/bootstrap --install-common-skills-globally`：安装到 `~/.agents/skills`。
- `rtk ../common-skills/scripts/install_common_skills --repo-root "$PWD" --project --if-needed`：项目级安装/刷新。
- `rtk ../common-skills/scripts/install_common_skills --repo-root "$PWD" --global --if-needed`：全局安装/刷新。

`skills-lock.json` 是 `npx skills` 管理的项目锁文件。非交互流程必须显式选择安装目标：传 `--project`、`--global`，或设置 `WARP_COMMON_SKILLS_INSTALL_TARGET`。如需测试远端 common-skills 分支，设置 `WARP_COMMON_SKILLS_REF=<branch>`。

## 架构与代码风格

- 这是 Rust Cargo workspace，主 app 在 `app/`，UI framework 在 `crates/warpui/` 与 `crates/warpui_core/`。
- WarpUI 使用 entity/handle 模式；view 通过 handle 引用其他 view/model，不直接持有所有权。
- `MouseStateHandle` 必须在构造时创建一次，render 中引用或 clone；不要在 render 中 inline `MouseStateHandle::default()`。
- 避免不必要的类型标注，尤其是 closure 参数。
- 优先使用 imports，避免过长 Rust path qualifier。cfg 分支内的一次性路径可例外。
- 带 `AppContext`、`ViewContext`、`ModelContext` 的函数，参数命名为 `ctx` 且放最后；若函数接收 closure，则 closure 放最后。
- unused 参数应完整移除并更新 call sites，不要仅加 `_` 前缀掩盖。
- 使用 inline format args，例如 `eprintln!("{message}")`。
- 不要把 `Itertools::format` 结果直接传给 logging macros；需要可复用字符串时用 `iter.join(", ")`。
- 不要删除无关注释；只有逻辑改变时才更新对应注释。
- 新增 toggleable setting 时，同步添加 Command Palette enable/disable 入口和需要的 context flags。
- match 尽量穷尽具体 variants，避免不必要的 `_` wildcard。

## Terminal Model Locking

- 调用 `TerminalModel::lock()` 前必须确认当前 call stack 没有已持有同一 model lock。
- 优先把已 lock 的 model reference 向下传递，而不是在下游重复 lock。
- 必须 lock 时，scope 保持尽可能短，且不要调用可能再次 lock 的函数。

## 测试规则

- Unit test 文件使用 `${filename}_tests.rs` 或 `mod_test.rs` 命名。
- 对应 module 底部添加：

```rust
#[cfg(test)]
#[path = "filename_tests.rs"]
mod tests;
```

- bug fix 应包含能捕获该问题的 regression test。
- 非平凡逻辑需要 unit tests。
- 用户可见流程如果可自动化，应优先考虑 `crates/integration/` 覆盖。

## PR 工作流

- push 或更新 PR 前，至少运行与改动范围匹配的 format/check/test；正式提交前按需要运行 `rtk ./script/presubmit`。
- PR 使用 `.github/pull_request_template.md`。
- 需要 changelog 时使用模板中的 `CHANGELOG-NEW-FEATURE:`、`CHANGELOG-IMPROVEMENT:`、`CHANGELOG-BUG-FIX:`、`CHANGELOG-IMAGE:`。
- 不要公开提交或公开 issue 披露非公开安全漏洞；参见 `SECURITY.md`。
