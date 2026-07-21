$ErrorActionPreference = 'Stop'
Set-StrictMode -Version 3.0

$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
. "$ScriptDir\tui_package_utils.ps1"

function Assert-True {
    param(
        [Boolean]$Condition,
        [String]$Message
    )
    if (-Not $Condition) {
        throw $Message
    }
}

$TestRoot = Join-Path ([System.IO.Path]::GetTempPath()) "warp-tui-package-test-$([Guid]::NewGuid().ToString('N'))"
try {
    $Source = Join-Path $TestRoot 'source'
    $Payload = Join-Path $Source 'warp-tui'
    $Resources = Join-Path $Payload 'resources'
    $Metadata = Join-Path $Resources 'bundled\metadata'
    $Runtime = Join-Path $Payload 'x64'
    New-Item -ItemType Directory -Path $Metadata, $Runtime -Force | Out-Null

    [System.IO.File]::WriteAllText((Join-Path $Payload 'warp-tui-dev.exe'), 'exe')
    [System.IO.File]::WriteAllText((Join-Path $Payload 'conpty.dll'), 'dll')
    [System.IO.File]::WriteAllText((Join-Path $Runtime 'OpenConsole.exe'), 'console')
    [System.IO.File]::WriteAllText((Join-Path $Resources 'THIRD_PARTY_LICENSES.txt'), 'licenses')
    [System.IO.File]::WriteAllText((Join-Path $Resources 'settings_schema.json'), '{}')
    [System.IO.File]::WriteAllText(
        (Join-Path $Metadata 'version.json'),
        '{"warp_version":"v1.2.3"}'
    )
    [System.IO.File]::WriteAllText(
        (Join-Path $Payload 'manifest.json'),
        '{"format_version":1,"channel":"dev","architecture":"x86_64","version":"v1.2.3","executable":"warp-tui-dev.exe","command":"warp-dev"}'
    )

    $ArchiveOne = Join-Path $TestRoot 'one.zip'
    $ArchiveTwo = Join-Path $TestRoot 'two.zip'
    New-DeterministicZip -SourceDirectory $Source -DestinationPath $ArchiveOne
    New-DeterministicZip -SourceDirectory $Source -DestinationPath $ArchiveTwo
    $HashOne = (Get-FileHash -Path $ArchiveOne -Algorithm SHA256).Hash
    $HashTwo = (Get-FileHash -Path $ArchiveTwo -Algorithm SHA256).Hash
    Assert-True ($HashOne -ceq $HashTwo) 'Deterministic archives had different hashes'

    $InstallRoot = Join-Path $TestRoot 'install'
    & "$ScriptDir\install-warp-tui.ps1" `
        -ArchivePath $ArchiveOne `
        -InstallRoot $InstallRoot `
        -AllowUnsigned `
        -NoPathUpdate
    Assert-True (
        (Get-Content (Join-Path $InstallRoot 'current.txt') -Raw).Trim() -ceq 'v1.2.3'
    ) 'Installer did not activate the packaged version'
    Assert-True (
        (Test-Path (Join-Path $InstallRoot 'bin\warp-dev.cmd') -PathType Leaf)
    ) 'Installer did not create the server-contract command launcher'

    # Installing the same immutable version is idempotent.
    & "$ScriptDir\install-warp-tui.ps1" `
        -ArchivePath $ArchiveOne `
        -InstallRoot $InstallRoot `
        -AllowUnsigned `
        -NoPathUpdate
    # A successful update preserves the previous activation as rollback state.
    [System.IO.File]::WriteAllText(
        (Join-Path $Metadata 'version.json'),
        '{"warp_version":"v1.2.4"}'
    )
    [System.IO.File]::WriteAllText(
        (Join-Path $Payload 'manifest.json'),
        '{"format_version":1,"channel":"dev","architecture":"x86_64","version":"v1.2.4","executable":"warp-tui-dev.exe","command":"warp-dev"}'
    )
    $UpdateArchive = Join-Path $TestRoot 'update.zip'
    New-DeterministicZip -SourceDirectory $Source -DestinationPath $UpdateArchive
    & "$ScriptDir\install-warp-tui.ps1" `
        -ArchivePath $UpdateArchive `
        -InstallRoot $InstallRoot `
        -AllowUnsigned `
        -NoPathUpdate
    Assert-True (
        (Get-Content (Join-Path $InstallRoot 'current.txt') -Raw).Trim() -ceq 'v1.2.4'
    ) 'Installer did not activate the update'
    Assert-True (
        (Get-Content (Join-Path $InstallRoot 'current.txt.rollback') -Raw).Trim() -ceq 'v1.2.3'
    ) 'Installer did not preserve rollback activation state'

    $BadSource = Join-Path $TestRoot 'bad-source'
    Copy-Item -Path $Source -Destination $BadSource -Recurse
    [System.IO.File]::WriteAllText((Join-Path $BadSource 'unexpected.exe'), 'unexpected')
    $BadArchive = Join-Path $TestRoot 'bad.zip'
    New-DeterministicZip -SourceDirectory $BadSource -DestinationPath $BadArchive

    $RejectedBadShape = $False
    try {
        & "$ScriptDir\install-warp-tui.ps1" `
            -ArchivePath $BadArchive `
            -InstallRoot $InstallRoot `
            -AllowUnsigned `
            -NoPathUpdate
    } catch {
        $RejectedBadShape = $True
    }
    Assert-True $RejectedBadShape 'Installer accepted an unexpected top-level archive entry'
    Assert-True (
        (Get-Content (Join-Path $InstallRoot 'current.txt') -Raw).Trim() -ceq 'v1.2.4'
    ) 'Rejected archive changed the active version'

    foreach ($InvalidVersion in @('..', 'v..1', '.hidden', '-v1', 'v1.')) {
        [System.IO.File]::WriteAllText(
            (Join-Path $Metadata 'version.json'),
            "{`"warp_version`":`"$InvalidVersion`"}"
        )
        [System.IO.File]::WriteAllText(
            (Join-Path $Payload 'manifest.json'),
            "{`"format_version`":1,`"channel`":`"dev`",`"architecture`":`"x86_64`",`"version`":`"$InvalidVersion`",`"executable`":`"warp-tui-dev.exe`",`"command`":`"warp-dev`"}"
        )
        $InvalidVersionArchive = Join-Path $TestRoot "invalid-version-$([Guid]::NewGuid().ToString('N')).zip"
        New-DeterministicZip -SourceDirectory $Source -DestinationPath $InvalidVersionArchive

        $RejectedInvalidVersion = $False
        try {
            & "$ScriptDir\install-warp-tui.ps1" `
                -ArchivePath $InvalidVersionArchive `
                -InstallRoot $InstallRoot `
                -AllowUnsigned `
                -NoPathUpdate
        } catch {
            $RejectedInvalidVersion = $True
        }
        Assert-True $RejectedInvalidVersion "Installer accepted unsafe version '$InvalidVersion'"
        Assert-True (
            (Get-Content (Join-Path $InstallRoot 'current.txt') -Raw).Trim() -ceq 'v1.2.4'
        ) "Unsafe version '$InvalidVersion' changed the active version"
    }

    Write-Output 'Windows TUI packaging tests passed.'
} finally {
    Remove-Item -Path $TestRoot -Recurse -Force -ErrorAction SilentlyContinue
}
