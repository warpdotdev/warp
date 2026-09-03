# Tests Warp's PowerShell rich-table helpers from pwsh.ps1 without launching Warp.
param(
    [Parameter(Mandatory = $true)]
    [string]$BootstrapPath
)

$ErrorActionPreference = 'Stop'

if (-not [System.IO.File]::Exists($BootstrapPath)) {
    throw "Bootstrap file not found: $BootstrapPath"
}

$source = [System.IO.File]::ReadAllText($BootstrapPath)
$start = $source.IndexOf('    function Warp-Get-PowerShellTableColumns')
$end = $source.IndexOf('    function Warp-Finish-Bootstrap')
if ($start -lt 0 -or $end -lt 0 -or $end -le $start) {
    throw 'Could not extract rich-table functions from pwsh.ps1'
}

$script:warpMessages = [System.Collections.Generic.List[object]]::new()
function Warp-Send-JsonMessage([System.Collections.Hashtable]$table) {
    $script:warpMessages.Add($table)
}

$global:_warpSessionId = 1
Invoke-Expression $source.Substring($start, $end - $start)

function Assert-True {
    param([bool]$Condition, [string]$Message)
    if (-not $Condition) {
        throw $Message
    }
}

function Get-HookNames {
    @($script:warpMessages | ForEach-Object { $_.hook })
}

function Get-RowCount {
    $count = 0
    foreach ($message in $script:warpMessages) {
        if ($message.hook -eq 'PowerShellTableRows') {
            $count += @($message.value.rows).Count
        }
    }
    $count
}

$simple = [pscustomobject]@{ Name = 'alpha'; Id = 1 }
$columns = Warp-Get-PowerShellTableColumns $simple
Assert-True ($null -ne $columns) 'implicit two-property object should be a table'
Assert-True ($columns.Count -eq 2) 'implicit table should keep both properties'

$formatted = [pscustomobject]@{ Name = 'alpha'; Id = 1 } | Format-Table
Assert-True ($null -eq (Warp-Get-PowerShellTableColumns $formatted[0])) `
    'Format-Table records must fall back before any OSC'

$wide = [pscustomobject]@{}
$wideNames = 1..65 | ForEach-Object { "P$_" }
foreach ($name in $wideNames) {
    Add-Member -InputObject $wide -NotePropertyName $name -NotePropertyValue 1
}
$propertySet = New-Object System.Management.Automation.PSPropertySet(
    'DefaultDisplayPropertySet',
    [string[]]$wideNames
)
$wide | Add-Member -Force -MemberType MemberSet -Name PSStandardMembers -Value (
    New-Object System.Management.Automation.PSMemberSet(
        'PSStandardMembers',
        [System.Management.Automation.PSMemberInfo[]]@($propertySet)
    )
)
Assert-True ($null -eq (Warp-Get-PowerShellTableColumns $wide)) `
    'more than 64 declared columns must fall back before any OSC'

$script:warpMessages.Clear()
$simple, ([pscustomobject]@{ Name = 'beta'; Id = 2 }) | Warp-Out-Default
$hooks = Get-HookNames
Assert-True ($hooks[0] -eq 'PowerShellTableBegin') 'implicit output should begin a table'
Assert-True ($hooks -contains 'PowerShellTableRows') 'implicit output should send rows'
Assert-True ($hooks[-1] -eq 'PowerShellTableEnd') 'implicit output should end the table'
Assert-True ((Get-RowCount) -eq 2) 'implicit output should keep object order'

$script:warpMessages.Clear()
[pscustomobject]@{ Name = 'alpha'; Id = 1 } | Format-Table | Warp-Out-Default
Assert-True ((Get-HookNames).Count -eq 0) 'explicit Format-Table must not emit table OSC'

$script:warpMessages.Clear()
$simple, 'plain text' | Warp-Out-Default
$hooks = Get-HookNames
Assert-True ($hooks[0] -eq 'PowerShellTableBegin') 'table then plain should still emit the table first'
Assert-True ($hooks[-1] -eq 'PowerShellTableEnd') 'table then plain should end the table before fallback'

$script:warpMessages.Clear()
1..30 | ForEach-Object { [pscustomobject]@{ Name = "$_"; Id = $_ } } | Warp-Out-Default
$rowMessages = @($script:warpMessages | Where-Object { $_.hook -eq 'PowerShellTableRows' })
Assert-True ($rowMessages.Count -eq 2) 'producer should flush every 25 rows'
Assert-True (@($rowMessages[0].value.rows).Count -eq 25) 'first row chunk should contain 25 rows'
Assert-True (@($rowMessages[1].value.rows).Count -eq 5) 'remainder should be a second chunk'

$script:warpMessages.Clear()
1..10001 | ForEach-Object { [pscustomobject]@{ Name = "$_"; Id = $_ } } | Warp-Out-Default
Assert-True ((Get-RowCount) -eq 10000) 'rich output must stop at 10,000 rows'
Assert-True ((Get-HookNames)[-1] -eq 'PowerShellTableEnd') 'overflow should end the table before fallback'

$env:WARP_POWERSHELL_RICH_TABLES = '1'
function Out-Default {
    param(
        [Parameter(ValueFromPipeline = $true)]
        [psobject]$InputObject
    )
    process { }
}
Warp-Install-PowerShellRichTables
$effective = Get-Command Out-Default | Select-Object -First 1
Assert-True ($effective.CommandType -eq 'Function') `
    'a profile-defined Out-Default function must not be replaced'

Write-Output 'powershell_rich_tables_behavior: ok'
