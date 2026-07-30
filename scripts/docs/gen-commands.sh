#!/usr/bin/env bash
# Renders the repo's runnable commands as a markdown table. Stdout only.
# Deterministic: same manifests -> byte-identical output.
# Portable: runs in any git repo, no arguments. jq is used when present.
set -euo pipefail

# Sources, in the order they are emitted: package.json scripts, Makefile
# targets, Cargo aliases, pyproject scripts, and executable scripts/*.sh. Each
# row's description comes from the source itself (the script body, the target's
# preceding comment, the shell script's header comment) — never invented.

cd "$(git rev-parse --show-toplevel)"

rows=""
emit() { rows+="| \`$1\` | $2 | $3 |"$'\n'; }

# ------------------------------------------------------------ package.json
if [[ -f package.json ]]; then
  if command -v jq >/dev/null 2>&1; then
    while IFS=$'\t' read -r name body; do
      [[ -z "$name" ]] && continue
      emit "npm run $name" "package.json" "\`${body//|/\\|}\`"
    done < <(jq -r '.scripts // {} | to_entries | sort_by(.key) | .[] | "\(.key)\t\(.value)"' package.json)
  else
    echo "gen-commands: package.json present but jq missing — script rows omitted" >&2
  fi
fi

# ---------------------------------------------------------------- Makefile
if [[ -f Makefile ]]; then
  while IFS= read -r target; do
    [[ -z "$target" ]] && continue
    # A `## comment` on the target line is the conventional self-documenting form.
    # `|| true`: under pipefail a no-match grep, or a SIGPIPE from head closing
    # the pipe early, would otherwise abort the whole script.
    desc="$(grep -E "^${target}:" Makefile 2>/dev/null | head -1 | sed -nE 's/.*##[[:space:]]*(.*)/\1/p' || true)"
    [[ -z "$desc" ]] && desc="—"
    emit "make $target" "Makefile" "$desc"
  done < <(grep -oE '^[a-zA-Z0-9_.-]+:' Makefile | tr -d ':' | sort -u)
fi

# ------------------------------------------------------------- Cargo.toml
if [[ -f Cargo.toml ]]; then
  emit "cargo build" "Cargo.toml" "Build the crate"
  emit "cargo test" "Cargo.toml" "Run the test suite"
  emit "cargo clippy" "Cargo.toml" "Lint"
fi

# ----------------------------------------------------------- pyproject.toml
if [[ -f pyproject.toml ]]; then
  while IFS= read -r name; do
    [[ -z "$name" ]] && continue
    emit "$name" "pyproject.toml" "Console script entrypoint"
  done < <(awk '
    /^\[project\.scripts\]/ { in_s = 1; next }
    /^\[/ { in_s = 0 }
    in_s && /=/ { split($0, a, "="); gsub(/[[:space:]"]/, "", a[1]); if (a[1] != "") print a[1] }
  ' pyproject.toml | sort -u)
fi

# ------------------------------------------------------------- scripts/*.sh
if [[ -d scripts ]]; then
  while IFS= read -r s; do
    [[ -x "$s" ]] || continue
    # First comment line under the shebang is the script's self-description.
    # `|| true`: a script whose lines 2-4 carry no comment yields a no-match
    # grep, which under pipefail would abort the whole script.
    desc="$(sed -n '2,4p' "$s" | grep -m1 '^#' | sed -E 's/^#[[:space:]]?//' | sed 's/|/\\|/g' || true)"
    [[ -z "$desc" ]] && desc="—"
    emit "$s" "shell" "$desc"
  done < <(git ls-files -- 'scripts/*.sh' | sort)
fi

if [[ -z "$rows" ]]; then
  printf '_No runnable commands detected._\n'
  exit 0
fi

printf '| Command | Source | What it does |\n'
printf '|---------|--------|--------------|\n'
printf '%s' "$rows"
