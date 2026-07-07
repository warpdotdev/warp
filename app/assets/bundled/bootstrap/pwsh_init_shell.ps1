# Prevent history from being written to file, among other interactive features.
Remove-Module -Name PSReadline

$global:_warpOriginalPrompt = $function:global:prompt

if ($PSEdition -eq 'Desktop' -or $IsWindows) {
    $EP = [Microsoft.PowerShell.ExecutionPolicy]
    # MachinePolicy and UserPolicy scopes cannot be overridden. If either is Restricted, there's nothing we can do.
    if ((Get-ExecutionPolicy -Scope MachinePolicy) -eq $EP::Restricted -or (Get-ExecutionPolicy -Scope UserPolicy) -eq $EP::Restricted) {
        Write-Error 'ExecutionPolicy is Restricted. Unable to Warpify this PowerShell session.'
    } elseif ((Get-ExecutionPolicy) -eq $EP::Restricted -and (Get-ExecutionPolicy -Scope MachinePolicy) -eq $EP::Undefined -and (Get-ExecutionPolicy -Scope UserPolicy) -eq $EP::Undefined) {
        $global:_warp_PSProcessExecPolicy = $(Get-ExecutionPolicy -Scope Process)
        Set-ExecutionPolicy -Scope Process -ExecutionPolicy RemoteSigned -Force
    }
}

# We must wait until pwsh attempts to show the first prompt before writing an OSC string.
# Trying to do so beforehand will prevent the "Write-Host" command from being submitted, as pwsh
# ignores submissions prior to the first prompt.
function prompt {
    # Reset the prompt back to the default to avoid infinite loops if sourcing the bootstrap script has an error.
    $function:global:prompt = $global:_warpOriginalPrompt
    $username = [Environment]::UserName
    $global:_warpSessionId = [uint64]@@WARP_SESSION_ID@@
    $msg = ConvertTo-Json -Compress -InputObject @{ hook = 'InitShell'; value = @{ session_id = $_warpSessionId; shell = 'pwsh'; user = $username; hostname = [System.Net.Dns]::GetHostName() } }
    $encodedMsg = [BitConverter]::ToString([System.Text.Encoding]::UTF8.GetBytes($msg)).Replace('-', '')
    $oscStart = "$([char]0x1b)]9278;"
    $oscEnd = "`a"
    $oscJsonMarker = 'd'
    $oscParameterSeparator = ';'
    Write-Host "${oscStart}${oscJsonMarker}${oscParameterSeparator}${encodedMsg}${oscEnd}"
    return $null
}
