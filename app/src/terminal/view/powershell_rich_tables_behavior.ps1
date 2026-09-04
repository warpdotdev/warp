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

function Get-EncodedMessageByteCount {
    param([System.Collections.Hashtable]$Message)

    $json = ConvertTo-Json -InputObject $Message -Compress -Depth 8
    (2 * [System.Text.Encoding]::UTF8.GetByteCount($json)) + 10
}
function Assert-RichTableHooks {
    param([string]$Description)

    $hooks = Get-HookNames
    Assert-True ($hooks[0] -eq 'PowerShellTableBegin') "$Description should begin a table"
    Assert-True ($hooks -contains 'PowerShellTableRows') "$Description should send rows"
    Assert-True ($hooks[-1] -eq 'PowerShellTableEnd') "$Description should end the table"
}

function New-CustomTableObject {
    param([string]$TypeName, [string]$Value)

    $object = [pscustomobject]@{ Value = $Value }
    $object.PSObject.TypeNames.Insert(0, $TypeName)
    $object
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
Assert-RichTableHooks 'implicit output'
Assert-True ((Get-RowCount) -eq 2) 'implicit output should keep object order'

$script:warpMessages.Clear()
[pscustomobject]@{ Name = 'alpha'; Id = 1 } | Format-Table | Warp-Out-Default
Assert-True ((Get-HookNames).Count -eq 0) 'explicit Format-Table must not emit table OSC'
$script:warpMessages.Clear()
Get-Alias | Select-Object -First 2 | Warp-Out-Default
Assert-RichTableHooks 'Get-Alias output'
$aliasBegin = $script:warpMessages |
    Where-Object { $_.hook -eq 'PowerShellTableBegin' } |
    Select-Object -First 1
$computedAliasColumn = $aliasBegin.value.columns |
    Where-Object { $_.name -eq 'Name' } |
    Select-Object -First 1
Assert-True ($null -ne $computedAliasColumn) 'Get-Alias should retain its computed Name column'
Assert-True ([string]::IsNullOrEmpty($computedAliasColumn.property_name)) `
    'computed columns should not declare a source property'
Assert-True ([string]::IsNullOrEmpty($computedAliasColumn.type_name)) `
    'computed columns should not declare a type'
Assert-True (-not $computedAliasColumn.ContainsKey('expression')) `
    'computed expressions must not be included in OSC metadata'

$script:warpMessages.Clear()
Get-Command Get-Item | Warp-Out-Default
Assert-RichTableHooks 'Get-Command output'

$script:warpMessages.Clear()
Get-ChildItem -LiteralPath $PSScriptRoot | Select-Object -First 2 | Warp-Out-Default
Assert-RichTableHooks 'Get-ChildItem output'

$customFormatPath = Join-Path ([System.IO.Path]::GetTempPath()) (
    "warp-rich-table-$([Guid]::NewGuid().ToString('N')).ps1xml"
)
$customFormat = @'
<Configuration>
  <ViewDefinitions>
    <View>
      <Name>WarpComputed</Name>
      <ViewSelectedBy><TypeName>Warp.Test.Computed</TypeName></ViewSelectedBy>
      <TableControl>
        <TableHeaders>
          <TableColumnHeader><Label>Computed</Label></TableColumnHeader>
        </TableHeaders>
        <TableRowEntries>
          <TableRowEntry>
            <TableColumnItems>
              <TableColumnItem>
                <ScriptBlock>$global:warpComputedEvaluations++; $_.Value.ToUpperInvariant()</ScriptBlock>
              </TableColumnItem>
            </TableColumnItems>
          </TableRowEntry>
        </TableRowEntries>
      </TableControl>
    </View>
    <View>
      <Name>WarpComputedFailure</Name>
      <ViewSelectedBy><TypeName>Warp.Test.ComputedFailure</TypeName></ViewSelectedBy>
      <TableControl>
        <TableHeaders>
          <TableColumnHeader><Label>Computed</Label></TableColumnHeader>
        </TableHeaders>
        <TableRowEntries>
          <TableRowEntry>
            <TableColumnItems>
              <TableColumnItem>
                <ScriptBlock>if ($_.Value -eq 'fail') { throw 'computed failure' }; $_.Value</ScriptBlock>
              </TableColumnItem>
            </TableColumnItems>
          </TableRowEntry>
        </TableRowEntries>
      </TableControl>
    </View>
    <View>
      <Name>WarpComputedCollection</Name>
      <ViewSelectedBy><TypeName>Warp.Test.ComputedCollection</TypeName></ViewSelectedBy>
      <TableControl>
        <TableHeaders>
          <TableColumnHeader><Label>Computed</Label></TableColumnHeader>
        </TableHeaders>
        <TableRowEntries>
          <TableRowEntry>
            <TableColumnItems>
              <TableColumnItem><ScriptBlock>1, 2</ScriptBlock></TableColumnItem>
            </TableColumnItems>
          </TableRowEntry>
        </TableRowEntries>
      </TableControl>
    </View>
  </ViewDefinitions>
</Configuration>
'@
try {
    [System.IO.File]::WriteAllText($customFormatPath, $customFormat)
    Update-FormatData -PrependPath $customFormatPath

    $global:warpComputedEvaluations = 0
    $script:warpMessages.Clear()
    @(
        New-CustomTableObject -TypeName 'Warp.Test.Computed' -Value 'alpha'
        New-CustomTableObject -TypeName 'Warp.Test.Computed' -Value 'beta'
    ) | Warp-Out-Default
    Assert-RichTableHooks 'custom computed view'
    Assert-True ($global:warpComputedEvaluations -eq 2) `
        'a computed column should be evaluated exactly once per object'
    $computedRows = @(
        $script:warpMessages |
            Where-Object { $_.hook -eq 'PowerShellTableRows' } |
            ForEach-Object { $_.value.rows }
    )
    Assert-True ($computedRows[0][0] -eq 'ALPHA') `
        'custom computed views should preserve their scalar display value'
    Assert-True ($computedRows[1][0] -eq 'BETA') `
        'custom computed views should preserve row order'
    $script:warpMessages.Clear()
    New-CustomTableObject -TypeName 'Warp.Test.Computed' -Value ('x' * 33000) |
        Warp-Out-Default
    Assert-True ((Get-HookNames).Count -eq 0) `
        'an oversized computed value must fall back before any OSC'

    $script:warpMessages.Clear()
    @(
        New-CustomTableObject -TypeName 'Warp.Test.ComputedFailure' -Value 'kept'
        New-CustomTableObject -TypeName 'Warp.Test.ComputedFailure' -Value 'fail'
    ) | Warp-Out-Default
    Assert-True ((Get-HookNames).Count -eq 0) `
        'a computed expression failure must fall back the buffered table before any OSC'

    $script:warpMessages.Clear()
    New-CustomTableObject -TypeName 'Warp.Test.ComputedCollection' -Value 'ignored' |
        Warp-Out-Default
    Assert-True ((Get-HookNames).Count -eq 0) `
        'a non-scalar computed expression must fall back before any OSC'
} finally {
    Remove-Item -LiteralPath $customFormatPath -ErrorAction Ignore
}

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
1..10 | ForEach-Object {
    [pscustomobject]@{ Name = (('x' * 8000) + $_); Id = $_ }
} | Warp-Out-Default
$rowMessages = @($script:warpMessages | Where-Object { $_.hook -eq 'PowerShellTableRows' })
Assert-True ($rowMessages.Count -gt 1) 'producer should split row chunks below the byte budget'
foreach ($message in $script:warpMessages) {
    Assert-True ((Get-EncodedMessageByteCount $message) -le 65536) `
        "encoded $($message.hook) message exceeded the byte budget"
}

$script:warpMessages.Clear()
$oversizedPropertyName = 'p' * 33000
$oversizedMetadata = [pscustomobject]@{}
Add-Member -InputObject $oversizedMetadata -NotePropertyName $oversizedPropertyName `
    -NotePropertyValue 'value'
$oversizedMetadata | Warp-Out-Default
Assert-True ((Get-HookNames).Count -eq 0) `
    'oversized metadata must fall back before emitting a table'

$script:warpMessages.Clear()
$simple, ([pscustomobject]@{ Name = ('x' * 33000); Id = 2 }) | Warp-Out-Default
Assert-True ((Get-HookNames).Count -eq 0) `
    'an oversized row must fall back the buffered table before emitting any OSC'

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
