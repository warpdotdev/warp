git rev-parse --is-inside-work-tree >/dev/null 2>/dev/null; or exit 0
git symbolic-ref --quiet --short HEAD >/dev/null 2>/dev/null; or exit 0

set remote_url (git remote get-url origin 2>/dev/null); or exit 0
string match -rq '^(git@github\.com:|https?://github\.com/|ssh://git@github\.com/)' -- $remote_url; or exit 0

set output (gh pr view --json url,number,state,isDraft,baseRefName --jq '{url: .url, number: .number, state: .state, draft: .isDraft, base_branch: .baseRefName}' 2>&1)
set exit_code $status

if test $exit_code -eq 0
    printf '%s\n' "$output"
else
    set joined_output (string join '\n' $output)
    string match -rq 'no (open )?pull requests found for branch ' -- $joined_output; and exit 0
    printf '%s\n' "$joined_output" >&2
    exit $exit_code
end
