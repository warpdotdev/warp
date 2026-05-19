param(
    [string]$Name = "google-pixel-9-pro-fold-x86",
    [string]$IsoPath,
    [ValidateRange(1, 4096)]
    [int]$DiskSizeGB = 16,
    [ValidateRange(512, 1048576)]
    [int]$MemoryMB = 4096,
    [ValidateRange(1, 256)]
    [int]$CpuCount = 4,
    [string]$VmRoot = (Join-Path $PSScriptRoot "vms"),
    [switch]$Install,
    [switch]$Live,
    [switch]$NoAccel
)

$ErrorActionPreference = "Stop"

function Require-Command {
    param([string]$CommandName)

    $command = Get-Command $CommandName -ErrorAction SilentlyContinue
    if (-not $command) {
        throw "$CommandName was not found on PATH. Restart your shell or add the QEMU install directory to PATH."
    }

    return $command.Source
}

function Resolve-IsoFile {
    param([string]$Path)

    $resolvedPath = Resolve-Path -LiteralPath $Path -ErrorAction Stop
    $item = Get-Item -LiteralPath $resolvedPath.Path -ErrorAction Stop
    if (-not $item.PSIsContainer) {
        return $item.FullName
    }

    throw "ISO path must point to a file: $Path"
}

if ($Install -and $Live) {
    throw "Use either -Install or -Live, not both."
}

if (($Install -or $Live) -and -not $IsoPath) {
    throw "-Install and -Live require -IsoPath."
}

$qemuImg = Require-Command "qemu-img"
$qemuSystem = Require-Command "qemu-system-x86_64"

$vmDir = Join-Path $VmRoot $Name
$diskPath = Join-Path $vmDir "$Name.qcow2"

New-Item -ItemType Directory -Force -Path $vmDir | Out-Null

if (-not $Live) {
    if (-not (Test-Path $diskPath)) {
        Write-Host "Creating Android VM disk: $diskPath ($DiskSizeGB GB)"
        & $qemuImg create -f qcow2 $diskPath "$($DiskSizeGB)G"
        if ($LASTEXITCODE -ne 0) {
            throw "qemu-img failed with exit code $LASTEXITCODE."
        }
    } else {
        Write-Host "Using existing Android VM disk: $diskPath"
    }
}

$qemuArgs = @(
    "-m", $MemoryMB,
    "-smp", $CpuCount,
    "-vga", "std",
    "-usb",
    "-device", "usb-tablet",
    "-netdev", "user,id=net0",
    "-device", "e1000,netdev=net0"
)

if (-not $NoAccel) {
    $qemuArgs = @("-accel", "whpx") + $qemuArgs
}

if (-not $Live) {
    $qemuArgs += @("-drive", "file=$diskPath,format=qcow2,if=ide")
}

if ($Install -or $Live) {
    $resolvedIso = Resolve-IsoFile $IsoPath
    Write-Host "Booting Android ISO: $resolvedIso"
    $qemuArgs += @("-cdrom", $resolvedIso, "-boot", "d")
}

if ($Live) {
    Write-Host "Starting Android live session with $MemoryMB MB RAM and $CpuCount CPU(s)."
} elseif ($Install) {
    Write-Host "Starting Android installer for VM '$Name' with $MemoryMB MB RAM and $CpuCount CPU(s)."
} else {
    Write-Host "Starting installed Android VM '$Name' with $MemoryMB MB RAM and $CpuCount CPU(s)."
}

Write-Host "Press Ctrl+Alt+G to release mouse/keyboard capture from the QEMU window."

& $qemuSystem @qemuArgs
exit $LASTEXITCODE
