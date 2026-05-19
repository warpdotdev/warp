git rev-parse --is-inside-work-tree 2>$null | Out-Null
if ($LASTEXITCODE -ne 0) { exit 0 }

$branch = git symbolic-ref --quiet --short HEAD 2>$null
if ($LASTEXITCODE -ne 0 -or [string]::IsNullOrWhiteSpace($branch)) { exit 0 }

$remoteUrl = git remote get-url origin 2>$null
if ($LASTEXITCODE -ne 0) { exit 0 }
if ($remoteUrl -notmatch '^(git@github\.com:|https?://github\.com/|ssh://git@github\.com/)') { exit 0 }

$output = gh pr view --json url,number,state,isDraft,baseRefName --jq '{url: .url, number: .number, state: .state, draft: .isDraft, base_branch: .baseRefName}' 2>&1 | Out-String
$exitCode = $LASTEXITCODE
$output = $output.TrimEnd()

if ($exitCode -eq 0) {
    if (-not [string]::IsNullOrWhiteSpace($output)) { $output }
    exit 0
}

if ($output -match 'no (open )?pull requests found for branch ') { exit 0 }
if (-not [string]::IsNullOrWhiteSpace($output)) { [Console]::Error.WriteLine($output) }
exit $exitCode
