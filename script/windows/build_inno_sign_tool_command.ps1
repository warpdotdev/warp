[CmdletBinding()]
param()

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$dlib = (
    Get-ChildItem -Recurse $env:LOCALAPPDATA -Filter 'Azure.CodeSigning.Dlib.dll' `
        -ErrorAction SilentlyContinue |
        Where-Object { $_.FullName -match 'x64' } |
        Select-Object -First 1
).FullName
if (-not $dlib) {
    throw 'Azure.CodeSigning.Dlib.dll was not installed by the signing action'
}

$signTool = (
    Get-ChildItem -Recurse 'C:\Program Files (x86)\Windows Kits' -Filter 'signtool.exe' |
        Where-Object { $_.FullName -match 'x64' } |
        Sort-Object FullName -Descending |
        Select-Object -First 1
).FullName
if (-not $signTool) {
    throw 'signtool.exe was not found in the Windows SDK'
}
if (-not $env:GITHUB_OUTPUT) {
    throw 'GITHUB_OUTPUT is required'
}

(Split-Path $signTool) >> $env:GITHUB_PATH
$metadataPath = Join-Path $env:RUNNER_TEMP 'tsc-metadata.json'
@{
    Endpoint = $env:TRUSTED_SIGNING_ENDPOINT
    CodeSigningAccountName = $env:TRUSTED_SIGNING_ACCOUNT
    CertificateProfileName = $env:TRUSTED_SIGNING_CERT_PROFILE
} | ConvertTo-Json | Set-Content $metadataPath

"sign_tool_cmd=signtool.exe sign /v /fd SHA256 /tr http://timestamp.acs.microsoft.com /td SHA256 /dlib $dlib /dmdf $metadataPath `$f" >> $env:GITHUB_OUTPUT