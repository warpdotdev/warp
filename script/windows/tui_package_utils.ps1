Set-StrictMode -Version 3.0

function New-DeterministicZip {
    param(
        [Parameter(Mandatory = $true)]
        [String]$SourceDirectory,

        [Parameter(Mandatory = $true)]
        [String]$DestinationPath
    )

    Add-Type -AssemblyName System.IO.Compression
    Add-Type -AssemblyName System.IO.Compression.FileSystem

    $SourceRoot = [System.IO.Path]::GetFullPath($SourceDirectory).TrimEnd(
        [System.IO.Path]::DirectorySeparatorChar,
        [System.IO.Path]::AltDirectorySeparatorChar
    )
    if (-Not (Test-Path $SourceRoot -PathType Container)) {
        throw "Zip source directory does not exist: $SourceRoot"
    }

    $DestinationFullPath = [System.IO.Path]::GetFullPath($DestinationPath)
    $DestinationDirectory = Split-Path -Parent $DestinationFullPath
    New-Item -ItemType Directory -Path $DestinationDirectory -Force | Out-Null
    Remove-Item -Path $DestinationFullPath -Force -ErrorAction SilentlyContinue

    $EpochSeconds = 315532800
    if ($env:SOURCE_DATE_EPOCH) {
        $ParsedEpoch = 0L
        if (-Not [long]::TryParse($env:SOURCE_DATE_EPOCH, [ref]$ParsedEpoch)) {
            throw "SOURCE_DATE_EPOCH must be an integer, got '$env:SOURCE_DATE_EPOCH'"
        }
        $EpochSeconds = [Math]::Max($ParsedEpoch, 315532800)
    }
    $EntryTimestamp = [DateTimeOffset]::FromUnixTimeSeconds($EpochSeconds)

    $Archive = [System.IO.Compression.ZipFile]::Open(
        $DestinationFullPath,
        [System.IO.Compression.ZipArchiveMode]::Create
    )
    try {
        $PrefixLength = $SourceRoot.Length + 1
        $Files = Get-ChildItem -Path $SourceRoot -File -Recurse |
            Sort-Object { $_.FullName.Substring($PrefixLength).Replace('\', '/') }

        foreach ($File in $Files) {
            $EntryName = $File.FullName.Substring($PrefixLength).Replace('\', '/')
            $Entry = $Archive.CreateEntry(
                $EntryName,
                [System.IO.Compression.CompressionLevel]::Optimal
            )
            $Entry.LastWriteTime = $EntryTimestamp

            $Input = [System.IO.File]::OpenRead($File.FullName)
            $Output = $Entry.Open()
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

function Assert-ValidAuthenticodeSignature {
    param(
        [Parameter(Mandatory = $true)]
        [String[]]$Path
    )

    if (-Not (Get-Command Get-AuthenticodeSignature -ErrorAction SilentlyContinue)) {
        throw 'Get-AuthenticodeSignature is unavailable; signature validation requires Windows PowerShell.'
    }

    foreach ($FilePath in $Path) {
        $Signature = Get-AuthenticodeSignature -FilePath $FilePath
        if ($Signature.Status -ne [System.Management.Automation.SignatureStatus]::Valid) {
            throw "Invalid Authenticode signature for '$FilePath': $($Signature.Status) ($($Signature.StatusMessage))"
        }
    }
}
