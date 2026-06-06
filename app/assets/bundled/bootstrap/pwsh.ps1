[Diagnostics.CodeAnalysis.SuppressMessageAttribute('PSUseApprovedVerbs', '', Scope = 'Function', Target = 'Warp-*', Justification = 'Warp-* functions are ours')]
param()

# Wrap things in a module to avoid cluttering the global scope. We assign it to '$null' to suppress
# the console output from creating the module.
# NOTE: If you do need a function to be global and also have access to variables in this scope, add
# the function name to the 'Export-ModuleMember' call at the end.
$null = New-Module -Name Warp-Module -ScriptBlock {
    # Byte sequence used to signal the start of an OSC for Warp JSON messages.
    $oscStart = "$([char]0x1b)]9278;"

    # Appended to $oscStart to signal that the following message is JSON-encoded.
    $oscJsonMarker = 'd'

    $oscParamSeparator = ';'

    # Byte used to signal the end of an OSC for Warp JSON messages.
    $oscEnd = "$([char]0x07)"

    # Writes a hex-encoded JSON message to the PTY.
    function Warp-Send-JsonMessage([System.Collections.Hashtable]$table) {
        $json = ConvertTo-Json -InputObject $table -Compress
        # Sends a message to the controlling terminal as an OSC control sequence.
        # TODO(CORE-2718): Determine if we need to hex encode the payload.
        # Note that because the JSON string may contain characters that we don't control (including
        # unicode), we encode it as hexadecimal string to avoid prematurely calling unhook if
        # one of the bytes in JSON is 9c (ST) or other (CAN, SUB, ESC).
        $encodedMessage = Warp-Encode-HexString $json
        Write-Host -NoNewline "$oscStart$oscJsonMarker$oscParamSeparator$encodedMessage$oscEnd"
    }

    # This script block contains commands and constants that are needed in background threads.
    # If you want to be able to use it in a background thread, stick it in this block
    $warpCommon = {
        # OSC used to mark the start of in-band command output.
        #
        # Printable characters received this OSC and oscEndGeneratorOutput are parsed and handled as
        # output for an in-band command.
        $oscStartGeneratorOutput = "$([char]0x1b)]9277;A$oscEnd"

        # OSC used to mark the end of in-band command output.
        #
        # Printable characters received between oscStartGeneratorOutput and this are parsed and
        # handled as output for an in-band command.
        $oscEndGeneratorOutput = "$([char]0x1b)]9277;B$oscEnd"

        $oscResetGrid = "$([char]0x1b)]9279$oscEnd"

        function Warp-Send-ResetGridOSC() {
            Write-Host -NoNewline $oscResetGrid
        }

        # Safely attempt to get Node.js version if available. Avoid literal 'node' invocation
        # to satisfy PSUseCompatibleCommands across target platforms.
        function Warp-TryGet-NodeVersion {
            try {
                $cmd = Get-Command -CommandType Application node 2>$null
                if ($null -eq $cmd) { return '' }
                $nv = & $cmd.Source --version 2>$null
                if ($null -ne $nv -and "$nv" -ne '') {
                    return $nv
                }
            } catch {
                # Log at verbose level so normal users are not spammed, but the catch is not empty.
                Write-Verbose "node --version failed: $($_.Exception.Message)"
            }
            return ''
        }

        # Encode a string as hex-encoded UTF-8.
        function Warp-Encode-HexString([string]$str) {
            [BitConverter]::ToString([System.Text.Encoding]::UTF8.GetBytes($str)).Replace('-', '')
        }

        # Hex-encodes the given argument and writes it to the PTY, wrapped in the OSC
        # sequences for generator output.
        #
        # The payload of the OSC is "<content_length>;<hex-encoded content>".
        function Warp-Send-GeneratorOutputOsc {
            param([string]$message)

            $hexEncodedMessage = Warp-Encode-HexString $message
            $byteCount = [System.Text.Encoding]::ASCII.GetByteCount($hexEncodedMessage)

            Write-Host -NoNewline "$oscStartGeneratorOutput$byteCount;$hexEncodedMessage$oscEndGeneratorOutput"
            Warp-Send-ResetGridOSC
        }

        # Do not run this in the main thread. It mucks around with some env vars
        function Warp-Run-InBandGenerator {
            [Diagnostics.CodeAnalysis.SuppressMessageAttribute('PSAvoidUsingInvokeExpression', '', Justification = 'We actually need it')]
            param([string]$commandId, [string]$command)

            try {
                # We do not have a good way to simultaneously capture
                # the command status $? and the command output of our command.
                # this is because Invoke-Expression will always set $? to true.
                # To get around this, we append a small bit of code to the original
                # command that makes Invoke-Expression throw if the last command
                # did not succeed.
                $modifiedCommand = "$command" + '; if (-Not $?) { throw }'

                # We set this immediately before running Invoke-Expression,
                # that way it will default to 0
                $LASTEXITCODE = 0

                # Note: parens are important here. Without them
                # parsing order gets messed up on the 2>&1
                $rawOutput = Invoke-Expression -Command "$modifiedCommand" 2>&1
                $exitCode = $LASTEXITCODE

                # If the generator command returns multi-line output,
                # we make sure to join the lines together with a newline, so
                # they are properly parsed by warp
                $stringifiedOutput = $rawOutput -join "$([char]0x0a)"

                # This is a best-effort attempt to get an error code.
                # We cannot duplicate our error code logic from Warp-Precmd
                # b/c Invoke-Expression will swallow the value of $? and always
                # return true. So we do our best to return a legit error code
                Write-Output "$commandId;$stringifiedOutput;$exitCode"
            } catch {
                # This catches a terminating error (ex: entering a command that does not exist)
                # In this case, we return an error code of 1
                Write-Output "$commandId;1;"
            }
        }
    }

    # Load the Warp Common functions in the current session
    . $warpCommon

    function Get-EpochTime {
        [decimal]([DateTime]::UtcNow - [DateTime]::new(1970, 1, 1, 0, 0, 0, 0)).Ticks / 1e7
    }

    function Warp-Bootstrapped {
        [Diagnostics.CodeAnalysis.SuppressMessageAttribute('PSUseDeclaredVarsMoreThanAssignments', 'WARP_BOOTSTRAPPED', Justification = 'False positive as we are assigning to global')]
        param([decimal]$rcStartTime, [decimal]$rcEndTime)

        $envVarNames = (Get-ChildItem env: | Select-Object -ExpandProperty Name | ForEach-Object { 'env:' + $_ }) + `
        (Get-Variable | Select-Object -ExpandProperty Name) -join ' '
        $aliasesRaw = Get-Command -CommandType Alias | Select-Object -ExpandProperty DisplayName
        $aliases = $aliasesRaw -join [Environment]::NewLine
        $functionNamesRaw = Get-Command -CommandType Function | Where-Object { -not $_.Name.StartsWith('Warp') } | Select-Object -ExpandProperty Name
        $functionNames = $functionNamesRaw -join [Environment]::NewLine
        $builtinsRaw = Get-Command -CommandType Cmdlet | Select-Object -ExpandProperty Name
        $builtins = $builtinsRaw -join [Environment]::NewLine
        $shellVersion = $PSVersionTable.PSVersion.ToString()
        # PowerShell wasn't cross-platform until version 6. Anything before that is definitely on Windows.
        $osCategory = if ($PSVersionTable.PSVersion.Major -le 5) {
            'Windows'
        } elseif ($IsLinux) {
            'Linux'
        } elseif ($IsMacOS) {
            'MacOS'
        } elseif ($IsWindows) {
            'Windows'
        } else {
            ''
        }

        # We do not have an equivalent to 'compgen -k' here, so we are dropping
        # in a hardcoded list. List is take from
        # https://learn.microsoft.com/en-us/powershell/module/microsoft.powershell.core/about/about_reserved_words?view=powershell-7.4
        $PSKeywords = @(
            'begin', 'break', 'catch', 'class', 'continue', 'data', 'define',
            'do', 'dynamicparam', 'else', 'elseif', 'end', 'enum', 'exit',
            'filter', 'finally', 'for', 'foreach', 'from', 'function', 'hidden',
            'if', 'in', 'param', 'process', 'return', 'static', 'switch', 'throw',
            'trap', 'try', 'until', 'using', 'var', 'while', 'inlinescript',
            'parallel', 'sequence', 'workflow'
        ) -join [environment]::NewLine

        $linuxDistribution = $null
        if ($osCategory -eq 'Linux') {
            $osReleaseFile = if (Test-Path -Path '/etc/os-release') {
                '/etc/os-release'
            } elseif (Test-Path -Path '/usr/lib/os-release') {
                '/usr/lib/os-release'
            } else {
                $null
            }
            if ($null -ne $osReleaseFile) {
                # This is meant to be the equivalent to the bash command
                # cat $os_release_file | sed -nE 's/^NAME="(.*)"$/\1/p'. We filter
                # specifically for the Name= line of the osRelease file, and then
                # pull out the OS name
                $linuxDistribution = switch -Regex -File $osReleaseFile {
                    '^\s*NAME="(.*)"' {
                        $Matches[1]
                        break
                    }
                }
            }
        }

        # TODO(PLAT-681) - finish the information here
        # for keywords, see 'Get-Help about_Language_Keywords'
        $bootstrappedMsg = @{
            hook = 'Bootstrapped'
            value = @{
                histfile = $(Get-PSReadLineOption).HistorySavePath
                shell = 'pwsh'
                home_dir = "$HOME"
                path = $env:PATH
                editor = "$env:EDITOR"
                env_var_names = $envVarNames
                abbreviations = ''
                aliases = $aliases
                function_names = $functionNames
                builtins = $builtins
                keywords = "$PSKeywords"
                shell_version = $shellVersion
                shell_options = ''
                rcfiles_start_time = "$rcStartTime"
                rcfiles_end_time = "$rcEndTime"
                shell_plugins = ''
                os_category = $osCategory
                linux_distribution = "$linuxDistribution"
                shell_path = (Get-Process -Id $PID).Path
            }
        }
        Warp-Send-JsonMessage $bootstrappedMsg
        $global:WARP_BOOTSTRAPPED = 1
    }

    function Warp-Preexec([string]$command) {
        $HOST.UI.RawUI.WindowTitle = $command
        $preexecMsg = @{
            hook = 'Preexec'
            value = @{
                command = $command
            }
        }
        Warp-Send-JsonMessage $preexecMsg
        Warp-Send-ResetGridOSC

        # If this preexec is called for user command, kill ongoing generator command jobs and clean
        # up the bookkeeping temp files used to bookkeep.
        if (-not "$command" -match '^Warp-Run-GeneratorCommand') {
            Warp-Stop-ActiveThread
        }

        # Clean up any completed warp jobs so they do not show up on the user's 'get-job'
        # commands
        Warp-Clean-CompletedThread

        # Remove any instance of the 'Warp-Run-GeneratorCommand' call from the user's history
        Clear-History -CommandLine 'Warp-Run-GeneratorCommand*'
    }

    function Warp-Finish-Update([string]$updateId) {
        $updateMsg = @{
            hook = 'FinishUpdate'
            value = @{
                update_id = $updateId
            }
        }
        Warp-Send-JsonMessage $updateMsg
    }

    function Warp-Handle-DistUpgrade {
        [Diagnostics.CodeAnalysis.SuppressMessageAttribute('PSAvoidUsingInvokeExpression', '', Justification = 'We actually need it')]
        param([string]$sourceFileName)

        $aptConfig = Get-Command -Type Application apt-config | Select-Object -First 1
        & $aptConfig shell '$aptSourcesDir' 'Dir::Etc::sourceparts/d' | Invoke-Expression

        $sourceFilePath = "${aptSourcesDir}${sourceFileName}"

        if (
            -not (Test-Path "${sourceFilePath}.list") -and
            -not (Test-Path "${sourceFilePath}.sources") -and
            (Test-Path "${sourceFilePath}.list.distUpgrade")
        ) {
            # DO NOT DO THIS. We should never run a command for user with 'sudo'. The only reason this
            # is safe here is because we insert this function into the input for the user to determine
            # if they want to execute (we never run it on their behalf without their permission).
            sudo cp "${sourceFilePath}.list.distUpgrade" "${sourceFilePath}.list"
        }
    }

    # We need this for a few reasons
    # 1. We need to make sure the environment variable GIT_OPTIONAL_LOCKS=0.
    #    See https://stackoverflow.com/questions/71836872/git-environment-variables-on-powershell-on-windows
    #    for why this is complicated
    # 2. We need to make sure that we are calling the Application git, and not
    #    an alias or cmdlet named Git
    #
    # NOTE: Inlining this call in the function has a weird side effect of outputting
    #    an escape sequence '^[i'. Since it made it more convenient to have a wrapper
    #    function anyway, I have not investigated this, but in case someone is working
    #    on this in the future, beware attempting to inline this function.
    function Warp-Git {
        $GIT_OPTIONAL_LOCKS = $env:GIT_OPTIONAL_LOCKS
        $env:GIT_OPTIONAL_LOCKS = 0
        try {
            &(Get-Command -CommandType Application git | Select-Object -First 1) $args
        } finally {
            $env:GIT_OPTIONAL_LOCKS = $GIT_OPTIONAL_LOCKS
        }
    }

    # Helper function that resets the values of '$?' and
    # $LASTEXITCODE. Note that it cannot force '$?' to $true
    # if it is currently $false
    #
    # Make sure when you call this you call it with -ErrorAction SilentlyContinue
    # or it will print out error information when it is invoked.
    function Warp-Restore-ErrorStatus {
        [CmdletBinding()]
        param([boolean]$status, [int]$code)

        $global:LASTEXITCODE = $code
        if ($status -eq $false) {
            $PSCmdlet.WriteError([System.Management.Automation.ErrorRecord]::new(
                    [Exception]::new("$([char]0x00)"),
                    'warp-reset-error',
                    [System.Management.Automation.ErrorCategory]::NotSpecified,
                    $null
                ))
        }
    }

    # Tracks whether or not powershell is unable to find a command.
    # See the $ExecutionContext.InvokeCommand.CommandNotFoundAction where it is set to $true,
    # and both $ExecutionContext.InvokeCommand.PostCommandLookupAction and Warp-Precmd where
    # it is set to $false.
    $script:commandNotFound = $false

    function Warp-Configure-PSReadLine {
        # Set-PSReadLineKeyHandler is the PowerShell equivalent of zsh's bindkey.
        Set-PSReadLineKeyHandler -Chord 'Alt+2' -Function BackwardDeleteLine

        # Input reporting. Note that ESC-1 is used instead of ESC-i as for all other shells. This
        # is because PowerShell on Windows does some virtual key code translation which depends on
        # the selected input language. On languages without an "i" on any key, this translation
        # fails and the binding gets dropped.
        Set-PSReadLineKeyHandler -Chord 'Alt+1' -ScriptBlock {
            $inputBuffer = $null
            $cursorPosition = $null
            [Microsoft.PowerShell.PSConsoleReadLine]::GetBufferState([ref]$inputBuffer, [ref]$cursorPosition)
            $inputBufferMsg = @{
                hook = 'InputBuffer'
                value = @{
                    buffer = $inputBuffer
                }
            }
            Warp-Send-JsonMessage $inputBufferMsg
            [Microsoft.PowerShell.PSConsoleReadLine]::BackwardDeleteLine()
            # This is triggered after precmd, so output here goes to the "early output" handler,
            # i.e. the background block. This clears the line the cursor is on. We clear it out b/c
            # at this point, the only stuff in the early output handler is typeahead, and that
            # shouldn't be displayed in a background block at all. It should be in the input
            # editor. Most shells will automatically emit the correct ANSI escape codes to delete
            # the contents of the early output handler when we kill the line editor's buffer.
            # However, PowerShell doesn't do this correctly due to cursor position mismatch. So,
            # we do it manually here instead.
            Write-Host -NoNewline "$([char]0x1b)[2K"
        }

        # Sets the prompt mode to custom prompt (PS1)
        # Is the equivalent of warp_change_prompt_modes_to_ps1 in other shells
        Set-PSReadLineKeyHandler -Chord 'Alt+p' -ScriptBlock {
            $env:WARP_HONOR_PS1 = '1'
            Warp-Redraw-Prompt
        }

        # Sets the prompt mode to warp prompt
        # Is the equivalent of warp_change_prompt_modes_to_warp_prompt in other shells
        Set-PSReadLineKeyHandler -Chord 'Alt+w' -ScriptBlock {
            $env:WARP_HONOR_PS1 = '0'
            Warp-Redraw-Prompt
        }

        Set-PSReadLineOption -AddToHistoryHandler {
            param([string]$line)

            if ($line -match '^Warp-Run-GeneratorCommand') {
                return $false
            }
            return $true
        }

        Warp-Disable-PSPrediction
    }

    # Force use of the Inline PredictionViewStyle. The ListView style can occasionally cause some
    # flickering when using Warp and it doesn't matter what the value of this setting is because
    # Warp has its own input editor.
    function Warp-Disable-PSPrediction {
        [Diagnostics.CodeAnalysis.SuppressMessageAttribute('PSUseCompatibleCommands', '', Justification = 'Errors are ignored')]
        [Diagnostics.CodeAnalysis.SuppressMessageAttribute('PSAvoidUsingEmptyCatchBlock', '', Justification = 'Errors expected')]
        param()
        try {
            Set-PSReadLineOption -PredictionSource None
            Set-PSReadLineOption -PredictionViewStyle InlineView
        } catch {
        }
    }

    function Warp-Precmd {
        [Diagnostics.CodeAnalysis.SuppressMessageAttribute('PSAvoidUsingPositionalParameters', '', Justification = 'Warp-Git should use positionals')]
        param([bool]$status, [int]$code)
        # Our logic here is:
        #
        # if $status == True, always set $exitCode to 0
        # if $status == False and $script:commandNotFound is true
        #     (meaning we triggered the CommandNotFoundHandler), set $exitCode to 127
        # if $status == False and $LASTEXITCODE is zero, set $exitCode to 1
        # else set $exitCode to $LASTEXITCODE
        #
        # Note that this is not going to be 100% accurate, as some cmdlets will fail
        # without setting a $LASTEXITCODE, meaning the $LASTEXITCODE will be stale.
        $warpCommandNotFound = $script:commandNotFound
        $script:commandNotFound = $false

        $exitCode = if ($status) {
            0
        } elseif ($warpCommandNotFound) {
            127
        } elseif ($code -eq 0) {
            1
        } else {
            $code
        }

        $newTitle = (Get-Location).Path
        # Replace the literal home dir with a tilde.
        if ($newTitle.StartsWith($HOME)) {
            $newTitle = '~' + $newTitle.Substring($HOME.length)
        }
        $HOST.UI.RawUI.WindowTitle = $newTitle

        $blockId = $script:nextBlockId++
        $commandFinishedMsg = @{
            hook = 'CommandFinished'
            value = @{
                exit_code = $exitCode
                next_block_id = "precmd-${global:_warpSessionId}-$blockId"
            }
        }
        Warp-Send-JsonMessage $commandFinishedMsg
        Warp-Send-ResetGridOSC

        Warp-Configure-PSReadLine

        # If this is being called for a generator command, short circuit and send an unpopulated
        # precmd payload (except for pwd), since we don't re-render the prompt after generator commands
        # are run.
        if ($script:generatorCommand -eq $true) {
            # TODO(CORE-2639): handle user PreCmds here

            $script:generatorCommand = $false

            $precmdMsg = @{
                hook = 'Precmd'
                value = @{
                    pwd = ''
                    ps1 = ''
                    git_head = ''
                    git_branch = ''
                    virtual_env = ''
                    conda_env = ''
                    session_id = $global:_warpSessionId
                    is_after_in_band_command = $true
                }
            }
            Warp-Send-JsonMessage $precmdMsg
        } else {
            # TODO(CORE-2678): Figure out resetting bindkeys here

            $virtualEnv = ''
            $condaEnv = ''
            $kubeConfig = ''
            $gitHead = ''
            $gitBranch = ''
            $nodeVersion = ''

            # Only fill these fields once we've finished bootstrapping, as the
            # blocks created during the bootstrap process don't have visible
            # prompts, and we don't want to invoke 'git' before we've sourced the
            # user's rcfiles and have a fully-populated PATH.
            if ($global:WARP_BOOTSTRAPPED -eq 1) {
                if (Test-Path env:VIRTUAL_ENV) {
                    $virtualEnv = $env:VIRTUAL_ENV
                }
                if (Test-Path env:CONDA_DEFAULT_ENV) {
                    $condaEnv = $env:CONDA_DEFAULT_ENV
                }
                if (Test-Path env:KUBECONFIG) {
                    $kubeConfig = $env:KUBECONFIG
                }

                # Compute Node.js version if node is available and we're in a Node project within a Git repo.
                $hasNodeCommand = Get-Command -CommandType Application node 2>$null
                if ($hasNodeCommand) {
                    try {
                        # Walk up from the current directory to find a package.json
                        $dir = Get-Item -LiteralPath (Get-Location).Path
                        $foundPackageJson = $false
                        $packageJsonDir = $null
                        while ($null -ne $dir) {
                            $candidate = Join-Path $dir.FullName 'package.json'
                            if (Test-Path -LiteralPath $candidate) {
                                $foundPackageJson = $true
                                $packageJsonDir = $dir.FullName
                                break
                            }
                            $dir = $dir.Parent
                        }

                        if ($foundPackageJson) {
                            # Verify package.json resides within a Git repository by walking up to find a .git directory
                            $probe = Get-Item -LiteralPath $packageJsonDir
                            $inGitRepo = $false
                            while ($null -ne $probe) {
                                if (Test-Path -LiteralPath (Join-Path $probe.FullName '.git')) {
                                    $inGitRepo = $true
                                    break
                                }
                                $probe = $probe.Parent
                            }

                            if ($inGitRepo) {
                                $nodeVersion = Warp-TryGet-NodeVersion
                            }
                        }
                    } catch {
                        # Log at verbose level so the catch block is not empty and diagnostics are available when needed.
                        Write-Verbose "Failed to compute Node.js context: $($_.Exception.Message)"
                    }
                }

                # We do not inline $hasGitCommand b/c the linter does not like seeing '>'
                # in an if statement; it thinks we are trying to do -gt incorrectly.
                # Since this is a good warning and we do not want to turn off this lint rule,
                # we do a little indirection here
                $hasGitCommand = Get-Command -CommandType Application git 2>$null
                if ($hasGitCommand) {
                    # This is deliberately not using || b/c || only works in Powershell >=7
                    $gitBranchTmp = Warp-Git symbolic-ref --short HEAD 2>$null
                    if ($null -ne $gitBranchTmp) {
                        $gitBranch = $gitBranchTmp
                        $gitHead = $gitBranchTmp
                    } else {
                        $gitHeadTmp = Warp-Git rev-parse --short HEAD 2>$null
                        if ($null -ne $gitHeadTmp) {
                            $gitHead = $gitHeadTmp
                        }
                    }
                }
            }

            $honor_ps1 = "$env:WARP_HONOR_PS1" -eq '1'

            $precmdMsg = @{
                hook = 'Precmd'
                value = @{
                    pwd = (Get-Location).Path
                    # TODO(PLAT-687) - honor the PS1
                    ps1 = ''
                    honor_ps1 = $honor_ps1
                    # TODO(PLAT-687) - pwsh does not by default support rprompt, but
                    # oh-my-posh does. If there is a way to easily extract the oh-my-posh
                    # rprompt, we might want to use it here
                    rprompt = ''
                    git_head = $gitHead
                    git_branch = $gitBranch
                    virtual_env = $virtualEnv
                    conda_env = $condaEnv
                    node_version = $nodeVersion
                    session_id = $global:_warpSessionId
                    kube_config = $kubeConfig
                }
            }
            Warp-Send-JsonMessage $precmdMsg
        }
    }

    $script:inBandCommandCount = 0
    $script:threadInner = @{}
    $script:threadOuter = @{}

    # The inner runspace pool maintains a pool of runspaces that can execute
    # arbitrary commands against the user's current environment without
    # writing to the screen. Initialize to minimum of 10 runspaces
    # to handle double the number of context chips we currently have
    # that use in-band commands
    $script:innerRunspacePool = [runspacefactory]::CreateRunspacePool(10, 20)
    $script:innerRunspacePool.ApartmentState = [System.Threading.ApartmentState]::STA
    $script:innerRunspacePool.ThreadOptions = 'ReuseThread'
    $script:innerRunspacePool.Open() | Out-Null

    # The outer runspace pool maintains a pool of runspaces that
    # share the same host as the user's session. This allows them
    # to send OSC commands via Write-Host. These outer runspaces
    # handle receiving results from the inner runspaces and formatting
    # those results into OSCs.
    # Initialized to minimum of 5 runspaces since we currently do not
    # run more than one outer command at a time.
    $script:outerRunspacePool = [runspacefactory]::CreateRunspacePool(5, 10, $Host)
    $script:outerRunspacePool.ApartmentState = [System.Threading.ApartmentState]::STA
    $script:outerRunspacePool.ThreadOptions = 'ReuseThread'
    $script:outerRunspacePool.Open() | Out-Null

    class WarpGeneratorCommand {
        [string]$CommandId
        [string]$Command
    }

    function Warp-Run-GeneratorCommandImpl {
        param(
            [WarpGeneratorCommand[]]$commands
        )

        $jobNumber = $script:inBandCommandCount++

        $batchNumber = 0
        $jobs = $commands | ForEach-Object {
            $commandId = $_.CommandId
            $command = $_.Command

            # Creates a powershell instance on one of our inner runspaces
            # that first loads all the warp common functions, and then
            # executes the in-band generator in the current directory
            $ps = [powershell]::Create()
            $ps.RunspacePool = $script:innerRunspacePool
            $ps.AddScript($warpCommon) | Out-Null
            $ps.AddScript({
                    param([string]$loc, [string]$commandId, [string]$command)
                    Set-Location $loc
                    Warp-Run-InBandGenerator -commandId $commandId -command "$command"
                }).AddParameters(@($PWD.Path, $commandId, "$command")) | Out-Null

            $script:threadInner["Warp-Inner-$jobNumber-$batchNumber"] = $psInner
            $batchNumber++

            @{
                commandId = $commandId
                ps = $ps
            }
        }

        # Creates the outer job, which waits on all the inner jobs
        # and then sends the results back to Warp via OSC
        $psOuter = [powershell]::Create()
        $psOuter.RunspacePool = $script:outerRunspacePool
        $psOuter.AddScript($warpCommon) | Out-Null
        $psOuter.AddScript({
                param([object[]]$jobs)

                $invocations = $jobs | ForEach-Object {
                    @{
                        commandId = $_.commandId
                        ps = $_.ps
                        async = $_.ps.BeginInvoke()
                    }
                }

                $invocations | ForEach-Object {
                    $commandId = $_.commandId
                    $ps = $_.ps
                    $async = $_.async

                    $output = "$commandId;1;"

                    try {
                        $output = $ps.EndInvoke($async)
                    } catch {
                        $output = "$commandId;1;"
                    }
                    Warp-Send-GeneratorOutputOsc $output
                }
            }).AddParameters(@($jobs)) | Out-Null

        # Note: we are beginning the invocation, but are explicitly
        # not stopping it as we do not want to block the main thread.
        $async = $psOuter.BeginInvoke()

        $script:threadOuter["Warp-Outer-$jobNumber"] = $psOuter
    }

    function Warp-Stop-ActiveThread {
        $script:threadInner.values | ForEach-Object {
            $_.Stop()
        }
    }

    function Warp-Clean-CompletedThread {
        # Powershell instances states > 2 are terminal.
        # See https://learn.microsoft.com/en-us/dotnet/api/system.management.automation.psinvocationstate
        if ($script:threadInner.Count -gt 0) {
            $script:threadInner.Keys.Clone() | ForEach-Object {
                $thread = $script:threadInner[$_]
                $state = [int]$thread.InvocationStateInfo.State
                if ($state -gt 2) {
                    $thread.Dispose()
                    $script:threadInner.Remove($_)
                }
            }
        }
        if ($script:threadOuter.Count -gt 0) {
            $script:threadOuter.Keys.Clone() | ForEach-Object {
                $thread = $script:threadOuter[$_]
                $state = [int]$thread.InvocationStateInfo.State
                if ($state -gt 2) {
                    $thread.Dispose()
                    $script:threadOuter.Remove($_)
                }
            }
        }
    }

    function Warp-Run-GeneratorCommand {
        [CmdletBinding()]
        param(
            [parameter(ValueFromRemainingArguments = $true)][string[]]$passedArgs
        )

        $status = $?
        $code = $global:LASTEXITCODE

        # Setting this environment variable prevents warp_precmd from emitting the
        # 'Block started' hook to the Rust app.
        $script:generatorCommand = $true

        # TODO(CORE-2639) If we ever start supporting user precmd or preexec
        # (which doesn't really exist in powershell, but :shrug:), we need
        # to properly handle them here like we do in bashzshfish

        # Converts the passed in args to WarpGeneratorCommand objects to group them together
        # note that if an odd number of arguments is passed in, the last arg will be silently ignored
        [WarpGeneratorCommand[]] $jobs = @()
        for ($i = 0; $i -lt $passedArgs.Length; $i += 2) {
            $commandId = $passedArgs[$i]
            $command = $passedArgs[$i + 1]

            if ($null -ne $command) {
                $jobs += [WarpGeneratorCommand]@{
                    commandId = $commandId
                    command = $command
                }
            }
        }

        try {
            Warp-Run-GeneratorCommandImpl -commands $jobs
        } finally {
            # NOTE: for some reason the Warp-Restore-ErrorStatus does not work
            # for this function, so we are inlining it in here.
            $global:LASTEXITCODE = $code
            if ($status -eq $false) {
                $PSCmdlet.WriteError([System.Management.Automation.ErrorRecord]::new(
                        [Exception]::new("$([char]0x00)"),
                        'warp-reset-error',
                        [System.Management.Automation.ErrorCategory]::NotSpecified,
                        $null
                    ))
            }
        }

    }

    function Warp-Render-Prompt {
        param([bool]$status, [int]$code, [bool]$isGeneratorCommand)

        # If this is a generator command, we do not want to recompute
        # the prompt, and instead want to return the original prompt.
        if ($isGeneratorCommand) {
            return $script:lastRenderedPrompt
        }

        # Reset error code for computing prompt
        $global:LASTEXITCODE = $code
        if (-not $status) {
            # Set's $? to false for the next function call,
            # so it can be used for computing the prompt
            Write-Error '' -ErrorAction Ignore
        }

        # Compute prompt and cache it as the last rendered prompt
        $basePrompt = & $global:_warpOriginalPrompt
        $script:lastRenderedPrompt = $basePrompt

        return $basePrompt
    }

    function Warp-Decorate-Prompt {
        param([string]$basePrompt)

        $e = "$([char]0x1b)"

        # Wrap prompt in Prompt Marker OSCs
        $startPromptMarker = "$e]133;A$oscEnd"
        $startRPromptMarker = "$e]133;P;k=r$oscEnd"
        if ("$env:WARP_HONOR_PS1" -eq '0') {
            $endPromptMarker = "$e]133;B$oscEnd$oscResetGrid"
        } else {
            $endPromptMarker = "$e]133;B$oscEnd"
        }
        $decoratedPrompt = "$basePrompt"

        # We only redecorate the prompt if it is not already decorated
        if (-not ($basePrompt -match '^\x1b]133;A')) {
            $decoratedPrompt = "$startPromptMarker$basePrompt$endPromptMarker"
            # Special case for ohmyposh that prints an rprompt. If it matches the format of ohmyposh
            # rprompt, we properly parse it into lprompt and rprompt
            if ($basePrompt -match '(?<lprompt>.*)[\x1b]7\s*(?<rprompt>\S.*)[\x1b]8') {
                $lprompt = $Matches.lprompt
                $rprompt = $Matches.rprompt
                $decoratedPrompt = "$startPromptMarker$lprompt$endPromptMarker${e}7$startRPromptMarker$rprompt$endPromptMarker${e}8"
            }
        }

        return $decoratedPrompt
    }

    $script:dontRunPrecmdForPrompt = $false
    # Redraws the prompt. Since our prompt also triggers the precmd hook
    # we need to signal that we do not want that to happen
    function Warp-Redraw-Prompt {
        param()

        $y = $Host.UI.RawUI.CursorPosition.Y
        $script:dontRunPrecmdForPrompt = $true
        try {
            [Microsoft.PowerShell.PSConsoleReadLine]::InvokePrompt($null, $y)
        } finally {
            $script:dontRunPrecmdForPrompt = $false
        }
    }

    function Warp-Prompt {
        param()

        # We need to capture all the data related to exit codes and such
        # as soon as possible for a few reasons
        # 1. We need to make sure that these values are as fresh as possible
        #    and are not impacted by our Warp- functions
        # 2. After we finish running Warp-Precmd and Warp-Render-Prompt, we want to set these values
        #    back to what they were originally
        $status = $?
        $code = $LASTEXITCODE
        $isGeneratorCommand = [bool]($script:generatorCommand -eq $true)

        if ($script:dontRunPrecmdForPrompt -ne $true) {
            Warp-Precmd -status $status -code $code
        }

        $script:preexecHandled = $false

        $renderedPrompt = Warp-Render-Prompt -status $status -code $code -isGeneratorCommand $isGeneratorCommand
        $decoratedPrompt = Warp-Decorate-Prompt -basePrompt $renderedPrompt
        $extraLines = ($decoratedPrompt -split "$([char]0x0a)").Length - 1
        Set-PSReadLineOption -ExtraPromptLineCount $extraLines

        # NOTE: Because we are in the prompt, we do not need to reset
        # the $? automatic variable (apparently $prompt does not impact it).
        # However, we do need to reset the LASTEXITCODE. If we ever refactor
        # this to not use the prompt, then watch out for $?
        $global:LASTEXITCODE = $code

        return $decoratedPrompt
    }

    if ((Test-Path env:WARP_INITIAL_WORKING_DIR) -and -not [String]::IsNullOrEmpty($env:WARP_INITIAL_WORKING_DIR)) {
        Set-Location $env:WARP_INITIAL_WORKING_DIR 2> $null
        Remove-Item -Path env:WARP_INITIAL_WORKING_DIR
    }

    # In some cases, the Clear-Host command will not interface properly with the blocklist.
    # Clear-Host defers to whatever the 'clear' command is defined, and if that command
    # is not set up to work with Warp (or has funky other behaviors) it can cause problems.
    #
    # Specific examples:
    # - The default /usr/bin/clear on mac creates a giant, empty block to clear content
    #   off of the screen.
    # - if miniconda is installed on an osx system, the miniconda 'clear' command will be
    #   invoked for 'Clear-Host', which does not play with Warp and winds up doing nothing.

    # Because of the above, we explicitly override both 'Clear-Host' and 'clear' to
    # instead send a DCS command to Warp instructing it to clear the blocklist.
    # We are explicitly NOT calling the underlying clear implementation:
    # 1. B/c traditional clear sends an escape sequence that ends up creating an
    #    empty block that is the full height of the screen.
    # 1. B/c our other bootstrap scripts (bash, zsh, fish) do not.

    # If we ever want to call the underlying clear command, we could do so by:
    # 1. Capturing it with '$_warp_original_clear = (Get-Command Clear-Host).Definition'
    # 2. Invoking it with 'Invoke-Expression $_warp_original_clear'

    # TODO(PLAT-781): On windows, these two functions should both clear the visible screen
    # AND the scrollback
    function Clear-Host() {
        $inputBufferMsg = @{
            hook = 'Clear'
            value = @{}
        }
        Warp-Send-JsonMessage $inputBufferMsg
    }

    function clear() {
        $inputBufferMsg = @{
            hook = 'Clear'
            value = @{}
        }
        Warp-Send-JsonMessage $inputBufferMsg
    }

    function Warp-Finish-Bootstrap {
        param([decimal]$rcStartTime, [decimal]$rcEndTime)
        # This is the closest we can get in PowerShell to a proper preexec hook. We wrap the
        # invocation of PSConsoleHostReadline, and call our preexec hook before returning the
        # returned value. This allows us to preserve the any custom implementations of
        # PSConsoleHostReadLine.
        $script:oldPSConsoleHostReadLine = $function:global:PSConsoleHostReadLine
        $function:global:PSConsoleHostReadLine = {
            $line = & $script:oldPSConsoleHostReadLine

            Warp-Preexec "$line"

            $line
        }

        # This handles the case when a command is not found (ex "ehco foo"). As long as it is a
        # user-executed command, we set the $script:commandNotFound variable to $true, so we know
        # that the command failed b/c of a command lookup failure.
        $ExecutionContext.InvokeCommand.CommandNotFoundAction = {
            $commandLine = $MyInvocation.Line
            # Only trigger the preexec hook for user-submitted commands
            # $EventArgs.CommandOrigin is either 'Runspace' or 'Internal'. Internal commands are run
            # automatically by PowerShell internals. Runspace is for user-submitted/configured stuff.
            # However, Runspace still includes stuff like the prompt function, PostCommandLookupAction,
            # and the stuff we set during this bootstrap. So, add a condition to prevent preexec from
            # triggering in those cases. Note that we prefix our own functions with the "Warp-" prefix
            # so that we can ignore them here.
            if ($EventArgs.CommandOrigin -ne 'Runspace' -or ($commandLine -match '^prompt$|^Warp-')) {
                return
            }
            $script:commandNotFound = $true
        }

        # This sets up our wrapper around $function:prompt, which runs the precmd hook
        # and computes the user's custom prompt.
        $function:global:prompt = (Get-Command Warp-Prompt).ScriptBlock
        Warp-Bootstrapped -rcStartTime $rcStartTime -rcEndTime $rcEndTime
    }

    ###########################################################
    # NOTE: NO non-bootstrap / non-user calls below this line #
    ###########################################################

    # Send a precmd message to the terminal to differentiate between the warp
    # bootstrap logic pasted into the PTY and the output of shell startup files.
    Warp-Precmd -status $global:? -code $global:LASTEXITCODE

    Export-ModuleMember -Function clear, Clear-Host, Get-EpochTime, Warp-Finish-Update, Warp-Handle-DistUpgrade, Warp-Run-GeneratorCommand, Warp-Finish-Bootstrap
}

# Finally, get ready to source the user's RC files. This must be done in the global scope (not
# inside Warp-Module) in order to obey the expected scoping in PowerShell's typical startup process.
. {
    $rcStartTime = Get-EpochTime
    # Source the user's RC files
    # https://learn.microsoft.com/en-us/powershell/module/microsoft.powershell.core/about/about_profiles?view=powershell-7.4#profile-types-and-locations
    # https://learn.microsoft.com/en-us/powershell/module/microsoft.powershell.core/about/about_profiles?view=powershell-7.4#the-profile-variable
    foreach ($file in @($PROFILE.AllUsersAllHosts, $PROFILE.AllUsersCurrentHost, $PROFILE.CurrentUserAllHosts, $PROFILE.CurrentUserCurrentHost)) {
        if ([System.IO.File]::Exists($file)) {
            try {
                . $file
            } catch {
                Write-Host -ForegroundColor Red $_.InvocationInfo.PositionMessage
                Write-Host -ForegroundColor DarkRed $_.Exception
            }
        }
    }

    # Append additional PATH entries if provided via WARP_PATH_APPEND.
    # This happens after we source RC files in case they reset PATH.
    if (-not [String]::IsNullOrEmpty($env:WARP_PATH_APPEND)) {
        $env:PATH = '{0}{1}{2}' -f $env:PATH, [IO.Path]::PathSeparator, $env:WARP_PATH_APPEND
        Remove-Item -Path env:WARP_PATH_APPEND
    }

    # This is a workaround for oh-my-posh's "transient prompt" feature. When enabled, it causes the
    # whole screen to clear on every command execution. It is implemented by overwriting the Enter
    # and ctrl-c key handlers. Resetting those back to default effectively disables it.
    # TODO(CORE-3234) - Find a workaround which allows transient prompt to work.
    $enterHandler = Get-PSReadLineKeyHandler | Where-Object -Property Key -EQ -Value 'Enter'
    if ($enterHandler -ne $null -and $enterHandler.Function -eq 'OhMyPoshEnterKeyHandler') {
        Set-PSReadLineKeyHandler -Chord Enter -Function AcceptLine
    }
    $ctrlcHandler = Get-PSReadLineKeyHandler | Where-Object -Property Key -EQ -Value 'Control+c'
    if ($ctrlcHandler -ne $null -and $ctrlcHandler.Function -eq 'OhMyPoshCtrlCKeyHandler') {
        Set-PSReadLineKeyHandler -Chord 'Control+c' -Function CopyOrCancelLine
    }

    $rcEndTime = Get-EpochTime

    # Capture the current prompt (potentially modified by a profile),
    # and then reset the prompt to our current noop prompt.
    $global:_warpOriginalPrompt = $function:global:prompt

    Warp-Finish-Bootstrap -rcStartTime $rcStartTime -rcEndTime $rcEndTime
    Remove-Variable -Name enterHandler, ctrlcHandler, rcStartTime, rcEndTime -Scope global -ErrorAction Ignore

    # Restore the process's original execution policy now that the user's RC files have been loaded.
    if ($global:_warp_PSProcessExecPolicy -ne $null) {
        Set-ExecutionPolicy -Scope Process -ExecutionPolicy $global:_warp_PSProcessExecPolicy
    }
}
