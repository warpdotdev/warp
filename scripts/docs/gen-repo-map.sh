#!/usr/bin/env bash
# Renders the repo's top-level structure as a markdown table. Stdout only.
# Deterministic: same tree -> byte-identical output.
# Portable: runs in any git repo, no arguments, no jq.
# shellcheck disable=SC2016  # backticks in the printf formats are markdown code
# spans in the rendered table, not command substitution.
set -euo pipefail

# Every column is derived from the repository itself — nothing is inferred or
# invented. The purpose column is the first non-empty, non-heading line of that
# directory's README.md when one exists, and "—" when it does not; a directory
# documents itself or stays blank.

cd "$(git rev-parse --show-toplevel)"

printf '| Path | Tracked files | Primary type | Purpose |\n'
printf '|------|---------------|--------------|---------|\n'

# Top-level tracked directories, sorted. Dotfile dirs are excluded: they are
# tooling surface, not the code map an agent is orienting with.
mapfile -t dirs < <(git ls-files | awk -F/ 'NF > 1 && $1 !~ /^\./ { print $1 }' | sort -u)

for d in "${dirs[@]}"; do
  count="$(git ls-files -- "$d" | wc -l | tr -d ' ')"

  # Dominant file extension by tracked count; ties break alphabetically.
  primary="$(git ls-files -- "$d" \
    | awk -F. 'NF > 1 { print $NF }' \
    | sort | uniq -c | sort -k1,1nr -k2,2 | head -1 \
    | awk '{ if ($2 != "") printf ".%s", $2 }')"
  [[ -z "$primary" ]] && primary="—"

  purpose="—"
  if [[ -f "$d/README.md" ]]; then
    # `|| true`: under pipefail, both a no-match grep and the SIGPIPE head
    # sends when it closes the pipe on a long README would abort the script.
    line="$(grep -vE '^\s*$|^#|^>|^\[!' "$d/README.md" | head -1 | sed 's/|/\\|/g' || true)"
    [[ -n "$line" ]] && purpose="$line"
  fi

  printf '| `%s/` | %s | %s | %s |\n' "$d" "$count" "$primary" "$purpose"
done

# Top-level tracked files carry orientation weight too (entrypoints, manifests).
mapfile -t rootfiles < <(git ls-files | awk -F/ 'NF == 1 && $1 !~ /^\./ { print $1 }' | sort)
if [[ ${#rootfiles[@]} -gt 0 ]]; then
  # Joined with ", " rather than a trailing space per entry, which would leave
  # trailing whitespace that markdown formatters strip — and that strip would
  # then read as generated-block drift on the next lint.
  printf '\n**Root files:** '
  printf '%s' "$(printf '`%s`, ' "${rootfiles[@]}" | sed 's/, $//')"
  printf '\n'
fi
