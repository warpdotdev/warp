Param(
    [Parameter(Mandatory = $true)]
    [String]$ArchivePath,

    [String]$InstallRoot = '',

    # Development/testing escape hatch. Production installation must validate
    # every executable and DLL before changing the active version.
    [Switch]$AllowUnsigned = $False,

    [Switch]$NoPathUpdate = $False
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version 3.0

Add-Type -AssemblyName System.IO.Compression
Add-Type -AssemblyName System.IO.Compression.FileSystem

function Get-ExpectedExecutable {
    param([String]$Channel)

    switch ($Channel) {
        'local' { 'warp-tui-local.exe' }
        'dev' { 'warp-tui-dev.exe' }
        'preview' { 'warp-tui-preview.exe' }
        'stable' { 'warp-tui.exe' }
        'oss' { 'warp-tui-oss.exe' }
        default { throw "Unsupported Warp TUI channel '$Channel'" }
    }
}

function Get-ExpectedCommand {
    param([String]$Channel)

    switch ($Channel) {
        'local' { 'warp-local' }
        'dev' { 'warp-dev' }
        'preview' { 'warp-preview' }
        'stable' { 'warp' }
        'oss' { 'warp-oss' }
        default { throw "Unsupported Warp TUI channel '$Channel'" }
    }
}
function Read-ZipEntryText {
    param(
        [System.IO.Compression.ZipArchive]$Archive,
        [String]$EntryName
    )

    $Entry = $Archive.Entries |
        Where-Object { $_.FullName -ceq $EntryName } |
        Select-Object -First 1
    if (-Not $Entry) {
        throw "Archive is missing required entry '$EntryName'"
    }

    $Reader = [System.IO.StreamReader]::new($Entry.Open(), [System.Text.Encoding]::UTF8)
    try {
        $Reader.ReadToEnd()
    } finally {
        $Reader.Dispose()
    }
}

function Get-ValidatedArchiveManifest {
    param([String]$Path)

    $Archive = [System.IO.Compression.ZipFile]::OpenRead(
        [System.IO.Path]::GetFullPath($Path)
    )
    try {
        $SeenEntries = @{}
        foreach ($Entry in $Archive.Entries) {
            $EntryName = $Entry.FullName
            if (
                -Not $EntryName.StartsWith('warp-tui/', [StringComparison]::Ordinal) -or
                $EntryName.Contains('\') -or
                $EntryName.Split('/') -contains '..'
            ) {
                throw "Unsafe or unexpected archive entry '$EntryName'"
            }

            $EntryKey = $EntryName.ToLowerInvariant()
            if ($SeenEntries.ContainsKey($EntryKey)) {
                throw "Archive contains duplicate entry '$EntryName'"
            }
            $SeenEntries[$EntryKey] = $True
        }

        $Manifest = Read-ZipEntryText -Archive $Archive -EntryName 'warp-tui/manifest.json' |
            ConvertFrom-Json
        if ($Manifest.format_version -ne 1) {
            throw "Unsupported Warp TUI archive format '$($Manifest.format_version)'"
        }
        if ($Manifest.channel -notin @('local', 'dev', 'preview', 'stable', 'oss')) {
            throw "Unsupported Warp TUI channel '$($Manifest.channel)'"
        }
        if ($Manifest.architecture -notin @('x86_64', 'aarch64')) {
            throw "Unsupported Warp TUI architecture '$($Manifest.architecture)'"
        }
        if (
            -Not $Manifest.version -or
            $Manifest.version -notmatch '^[A-Za-z0-9][A-Za-z0-9._+-]*$' -or
            $Manifest.version.Contains('..') -or
            $Manifest.version.EndsWith('.')
        ) {
            throw "Invalid Warp TUI version '$($Manifest.version)'"
        }

        $ExpectedExecutable = Get-ExpectedExecutable -Channel $Manifest.channel
        if ($Manifest.executable -cne $ExpectedExecutable) {
            throw "Manifest executable '$($Manifest.executable)' does not match channel '$($Manifest.channel)'"
        }
        $ExpectedCommand = Get-ExpectedCommand -Channel $Manifest.channel
        if ($Manifest.command -cne $ExpectedCommand) {
            throw "Manifest command '$($Manifest.command)' does not match channel '$($Manifest.channel)'"
        }

        $RuntimeArch = if ($Manifest.architecture -eq 'x86_64') { 'x64' } else { 'arm64' }
        $RequiredEntries = @(
            'warp-tui/manifest.json',
            "warp-tui/$ExpectedExecutable",
            'warp-tui/conpty.dll',
            "warp-tui/$RuntimeArch/OpenConsole.exe",
            'warp-tui/resources/bundled/metadata/version.json',
            'warp-tui/resources/THIRD_PARTY_LICENSES.txt',
            'warp-tui/resources/settings_schema.json'
        )
        foreach ($RequiredEntry in $RequiredEntries) {
            if (-Not $SeenEntries.ContainsKey($RequiredEntry.ToLowerInvariant())) {
                throw "Archive is missing required entry '$RequiredEntry'"
            }
        }

        foreach ($Entry in $Archive.Entries) {
            $EntryName = $Entry.FullName
            $ExpectedPortableExecutables = @(
                "warp-tui/$ExpectedExecutable",
                'warp-tui/conpty.dll',
                "warp-tui/$RuntimeArch/OpenConsole.exe"
            )
            $IsAllowed = (
                $EntryName -in @(
                    'warp-tui/',
                    'warp-tui/manifest.json',
                    "warp-tui/$ExpectedExecutable",
                    'warp-tui/conpty.dll',
                    "warp-tui/$RuntimeArch/",
                    "warp-tui/$RuntimeArch/OpenConsole.exe"
                ) -or
                $EntryName.StartsWith('warp-tui/resources/', [StringComparison]::Ordinal)
            )
            if (-Not $IsAllowed) {
                throw "Archive contains unexpected entry '$EntryName'"
            }
            if (
                -Not $EntryName.EndsWith('/') -and
                [System.IO.Path]::GetExtension($EntryName) -in @('.exe', '.dll') -and
                $EntryName -notin $ExpectedPortableExecutables
            ) {
                throw "Archive contains unexpected executable payload '$EntryName'"
            }
        }

        $VersionMetadata = Read-ZipEntryText `
            -Archive $Archive `
            -EntryName 'warp-tui/resources/bundled/metadata/version.json' |
            ConvertFrom-Json
        if ($VersionMetadata.warp_version -cne $Manifest.version) {
            throw "Version metadata '$($VersionMetadata.warp_version)' does not match manifest '$($Manifest.version)'"
        }

        @{
            Channel = [String]$Manifest.channel
            Architecture = [String]$Manifest.architecture
            RuntimeArch = $RuntimeArch
            Version = [String]$Manifest.version
            Executable = $ExpectedExecutable
            Command = $ExpectedCommand
        }
    } finally {
        $Archive.Dispose()
    }
}

function Expand-ValidatedArchive {
    param(
        [String]$Path,
        [String]$Destination
    )

    $DestinationRoot = [System.IO.Path]::GetFullPath($Destination).TrimEnd(
        [System.IO.Path]::DirectorySeparatorChar,
        [System.IO.Path]::AltDirectorySeparatorChar
    )
    New-Item -ItemType Directory -Path $DestinationRoot -Force | Out-Null

    $Archive = [System.IO.Compression.ZipFile]::OpenRead(
        [System.IO.Path]::GetFullPath($Path)
    )
    try {
        foreach ($Entry in $Archive.Entries) {
            $RelativePath = $Entry.FullName.Replace(
                '/',
                [System.IO.Path]::DirectorySeparatorChar
            )
            $DestinationPath = [System.IO.Path]::GetFullPath(
                (Join-Path $DestinationRoot $RelativePath)
            )
            $RequiredPrefix = "$DestinationRoot$([System.IO.Path]::DirectorySeparatorChar)"
            if (-Not $DestinationPath.StartsWith($RequiredPrefix, [StringComparison]::OrdinalIgnoreCase)) {
                throw "Archive entry escapes extraction root: '$($Entry.FullName)'"
            }

            if ($Entry.FullName.EndsWith('/')) {
                New-Item -ItemType Directory -Path $DestinationPath -Force | Out-Null
                continue
            }

            New-Item -ItemType Directory -Path (Split-Path -Parent $DestinationPath) -Force |
                Out-Null
            $Input = $Entry.Open()
            $Output = [System.IO.File]::Create($DestinationPath)
            try {
                $Input.CopyTo($Output)
            } finally {
                $Output.Dispose()
                $Input.Dispose()
            }
        }
    } finally {
        $Archive.Dispose()
    }
}

function Assert-PayloadSignatures {
    param(
        [String]$PayloadRoot,
        [hashtable]$Manifest
    )

    if (-Not (Get-Command Get-AuthenticodeSignature -ErrorAction SilentlyContinue)) {
        throw 'Get-AuthenticodeSignature is unavailable; install on Windows or pass -AllowUnsigned only for development tests.'
    }

    $Paths = @(
        (Join-Path $PayloadRoot $Manifest.Executable),
        (Join-Path $PayloadRoot 'conpty.dll'),
        (Join-Path $PayloadRoot "$($Manifest.RuntimeArch)\OpenConsole.exe")
    )
    foreach ($Path in $Paths) {
        $Signature = Get-AuthenticodeSignature -FilePath $Path
        if ($Signature.Status -ne [System.Management.Automation.SignatureStatus]::Valid) {
            throw "Invalid Authenticode signature for '$Path': $($Signature.Status) ($($Signature.StatusMessage))"
        }
    }
}

function Assert-IdenticalDirectory {
    param(
        [String]$Expected,
        [String]$Actual
    )

    $ExpectedRoot = [System.IO.Path]::GetFullPath($Expected)
    $ActualRoot = [System.IO.Path]::GetFullPath($Actual)
    $ExpectedFiles = Get-ChildItem -Path $ExpectedRoot -File -Recurse |
        ForEach-Object {
            @{
                RelativePath = $_.FullName.Substring($ExpectedRoot.Length).TrimStart('\', '/')
                Hash = (Get-FileHash -Path $_.FullName -Algorithm SHA256).Hash
            }
        }
    $ActualFiles = Get-ChildItem -Path $ActualRoot -File -Recurse |
        ForEach-Object {
            @{
                RelativePath = $_.FullName.Substring($ActualRoot.Length).TrimStart('\', '/')
                Hash = (Get-FileHash -Path $_.FullName -Algorithm SHA256).Hash
            }
        }

    $ExpectedMap = @{}
    foreach ($File in $ExpectedFiles) {
        $ExpectedMap[$File.RelativePath.ToLowerInvariant()] = $File.Hash
    }
    $ActualMap = @{}
    foreach ($File in $ActualFiles) {
        $ActualMap[$File.RelativePath.ToLowerInvariant()] = $File.Hash
    }

    if ($ExpectedMap.Count -ne $ActualMap.Count) {
        throw "Existing version directory '$Actual' differs from the archive"
    }
    foreach ($RelativePath in $ExpectedMap.Keys) {
        if (
            -Not $ActualMap.ContainsKey($RelativePath) -or
            $ActualMap[$RelativePath] -cne $ExpectedMap[$RelativePath]
        ) {
            throw "Existing version directory '$Actual' differs at '$RelativePath'"
        }
    }
}

function Set-UserPathEntry {
    param([String]$Directory)

    $UserPath = [Environment]::GetEnvironmentVariable('Path', 'User')
    $Entries = @($UserPath -split ';' | Where-Object { $_ })
    if ($Entries | Where-Object { $_.TrimEnd('\') -ieq $Directory.TrimEnd('\') }) {
        return
    }

    $NewPath = (@($Entries) + $Directory) -join ';'
    [Environment]::SetEnvironmentVariable('Path', $NewPath, 'User')

    if ([Environment]::OSVersion.Platform -eq [PlatformID]::Win32NT) {
        if (-Not ('WarpTui.NativeMethods' -as [type])) {
            Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;

namespace WarpTui {
    public static class NativeMethods {
        [DllImport("user32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
        public static extern IntPtr SendMessageTimeout(
            IntPtr hWnd,
            uint msg,
            UIntPtr wParam,
            string lParam,
            uint flags,
            uint timeout,
            out UIntPtr result);
    }
}
'@
        }

        $Result = [UIntPtr]::Zero
        [void][WarpTui.NativeMethods]::SendMessageTimeout(
            [IntPtr]0xffff,
            0x001A,
            [UIntPtr]::Zero,
            'Environment',
            0x0002,
            5000,
            [ref]$Result
        )
    }
}

$ResolvedArchivePath = (Get-Item $ArchivePath | Select-Object -ExpandProperty FullName)
$Manifest = Get-ValidatedArchiveManifest -Path $ResolvedArchivePath
if ([Environment]::OSVersion.Platform -eq [PlatformID]::Win32NT) {
    $ExpectedOsArchitecture = if ($Manifest.Architecture -eq 'x86_64') {
        [System.Runtime.InteropServices.Architecture]::X64
    } else {
        [System.Runtime.InteropServices.Architecture]::Arm64
    }
    if ([System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture -ne $ExpectedOsArchitecture) {
        throw "Archive architecture '$($Manifest.Architecture)' does not match this Windows installation."
    }
}
if (-Not $InstallRoot) {
    $InstallRoot = Join-Path (
        [Environment]::GetFolderPath([Environment+SpecialFolder]::LocalApplicationData)
    ) "Warp\WarpTui\$($Manifest.Channel)"
}
$InstallRoot = [System.IO.Path]::GetFullPath($InstallRoot)

$StagingParent = Join-Path ([System.IO.Path]::GetTempPath()) 'warp-tui-install'
New-Item -ItemType Directory -Path $StagingParent -Force | Out-Null
$StagingDirectory = Join-Path $StagingParent ([Guid]::NewGuid().ToString('N'))

try {
    Expand-ValidatedArchive -Path $ResolvedArchivePath -Destination $StagingDirectory
    $StagedPayload = Join-Path $StagingDirectory 'warp-tui'

    if (-Not $AllowUnsigned) {
        Assert-PayloadSignatures -PayloadRoot $StagedPayload -Manifest $Manifest
    }

    $VersionsDirectory = Join-Path $InstallRoot 'versions'
    $VersionDirectory = Join-Path $VersionsDirectory $Manifest.Version
    New-Item -ItemType Directory -Path $VersionsDirectory -Force | Out-Null
    if (Test-Path $VersionDirectory -PathType Container) {
        Assert-IdenticalDirectory -Expected $StagedPayload -Actual $VersionDirectory
    } else {
        Move-Item -Path $StagedPayload -Destination $VersionDirectory
    }

    $BinDirectory = Join-Path $InstallRoot 'bin'
    New-Item -ItemType Directory -Path $BinDirectory -Force | Out-Null
    $LauncherName = "$($Manifest.Command).cmd"
    $LauncherPath = Join-Path $BinDirectory $LauncherName
    $LauncherTemporaryPath = "$LauncherPath.new"
    $LauncherContent = @"
@echo off
setlocal
set /p "WARP_TUI_VERSION="<"%~dp0..\current.txt"
"%~dp0..\versions\%WARP_TUI_VERSION%\$($Manifest.Executable)" %*
"@
    [System.IO.File]::WriteAllText(
        $LauncherTemporaryPath,
        "$LauncherContent`r`n",
        [System.Text.ASCIIEncoding]::new()
    )
    Move-Item -Path $LauncherTemporaryPath -Destination $LauncherPath -Force

    $CurrentPath = Join-Path $InstallRoot 'current.txt'
    $CurrentTemporaryPath = "$CurrentPath.new"
    $CurrentBackupPath = "$CurrentPath.rollback"
    [System.IO.File]::WriteAllText(
        $CurrentTemporaryPath,
        "$($Manifest.Version)`r`n",
        [System.Text.ASCIIEncoding]::new()
    )
    if (Test-Path $CurrentPath -PathType Leaf) {
        [System.IO.File]::Replace(
            $CurrentTemporaryPath,
            $CurrentPath,
            $CurrentBackupPath,
            $True
        )
    } else {
        [System.IO.File]::Move($CurrentTemporaryPath, $CurrentPath)
    }

    if (-Not $NoPathUpdate) {
        Set-UserPathEntry -Directory $BinDirectory
    }

    Write-Output "Installed Warp TUI $($Manifest.Version) for $($Manifest.Channel)."
    Write-Output "Command: $($Manifest.Command)"
    Write-Output "Install root: $InstallRoot"
} finally {
    Remove-Item -Path $StagingDirectory -Recurse -Force -ErrorAction SilentlyContinue
}
