#!/usr/bin/env powershell
#
# Bundle the application for release.

Param (
    # Build dev bundles by default.
    [Switch]$DEBUG_BUILD = $False,

    [Alias('check-only')]
    [Switch]$CHECK_ONLY,
    [ValidateSet('app', 'tui')]
    [String]$ARTIFACT = 'app',

    [ValidateSet('local', 'dev', 'preview', 'stable', 'oss')]
    [String]$CHANNEL = 'dev',

    [Alias('release-tag')]
    [String]$RELEASE_TAG = '',
    [String]$FEATURES = 'release_bundle,crash_reporting,gui',

    # Builds only the Warp binary, skips the installer.
    [Switch]$SKIP_BUILD_INSTALLER = $False,
    # Builds only the installer, skips the Warp binary. Use this if the Warp
    # binary has already been built.
    [Switch]$SKIP_BUILD_BINARY = $False,

    [ValidateSet('x64', 'arm64')]
    [String]$ARCH = '',
    [Switch]$REQUIRE_SIGNATURES = $False,

    # A signtool command for Inno Setup to sign the setup engine and uninstaller.
    # Uses $f as the file placeholder, e.g.:
    #   'signtool.exe sign /fd SHA256 ... $f'
    # When empty, the installer is built without signing.
    [Alias('sign-tool-cmd')]
    [String]$SIGN_TOOL_CMD = ''
)

if ($RELEASE_TAG) {
    $env:GIT_RELEASE_TAG = $RELEASE_TAG
}

# Use provided ARCH parameter if set, otherwise detect from system
if (-not $ARCH) {
    if ($env:PROCESSOR_ARCHITECTURE -eq 'AMD64') {
        $ARCH = 'x64'
    } elseif ($env:PROCESSOR_ARCHITECTURE -eq 'ARM64') {
        $ARCH = 'arm64'
    } else {
        throw "Unsupported processor architecture: $env:PROCESSOR_ARCHITECTURE"
    }
}

if ($ARCH -eq 'arm64') {
    $FILE_ENDING = 'Setup-arm64'
    $PLATFORM_TARGET = 'aarch64-pc-windows-msvc'
} else {
    # If x64, then we just use the filename "WarpSetup.exe" for example
    $FILE_ENDING = 'Setup'
    $PLATFORM_TARGET = 'x86_64-pc-windows-msvc'
}

# Windows-on-ARM64 hosts can run x64 binaries via built-in emulation, but x64
# hosts cannot run arm64 binaries at all, so only that direction is unsafe.
# Resolve it once so the settings-schema default below (which assumes the
# just-built binary is executable) can fail clearly instead of the process
# simply failing to start.
#
# PROCESSOR_ARCHITECTURE reports the architecture of the running process, not
# the host: an x64 PowerShell process running under WOW64 on a Windows-on-ARM
# machine reports AMD64 even though the host is natively ARM64.
# PROCESSOR_ARCHITEW6432 carries the true native host architecture in that
# case, so prefer it when present.
$NATIVE_PROCESSOR_ARCHITECTURE = if ($env:PROCESSOR_ARCHITEW6432) {
    $env:PROCESSOR_ARCHITEW6432
} else {
    $env:PROCESSOR_ARCHITECTURE
}
$HOST_ARCH = if ($NATIVE_PROCESSOR_ARCHITECTURE -eq 'AMD64') {
    'x64'
} elseif ($NATIVE_PROCESSOR_ARCHITECTURE -eq 'ARM64') {
    'arm64'
} else {
    throw "Unsupported host architecture: $NATIVE_PROCESSOR_ARCHITECTURE"
}
$CAN_EXECUTE_ARCH = -not ($ARCH -eq 'arm64' -and $HOST_ARCH -eq 'x64')

$ErrorActionPreference = 'Stop'

function Assert-ValidSignature {
    [Diagnostics.CodeAnalysis.SuppressMessageAttribute(
        'PSUseCompatibleCommands',
        '',
        Justification = 'Release signature validation only runs on Windows.'
    )]
    param([string] $Path)

    $signature = Get-AuthenticodeSignature -LiteralPath $Path
    if ("$($signature.Status)" -ne 'Valid') {
        throw "File does not have a valid Authenticode signature: $Path ($($signature.Status))"
    }
}

$WORKSPACE_ROOT_DIR = $PWD.Path
$CARGO_TARGET_DIR = $WORKSPACE_ROOT_DIR + '\target'
$WINDOWS_INSTALLER_DIR = $WORKSPACE_ROOT_DIR + '\script\windows'
$IS_TUI = $ARTIFACT -eq 'tui'

if ($DEBUG_BUILD) {
    $CARGO_PROFILE = 'dev'
} elseif ($IS_TUI -and (("$CHANNEL" -eq 'local') -or ("$CHANNEL" -eq 'dev'))) {
    $CARGO_PROFILE = 'rclida'
} elseif ($IS_TUI) {
    $CARGO_PROFILE = 'rcli'
} elseif (("$CHANNEL" -eq 'local') -or ("$CHANNEL" -eq 'dev')) {
    # For dev bundles, we want to enable debug assertions to
    # catch violations that would otherwise silently pass in
    # a normal release build (e.g. in stable).
    $CARGO_PROFILE = 'rltoda'
} else {
    $CARGO_PROFILE = 'rlto'
}

if ($CARGO_PROFILE -eq 'dev') {
    $CARGO_TARGET_OUTPUT_DIR = "$CARGO_TARGET_DIR" + '\' + $PLATFORM_TARGET + '\debug'
} else {
    $CARGO_TARGET_OUTPUT_DIR = "$CARGO_TARGET_DIR" + '\' + $PLATFORM_TARGET + '\' + "$CARGO_PROFILE"
}
$BUNDLE_ID = "dev.warp.$app_name"

# Update parameters based on the target release channel.
#
# APP_NAME here must match the value used in Rust as the
# application name; see app/src/channel.rs.
#
# WARP_BIN is the name of the binary produced by cargo;
# BINARY_NAME is the desired name of the binary in the final package.
if ("$CHANNEL" -eq 'local') {
    $WARP_BIN = 'warp'
    $BINARY_NAME = 'warp.exe'
    $APP_NAME = 'WarpLocal'
} elseif ("$CHANNEL" -eq 'dev') {
    $WARP_BIN = 'dev'
    $BINARY_NAME = 'dev.exe'
    $APP_NAME = 'WarpDev'
    $FEATURES = "$FEATURES,agent_mode_debug"
} elseif ("$CHANNEL" -eq 'preview') {
    $WARP_BIN = 'preview'
    $BINARY_NAME = 'preview.exe'
    $APP_NAME = 'WarpPreview'
    $FEATURES = "$FEATURES,preview_channel"
} elseif ("$CHANNEL" -eq 'stable') {
    $WARP_BIN = 'stable'
    $BINARY_NAME = 'warp.exe'
    $APP_NAME = 'Warp'
} elseif ("$CHANNEL" -eq 'oss') {
    $WARP_BIN = 'warp-oss'
    $BINARY_NAME = 'warp-oss.exe'
    $APP_NAME = 'WarpOss'
    # The OSS channel does not ship Sentry, so drop the crash_reporting feature
    # (which would otherwise pull in the Sentry SDK as a dependency).
    $FEATURES = 'release_bundle,gui'
}

if ($IS_TUI) {
    $WARP_BIN = switch ($CHANNEL) {
        'local' { 'warp-tui' }
        'oss' { 'warp-tui-oss' }
        Default { "warp-tui-$CHANNEL" }
    }
    $BINARY_NAME = "$WARP_BIN.exe"
    $APP_NAME = switch ($CHANNEL) {
        'local' { 'WarpAgentCLI' }
        'dev' { 'WarpAgentCLIDev' }
        'preview' { 'WarpAgentCLIPreview' }
        'stable' { 'WarpAgentCLI' }
        'oss' { 'WarpAgentCLIOss' }
    }
    $CLI_NAME = switch ($CHANNEL) {
        'local' { 'warp' }
        'dev' { 'warp-dev' }
        'preview' { 'warp-preview' }
        'stable' { 'warp' }
        'oss' { 'warp-oss' }
    }
    $INSTALL_DIR_NAME = switch ($CHANNEL) {
        'local' { 'tui-local' }
        'dev' { 'tui-dev' }
        'preview' { 'tui-preview' }
        'stable' { 'tui' }
        'oss' { 'tui-oss' }
    }
    $FEATURES = 'release_bundle,standalone,voice_input'
    if ("$CHANNEL" -ne 'oss') {
        $FEATURES = "$FEATURES,crash_reporting"
    }
} else {
    # All app channels ship the v3 classifier and v2 heuristic.
    $FEATURES = "$FEATURES,nld_classifier_v3,nld_heuristic_v2"
}

$BINARY_PATH = "$CARGO_TARGET_OUTPUT_DIR\$BINARY_NAME"
$BUNDLE_ID = "dev.warp.$APP_NAME"
$INSTALLER_OUTPUT_DIR = "$WINDOWS_INSTALLER_DIR\Output"
$INSTALLER_NAME = "$($APP_NAME)$($FILE_ENDING)"
$INSTALLER_PATH = "$($INSTALLER_OUTPUT_DIR)\$($INSTALLER_NAME).exe"
$PDB_BASENAME = if ($IS_TUI) {
    # rustc normalizes hyphens to underscores in crate names, and MSVC uses
    # that normalized crate name for the PDB even though Cargo exposes the
    # executable under its original hyphenated target name.
    $WARP_BIN.Replace('-', '_')
} else {
    $WARP_BIN
}
$PDB_PATH = "$CARGO_TARGET_OUTPUT_DIR\$PDB_BASENAME.pdb"
$CARGO_PACKAGE = if ($IS_TUI) { 'warp_tui' } else { 'warp' }
$INSTALLER_SCRIPT = if ($IS_TUI) {
    "$WINDOWS_INSTALLER_DIR\tui-installer.iss"
} else {
    "$WINDOWS_INSTALLER_DIR\windows-installer.iss"
}

# The CARGO_FULL_PROFILE environment variable is read by the `cargo` build
# script (`app/build.rs`) to determine where to place `conpty.dll`.
if ($DEBUG_BUILD) {
    $env:CARGO_FULL_PROFILE = 'debug'
} else {
    $env:CARGO_FULL_PROFILE = $CARGO_PROFILE
}

# If we only want to check that compilation will succeed, perform the checks
# then exit.  We use this script to invoke `cargo check` to ensure that we are
# using the same feature flags and profile that we would be using in production.
if ($CHECK_ONLY) {
    cargo check -p $CARGO_PACKAGE --profile "$CARGO_PROFILE" --bin "$WARP_BIN" --features "$FEATURES" --target $PLATFORM_TARGET
    if (-Not $?) {
        Write-Error "Failed to verify Warp $WARP_BIN compilation with profile $CARGO_PROFILE"
        exit 1
    }
    exit 0
}

if (-Not $SKIP_BUILD_BINARY) {
    Write-Output "Building Warp for channel $CHANNEL and bundle id $BUNDLE_ID"
    $env:CARGO_BIN_NAME = $CHANNEL
    $env:WARP_APP_NAME = $APP_NAME
    cargo build -p $CARGO_PACKAGE --profile "$CARGO_PROFILE" --bin "$WARP_BIN" --features "$FEATURES" --target $PLATFORM_TARGET
    if (-Not $?) {
        Write-Error "Failed to build Warp $WARP_BIN binary with profile $CARGO_PROFILE"
        exit 1
    }

    # If we desire an executable name different from the cargo bin, rename it.
    if ("$WARP_BIN.exe" -ne $BINARY_NAME) {
        $binarySource = "$CARGO_TARGET_OUTPUT_DIR\$WARP_BIN.exe"
        Write-Output "Renaming executable $WARP_BIN.exe to $BINARY_NAME"
        Move-Item -Path "$binarySource" -Destination "$BINARY_PATH" -Force
    }
}

if ($SKIP_BUILD_INSTALLER) {
    # If this is being run within a GitHub action, set an output variable with the
    # location of the binary so it can be referenced by subsequent actions.
    if ($env:GITHUB_ACTIONS -eq 'true') {
        Write-Output '::echo::on'
        "target_profile_dir=$CARGO_TARGET_OUTPUT_DIR" >> "$env:GITHUB_OUTPUT"
        "binary_path=$BINARY_PATH" >> "$env:GITHUB_OUTPUT"
        "pdb_file_path=$PDB_PATH" >> "$env:GITHUB_OUTPUT"
        Write-Output '::echo::off'
    }
    exit 0
}

Write-Output "Built for $ARCH with executable at $BINARY_PATH"

# Prepare bundled resources
if ($env:SKIP_SETTINGS_SCHEMA -ne '1' -and -not $env:SETTINGS_SCHEMA_EXECUTABLE -and -not $env:SETTINGS_SCHEMA_SOURCE) {
    if ($IS_TUI) {
        Write-Error 'TUI bundles require SETTINGS_SCHEMA_SOURCE or SETTINGS_SCHEMA_EXECUTABLE.'
        exit 1
    } elseif ($SKIP_BUILD_BINARY) {
        Write-Error '-skip_build_binary requires SETTINGS_SCHEMA_SOURCE or SETTINGS_SCHEMA_EXECUTABLE.'
        exit 1
    } elseif (-not $CAN_EXECUTE_ARCH) {
        Write-Error "Cannot execute the just-built $ARCH binary on this $HOST_ARCH host to generate a settings schema; pass SETTINGS_SCHEMA_SOURCE or SETTINGS_SCHEMA_EXECUTABLE."
        exit 1
    }
    $env:SETTINGS_SCHEMA_EXECUTABLE = $BINARY_PATH
}
$BUNDLED_RESOURCES_DIR = "$CARGO_TARGET_OUTPUT_DIR\resources"
Write-Output 'Preparing bundled resources...'
& "$WINDOWS_INSTALLER_DIR\prepare_bundled_resources.ps1" -DestinationDir "$BUNDLED_RESOURCES_DIR" -Channel "$CHANNEL"
if (-Not $?) {
    Write-Error 'Failed to prepare bundled resources'
    exit 1
}
if ($IS_TUI) {
    $WINDOWS_ASSETS_DIR = "$WORKSPACE_ROOT_DIR\app\assets\windows\$ARCH"
    $requiredPayloadFiles = @(
        $BINARY_PATH,
        (Join-Path $WINDOWS_ASSETS_DIR 'conpty.dll'),
        (Join-Path $WINDOWS_ASSETS_DIR 'OpenConsole.exe'),
        (Join-Path $WINDOWS_ASSETS_DIR 'vcruntime140.dll'),
        (Join-Path $WINDOWS_ASSETS_DIR 'vcruntime140_1.dll'),
        (Join-Path $WINDOWS_ASSETS_DIR 'msvcp140.dll')
    )
    foreach ($requiredFile in $requiredPayloadFiles) {
        if (-not (Test-Path -LiteralPath $requiredFile -PathType Leaf)) {
            throw "Required Warp Agent CLI payload file does not exist: $requiredFile"
        }
        if ($REQUIRE_SIGNATURES) {
            Assert-ValidSignature -Path $requiredFile
        }
    }
}

Write-Output 'Building Warp installer'
$ISCC_ARGS = @(
    "$INSTALLER_SCRIPT",
    "/DReleaseChannel=$CHANNEL",
    "/DMyAppExeName=$BINARY_NAME",
    "/DTargetProfileDir=$CARGO_TARGET_OUTPUT_DIR",
    "/DMyAppName=$APP_NAME",
    "/DMyAppVersion=$env:GIT_RELEASE_TAG",
    "/DArch=$ARCH",
    "/DOutputName=$INSTALLER_NAME"
)
if ($IS_TUI) {
    $ISCC_ARGS += @(
        "/DWindowsAssetsDir=$WINDOWS_ASSETS_DIR",
        "/DCLIName=$CLI_NAME",
        "/DInstallDirName=$INSTALL_DIR_NAME"
    )
}
# Also accept the sign tool command via env var
if (-not $SIGN_TOOL_CMD -and $env:SIGN_TOOL_CMD) {
    $SIGN_TOOL_CMD = $env:SIGN_TOOL_CMD
}
if ($SIGN_TOOL_CMD) {
    $ISCC_ARGS += '/DSIGN_TOOL=1'
    $ISCC_ARGS += "/Scodesign=$SIGN_TOOL_CMD"
}
& ISCC @ISCC_ARGS
if (-Not $?) {
    Write-Error "Failed to build $APP_NAME installer"
    exit 1
}

# If this is being run within a GitHub action, set an output variable with the
# location of the installer so it can be referenced by subsequent actions.
if ($env:GITHUB_ACTIONS -eq 'true') {
    Write-Output '::echo::on'
    $INSTALLER_PATH = $INSTALLER_PATH -replace '\\', '/'
    "installer_path=$INSTALLER_PATH" >> "$env:GITHUB_OUTPUT"
    "pdb_file_path=$PDB_PATH" >> "$env:GITHUB_OUTPUT"
    Write-Output '::echo::off'
}

if ($IS_TUI) {
    Write-Output "Application installer: $INSTALLER_PATH"
}
