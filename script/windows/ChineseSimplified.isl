; Simplified Chinese installer messages for Warp's Windows installer.
; Missing messages fall back to Inno Setup's built-in defaults.

[LangOptions]
LanguageName=简体中文
LanguageID=$0804
LanguageCodePage=65001
DialogFontName=Microsoft YaHei UI
DialogFontSize=9
WelcomeFontName=Microsoft YaHei UI
WelcomeFontSize=14

[Messages]
SetupAppTitle=安装
SetupWindowTitle=安装 - %1
UninstallAppTitle=卸载
UninstallAppFullTitle=%1 卸载

InformationTitle=信息
ConfirmTitle=确认
ErrorTitle=错误

SetupLdrStartupMessage=这将安装 %1。是否继续？
SetupAlreadyRunning=安装程序已经在运行。
WindowsVersionNotSupported=此程序不支持你的 Windows 版本。
SetupAppRunningError=安装程序检测到 %1 当前正在运行。%n%n请关闭它的所有实例，然后点击“确定”继续，或点击“取消”退出。
UninstallAppRunningError=卸载程序检测到 %1 当前正在运行。%n%n请关闭它的所有实例，然后点击“确定”继续，或点击“取消”退出。

PrivilegesRequiredOverrideTitle=选择安装模式
PrivilegesRequiredOverrideInstruction=选择安装模式
PrivilegesRequiredOverrideText1=%1 可以为所有用户安装（需要管理员权限），也可以仅为你安装。
PrivilegesRequiredOverrideText2=%1 可以仅为你安装，也可以为所有用户安装（需要管理员权限）。
PrivilegesRequiredOverrideAllUsers=为所有用户安装(&A)
PrivilegesRequiredOverrideAllUsersRecommended=为所有用户安装（推荐）(&A)
PrivilegesRequiredOverrideCurrentUser=仅为我安装(&M)
PrivilegesRequiredOverrideCurrentUserRecommended=仅为我安装（推荐）(&M)

ExitSetupTitle=退出安装
ExitSetupMessage=安装尚未完成。如果现在退出，程序将不会被安装。%n%n你可以稍后再次运行安装程序完成安装。%n%n是否退出安装？

ButtonBack=< 上一步(&B)
ButtonNext=下一步(&N) >
ButtonInstall=安装(&I)
ButtonOK=确定
ButtonCancel=取消
ButtonYes=是(&Y)
ButtonYesToAll=全部是(&A)
ButtonNo=否(&N)
ButtonNoToAll=全部否(&O)
ButtonFinish=完成(&F)
ButtonBrowse=浏览(&B)...
ButtonWizardBrowse=浏览(&R)...
ButtonNewFolder=新建文件夹(&M)

SelectLanguageTitle=选择安装语言
SelectLanguageLabel=选择安装过程中使用的语言。

ClickNext=点击“下一步”继续，或点击“取消”退出安装程序。
BrowseDialogTitle=浏览文件夹
BrowseDialogLabel=在下方列表中选择一个文件夹，然后点击“确定”。
NewFolderName=新建文件夹

WelcomeLabel1=欢迎使用 [name] 安装向导
WelcomeLabel2=这将在你的计算机上安装 [name/ver]。%n%n建议你在继续前关闭其他应用程序。

WizardSelectDir=选择安装位置
SelectDirDesc=你想把 [name] 安装到哪里？
SelectDirLabel3=安装程序将把 [name] 安装到以下文件夹。
SelectDirBrowseLabel=如需继续，请点击“下一步”。如需选择其他文件夹，请点击“浏览”。
DiskSpaceGBLabel=至少需要 [gb] GB 可用磁盘空间。
DiskSpaceMBLabel=至少需要 [mb] MB 可用磁盘空间。

WizardSelectTasks=选择附加任务
SelectTasksDesc=你想执行哪些附加任务？
SelectTasksLabel2=选择安装 [name] 时要执行的附加任务，然后点击“下一步”。

WizardReady=准备安装
ReadyLabel1=安装程序已准备好在你的计算机上安装 [name]。
ReadyLabel2a=点击“安装”开始安装，或点击“上一步”查看或更改设置。
ReadyMemoDir=目标位置：
ReadyMemoType=安装类型：
ReadyMemoComponents=选择的组件：
ReadyMemoGroup=开始菜单文件夹：
ReadyMemoTasks=附加任务：

WizardInstalling=正在安装
InstallingLabel=请稍候，安装程序正在你的计算机上安装 [name]。

FinishedHeadingLabel=[name] 安装完成
FinishedLabelNoIcons=安装程序已完成在你的计算机上安装 [name]。
FinishedLabel=安装程序已完成在你的计算机上安装 [name]。可以通过已创建的快捷方式启动此应用程序。
ClickFinish=点击“完成”退出安装程序。
FinishedRestartLabel=要完成 [name] 的安装，必须重新启动计算机。是否立即重新启动？
FinishedRestartMessage=要完成 [name] 的安装，必须重新启动计算机。%n%n是否立即重新启动？

ConfirmUninstall=确定要完全移除 %1 及其所有组件吗？
UninstallOnlyOnWin64=此程序只能在 64 位 Windows 上卸载。
OnlyAdminCanUninstall=只有具备管理员权限的用户才能卸载此程序。
UninstallStatusLabel=请稍候，%1 正在从你的计算机中移除。
UninstalledAll=%1 已成功从你的计算机中移除。

CreateDesktopIcon=创建桌面图标(&D)
AdditionalIcons=附加图标：
LaunchProgram=启动 %1
