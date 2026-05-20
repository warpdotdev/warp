git rev-parse --is-inside-work-tree >/dev/null 2>&1 || exit 0
git symbolic-ref --quiet --short HEAD >/dev/null 2>&1 || exit 0

remote_url=$(git remote get-url origin 2>/dev/null) || exit 0
case "$remote_url" in
    git@github.com:*|https://github.com/*|http://github.com/*|ssh://git@github.com/*)
        ;;
    *)
        exit 0
        ;;
esac

output=$(gh pr view --state all --json url,state,isDraft,mergeable,mergeStateStatus \
    --jq '{url,state,isDraft,mergeable,mergeStateStatus}' 2>&1)
exit_code=$?

if [ $exit_code -eq 0 ]; then
    printf '%s\n' "$output"
else
    case "$output" in
        *'no pull requests found for branch '*|*'no open pull requests found for branch '*)
            exit 0
            ;;
    esac
    printf '%s\n' "$output" >&2
    exit $exit_code
fi
