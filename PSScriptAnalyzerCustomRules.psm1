<#
.SYNOPSIS
    Use PS5-safe escape codes instead of `e and `u
.DESCRIPTION
    `e and `u were added in PS 6, so they will not work in PS5 strings
.EXAMPLE
.INPUTS
.OUTPUTS
.NOTES
#>
function Test-StringEscapeCode {
    [CmdletBinding()]
    [OutputType([Microsoft.Windows.PowerShell.ScriptAnalyzer.Generic.DiagnosticRecord[]])]
    Param
    (
        [Parameter(Mandatory = $true)]
        [ValidateNotNullOrEmpty()]
        [System.Management.Automation.Language.ScriptBlockAst]
        $ScriptBlockAst
    )

    Process {
        [Microsoft.Windows.PowerShell.ScriptAnalyzer.Generic.DiagnosticRecord[]]$results = @()

        try {
            $expressions = $ScriptBlockAst.FindAll({
                    param ([System.Management.Automation.Language.Ast]$Ast)

                    ($Ast.GetType().Name -Match 'StringConstantExpression|ExpandableStringExpression') -and ($Ast.StringConstantType -eq 'DoubleQuoted')
                }, $false)

            foreach ($expr in $expressions) {
                $extent = $expr.Extent
                if ($extent.Text -Match '([^`]|^)`e' ) {
                    $correction = $expr.Extent.Text -Replace '(?<lead>[^`]|^)`e', '${lead}$([char]0x1b)'
                    $objParams = @{
                        TypeName = 'Microsoft.Windows.PowerShell.ScriptAnalyzer.Generic.CorrectionExtent'
                        ArgumentList = $extent.StartLineNumber, $extent.EndLineNumber, $extent.StartColumnNumber,
                        $extent.EndColumnNumber, $correction, $MyInvocation.MyCommand.Definition
                    }
                    $correctionExtent = New-Object @objParams
                    $suggestedCorrections = New-Object System.Collections.ObjectModel.Collection[$($objParams.TypeName)]
                    $suggestedCorrections.add($correctionExtent) | Out-Null

                    $result = [Microsoft.Windows.PowerShell.ScriptAnalyzer.Generic.DiagnosticRecord]@{
                        'Message' = 'The escape sequence "`e" is not supported in powershell 5'
                        'Extent' = $extent
                        'RuleName' = $PSCmdlet.MyInvocation.InvocationName
                        'Severity' = 'Warning'
                        'SuggestedCorrections' = $suggestedCorrections
                    }
                    $results += $result

                } elseif ($extent.Text -Match '([^`]|^)`u{(?<sequence>[A-Za-z0-9]+)}') {
                    $sequence = $Matches.sequence

                    $correction = -Replace '(?<lead>[^`]|^)`u\{(?<code>[A-Za-z0-9]+)}', '${lead}$([char]0x${code})'
                    $objParams = @{
                        TypeName = 'Microsoft.Windows.PowerShell.ScriptAnalyzer.Generic.CorrectionExtent'
                        ArgumentList = $extent.StartLineNumber, $extent.EndLineNumber, $extent.StartColumnNumber,
                        $extent.EndColumnNumber, $correction, $MyInvocation.MyCommand.Definition
                    }
                    $correctionExtent = New-Object @objParams
                    $suggestedCorrections = New-Object System.Collections.ObjectModel.Collection[$($objParams.TypeName)]
                    $suggestedCorrections.add($correctionExtent) | Out-Null

                    $result = [Microsoft.Windows.PowerShell.ScriptAnalyzer.Generic.DiagnosticRecord]@{
                        'Message' = "The escape sequence `"``u{$sequence}`" is not supported in powershell 5"
                        'Extent' = $assignmentAst.Extent
                        'RuleName' = $PSCmdlet.MyInvocation.InvocationName
                        'Severity' = 'Warning'
                        'SuggestedCorrections' = $suggestedCorrections
                    }
                    $results += $result
                }
            }
            return $results
        } catch {
            $PSCmdlet.ThrowTerminatingError($PSItem)
        }
    }
}

Export-ModuleMember -Function Test-StringEscapeCode
