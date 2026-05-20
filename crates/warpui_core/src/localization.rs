use std::{borrow::Cow, sync::OnceLock};

#[cfg(windows)]
fn system_prefers_zh_cn() -> bool {
    use windows::Win32::Globalization::GetUserDefaultLocaleName;

    let mut locale_name = [0u16; 85];
    let len = unsafe { GetUserDefaultLocaleName(&mut locale_name) };
    if len <= 0 {
        return false;
    }

    String::from_utf16_lossy(&locale_name[..(len as usize).saturating_sub(1)])
        .to_ascii_lowercase()
        .starts_with("zh")
}

#[cfg(not(windows))]
fn system_prefers_zh_cn() -> bool {
    false
}

fn zh_cn_enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| {
        if std::env::var_os("WARP_FORCE_ZH_CN").is_some() {
            return true;
        }

        let locale = std::env::var("WARP_LOCALE")
            .ok()
            .or_else(|| std::env::var("LC_ALL").ok())
            .or_else(|| std::env::var("LC_MESSAGES").ok())
            .or_else(|| std::env::var("LANG").ok());

        locale
            .as_deref()
            .is_some_and(|locale| locale.to_ascii_lowercase().starts_with("zh"))
            || system_prefers_zh_cn()
    })
}

pub fn zh_cn(text: &str) -> Option<&'static str> {
    match text {
        "About Warp" => Some("关于 Warp"),
        "About" => Some("关于"),
        "Account" => Some("账号"),
        "Action" => Some("操作"),
        "Actions" => Some("操作"),
        "Add" => Some("添加"),
        "Add context" => Some("添加上下文"),
        "Add file" => Some("添加文件"),
        "Add folder" => Some("添加文件夹"),
        "Add item" => Some("添加项目"),
        "Add new" => Some("新增"),
        "Add to context" => Some("添加到上下文"),
        "Advanced" => Some("高级"),
        "AI" => Some("AI"),
        "All" => Some("全部"),
        "Allow" => Some("允许"),
        "Appearance" => Some("外观"),
        "Apply" => Some("应用"),
        "Apply changes" => Some("应用更改"),
        "Archive" => Some("归档"),
        "Ask" => Some("询问"),
        "Attach" => Some("附加"),
        "Back" => Some("返回"),
        "Blocks" => Some("块"),
        "Browse" => Some("浏览"),
        "Cancel" => Some("取消"),
        "Change" => Some("更改"),
        "Chat" => Some("聊天"),
        "Choose" => Some("选择"),
        "Clear" => Some("清除"),
        "Clear all" => Some("全部清除"),
        "Close" => Some("关闭"),
        "Close tab" => Some("关闭标签页"),
        "Close window" => Some("关闭窗口"),
        "Code" => Some("代码"),
        "Command Palette" => Some("命令面板"),
        "Commands" => Some("命令"),
        "Confirm" => Some("确认"),
        "Connect" => Some("连接"),
        "Continue" => Some("继续"),
        "Copied" => Some("已复制"),
        "Copy" => Some("复制"),
        "Copy link" => Some("复制链接"),
        "Create" => Some("创建"),
        "Create new" => Some("新建"),
        "Debug" => Some("调试"),
        "Default" => Some("默认"),
        "Delete" => Some("删除"),
        "Deny" => Some("拒绝"),
        "Disable" => Some("禁用"),
        "Disabled" => Some("已禁用"),
        "Discard" => Some("放弃"),
        "Done" => Some("完成"),
        "Download" => Some("下载"),
        "Drive" => Some("Drive"),
        "Edit" => Some("编辑"),
        "Enable" => Some("启用"),
        "Enabled" => Some("已启用"),
        "Error" => Some("错误"),
        "Exit" => Some("退出"),
        "Export" => Some("导出"),
        "File" => Some("文件"),
        "Files" => Some("文件"),
        "Filter" => Some("筛选"),
        "Find" => Some("查找"),
        "General" => Some("通用"),
        "Get started" => Some("开始使用"),
        "Help" => Some("帮助"),
        "Hide" => Some("隐藏"),
        "History" => Some("历史"),
        "Import" => Some("导入"),
        "Install" => Some("安装"),
        "Invite" => Some("邀请"),
        "Keybindings" => Some("快捷键"),
        "Learn more" => Some("了解更多"),
        "Loading" => Some("正在加载"),
        "Log in" => Some("登录"),
        "Log out" => Some("退出登录"),
        "Login" => Some("登录"),
        "More" => Some("更多"),
        "Name" => Some("名称"),
        "New" => Some("新建"),
        "New Tab" => Some("新建标签页"),
        "New Terminal Tab" => Some("新建终端标签页"),
        "New Window" => Some("新建窗口"),
        "Next" => Some("下一步"),
        "No" => Some("否"),
        "No images" => Some("没有图片"),
        "None" => Some("无"),
        "Not now" => Some("暂不"),
        "OK" => Some("确定"),
        "Off" => Some("关闭"),
        "On" => Some("开启"),
        "Open" => Some("打开"),
        "Open Recent" => Some("打开最近项目"),
        "Open Settings" => Some("打开设置"),
        "Open file" => Some("打开文件"),
        "Open folder" => Some("打开文件夹"),
        "Options" => Some("选项"),
        "Preferences" => Some("偏好设置"),
        "Privacy Policy..." => Some("隐私政策..."),
        "Profile" => Some("配置文件"),
        "Quit" => Some("退出"),
        "Remove" => Some("移除"),
        "Rename" => Some("重命名"),
        "Reset" => Some("重置"),
        "Retry" => Some("重试"),
        "Run" => Some("运行"),
        "Save" => Some("保存"),
        "Save All" => Some("全部保存"),
        "Save changes" => Some("保存更改"),
        "Search" => Some("搜索"),
        "Select" => Some("选择"),
        "Select all" => Some("全选"),
        "Settings" => Some("设置"),
        "Sign in" => Some("登录"),
        "Sign out" => Some("退出登录"),
        "Skip" => Some("跳过"),
        "Start" => Some("开始"),
        "Stop" => Some("停止"),
        "Submit" => Some("提交"),
        "Tab" => Some("标签页"),
        "Terminal" => Some("终端"),
        "Theme" => Some("主题"),
        "Undo" => Some("撤销"),
        "Uninstall" => Some("卸载"),
        "Update" => Some("更新"),
        "Upload" => Some("上传"),
        "View" => Some("视图"),
        "Window" => Some("窗口"),
        "Yes" => Some("是"),

        "Set Warp as Default Terminal" => Some("将 Warp 设为默认终端"),
        "Create anonymous user" => Some("创建匿名用户"),
        "Save New..." => Some("另存为新配置..."),
        "New Personal Notebook" => Some("新建个人笔记本"),
        "New Personal AI Prompt" => Some("新建个人 AI 提示词"),
        "New Team Notebook" => Some("新建团队笔记本"),
        "New Team AI Prompt" => Some("新建团队 AI 提示词"),
        "Search Drive" => Some("搜索 Drive"),
        "Team Settings" => Some("团队设置"),
        "Open Repository" => Some("打开代码仓库"),
        "Close Current Session" => Some("关闭当前会话"),
        "Copy on Select within the Terminal" => Some("在终端中选中即复制"),
        "Enable Shell Debug Mode (-x) for New Sessions" => {
            Some("为新会话启用 Shell 调试模式 (-x)")
        }
        "Disable Shell Debug Mode (-x) for New Sessions" => {
            Some("为新会话禁用 Shell 调试模式 (-x)")
        }
        "Enable In-band Generators for New Sessions" => Some("为新会话启用带内生成器"),
        "Disable in-band generators for new sessions" => Some("为新会话禁用带内生成器"),
        "Enable PTY Recording Mode (warp.pty.recording)" => {
            Some("启用 PTY 录制模式 (warp.pty.recording)")
        }
        "Disable PTY Recording Mode (warp.pty.recording)" => {
            Some("禁用 PTY 录制模式 (warp.pty.recording)")
        }
        "Show Initialization Block" => Some("显示初始化块"),
        "Hide Initialization Block" => Some("隐藏初始化块"),
        "Show In-band Command Blocks" => Some("显示带内命令块"),
        "Hide In-band Command Blocks" => Some("隐藏带内命令块"),
        "Show Warpified SSH Blocks" => Some("显示 Warp 化 SSH 块"),
        "Hide Warpified SSH Blocks" => Some("隐藏 Warp 化 SSH 块"),

        "Natural language detection" => Some("自然语言检测"),
        "Natural language denylist" => Some("自然语言拒绝列表"),
        "Commands listed here will never trigger natural language detection." => {
            Some("此处列出的命令永远不会触发自然语言检测。")
        }
        "AI command suggestions" => Some("AI 命令建议"),
        "Active AI" => Some("活动 AI"),
        "Agent Mode" => Some("Agent 模式"),
        "Agent" => Some("Agent"),
        "Agents" => Some("Agent"),
        "Add Profile" => Some("添加配置文件"),
        "+ Add custom model" => Some("+ 添加自定义模型"),
        "+ Add new repo" => Some(" + 添加新仓库"),
        "ACTIVE" => Some("进行中"),
        "Add MCP servers to extend the Warp Agent's capabilities. MCP servers expose data sources or tools to agents through a standardized interface, essentially acting like plugins. " => {
            Some("添加 MCP 服务器以扩展 Warp Agent 的能力。MCP 服务器通过标准化接口向 Agent 暴露数据源或工具，作用类似插件。")
        }
        "Add custom endpoint" => Some("添加自定义端点"),
        "Add regex" => Some("添加正则"),
        "Add regex pattern" => Some("添加正则模式"),
        "Allow in specific directories" => Some("允许指定目录"),
        "AppearanceSettingsPage" => Some("外观设置页"),
        "Aurora" => Some("极光"),
        "Auto-spawn servers from third-party agents" => Some("从第三方 Agent 自动启动服务器"),
        "Automatically detect and spawn MCP servers from globally-scoped third-party AI agent configuration files (e.g. in your home directory). Servers detected inside a repository are never spawned automatically and must be enabled individually from the MCP settings page. " => {
            Some("自动检测并启动全局范围内第三方 AI Agent 配置文件中的 MCP 服务器（例如主目录中的配置）。仓库内检测到的服务器不会自动启动，必须在 MCP 设置页中逐个启用。")
        }
        "Apply code diffs" => Some("应用代码差异"),
        "Apply code diffs:" => Some("应用代码差异："),
        "Ask questions" => Some("询问问题"),
        "Ask questions:" => Some("询问问题："),
        "Ask Agent" => Some("询问 Agent"),
        "Agent decides" => Some("由 Agent 决定"),
        "Always allow" => Some("始终允许"),
        "Always ask" => Some("始终询问"),
        "Ask on first write" => Some("首次写入时询问"),
        "Auto" => Some("自动"),
        "Auto (cost-efficient)" => Some("自动（节省成本）"),
        "Auto-sync plans to Warp Drive" => Some("自动同步计划到 Warp Drive"),
        "Auto-sync plans to Warp Drive:" => Some("自动同步计划到 Warp Drive："),
        "Base model" => Some("基础模型"),
        "Base model:" => Some("基础模型："),
        "Billing and usage" => Some("账单和用量"),
        "Branch" => Some("分支"),
        "Change keybinding" => Some("更改快捷键"),
        "Call MCP servers" => Some("调用 MCP 服务器"),
        "Call MCP servers:" => Some("调用 MCP 服务器："),
        "Call web tools" => Some("调用 Web 工具"),
        "Call web tools:" => Some("调用 Web 工具："),
        "Cloud platform" => Some("云平台"),
        "Code Review" => Some("代码审查"),
        "Code Diff" => Some("代码差异"),
        "Codebase Context" => Some("代码库上下文"),
        "Command Search" => Some("命令搜索"),
        "Command allowlist" => Some("命令允许列表"),
        "Command allowlist:" => Some("命令允许列表："),
        "Command denylist" => Some("命令拒绝列表"),
        "Command denylist:" => Some("命令拒绝列表："),
        "Command / Conversation" => Some("命令 / 会话"),
        "Commands, comma separated" => Some("命令，用逗号分隔"),
        "Compare plans" => Some("比较套餐"),
        "Configure keyboard shortcuts" => Some("配置键盘快捷键"),
        "Contact support" => Some("联系支持"),
        "Conversation" => Some("会话"),
        "Conversations cannot be deleted while in progress." => Some("会话进行中，无法删除。"),
        "Copy transcript to clipboard" => Some("复制记录到剪贴板"),
        "Cursor" => Some("光标"),
        "Computer use" => Some("计算机使用"),
        "Computer use:" => Some("计算机使用："),
        "Computer use model" => Some("计算机使用模型"),
        "Directory allowlist" => Some("目录允许列表"),
        "Directory allowlist:" => Some("目录允许列表："),
        "Documentation" => Some("文档"),
        "Editor and Code Review" => Some("编辑器和代码审查"),
        "Edit custom endpoint" => Some("编辑自定义端点"),
        "Edit Variables" => Some("编辑变量"),
        "Endpoint added" => Some("端点已添加"),
        "Endpoint removed" => Some("端点已移除"),
        "Endpoint saved" => Some("端点已保存"),
        "Environments" => Some("环境"),
        "Environment Variables" => Some("环境变量"),
        "Execution Profile" => Some("执行配置"),
        "Execute commands" => Some("执行命令"),
        "Execute commands:" => Some("执行命令："),
        "Focused session" => Some("聚焦会话"),
        "Feedback" => Some("反馈"),
        "Features" => Some("功能"),
        "Fork in new pane" => Some("在新面板中派生"),
        "Fork in new tab" => Some("在新标签页中派生"),
        "Free" => Some("免费"),
        "Full terminal use" => Some("完整终端使用"),
        "Full terminal use:" => Some("完整终端使用："),
        "Full terminal use model" => Some("完整终端使用模型"),
        "Global Search" => Some("全局搜索"),
        "Indexing and projects" => Some("索引和项目"),
        "Interact with running commands" => Some("与运行中的命令交互"),
        "Interact with running commands:" => Some("与运行中的命令交互："),
        "Input" => Some("输入"),
        "Invite a friend" => Some("邀请朋友"),
        "Invite a friend to Warp" => Some("邀请朋友使用 Warp"),
        "JSON" => Some("JSON"),
        "Keybinding" => Some("快捷键"),
        "Manage MCP servers" => Some("管理 MCP 服务器"),
        "Manage billing" => Some("管理账单"),
        "Manage your data" => Some("管理你的数据"),
        "Knowledge" => Some("知识库"),
        "MCP allowlist" => Some("MCP 允许列表"),
        "MCP allowlist:" => Some("MCP 允许列表："),
        "MCP denylist" => Some("MCP 拒绝列表"),
        "MCP denylist:" => Some("MCP 拒绝列表："),
        "MCP servers" => Some("MCP 服务器"),
        "MCP Servers" => Some("MCP 服务器"),
        "Keyboard Shortcuts" => Some("键盘快捷键"),
        "Keyboard shortcuts" => Some("键盘快捷键"),
        "MODELS" => Some("模型"),
        "Models" => Some("模型"),
        "Network log console" => Some("网络日志控制台"),
        "New session" => Some("新建会话"),
        "Next Command" => Some("下一条命令"),
        "No matching conversations" => Some("没有匹配的会话"),
        "No tabs match your search." => Some("没有匹配搜索的标签页。"),
        "No conversations yet" => Some("暂无会话"),
        "No results found." => Some("未找到结果。"),
        "No tabs open" => Some("没有打开的标签页"),
        "Never" => Some("从不"),
        "Never ask" => Some("从不询问"),
        "Ask unless auto-approve" => Some("非自动批准时询问"),
        "Only for named colors" => Some("仅命名颜色"),
        "Only on hover" => Some("仅悬停时"),
        "Open repository" => Some("打开代码仓库"),
        "Open settings file" => Some("打开设置文件"),
        "Other" => Some("其他"),
        "Oz Cloud API Keys" => Some("Oz 云 API 密钥"),
        "PAST" => Some("过去"),
        "PERMISSIONS" => Some("权限"),
        "Panes" => Some("面板"),
        "Permissions" => Some("权限"),
        "Personal" => Some("个人"),
        "Plan" => Some("计划"),
        "Press new keyboard shortcut" => Some("按下新的键盘快捷键"),
        "Privacy policy" => Some("隐私政策"),
        "Prompt Suggestions" => Some("提示建议"),
        "Privacy" => Some("隐私"),
        "Profiles" => Some("配置文件"),
        "Profiles let you define how your Agent operates — from the actions it can take and when it needs approval, to the models it uses for tasks like coding and planning. You can also scope them to individual projects." => {
            Some("配置文件可定义 Agent 的运行方式，包括它能执行哪些操作、何时需要审批，以及编码、规划等任务使用的模型。你也可以将配置限定到单个项目。")
        }
        "Read files" => Some("读取文件"),
        "Read files:" => Some("读取文件："),
        "Read only" => Some("只读"),
        "Recommended" => Some("推荐"),
        "Referrals" => Some("推荐"),
        "Refer a friend" => Some("推荐朋友"),
        "Remove from team" => Some("从团队移除"),
        "Relaunch Warp" => Some("重新启动 Warp"),
        "Right" => Some("右侧"),
        "Search repos" => Some("搜索代码仓库"),
        "Search sessions" => Some("搜索会话"),
        "Search sessions, agents, files..." => Some("搜索会话、Agent、文件..."),
        "Search tabs" => Some("搜索标签页"),
        "Search tabs..." => Some("搜索标签页..."),
        "See supported providers." => Some("查看支持的提供方。"),
        "Secret redaction" => Some("密钥遮盖"),
        "Select coding agent" => Some("选择编码 Agent"),
        "Select MCP servers" => Some("选择 MCP 服务器"),
        "Send crash reports" => Some("发送崩溃报告"),
        "Settings sync" => Some("设置同步"),
        "Share conversation" => Some("分享会话"),
        "Shared blocks" => Some("共享块"),
        "Shell (PS1)" => Some("Shell (PS1)"),
        "Show" => Some("显示"),
        "Show agent tips" => Some("显示 Agent 提示"),
        "Show details on hover" => Some("悬停时显示详情"),
        "Show input hint text" => Some("显示输入提示文本"),
        "Show less" => Some("收起"),
        "Sign up" => Some("注册"),
        "Slack" => Some("Slack 社区"),
        "Split Pane" => Some("拆分面板"),
        "Store AI conversations in the cloud" => Some("在云端存储 AI 会话"),
        "Summary" => Some("摘要"),
        "Supervised" => Some("受监督"),
        "Suggested Code Banners" => Some("建议代码横幅"),
        "Suggested Rules" => Some("建议规则"),
        "Tab configs" => Some("标签页配置"),
        "Tab item" => Some("标签项"),
        "Tabs" => Some("标签页"),
        "Text" => Some("文本"),
        "Teams" => Some("团队"),
        "Third party CLI agents" => Some("第三方 CLI Agent"),
        "Unknown" => Some("未知"),
        "Unlimited" => Some("无限制"),
        "Unsaved" => Some("未保存"),
        "Up to date" => Some("已是最新"),
        "Usage" => Some("用量"),
        "Use" => Some("使用"),
        "Upgrade" => Some("升级"),
        "Version" => Some("版本"),
        "View documentation" => Some("查看文档"),
        "View all available system fonts" => Some("查看所有可用系统字体"),
        "View network logging" => Some("查看网络日志"),
        "View options" => Some("查看选项"),
        "View Warp logs" => Some("查看 Warp 日志"),
        "Visit the data management page" => Some("访问数据管理页面"),
        "Warp Agent" => Some("Warp Agent"),
        "Warp Drive" => Some("Warp Drive"),
        "Warpify" => Some("Warpify"),
        "Rules" => Some("规则"),
        "Rules help the Warp Agent follow your conventions, whether for codebases or specific workflows. " => {
            Some("规则可以帮助 Warp Agent 遵循你的约定，无论是针对代码库还是特定工作流。")
        }
        "Natural Language Autosuggestions" => Some("自然语言自动建议"),
        "Let AI suggest the next command to run based on your command history, outputs, and common workflows." => {
            Some("让 AI 根据你的命令历史、输出和常见工作流建议下一条要运行的命令。")
        }
        "Let AI suggest natural language prompts, as inline banners in the input, based on recent commands and their outputs." => {
            Some("让 AI 根据最近的命令及其输出，在输入框中以内联横幅形式建议自然语言提示。")
        }
        "Let AI suggest code diffs and queries as inline banners in the blocklist, based on recent commands and their outputs." => {
            Some("让 AI 根据最近的命令及其输出，在块列表中以内联横幅形式建议代码差异和查询。")
        }
        "Let AI suggest natural language autosuggestions, based on recent commands and their outputs." => {
            Some("让 AI 根据最近的命令及其输出建议自然语言自动补全。")
        }
        "Let AI generate a title for your shared block based on the command and output." => {
            Some("让 AI 根据命令和输出为你的共享块生成标题。")
        }
        "Let AI generate commit messages and pull request titles and descriptions." => {
            Some("让 AI 生成提交消息以及拉取请求标题和描述。")
        }
        "Shared Block Title Generation" => Some("共享块标题生成"),
        "Commit & Pull Request Generation" => Some("提交和拉取请求生成"),
        "Toolbar layout" => Some("工具栏布局"),
        "Show model picker in prompt" => Some("在提示框中显示模型选择器"),
        "Commands that enable the toolbar" => Some("启用工具栏的命令"),
        "Loading..." => Some("正在加载..."),
        "Current" => Some("当前"),
        "New conversation" => Some("新建会话"),
        "View all" => Some("查看全部"),
        "Refresh" => Some("刷新"),
        "When a command takes longer than" => Some("当命令运行超过"),
        "seconds to complete" => Some("秒才完成"),
        "seconds" => Some("秒"),
        "Working Directory" => Some("工作目录"),
        "What's new" => Some("最新动态"),
        "What's new in Oz" => Some("Oz 最新动态"),
        "Workspace" => Some("工作区"),
        "Your active and past conversations with local and ambient agents will appear here." => {
            Some("你与本地和环境 Agent 的当前及历史会话会显示在这里。")
        }
        "Your organization disallows AI when the active pane contains content from a remote session" => {
            Some("当活动面板包含远程会话内容时，你的组织不允许使用 AI。")
        }
        "To use AI features, please create an account." => Some("要使用 AI 功能，请创建账号。"),
        "Secret visual redaction mode" => Some("密钥视觉遮盖模式"),
        "No enterprise regexes have been configured by your organization." => {
            Some("你的组织尚未配置企业正则。")
        }
        "This setting is managed by your organization." => Some("此设置由你的组织管理。"),
        "Enabled by your organization." => Some("已由你的组织启用。"),
        "Crash reports assist with debugging and stability improvements." => {
            Some("崩溃报告有助于调试和提升稳定性。")
        }
        "Autodetect agent prompts in terminal input" => Some("在终端输入中自动检测 Agent 提示"),
        "Autodetect terminal commands in agent input" => Some("在 Agent 输入中自动检测终端命令"),
        "Encountered an incorrect detection? " => Some("遇到了错误检测？"),
        "Encountered an incorrect input detection? " => Some("遇到了错误输入检测？"),
        "Let us know" => Some("告诉我们"),
        "Let us know." => Some("告诉我们。"),
        "Enabling natural language detection will detect when natural language is written in the terminal input, and then automatically switch to Agent Mode for AI queries." => {
            Some("启用自然语言检测后，Warp 会在终端输入中检测自然语言，并自动切换到 Agent 模式处理 AI 查询。")
        }
        "Include agent-executed commands in history" => Some("将 Agent 执行的命令加入历史记录"),
        "Voice" => Some("语音"),
        "Voice Input" => Some("语音输入"),
        "Voice input allows you to control Warp by speaking directly to your terminal (powered by " => {
            Some("语音输入允许你直接对终端说话来控制 Warp（由 ")
        }
        "Key for Activating Voice Input" => Some("激活语音输入的按键"),
        "Press and hold to activate." => Some("按住即可激活。"),
        "Send Feedback" => Some("发送反馈"),
        "Send Feedback..." => Some("发送反馈..."),
        "Warp Documentation..." => Some("Warp 文档..."),
        "New features" => Some("新功能"),
        "Delete MCP" => Some("删除 MCP"),
        "Delete MCP server?" => Some("删除 MCP 服务器？"),
        "Delete shared MCP server?" => Some("删除共享 MCP 服务器？"),
        "Remove shared MCP server from team?" => Some("从团队中移除共享 MCP 服务器？"),
        "This will uninstall and remove this MCP server from all your devices." => {
            Some("这会从你的所有设备卸载并移除此 MCP 服务器。")
        }
        "This will uninstall and remove this MCP server from Warp and across all of your teammates' devices." => {
            Some("这会从 Warp 以及所有队友的设备中卸载并移除此 MCP 服务器。")
        }
        "Copy Block" => Some("复制块"),
        "Copy Block Command" => Some("复制块命令"),
        "Copy Block Output" => Some("复制块输出"),
        "Create Block Permalink" => Some("创建块永久链接"),
        "Select Block Above" => Some("选择上方块"),
        "Select Block Below" => Some("选择下方块"),
        "Select All Blocks" => Some("选择全部块"),
        "Scroll to Top of Selected Blocks" => Some("滚动到所选块顶部"),
        "Scroll to Bottom of Selected Blocks" => Some("滚动到所选块底部"),

        // Workspace, tools panel, and project explorer.
        "New worktree config" => Some("新建工作树配置"),
        "New tab config" => Some("新建标签页配置"),
        "Reopen closed session" => Some("重新打开已关闭会话"),
        "Tools panel" => Some("工具面板"),
        "Project explorer" => Some("项目资源管理器"),
        "Project explorer unavailable" => Some("项目资源管理器不可用"),
        "The Project Explorer requires access to your local workspace. Open a new session or navigate to an active session to view." => {
            Some("项目资源管理器需要访问你的本地工作区。请打开新会话，或切换到活动会话后查看。")
        }
        "Make default" => Some("设为默认"),
        "Search directories..." => Some("搜索目录..."),
        "(Parent Directory)" => Some("（上级目录）"),
        "Change working directory" => Some("更改工作目录"),

        // Codebase indexing and editor/code-review settings.
        "Codebase Indexing" => Some("代码库索引"),
        "Codebase indexing" => Some("代码库索引"),
        "Warp can automatically index code repositories as you navigate them, helping agents quickly understand context and provide solutions. Code is never stored on the server. If a codebase is unable to be indexed, Warp can still navigate your codebase and gain insights via grep and find tool calling." => {
            Some("Warp 可以在你浏览代码仓库时自动为其建立索引，帮助 Agent 快速理解上下文并提供解决方案。代码永远不会存储在服务器上。如果某个代码库无法建立索引，Warp 仍可通过 grep 和 find 工具调用浏览代码库并获取信息。")
        }
        "Initialized / indexed folders" => Some("已初始化/已索引的文件夹"),
        "No folders have been initialized yet." => Some("尚未初始化任何文件夹。"),
        "Index new folder" => Some("索引新文件夹"),
        "Choose an editor to open file links" => Some("选择用于打开文件链接的编辑器"),
        "Default App" => Some("默认应用"),
        "Choose an editor to open files from the code review panel, project explorer, and global search" => {
            Some("选择用于从代码审查面板、项目资源管理器和全局搜索打开文件的编辑器")
        }
        "Choose a layout to open files in Warp" => Some("选择在 Warp 中打开文件的布局"),
        "Group files into single editor pane" => Some("将文件分组到单个编辑器面板"),
        "When this setting is on, any files opened in the same tab will be automatically grouped into a single editor pane." => {
            Some("开启此设置后，在同一标签页中打开的所有文件都会自动分组到单个编辑器面板。")
        }
        "Open Markdown files in Warp's Markdown Viewer by default" => {
            Some("默认使用 Warp 的 Markdown 查看器打开 Markdown 文件")
        }
        "Auto open code review panel" => Some("自动打开代码审查面板"),
        "When this setting is on, the code review panel will open on the first accepted diff of a conversation" => {
            Some("开启此设置后，会话中第一个差异被接受时会打开代码审查面板")
        }
        "Show code review button" => Some("显示代码审查按钮"),
        "Show a button in the top right of the window to toggle the code review panel." => {
            Some("在窗口右上角显示用于切换代码审查面板的按钮。")
        }
        "Show diff stats on code review button" => Some("在代码审查按钮上显示差异统计"),
        "Show lines added and removed counts on the code review button." => {
            Some("在代码审查按钮上显示新增和删除的行数。")
        }
        "Adds an IDE-style project explorer / file tree to the left side tools panel." => {
            Some("在左侧工具面板中添加 IDE 风格的项目资源管理器/文件树。")
        }
        "Global file search" => Some("全局文件搜索"),
        "Adds global file search to the left side tools panel." => {
            Some("在左侧工具面板中添加全局文件搜索。")
        }

        // Environments and teams.
        "Environments define where your ambient agents run. Set one up in minutes via GitHub (recommended), Warp-assisted setup, or manual configuration." => {
            Some("环境用于定义你的环境 Agent 在哪里运行。你可以通过 GitHub（推荐）、Warp 辅助设置或手动配置在几分钟内完成设置。")
        }
        "You haven’t set up any environments yet." => Some("你还没有设置任何环境。"),
        "Choose how you’d like to set up your environment:" => Some("选择你想要设置环境的方式："),
        "Quick setup" => Some("快速设置"),
        "Suggested" => Some("推荐"),
        "Select the GitHub repositories you’d like to work with and we’ll suggest a base image and config" => {
            Some("选择你想使用的 GitHub 仓库，我们会推荐基础镜像和配置")
        }
        "Use the agent" => Some("使用 Agent"),
        "Choose a locally set up project and we’ll help you set up an environment based on it" => {
            Some("选择一个本地已设置的项目，我们会帮你基于它设置环境")
        }
        "Launch agent" => Some("启动 Agent"),
        "Create a team" => Some("创建团队"),
        "When you create a team, you can collaborate on agent-driven development by sharing cloud agent runs, environments, automations, and artifacts. You can also create a shared knowledge store for teammates and agents alike." => {
            Some("创建团队后，你可以通过共享云端 Agent 运行、环境、自动化和产物来协作进行 Agent 驱动开发。你还可以为队友和 Agent 创建共享知识库。")
        }
        "Team name" => Some("团队名称"),

        // Appearance settings.
        "Themes" => Some("主题"),
        "Create your own custom theme" => Some("创建你自己的自定义主题"),
        "Sync with OS" => Some("跟随系统"),
        "Automatically switch between light and dark themes when your system does." => {
            Some("当系统切换浅色和深色主题时自动跟随切换。")
        }
        "Current theme" => Some("当前主题"),
        "Dark" => Some("深色"),
        "Open new windows with custom size" => Some("用自定义大小打开新窗口"),
        "Use Window Blur (Acrylic texture)" => Some("使用窗口模糊（亚克力纹理）"),
        "Zoom" => Some("缩放"),
        "Adjusts the default zoom level across all windows" => Some("调整所有窗口的默认缩放级别"),
        "Reset to default" => Some("重置为默认值"),
        "Tools panel visibility is consistent across tabs" => Some("工具面板可见性在所有标签页中保持一致"),
        "Input type" => Some("输入类型"),
        "Input position" => Some("输入位置"),
        "Pin to the bottom (Warp mode)" => Some("固定到底部（Warp 模式）"),
        "Pane" => Some("面板"),
        "Dim inactive panes" => Some("调暗非活动面板"),
        "Focus follows mouse" => Some("焦点跟随鼠标"),
        "Compact mode" => Some("紧凑模式"),
        "Show Jump to Bottom of Block button" => Some("显示跳到块底部按钮"),
        "Show block dividers" => Some("显示块分隔线"),
        "Terminal font" => Some("终端字体"),
        "Font weight" => Some("字体粗细"),
        "Normal" => Some("常规"),
        "Font size (px)" => Some("字号（px）"),
        "Line height" => Some("行高"),
        "Agent font" => Some("Agent 字体"),
        "Notebook font size" => Some("笔记本字号"),
        "Match terminal" => Some("匹配终端"),
        "Enforce minimum contrast" => Some("强制最低对比度"),
        "Show ligatures in terminal" => Some("在终端中显示连字"),
        "Cursor type" => Some("光标类型"),
        "Bar" => Some("竖线"),
        "Block" => Some("块"),
        "Underline" => Some("下划线"),
        "Blinking cursor" => Some("光标闪烁"),
        "Show tab indicators" => Some("显示标签页指示器"),
        "Show the tab bar" => Some("显示标签栏"),
        "When windowed" => Some("窗口模式时"),
        "Tab close button position" => Some("标签页关闭按钮位置"),
        "Preserve active tab color for new tabs" => Some("为新标签页保留活动标签页颜色"),
        "Use vertical tab layout" => Some("使用垂直标签页布局"),
        "Show vertical tabs panel in restored windows" => Some("在恢复的窗口中显示垂直标签页面板"),
        "When enabled, reopening or restoring a window opens the vertical tabs panel even if it was closed when the window was last saved." => {
            Some("启用后，重新打开或恢复窗口时会打开垂直标签页面板，即使上次保存窗口时它是关闭的。")
        }
        "Use latest user prompt as conversation title in tab names" => {
            Some("在标签页名称中使用最新用户提示作为会话标题")
        }
        "Show the latest user prompt instead of the generated conversation title for Oz and third-party agent sessions in vertical tabs." => {
            Some("在垂直标签页中，为 Oz 和第三方 Agent 会话显示最新用户提示，而不是生成的会话标题。")
        }
        "Header toolbar layout" => Some("顶部工具栏布局"),
        "Available items" => Some("可用项目"),
        "Left side" => Some("左侧"),
        "Right side" => Some("右侧"),
        "Restore default" => Some("恢复默认"),
        "Tabs Panel" => Some("标签页面板"),
        "Notifications" => Some("通知"),
        "Directory tab colors" => Some("目录标签页颜色"),
        "Automatically color tabs based on the directory or repo you're working in." => {
            Some("根据你正在使用的目录或仓库自动为标签页着色。")
        }
        "+ Add directory color" => Some("+ 添加目录颜色"),
        "Full-screen Apps" => Some("全屏应用"),
        "Use custom padding in alt-screen" => Some("在备用屏幕中使用自定义内边距"),
        "Uniform padding (px)" => Some("统一内边距（px）"),

        // General and feature settings.
        "Default mode for new sessions" => Some("新会话的默认模式"),
        "Restore windows, tabs, and panes on startup" => Some("启动时恢复窗口、标签页和面板"),
        "Show sticky command header" => Some("显示固定命令标题"),
        "Show tooltip on click on links" => Some("点击链接时显示工具提示"),
        "Show warning before quitting/logging out" => Some("退出或注销前显示警告"),
        "Start Warp at login" => Some("登录时启动 Warp"),
        "Show changelog toast after updates" => Some("更新后显示更新日志提示"),
        "Lines scrolled by mouse wheel interval" => Some("鼠标滚轮每次滚动的行数"),
        "Session" => Some("会话"),
        "Maximum rows in a block" => Some("块中的最大行数"),
        "Setting the limit above 100k lines may impact performance. Maximum rows supported is 10 million." => {
            Some("将限制设置为超过 10 万行可能会影响性能。支持的最大行数为 1000 万。")
        }
        "Setting the limit above 100k lines may impact performance. Maximum rows supported is {max_rows}." => {
            Some("将限制设置为超过 10 万行可能会影响性能。支持的最大行数为 {max_rows}。")
        }
        "Warp SSH Wrapper" => Some("Warp SSH 包装器"),
        "Default shell for new sessions" => Some("新会话的默认 Shell"),
        "Working directory for new sessions" => Some("新会话的工作目录"),
        "Previous session's directory" => Some("上一个会话的目录"),
        "Enable reopening of closed sessions" => Some("允许重新打开已关闭会话"),
        "Grace period (seconds)" => Some("宽限时间（秒）"),
        "Keys" => Some("按键"),
        "Left Alt key is Meta" => Some("左 Alt 键作为 Meta"),
        "Right Alt key is Meta" => Some("右 Alt 键作为 Meta"),
        "Ctrl+Tab behavior:" => Some("Ctrl+Tab 行为："),
        "Activate previous/next tab" => Some("激活上一个/下一个标签页"),
        "Global hotkey:" => Some("全局热键："),
        "Text Editing" => Some("文本编辑"),
        "Autocomplete quotes, parentheses, and brackets" => Some("自动补全引号、圆括号和方括号"),
        "Edit code and commands with Vim keybindings" => Some("使用 Vim 键位编辑代码和命令"),
        "Terminal Input" => Some("终端输入"),
        "Error underlining for commands" => Some("为命令错误添加下划线"),
        "Syntax highlighting for commands" => Some("命令语法高亮"),
        "Open completions menu as you type" => Some("输入时打开补全菜单"),
        "Suggest corrected commands" => Some("建议修正后的命令"),
        "Expand aliases as you type" => Some("输入时展开别名"),
        "Middle-click to paste" => Some("中键点击粘贴"),
        "Show autosuggestion keybinding hint" => Some("显示自动建议快捷键提示"),
        "Show autosuggestion ignore button" => Some("显示忽略自动建议按钮"),
        "Enable '@' context menu in terminal mode" => Some("在终端模式中启用“@”上下文菜单"),
        "Outline codebase symbols for '@' context menu" => Some("为“@”上下文菜单列出代码库符号"),
        "Show terminal input message line" => Some("显示终端输入消息行"),
        "Tab key behavior" => Some("Tab 键行为"),
        "User defined" => Some("用户定义"),
        "Enable Mouse Reporting" => Some("启用鼠标报告"),
        "Enable Scroll Reporting" => Some("启用滚动报告"),
        "Enable Focus Reporting" => Some("启用焦点报告"),
        "Use Audible Bell" => Some("使用声音提示"),
        "Double-click smart selection" => Some("双击智能选择"),
        "Copy on select" => Some("选中即复制"),
        "New tab placement" => Some("新标签页位置"),
        "After current tab" => Some("当前标签页之后"),
        "Receive desktop notifications from Warp" => Some("接收来自 Warp 的桌面通知"),
        "Show in-app agent notifications" => Some("显示应用内 Agent 通知"),
        "Toast notifications stay visible for" => Some("提示通知保持可见"),
        "Workflows" => Some("工作流"),
        "Show Global Workflows in Command Search (ctrl-r)" => {
            Some("在命令搜索（ctrl-r）中显示全局工作流")
        }
        "System" => Some("系统"),
        "Preferred graphics backend" => Some("首选图形后端"),

        // Keybindings and command names.
        "Add your own custom keybindings to existing actions below." => {
            Some("在下方为现有操作添加你自己的自定义快捷键。")
        }
        "to reference these keybindings in a side pane at anytime." => {
            Some("可随时在侧边窗格中查看这些快捷键。")
        }
        "Command" => Some("命令"),
        "Search by name or by keys (ex. \"cmd\")" => Some("按名称或按键搜索（例如“cmd”）"),
        "Accept Autosuggestion" => Some("接受自动建议"),
        "Accept Prompt Suggestion" => Some("接受提示建议"),
        "Activate Next Pane" => Some("激活下一个面板"),
        "Activate Next Tab" => Some("激活下一个标签页"),
        "Activate Previous Pane" => Some("激活上一个面板"),
        "Activate Previous Tab" => Some("激活上一个标签页"),
        "Add Cursor Above" => Some("在上方添加光标"),
        "Add Cursor Below" => Some("在下方添加光标"),
        "Add Repository" => Some("添加仓库"),
        "Add Selection for Next Occurrence" => Some("选择下一个匹配项"),
        "Alternate Terminal Paste" => Some("备用终端粘贴"),
        "Attach Selected Block as Agent Context" => Some("将所选块附加为 Agent 上下文"),
        "Attach Selected Text as Agent Context" => Some("将所选文本附加为 Agent 上下文"),
        "Backward Tabulation Within an Executing Command" => Some("在执行中的命令内反向制表"),
        "Bookmark Selected Block" => Some("为所选块添加书签"),
        "Clear Blocks" => Some("清除块"),
        "Clear Command Editor" => Some("清除命令编辑器"),
        "Clear Screen" => Some("清屏"),
        "Clear Selected Lines" => Some("清除所选行"),
        "Clear and Reset AI Context Menu Query" => Some("清除并重置 AI 上下文菜单查询"),
        "Close All Tabs" => Some("关闭所有标签页"),
        "Close Focused Panel" => Some("关闭聚焦面板"),
        "Close Other Tabs" => Some("关闭其他标签页"),
        "Close Saved Tabs" => Some("关闭已保存标签页"),
        "Close Tabs Below" => Some("关闭下方标签页"),
        "Close Window" => Some("关闭窗口"),
        "Close the Current Tab" => Some("关闭当前标签页"),
        "Copy Access Token to Clipboard" => Some("将访问令牌复制到剪贴板"),
        "Copy Command" => Some("复制命令"),
        "Copy Command Output" => Some("复制命令输出"),
        "Copy Command and Output" => Some("复制命令和输出"),
        "Copy Git Branch" => Some("复制 Git 分支"),
        "Copy Rich-Text Buffer" => Some("复制富文本缓冲区"),

        // Warpify settings.
        "Configure whether Warp attempts to “Warpify” (add support for blocks, input modes, etc) certain shells." => {
            Some("配置 Warp 是否尝试对某些 Shell 进行“Warpify”（添加对块、输入模式等的支持）。")
        }
        "Subshells" => Some("子 Shell"),
        "Subshells supported: bash, zsh, and fish." => Some("支持的子 Shell：bash、zsh 和 fish。"),
        "Added commands" => Some("已添加命令"),
        "Denylisted commands" => Some("拒绝列表命令"),
        "command (supports regex)" => Some("命令（支持正则）"),

        // Referrals, Drive, and privacy.
        "Sign up to participate in Warp's referral program" => Some("注册参加 Warp 推荐计划"),
        "Get exclusive Warp goodies when you refer someone*" => Some("推荐他人即可获得 Warp 专属好礼*"),
        "Exclusive theme" => Some("专属主题"),
        "Keycaps + stickers" => Some("键帽 + 贴纸"),
        "T-shirt" => Some("T 恤"),
        "Notebook" => Some("笔记本"),
        "Baseball cap" => Some("棒球帽"),
        "Hoodie" => Some("连帽衫"),
        "Premium Hydro Flask" => Some("高级 Hydro Flask 水杯"),
        "Backpack" => Some("背包"),
        "Certain restrictions apply." => Some("需遵守部分限制。"),
        " If you have any questions about the referral program, please contact referrals@warp.dev." => {
            Some(" 如果你对推荐计划有任何问题，请联系 referrals@warp.dev。")
        }
        "To use Warp Drive, please create an account." => Some("要使用 Warp Drive，请创建账号。"),
        "Warp Drive is a workspace in your terminal where you can save Workflows, Notebooks, Prompts, and Environment Variables for personal use or to share with a team." => {
            Some("Warp Drive 是终端中的工作区，你可以在其中保存工作流、笔记本、提示和环境变量，供个人使用或与团队共享。")
        }
        "When this setting is enabled, Warp will scan blocks, the contents of Warp Drive objects, and Oz prompts for potential sensitive information and prevent saving or sending this data to any servers. You can customize this list via regexes." => {
            Some("启用此设置后，Warp 会扫描块、Warp Drive 对象内容和 Oz 提示中的潜在敏感信息，并阻止保存这些数据或将其发送到任何服务器。你可以通过正则表达式自定义此列表。")
        }
        "We've built a native console that allows you to view all communications from Warp to external servers to ensure you feel comfortable that your work is always kept safe." => {
            Some("我们内置了原生控制台，可让你查看 Warp 与外部服务器之间的所有通信，确保你确信自己的工作始终安全。")
        }
        "Read Warp's privacy policy" => Some("阅读 Warp 隐私政策"),
        _ => None,
    }
}

pub fn localize_cow(text: Cow<'static, str>) -> Cow<'static, str> {
    if !zh_cn_enabled() {
        return text;
    }

    match text {
        Cow::Borrowed(value) => zh_cn(value)
            .map(Cow::Borrowed)
            .unwrap_or(Cow::Borrowed(value)),
        Cow::Owned(value) => zh_cn(&value)
            .map(Cow::Borrowed)
            .unwrap_or(Cow::Owned(value)),
    }
}

pub fn localize_string(text: impl Into<String>) -> String {
    let text = text.into();
    if !zh_cn_enabled() {
        return text;
    }

    if let Some(localized) = zh_cn(&text) {
        return localized.to_string();
    }

    if let Some(name) = text
        .strip_prefix("Edit ")
        .and_then(|rest| rest.strip_suffix(" MCP Server"))
    {
        return format!("编辑 {name} MCP 服务器");
    }
    if let Some(name) = text
        .strip_prefix("Successfully logged out of ")
        .and_then(|rest| rest.strip_suffix(" MCP server"))
    {
        return format!("已成功退出 {name} MCP 服务器登录");
    }
    if let Some(version) = text.strip_prefix("Current version is ") {
        return format!("当前版本是 {version}");
    }
    if let Some(version) = text
        .strip_prefix("Install update (")
        .and_then(|rest| rest.strip_suffix(')'))
    {
        return format!("安装更新 ({version})");
    }
    if let Some(version) = text
        .strip_prefix("Updating to (")
        .and_then(|rest| rest.strip_suffix(')'))
    {
        return format!("正在更新到 ({version})");
    }
    if let Some(name) = text.strip_prefix("Copy ") {
        return format!("复制 {}", localize_string(name));
    }
    if let Some(name) = text.strip_prefix("New ") {
        if let Some(localized_name) = zh_cn(name) {
            return format!("新建{localized_name}");
        }
    }
    if let Some(count) = text
        .strip_prefix("and ")
        .and_then(|rest| rest.strip_suffix(" more"))
    {
        return format!("还有 {count} 个");
    }
    if let Some(value) = text.strip_prefix("Window Opacity: ") {
        return format!("窗口透明度：{value}");
    }
    if let Some(value) = text.strip_prefix("Current backend: ") {
        return format!("当前后端：{value}");
    }
    if let Some(value) = text.strip_prefix("Toast notifications stay visible for ") {
        return format!("提示通知保持可见 {value}");
    }

    text
}
