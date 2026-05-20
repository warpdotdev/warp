git rev-parse --is-inside-work-tree 2>$null | Out-Null
if ($LASTEXITCODE -ne 0) { exit 0 }

$branch = git symbolic-ref --quiet --short HEAD 2>$null
if ($LASTEXITCODE -ne 0 -or [string]::IsNullOrWhiteSpace($branch)) { exit 0 }

$remoteUrl = git remote get-url origin 2>$null
if ($LASTEXITCODE -ne 0) { exit 0 }
if ($remoteUrl -notmatch '^(git@github\.com:|https?://github\.com/|ssh://git@github\.com/)') { exit 0 }

$output = gh pr view --state all --json url,state,isDraft,mergeable,mergeStateStatus `
    --jq '{url,state,isDraft,mergeable,mergeStateStatus}' 2>&1 | Out-String
$exitCode = $LASTEXITCODE
$output = $output.TrimEnd()

if ($exitCode -eq 0) {
    if (-not [string]::IsNullOrWhiteSpace($output)) { $output }
    exit 0
}

if ($output -match 'no (open )?pull requests found for branch ') { exit 0 }
if (-not [string]::IsNullOrWhiteSpace($output)) { [Console]::Error.WriteLine($output) }
exit $exitCode
