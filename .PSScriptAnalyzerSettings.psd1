@{
    Severity = @(
        'Error'
        'Warning'
        'Information'
    )
    CustomRulePath = @(
        'PSScriptAnalyzerCustomRules.psm1'
    )
    IncludeDefaultRules = $true
    ExcludeRules = @(
        # In an ideal world we'd keep this on and have people opt out on a variable
        # by variable basis, but PSScriptAnalyzer does not have that degree of control.
        'PSAvoidGlobalVars'
        # Normally we'd disable this in-line, but there are issues with using inline
        # diagnostic controls in pwsh_init_shell.ps1
        'PSAvoidUsingWriteHost'
        # TODO(CORE-2985): Evaluate if we want to turn this on.
        # Disabling this for now, most of our Warp functions should not be invoked by
        # users directly anyway.
        'PSProvideCommentHelp'
    )
    Rules = @{
        'Test-StringEscapeCode' = @{
            Enable = $true
        }
        PSAvoidExclaimOperator = @{
            Enable = $true
        }

        PSAvoidSemicolonsAsLineTerminators = @{
            Enable = $true
        }

        PSAvoidUsingDoubleQuotesForConstantString = @{
            Enable = $true
        }

        PSPlaceOpenBrace = @{
            Enable = $true
            OnSameLine = $true
            NewLineAfter = $true
            IgnoreOneLineBlock = $true
        }

        PSPlaceCloseBrace = @{
            Enable = $true
            NewLineAfter = $false
            IgnoreOneLineBlock = $true
            NoEmptyLineBefore = $false
        }

        PSUseConsistentIndentation = @{
            Enable = $true
            Kind = 'space'
            PipelineIndentation = 'IncreaseIndentationForFirstPipeline'
            IndentationSize = 4
        }

        PSUseConsistentWhitespace = @{
            Enable = $true
            CheckInnerBrace = $true
            CheckOpenBrace = $false
            CheckOpenParen = $false
            CheckOperator = $false
            CheckPipe = $true
            CheckPipeForRedundantWhitespace = $false
            CheckSeparator = $true
            CheckParameter = $false
        }

        PSUseCorrectCasing = @{
            Enable = $true
        }

        PSAlignAssignmentStatement = @{
            Enable = $false
            CheckHashtable = $false
        }

        PSUseCompatibleSyntax = @{
            Enable = $true
            TargetVersions = @('5.1', '6.2', '7.2')
        }

        PSUseCompatibleCommands = @{
            Enable = $true
            TargetProfiles = @(
                # Windows 10 Powershell 5
                'win-48_x64_10.0.17763.0_5.1.17763.316_x64_4.0.30319.42000_framework'
                # Windows 10 Powershell 6
                'win-4_x64_10.0.18362.0_6.2.4_x64_4.0.30319.42000_core'
                # Windows 10 Powershell 7
                'win-4_x64_10.0.18362.0_7.0.0_x64_3.1.2_core'
                # Ubuntu Powershell 6
                'ubuntu_x64_18.04_6.2.4_x64_4.0.30319.42000_core'
                # Ubuntu Powershell 7
                'ubuntu_x64_18.04_7.0.0_x64_3.1.2_core'
            )
        }
        PSUseCompatibleTypes = @{
            Enable = $true
            TargetProfiles = @(
                # Windows 10 Powershell 5
                'win-48_x64_10.0.17763.0_5.1.17763.316_x64_4.0.30319.42000_framework'
                # Windows 10 Powershell 6
                'win-4_x64_10.0.18362.0_6.2.4_x64_4.0.30319.42000_core'
                # Windows 10 Powershell 7
                'win-4_x64_10.0.18362.0_7.0.0_x64_3.1.2_core'
                # Ubuntu Powershell 6
                'ubuntu_x64_18.04_6.2.4_x64_4.0.30319.42000_core'
                # Ubuntu Powershell 7
                'ubuntu_x64_18.04_7.0.0_x64_3.1.2_core'
            )
        }
    }
}
