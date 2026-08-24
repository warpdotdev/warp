[CmdletBinding()]
param(
    [string] $Version = 'latest'
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

# We resolve the sentry-cli download ourselves rather than using matbour/setup-sentry-cli,
# whose platform table has no win32/arm64 entry and throws outright on the native Windows
# arm64 runners. getsentry/sentry-cli does publish a Windows aarch64 build.
$arch = switch ($env:RUNNER_ARCH) {
    'X64' { 'x86_64' }
    'ARM64' { 'aarch64' }
    'X86' { 'i686' }
    default { throw "Unsupported runner architecture: '$env:RUNNER_ARCH'" }
}

if (-not $env:GITHUB_PATH) {
    throw 'GITHUB_PATH is required'
}
if (-not $env:GITHUB_ENV) {
    throw 'GITHUB_ENV is required'
}

$installDir = Join-Path $env:RUNNER_TEMP 'sentry-cli'
New-Item -ItemType Directory -Force -Path $installDir | Out-Null
$executable = Join-Path $installDir 'sentry-cli.exe'

$uri = "https://downloads.sentry-cdn.com/sentry-cli/$Version/sentry-cli-Windows-$arch.exe"
Write-Host "Installing sentry-cli ($Version) for $arch from $uri"

# Invoke-WebRequest only grew -MaximumRetryCount in PowerShell 6.1, and this tree is linted
# for Windows PowerShell 5.1 compatibility, so retry by hand. A transient CDN blip should
# not sink a release leg that has already spent an hour building and signing.
$maxAttempts = 3
for ($attempt = 1; ; $attempt++) {
    try {
        Invoke-WebRequest -Uri $uri -OutFile $executable
        break
    } catch {
        if ($attempt -ge $maxAttempts) {
            throw
        }
        Write-Host "Attempt $attempt of $maxAttempts failed: $($_.Exception.Message)"
        Start-Sleep -Seconds 5
    }
}

$installDir >> $env:GITHUB_PATH

# Forward the caller's Sentry configuration to later steps, matching what
# matbour/setup-sentry-cli exported on the other platforms.
foreach ($name in @('SENTRY_URL', 'SENTRY_AUTH_TOKEN', 'SENTRY_ORG', 'SENTRY_PROJECT')) {
    $value = [Environment]::GetEnvironmentVariable($name)
    if ($value) {
        "$name=$value" >> $env:GITHUB_ENV
    }
}

& $executable --version
