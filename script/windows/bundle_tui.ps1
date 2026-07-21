Param(
    [Switch]$CheckOnly = $False,

    [ValidateSet('local', 'dev', 'preview', 'stable', 'oss')]
    [String]$Channel = 'dev',

    [String]$ReleaseTag = '',
    [Parameter(Mandatory = $true)]
    [String]$CargoProfile,
    [Parameter(Mandatory = $true)]
    [String]$PlatformTarget,
    [Parameter(Mandatory = $true)]
    [String]$CargoTargetOutputDir,

    [ValidateSet('x64', 'arm64')]
    [String]$Arch,

    [Switch]$SkipPackage = $False,
    [Switch]$SkipBuildBinary = $False,
    [Switch]$RequireAuthenticode = $False
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version 3.0

$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$RepoRoot = (Get-Item "$ScriptDir\..\.." | Select-Object -ExpandProperty FullName)
. "$ScriptDir\tui_package_utils.ps1"

if ($SkipPackage -and $SkipBuildBinary) {
    throw 'SkipPackage and SkipBuildBinary cannot both be set.'
}

$ChannelConfig = switch ($Channel) {
    'local' {
        @{
            CargoBin = 'warp-tui'
            PackageExe = 'warp-tui-local.exe'
            Command = 'warp-local'
            Features = 'release_bundle,standalone,crash_reporting'
        }
    }
    'dev' {
        @{
            CargoBin = 'warp-tui-dev'
            PackageExe = 'warp-tui-dev.exe'
            Command = 'warp-dev'
            Features = 'release_bundle,standalone,crash_reporting'
        }
    }
    'preview' {
        @{
            CargoBin = 'warp-tui-preview'
            PackageExe = 'warp-tui-preview.exe'
            Command = 'warp-preview'
            Features = 'release_bundle,standalone,crash_reporting'
        }
    }
    'stable' {
        @{
            CargoBin = 'warp-tui-stable'
            PackageExe = 'warp-tui.exe'
            Command = 'warp'
            Features = 'release_bundle,standalone,crash_reporting'
        }
    }
    'oss' {
        @{
            CargoBin = 'warp-tui-oss'
            PackageExe = 'warp-tui-oss.exe'
            Command = 'warp-oss'
            Features = 'release_bundle,standalone'
        }
    }
}

$CargoBin = $ChannelConfig.CargoBin
$PackageExe = $ChannelConfig.PackageExe
$Command = $ChannelConfig.Command
$Features = $ChannelConfig.Features
$CargoBinaryPath = Join-Path $CargoTargetOutputDir "$CargoBin.exe"
$PdbPath = Join-Path $CargoTargetOutputDir "$CargoBin.pdb"
$NormalizedArch = if ($Arch -eq 'x64') { 'x86_64' } else { 'aarch64' }

$env:CARGO_FULL_PROFILE = if ($CargoProfile -eq 'dev') { 'debug' } else { $CargoProfile }
if ($ReleaseTag) {
    $env:GIT_RELEASE_TAG = $ReleaseTag
} elseif ($env:GIT_RELEASE_TAG) {
    $ReleaseTag = $env:GIT_RELEASE_TAG
}

$CargoArgs = @(
    '-p', 'warp_tui',
    '--profile', $CargoProfile,
    '--bin', $CargoBin,
    '--features', $Features,
    '--target', $PlatformTarget
)

if ($CheckOnly) {
    & cargo check @CargoArgs
    if (-Not $?) {
        throw "Failed to verify Warp TUI $CargoBin compilation with profile $CargoProfile"
    }
    return
}

if (-Not $SkipBuildBinary) {
    Write-Output "Building Warp TUI for channel $Channel ($PlatformTarget)"
    $env:CARGO_BIN_NAME = $Channel
    $env:WARP_APP_NAME = "WarpTui$($Channel.Substring(0, 1).ToUpper())$($Channel.Substring(1))"
    & cargo build @CargoArgs
    if (-Not $?) {
        throw "Failed to build Warp TUI $CargoBin with profile $CargoProfile"
    }
}

if (-Not (Test-Path $CargoBinaryPath -PathType Leaf)) {
    throw "Warp TUI executable was not produced at $CargoBinaryPath"
}

if ($SkipPackage) {
    if ($env:GITHUB_ACTIONS -eq 'true') {
        Write-Output '::echo::on'
        "target_profile_dir=$($CargoTargetOutputDir -replace '\\', '/')" >> "$env:GITHUB_OUTPUT"
        "binary_path=$($CargoBinaryPath -replace '\\', '/')" >> "$env:GITHUB_OUTPUT"
        "pdb_file_path=$($PdbPath -replace '\\', '/')" >> "$env:GITHUB_OUTPUT"
        Write-Output '::echo::off'
    }
    return
}

if (-Not $ReleaseTag) {
    throw 'ReleaseTag (or GIT_RELEASE_TAG) is required when packaging Warp TUI.'
}

$DistDir = Join-Path $RepoRoot "target\windows-tui-dist\$Channel\$Arch"
$PayloadStage = Join-Path $DistDir 'payload'
$PayloadRoot = Join-Path $PayloadStage 'warp-tui'
$ResourcesDir = Join-Path $PayloadRoot 'resources'
$SymbolsStage = Join-Path $DistDir 'symbols-stage'
$SymbolsRoot = Join-Path $SymbolsStage 'warp-tui-symbols'

Remove-Item -Path $PayloadStage, $SymbolsStage -Recurse -Force -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Path $PayloadRoot, $SymbolsRoot -Force | Out-Null

$PackagedBinaryPath = Join-Path $PayloadRoot $PackageExe
Copy-Item -Path $CargoBinaryPath -Destination $PackagedBinaryPath -Force

$WindowsAssetsDir = Join-Path $RepoRoot "app\assets\windows\$Arch"
$ConptyPath = Join-Path $PayloadRoot 'conpty.dll'
$OpenConsoleDir = Join-Path $PayloadRoot $Arch
$OpenConsolePath = Join-Path $OpenConsoleDir 'OpenConsole.exe'
New-Item -ItemType Directory -Path $OpenConsoleDir -Force | Out-Null
Copy-Item -Path (Join-Path $WindowsAssetsDir 'conpty.dll') -Destination $ConptyPath -Force
Copy-Item -Path (Join-Path $WindowsAssetsDir 'OpenConsole.exe') -Destination $OpenConsolePath -Force

Write-Output 'Preparing standalone Warp TUI resources...'
& "$ScriptDir\prepare_bundled_resources.ps1" `
    -DestinationDir "$ResourcesDir" `
    -Artifact tui `
    -Channel "$Channel" `
    -CargoProfile "$CargoProfile"
if (-Not $?) {
    throw 'Failed to prepare Warp TUI resources'
}

$VersionMetadataPath = Join-Path $ResourcesDir 'bundled\metadata\version.json'
if (-Not (Test-Path $VersionMetadataPath -PathType Leaf)) {
    throw "Version metadata was not generated at $VersionMetadataPath"
}

$Manifest = [ordered]@{
    format_version = 1
    channel = $Channel
    architecture = $NormalizedArch
    version = $ReleaseTag
    executable = $PackageExe
    command = $Command
}
$ManifestJson = $Manifest | ConvertTo-Json
[System.IO.File]::WriteAllText(
    (Join-Path $PayloadRoot 'manifest.json'),
    "$ManifestJson`n",
    [System.Text.UTF8Encoding]::new($False)
)

if ($RequireAuthenticode) {
    Assert-ValidAuthenticodeSignature -Path @(
        $PackagedBinaryPath,
        $ConptyPath,
        $OpenConsolePath
    )
}

if (-Not (Test-Path $PdbPath -PathType Leaf)) {
    throw "Warp TUI symbols were not produced at $PdbPath"
}
Copy-Item -Path $PdbPath -Destination (Join-Path $SymbolsRoot "$([System.IO.Path]::GetFileNameWithoutExtension($PackageExe)).pdb") -Force
Copy-Item -Path (Join-Path $WindowsAssetsDir 'conpty.pdb') -Destination $SymbolsRoot -Force
$OpenConsoleSymbolsDir = Join-Path $SymbolsRoot $Arch
New-Item -ItemType Directory -Path $OpenConsoleSymbolsDir -Force | Out-Null
Copy-Item -Path (Join-Path $WindowsAssetsDir 'OpenConsole.pdb') -Destination $OpenConsoleSymbolsDir -Force

$ArchiveName = "warp-tui-$Channel-windows-$NormalizedArch.zip"
$SymbolsName = "warp-tui-$Channel-windows-$NormalizedArch-symbols.zip"
$ArchivePath = Join-Path $DistDir $ArchiveName
$SymbolsArchivePath = Join-Path $DistDir $SymbolsName
New-DeterministicZip -SourceDirectory $PayloadStage -DestinationPath $ArchivePath
New-DeterministicZip -SourceDirectory $SymbolsStage -DestinationPath $SymbolsArchivePath

Write-Output "Built Warp TUI archive: $ArchivePath"
Write-Output "Built Warp TUI symbols: $SymbolsArchivePath"
Write-Output 'Authenticode signing is intentionally external to bundle.ps1; use build-only, sign, then package-only in release automation.'

if ($env:GITHUB_ACTIONS -eq 'true') {
    Write-Output '::echo::on'
    "target_profile_dir=$($CargoTargetOutputDir -replace '\\', '/')" >> "$env:GITHUB_OUTPUT"
    "binary_path=$($CargoBinaryPath -replace '\\', '/')" >> "$env:GITHUB_OUTPUT"
    "pdb_file_path=$($PdbPath -replace '\\', '/')" >> "$env:GITHUB_OUTPUT"
    "archive_path=$($ArchivePath -replace '\\', '/')" >> "$env:GITHUB_OUTPUT"
    "symbols_path=$($SymbolsArchivePath -replace '\\', '/')" >> "$env:GITHUB_OUTPUT"
    "installer_path=$($ScriptDir -replace '\\', '/')/install-warp-tui.ps1" >> "$env:GITHUB_OUTPUT"
    Write-Output '::echo::off'
}
