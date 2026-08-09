[CmdletBinding()]
param()

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

function Write-FixtureFile {
    param(
        [string] $Path,
        [string] $Content
    )

    New-Item -ItemType Directory -Path (Split-Path $Path) -Force | Out-Null
    [System.IO.File]::WriteAllText($Path, $Content)
}

function Build-FixtureInstaller {
    param(
        [string] $Version,
        [string] $OutputName,
        [string] $InputDir,
        [string] $AssetsDir,
        [string] $OutputDir
    )

    $arguments = @(
        (Join-Path $PSScriptRoot 'tui-installer.iss'),
        '/DReleaseChannel=dev',
        '/DMyAppExeName=warp-tui-dev.exe',
        "/DTargetProfileDir=$InputDir",
        '/DMyAppName=WarpAgentCLIDev',
        "/DMyAppVersion=$Version",
        '/DArch=x64',
        "/DWindowsAssetsDir=$AssetsDir",
        '/DCLIName=warp-dev',
        '/DInstallDirName=tui-dev',
        "/DOutputName=$OutputName",
        "/O$OutputDir"
    )
    & ISCC @arguments | Out-Host
    if ($LASTEXITCODE -ne 0) {
        throw "ISCC failed with exit code $LASTEXITCODE"
    }
    return Join-Path $OutputDir "$OutputName.exe"
}

function Invoke-Installer {
    param(
        [string] $Installer,
        [string] $InstallDir,
        [string] $BinDir,
        [switch] $Uninstall
    )

    $arguments = @('/SP-', '/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART')
    if ($Uninstall) {
        $arguments += "/warp_bin_dir=`"$BinDir`""
    } else {
        $arguments += @(
            '/CURRENTUSER',
            "/DIR=`"$InstallDir`"",
            "/warp_bin_dir=`"$BinDir`"",
            '/skip_path_update=1'
        )
    }
    $process = Start-Process -FilePath $Installer -ArgumentList $arguments -Wait -PassThru
    if ($process.ExitCode -ne 0) {
        throw "$Installer exited with code $($process.ExitCode)"
    }
}

if (-not (Get-Command ISCC -ErrorAction SilentlyContinue)) {
    throw 'ISCC is required to test the Windows TUI installer'
}

$testRoot = Join-Path ([System.IO.Path]::GetTempPath()) "warp tui installer $([guid]::NewGuid())"
$inputDir = Join-Path $testRoot 'input'
$assetsDir = Join-Path $testRoot 'assets'
$outputDir = Join-Path $testRoot 'output'
$installDir = Join-Path $testRoot 'managed root'
$binDir = Join-Path $testRoot 'command root'
$runningProcess = $null

try {
    New-Item -ItemType Directory -Path (Join-Path $inputDir 'resources') -Force | Out-Null
    New-Item -ItemType Directory -Path $assetsDir -Force | Out-Null
    Copy-Item "$env:SystemRoot\System32\WindowsPowerShell\v1.0\powershell.exe" (
        Join-Path $inputDir 'warp-tui-dev.exe'
    )
    Write-FixtureFile (Join-Path $inputDir 'resources\marker.txt') 'fixture'
    foreach ($asset in @(
            'conpty.dll',
            'OpenConsole.exe',
            'vcruntime140.dll',
            'vcruntime140_1.dll',
            'msvcp140.dll'
        )) {
        Write-FixtureFile (Join-Path $assetsDir $asset) $asset
    }

    $version1 = 'v0.2026.07.29.00.00.dev_01'
    $version2 = 'v0.2026.07.29.00.00.dev_02'
    $installer1 = Build-FixtureInstaller $version1 'WarpAgentCLIDevSetup-v1' `
        $inputDir $assetsDir $outputDir
    $installer2 = Build-FixtureInstaller $version2 'WarpAgentCLIDevSetup-v2' `
        $inputDir $assetsDir $outputDir

    Invoke-Installer $installer1 $installDir $binDir
    if ((Get-Content (Join-Path $installDir 'current') -Raw).Trim() -cne $version1) {
        throw 'The first install did not activate v1'
    }
    $launcherPath = Join-Path $binDir 'warp-dev.cmd'
    if (-not (Test-Path -LiteralPath $launcherPath -PathType Leaf)) {
        throw 'The installer did not create the channel launcher'
    }
    if (-not (Test-Path -LiteralPath (Join-Path $installDir 'icon.ico') -PathType Leaf)) {
        throw 'The installer did not install the Warp icon'
    }

    $version1Binary = Join-Path $installDir "versions\$version1\warp-tui-dev.exe"
    $runningProcess = Start-Process -FilePath $version1Binary -ArgumentList @(
        '-NoLogo',
        '-NoProfile',
        '-Command',
        'Start-Sleep -Seconds 60'
    ) -PassThru
    Start-Sleep -Milliseconds 500
    if ($runningProcess.HasExited) {
        throw 'The v1 fixture process exited before the upgrade'
    }

    Invoke-Installer $installer2 $installDir $binDir
    if ((Get-Content (Join-Path $installDir 'current') -Raw).Trim() -cne $version2) {
        throw 'The upgrade did not activate v2'
    }
    if ((Get-Content (Join-Path $installDir 'previous') -Raw).Trim() -cne $version1) {
        throw 'The upgrade did not record v1 as the rollback version'
    }
    if ($runningProcess.HasExited) {
        throw 'The upgrade terminated the running v1 process'
    }
    if (-not (Test-Path -LiteralPath $version1Binary -PathType Leaf)) {
        throw 'The upgrade removed the running v1 payload'
    }

    $registryPath = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\warp-agent-cli-dev_is1'
    $registration = Get-ItemProperty -LiteralPath $registryPath
    $registeredVersion = $registration.DisplayVersion
    if ($registeredVersion -cne $version2) {
        throw "ARP registered '$registeredVersion', expected '$version2'"
    }
    if ($registration.DisplayName -cne "WarpAgentCLIDev $version2") {
        throw "ARP registered unexpected display name '$($registration.DisplayName)'"
    }

    Stop-Process -Id $runningProcess.Id -Force
    $runningProcess.WaitForExit()
    $runningProcess = $null
    $untrustedBinDir = Join-Path $testRoot 'untrusted command root'
    $untrustedLauncher = Join-Path $untrustedBinDir 'warp-dev.cmd'
    Write-FixtureFile $untrustedLauncher 'must remain'
    Invoke-Installer (Join-Path $installDir 'unins000.exe') $installDir $untrustedBinDir -Uninstall
    if (Test-Path -LiteralPath $launcherPath) {
        throw 'Uninstall left the channel launcher behind'
    }
    if ((Get-Content -LiteralPath $untrustedLauncher -Raw) -cne 'must remain') {
        throw 'Uninstall honored an untrusted command-directory override'
    }
    if (Test-Path -LiteralPath $registryPath) {
        throw 'Uninstall left the ARP registration behind'
    }
} finally {
    if ($runningProcess -and -not $runningProcess.HasExited) {
        Stop-Process -Id $runningProcess.Id -Force
    }
    if (Test-Path -LiteralPath $testRoot) {
        Remove-Item -LiteralPath $testRoot -Recurse -Force
    }
}

Write-Output 'Windows TUI installer tests passed'