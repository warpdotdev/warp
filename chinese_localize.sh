#!/bin/bash
# Batch Chinese localization script for Warp terminal
# Apply all translations from the 6 parallel agents

cd "$(dirname "$0")"

replace() {
    local file="$1"
    local old="$2"
    local new="$3"
    if grep -qF "$old" "$file" 2>/dev/null; then
        sed -i "s/$old/$new/g" "$file"
        echo "OK: $old"
    else
        echo "SKIP: $old"
    fi
}

echo "=== Agent-6: Code Editor & File Tree ==="
replace "app/src/code/editor/comment_editor.rs" '"Comment"' '"评论"'
replace "app/src/code/editor/comment_editor.rs" '"Cancel"' '"取消"'
replace "app/src/code/editor/comment_editor.rs" '"Remove"' '"删除"'
replace "app/src/code/editor/comment_editor.rs" '"Update"' '"更新"'
replace "app/src/code/editor/view/actions.rs" '"Delete"' '"删除"'
replace "app/src/code/file_tree/view.rs" '"Open in new pane"' '"在新窗格中打开"'
replace "app/src/code/file_tree/view.rs" '"Open in new tab"' '"在新标签页中打开"'
replace "app/src/code/file_tree/view.rs" '"Open file"' '"打开文件"'
replace "app/src/code/file_tree/view.rs" '"cd to directory"' '"切换到目录"'
replace "app/src/code/file_tree/view.rs" '"Reveal in Finder"' '"在访达中显示"'
replace "app/src/code/file_tree/view.rs" '"Reveal in Explorer"' '"在资源管理器中显示"'
replace "app/src/code/file_tree/view.rs" '"Reveal in file manager"' '"在文件管理器中显示"'
replace "app/src/code/file_tree/view.rs" '"Rename"' '"重命名"'
replace "app/src/code/file_tree/view.rs" '"Attach as context"' '"附加为上下文"'
replace "app/src/code/file_tree/view.rs" '"Copy path"' '"复制路径"'
replace "app/src/code/file_tree/view.rs" '"Copy relative path"' '"复制相对路径"'

# code review
replace "app/src/code_review/code_review_view.rs" '"Hide file navigation"' '"隐藏文件导航"'
replace "app/src/code_review/code_review_view.rs" '"Show file navigation"' '"显示文件导航"'
replace "app/src/code_review/code_review_view.rs" '"Maximize"' '"最大化"'
replace "app/src/code_review/code_review_view.rs" '"Restore"' '"还原"'
replace "app/src/code_review/code_review_view.rs" '"Commit"' '"提交"'
replace "app/src/code_review/code_review_view.rs" '"Undo"' '"撤销"'
replace "app/src/code_review/code_review_view.rs" '"Discard changes"' '"丢弃更改"'
replace "app/src/code_review/code_review_view.rs" '"Initialize codebase"' '"初始化代码库"'
replace "app/src/code_review/code_review_view.rs" '"Open repository"' '"打开仓库"'
replace "app/src/code_review/code_review_view.rs" '"No changes to commit"' '"没有更改可提交"'
replace "app/src/code_review/code_review_view.rs" '"No git actions available"' '"没有可用的 Git 操作"'
replace "app/src/code_review/code_review_view.rs" '"Push"' '"推送"'
replace "app/src/code_review/code_review_view.rs" '"Create PR"' '"创建 PR"'
replace "app/src/code_review/code_review_view.rs" '"Publish"' '"发布"'
replace "app/src/code_review/code_review_view.rs" '"Add file diff as context"' '"添加文件差异作为上下文"'
replace "app/src/code_review/code_review_view.rs" '"Copy file path"' '"复制文件路径"'
replace "app/src/code_review/code_review_view.rs" '"Add diff set as context"' '"添加差异集作为上下文"'
replace "app/src/code_review/code_review_view.rs" '"Show saved comment"' '"显示已保存的评论"'
replace "app/src/code_review/code_review_view.rs" '"Add comment"' '"添加评论"'
replace "app/src/code_review/code_review_view.rs" '"Discard all"' '"丢弃全部"'

echo ""
echo "=== Agent-5: Tab & Settings ==="
replace "app/src/tab.rs" '"Stop sharing"' '"停止共享"'
replace "app/src/tab.rs" '"Share session"' '"共享会话"'
replace "app/src/tab.rs" '"Stop sharing all"' '"停止全部共享"'
replace "app/src/tab.rs" '"Copy link"' '"复制链接"'
replace "app/src/tab.rs" '"Copy tab title"' '"复制标签标题"'
replace "app/src/tab.rs" '"Copy pane title"' '"复制窗格标题"'
replace "app/src/tab.rs" '"Copy branch"' '"复制分支"'
replace "app/src/tab.rs" '"Copy working directory"' '"复制工作目录"'
replace "app/src/tab.rs" '"Copy pull request link"' '"复制 Pull Request 链接"'
replace "app/src/tab.rs" '"Rename tab"' '"重命名标签"'
replace "app/src/tab.rs" '"Reset tab name"' '"重置标签名"'
replace "app/src/tab.rs" '"Close tab"' '"关闭标签"'
replace "app/src/tab.rs" '"Close other tabs"' '"关闭其他标签"'
replace "app/src/tab.rs" '"Save as new config"' '"另存为新配置"'
replace "app/src/tab.rs" '"New group with tab"' '"新建标签组"'
replace "app/src/tab.rs" '"Move to group"' '"移动到组"'
replace "app/src/tab.rs" '"Default (no color)"' '"默认（无颜色）"'
replace "app/src/tab.rs" '"Cloud agent run"' '"云端代理运行"'
replace "app/src/tab.rs" '"Move Tab Down"' '"向下移动标签"'
replace "app/src/tab.rs" '"Move Tab Right"' '"向右移动标签"'
replace "app/src/tab.rs" '"Move Tab Up"' '"向上移动标签"'
replace "app/src/tab.rs" '"Move Tab Left"' '"向左移动标签"'
replace "app/src/tab.rs" '"Close Tabs Below"' '"关闭下方标签"'
replace "app/src/tab.rs" '"Close Tabs Right"' '"关闭右侧标签"'

# banner
replace "app/src/banner/view.rs" '"Don'\''t show me again"' '"不再显示"'

# settings main page
replace "app/src/settings_view/main_page.rs" '"Sign up"' '"注册"'
replace "app/src/settings_view/main_page.rs" '"Free"' '"免费"'
replace "app/src/settings_view/main_page.rs" '"Compare plans"' '"比较套餐"'
replace "app/src/settings_view/main_page.rs" '"Contact support"' '"联系支持"'
replace "app/src/settings_view/main_page.rs" '"Manage billing"' '"管理账单"'
replace "app/src/settings_view/main_page.rs" '"Settings sync"' '"设置同步"'
replace "app/src/settings_view/main_page.rs" '"Refer a friend"' '"推荐好友"'
replace "app/src/settings_view/main_page.rs" '"Log out"' '"退出登录"'

# settings common
replace "app/src/settings_view/settings_page.rs" '"Reset to default"' '"重置为默认值"'

# features page
replace "app/src/settings_view/features_page.rs" '"Configure Global Hotkey"' '"配置全局热键"'
replace "app/src/settings_view/features_page.rs" '"Cancel"' '"取消"'
replace "app/src/settings_view/features_page.rs" '"Save"' '"保存"'

echo ""
echo "=== Agent-1: Additional app_menus changes ==="
replace "app/src/app_menus.rs" '"另存为新配置..."' '"另存为新配置..."'  # already applied

echo ""
echo "=== Settings View remaining files ==="
replace "app/src/settings_view/mod.rs" '"Billing and usage"' '"计费与用量"'
replace "app/src/settings_view/mod.rs" '"Keyboard shortcuts"' '"快捷键"'
replace "app/src/settings_view/mod.rs" '"Shared blocks"' '"共享块"'
replace "app/src/settings_view/mod.rs" '"MCP Servers"' '"MCP 服务器"'
replace "app/src/settings_view/mod.rs" '"Profiles"' '"配置文件"'
replace "app/src/settings_view/mod.rs" '"Knowledge"' '"知识库"'
replace "app/src/settings_view/mod.rs" '"Third party CLI agents"' '"第三方 CLI 代理"'
replace "app/src/settings_view/mod.rs" '"Indexing and projects"' '"索引与项目"'
replace "app/src/settings_view/mod.rs" '"Editor and Code Review"' '"编辑器与代码审查"'
replace "app/src/settings_view/mod.rs" '"Environments"' '"环境"'
replace "app/src/settings_view/mod.rs" '"Oz Cloud API Keys"' '"Oz Cloud API 密钥"'
replace "app/src/settings_view/mod.rs" '"BETA"' '"测试版"'

# about_page
replace "app/src/settings_view/about_page.rs" '"Copyright 2026 Warp"' '"版权所有 2026 Warp"'

# privacy_page
replace "app/src/settings_view/privacy_page.rs" '"Secret redaction"' '"机密信息脱敏"'
replace "app/src/settings_view/privacy_page.rs" '"Custom secret redaction"' '"自定义机密信息脱敏"'
replace "app/src/settings_view/privacy_page.rs" '"Add regex pattern"' '"添加正则表达式"'
replace "app/src/settings_view/privacy_page.rs" '"Personal"' '"个人"'
replace "app/src/settings_view/privacy_page.rs" '"Enterprise"' '"企业"'
replace "app/src/settings_view/privacy_page.rs" '"Recommended"' '"推荐"'
replace "app/src/settings_view/privacy_page.rs" '"Add all"' '"全部添加"'
replace "app/src/settings_view/privacy_page.rs" '"Add regex"' '"添加正则"'
replace "app/src/settings_view/privacy_page.rs" '"Send crash reports"' '"发送崩溃报告"'
replace "app/src/settings_view/privacy_page.rs" '"Manage your data"' '"管理你的数据"'
replace "app/src/settings_view/privacy_page.rs" '"Privacy policy"' '"隐私政策"'

# appearance_page
replace "app/src/settings_view/appearance_page.rs" '"Themes"' '"主题"'
replace "app/src/settings_view/appearance_page.rs" '"Window"' '"窗口"'
replace "app/src/settings_view/appearance_page.rs" '"Blocks"' '"区块"'
replace "app/src/settings_view/appearance_page.rs" '"Text"' '"文本"'
replace "app/src/settings_view/appearance_page.rs" '"Tabs"' '"标签页"'
replace "app/src/settings_view/appearance_page.rs" '"Cursor"' '"光标"'
replace "app/src/settings_view/appearance_page.rs" '"Icon"' '"图标"'
replace "app/src/settings_view/appearance_page.rs" '"Input"' '"输入"'
replace "app/src/settings_view/appearance_page.rs" '"Panes"' '"窗格"'

# terminal/shared_sessions.rs
replace "app/src/terminal/view/inline_banner/shared_sessions.rs" '"Today"' '"今天"'

echo ""
echo "=== Verify remaining unapplied changes ==="
