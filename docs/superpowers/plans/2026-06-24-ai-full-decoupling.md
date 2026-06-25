# Third-party CLI Agent Only Replay Guide

**目标:** 从干净 `master` 重新执行，或在合并最新 `master` 后重放本分支目标：只保留 Codex、Claude Code、Gemini CLI、OpenCode 等第三方 CLI agent 的本地使用支持；清理 Warp 自带 AI assistant、AI command search、Oz/cloud agent 编排、agent management、computer use、cloud handoff 等内建 AI 工具入口和运行路径。

本指南是语义重放文档，不要求逐行套用旧 patch。若 `master` 已演进，优先保持用户可见行为和编译结果。

## 边界

| 分类 | 处理 |
| --- | --- |
| 第三方 CLI agent | 保留。包括 CLI agent session、Codex/Claude/Gemini/OpenCode tab config、第三方 CLI agent 设置页、必要的 MCP server 设置。 |
| Warp 内建 AI assistant | 清理用户入口。包括 Ask Warp AI、AI assistant panel 打开路径、右键菜单、block toolbelt Ask/Attach 按钮、workspace 顶栏入口。 |
| AI command search | 清理。包括 natural-language command search keybinding、command palette/resource center/tip/voltron 推荐入口、universal search 的 Open/Translate Warp AI action。 |
| Oz / cloud agent 编排平台 | 清理。包括 bundled Oz platform skill、Oz launch/changelog/onboarding、cloud agent management、run/task/schedule/handoff/computer-use 入口。 |
| 历史 schema / telemetry enum | 谨慎保留。`warp_ai_width`、workspace billing policy、旧 telemetry enum 等如果只是兼容旧数据且不可触发 UI，可暂留，避免迁移风险扩大。 |
| 品牌/CLI 命名 | 当前分支已将 CLI/channel/bundle/remote installer 迁移到 `zerp` 命名；重放时按目标产品名统一处理。 |

## 推荐执行顺序

```
clean master
  |
  v
锁定第三方 CLI agent 保留面
  |
  v
清理 Warp 内建 AI 用户入口
  |-- slash/static commands
  |-- settings/nav/onboarding
  |-- new tab/session configs
  |-- terminal input/context menu/block toolbelt
  |-- command palette/resource center/tips
  |
  v
清理 Oz/cloud/computer-use 运行面
  |-- bundled skills
  |-- launch/changelog/handoff/modals
  |-- local/cloud child orchestration
  |
  v
统一 CLI/品牌命名
  |
  v
格式化、编译、目标搜索、回归测试
```

## 重放步骤

### 1. 记录基线

```bash
rtk git status --short --branch
rtk git rev-parse HEAD
rtk git diff --name-status
rtk git diff --shortstat
rtk git ls-files -o --exclude-standard
```

如需参考旧工作区：

```bash
rtk git diff --binary > /tmp/ai-full-decoupling-current.patch
rtk git diff --name-status > /tmp/ai-full-decoupling-name-status.txt
```

### 2. 先加/保留行为测试

优先用测试锁住这些行为：

| 行为 | 期望 |
| --- | --- |
| slash commands | Warp 内建 agent commands 不注册；普通 terminal/workflow commands 仍可用。 |
| settings nav | 只展示 MCP servers、Third-party CLI agents 等保留页；旧 AI/Oz/Profile/Knowledge 页不再出现。 |
| new tab configs | 默认 Terminal；第三方 CLI agents 可见；内建 Agent/Cloud Agent 不可见。 |
| onboarding | 不引导 Warp AI/Oz；默认 terminal；第三方 CLI 作为唯一 AI setup 方向。 |
| resource center / command palette | 不推荐 AI command search 或 Ask Warp AI。 |

### 3. 清理入口层

从用户可见入口开始清理，避免后面保留不可达兼容代码时误判：

```bash
rtk rg -n "Ask Warp AI|AI command search|toggle_ai_assistant|toggle_natural_language_command_search|OpenWarpAI|TranslateUsingWarpAI|WarpAIDataSource"
rtk rg -n "OpenOzLaunchModal|oz_platform|agent_management|cloud_agent|computer_use|handoff"
```

重点文件：

| 区域 | 文件 |
| --- | --- |
| slash commands | `app/src/search/slash_command_menu/static_commands/commands.rs` |
| command search | `app/src/search/command_search/**`, `app/src/workspace/view.rs` |
| command palette/tips/resource center | `app/src/command_palette.rs`, `app/src/tips/**`, `app/src/resource_center/**` |
| terminal input/context menu/toolbelt | `app/src/terminal/input.rs`, `app/src/terminal/view.rs`, `app/src/terminal/block_list_element.rs` |
| workspace buttons/actions | `app/src/workspace/{action,mod,view}.rs` |
| onboarding | `crates/onboarding/src/**` |
| settings | `app/src/settings_view/**`, `app/src/settings/**` |

### 4. 清理 Oz/cloud 运行面

删除或断开这些运行能力：

| 能力 | 处理 |
| --- | --- |
| bundled Oz platform skill | 删除 `resources/bundled/skills/oz-platform/**`，移除注册/引用。 |
| cloud/Oz launch modal | 删除入口 action、keybinding、workspace render/handler。 |
| agent management | 从导航、toolbar、本地控制 surface 中移除或返回 unsupported。 |
| computer use | 删除 crate/deps/feature/action 映射，server tool call 不再有 client representation。 |
| run/task/schedule/harness support CLI | 删除 command family 或改为不可用。 |
| local/cloud child orchestration | 保留必要类型兼容时，执行器应返回 unavailable，不再实际启动 Warp 自带编排。 |

### 5. 保留第三方 CLI agent

不要误删这些路径，除非后续目标明确变化：

```text
app/src/terminal/cli_agent_sessions/**
app/src/tab_configs/**
app/src/settings_view/third_party_cli_agents_page.rs
app/src/settings_view/agent_mcp_servers_page.rs
crates/warp_cli/src/agent.rs 中仍被 CLI agent session 依赖的共享类型
```

验证搜索：

```bash
rtk rg -n "Codex|Claude Code|Gemini|OpenCode|ThirdPartyCLIAgents|AgentMCPServers" app/src crates/onboarding
```

### 6. 品牌/CLI 命名

若目标分支继续采用 Zerp 命名，统一这些面：

| 区域 | 检查点 |
| --- | --- |
| CLI binary | `zerp-cli`、CLI metadata、remote install script。 |
| channels/bundle | desktop entry、bundle id、installer assets。 |
| config/env | 产品 env vars 和 config dirs 使用目标产品名。 |
| docs/tests | 测试断言和文档不再把 CLI 写成旧名。 |

### 7. 验证门禁

至少运行：

```bash
rtk cargo fmt --all --check
rtk cargo check -p onboarding
rtk cargo test -p onboarding model::tests
rtk cargo test -p warp search::slash_command_menu::static_commands::commands
rtk cargo test -p warp tab_configs::session_config
rtk cargo test -p warp settings_view::tests
rtk cargo check -p warp --bin zerp
```

目标搜索：

```bash
rtk rg -n "Ask Warp AI|AI command search|WarpAIDataSource|OpenWarpAI|TranslateUsingWarpAI|toggle_ai_assistant|toggle_natural_language_command_search" app/src crates/onboarding
rtk rg -n "oz-platform|OpenOzLaunchModal|ResetOzLaunchModalState|oz_changelog|oz_launch" app/src crates resources
```

允许残留：

| 残留 | 原因 |
| --- | --- |
| `warp_ai_width` | app snapshot/schema 兼容字段。 |
| `warp_ai_policy` | workspace/billing API 兼容字段。 |
| 旧 telemetry enum | 历史事件兼容；只要没有发送点即可。 |
| `ai_assistant` 模块源码 | 可后续深删；当前重点是无用户入口、无执行路径。 |

### 8. `.gitignore` 和收尾

```bash
rtk git status --short
rtk git check-ignore --no-index -v <new-file-path>
rtk git ls-files -ci --exclude-standard
```

新增文件必须不被 ignore。若 `git check-ignore` 对新增文件返回非 0，表示未被 ignore，属于预期。

## 复查清单

- [ ] 默认新 session 是 terminal，不是 Warp Agent。
- [ ] 第三方 CLI agents 仍能从 new tab/settings 找到。
- [ ] settings/onboarding 不再出现 Warp AI/Oz/cloud agent 设置页。
- [ ] slash command、command palette、resource center、tips 不再出现 Warp 内建 AI 工具。
- [ ] terminal 右键菜单、input context menu、block toolbelt 不再出现 Ask/Attach AI。
- [ ] local-control 的 AI assistant/agent-management surface 不会打开旧 UI。
- [ ] `resources/bundled/skills/oz-platform` 已删除。
- [ ] 编译和目标测试通过。
