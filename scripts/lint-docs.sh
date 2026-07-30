#!/usr/bin/env bash
# Doc-drift gate. Portable across repos — carries no repo-specific paths.
# Contract: docs/repo-docs-contract.md. Pure local: no network calls.
set -euo pipefail

# Three independent checks over tracked markdown:
#   1. link integrity   — every ](path) ref resolves on disk
#   2. generated blocks — <!-- BEGIN:GENERATED cmd="..." --> matches its renderer
#   3. verification age — <!-- DOC:VERIFIES paths="..." --> stamp is not older
#                         than the last commit touching those paths
#
# --fix rebuilds generated blocks in place. It deliberately does NOT restamp
# verification dates: a "verified" claim asserts that a human or agent re-read
# the doc, and auto-satisfying it would turn check 3 into a formality. Only
# mechanically-derivable content is auto-fixable.

cd "$(git rev-parse --show-toplevel 2>/dev/null || dirname "$0")"

FIX=0
[[ "${1:-}" == "--fix" ]] && FIX=1

fail=0
fixed=0
report() { echo "$@" >&2; fail=1; }

# Enumerate tracked markdown via `git ls-files` rather than `find`: it respects
# .gitignore, so a stale worktree checkout under .claude/worktrees/ can never
# leak into the lint set (the failure mode scripts/lint.sh documents).
mapfile -t files < <(git ls-files -- '*.md' 2>/dev/null || true)
if [[ ${#files[@]} -eq 0 ]]; then
  echo "lint-docs: no tracked markdown found" >&2
  exit 0
fi

# Optional scope exclusions, for repos that vendor third-party documentation the
# owner does not control (a fork's upstream tree, bundled skill docs). Format,
# one per line: a path prefix, then `#`, then the reason. The reason is required
# — an exclusion nobody can justify is how a gate quietly stops covering things.
#
# A prefix that matches nothing is itself reported: the ignore list is subject to
# the same anti-drift rule as everything else it protects.
IGNORE_FILE=".docs-lint-ignore"
ignore_prefixes=()
if [[ -f "$IGNORE_FILE" ]]; then
  while IFS= read -r raw; do
    [[ -z "${raw// }" || "$raw" == \#* ]] && continue
    prefix="${raw%%#*}"
    prefix="${prefix%"${prefix##*[![:space:]]}"}"
    [[ -z "$prefix" ]] && continue
    if [[ "$raw" != *"#"* ]]; then
      report "$IGNORE_FILE:0 exclusion \"$prefix\" carries no \"# reason\""
      continue
    fi
    ignore_prefixes+=("$prefix")
  done < "$IGNORE_FILE"

  for prefix in "${ignore_prefixes[@]}"; do
    matched=0
    for f in "${files[@]}"; do
      [[ "$f" == "$prefix"* ]] && { matched=1; break; }
    done
    (( matched )) || report "$IGNORE_FILE:0 exclusion \"$prefix\" matches no tracked markdown — stale"
  done
fi

is_ignored() {
  local f="$1" p
  for p in "${ignore_prefixes[@]}"; do
    [[ "$f" == "$p"* ]] && return 0
  done
  return 1
}

# Emits "NR:content" for every line outside a fenced code block.
#
# All three checks consume this rather than reading the file directly. A doc
# that documents this contract necessarily shows the markers verbatim, and a
# scanner that cannot tell a live marker from an illustrated one will rewrite
# the documentation of the feature into an instance of it.
nonfenced() {
  awk 'BEGIN { fence = 0 } /^```/ { fence = !fence; next } !fence { print NR ":" $0 }' "$1"
}

# ---------------------------------------------------------------- check 1
# Link integrity. Inline-code path tokens (`foo.sh` in backticks) are NOT
# linted — those are documentation shorthand, not typed refs, and basename
# matching is ambiguous.
check_links() {
  local f="$1" tmp

  # A doc may opt out when its refs are not meant to resolve here — a template
  # whose paths resolve at the deployed location, for instance. The exemption
  # is declared in the file, with a reason, rather than hidden in a path
  # exclusion list the reader of the doc would never see.
  if nonfenced "$f" | grep -q '<!-- DOC:NOLINK-CHECK'; then
    return 0
  fi

  tmp="$(mktemp)"
  # Strip inline code spans too: a `](url)` inside backticks is documentation
  # showing link syntax, not a typed ref.
  nonfenced "$f" | awk '{ gsub(/`[^`]*`/, "", $0); print }' > "$tmp"

  local entry line rest match target dir resolved
  while IFS= read -r entry; do
    [[ -z "$entry" ]] && continue
    line="${entry%%:*}"
    rest="${entry#*:}"
    # Check every ](path) ref on the line, not just the first.
    while IFS= read -r match; do
      [[ -z "$match" ]] && continue
      target="${match#](}"
      target="${target%)}"
      case "$target" in http*|mailto:*|'#'*) continue ;; esac
      target="${target%%#*}"
      [[ -z "$target" ]] && continue
      target="${target#./}"
      dir="$(dirname "$f")"
      if [[ "$target" = /* ]]; then
        [[ -e "$target" ]] && continue
      else
        resolved=""
        if [[ -d "$dir" ]]; then
          resolved="$(cd "$dir" && readlink -f "$target" 2>/dev/null)" || true
        fi
        [[ -n "$resolved" && -e "$resolved" ]] && continue
        [[ -e "$target" ]] && continue
      fi
      report "$f:$line broken md-link ref → $target"
    done < <(printf '%s\n' "$rest" | grep -oE '\]\([^)]+\)' || true)
  done < <(grep -E '\]\([^)]+\)' "$tmp" || true)

  rm -f "$tmp"
}

# ---------------------------------------------------------------- check 2
# Generated-block drift. Each block names the command that produces it, so the
# linter needs no per-repo lookup table — the doc declares how to rebuild
# itself, and a new generated block anywhere is covered the moment it is added.
#
#   <!-- BEGIN:GENERATED cmd="scripts/docs/gen-repo-map.sh" -->
#   ...renderer stdout...
#   <!-- END:GENERATED -->
check_generated() {
  local f="$1" total begins ends

  begins="$(nonfenced "$f" | grep -c '<!-- BEGIN:GENERATED' || true)"
  ends="$(nonfenced "$f" | grep -c '<!-- END:GENERATED -->' || true)"
  if [[ "$begins" != "$ends" ]]; then
    report "$f:0 unbalanced GENERATED markers ($begins BEGIN, $ends END)"
    return 0
  fi
  total="$begins"
  [[ "$total" -eq 0 ]] && return 0

  local i marker lineno cmd embedded fresh tmp freshfile
  for (( i = 1; i <= total; i++ )); do
    # Re-read the marker each pass: a --fix on an earlier block shifts the line
    # numbers of every block after it.
    marker="$(nonfenced "$f" | grep '<!-- BEGIN:GENERATED' | sed -n "${i}p")"
    lineno="${marker%%:*}"

    cmd="$(printf '%s\n' "${marker#*:}" | sed -nE 's/.*cmd="([^"]+)".*/\1/p')"
    if [[ -z "$cmd" ]]; then
      report "$f:$lineno GENERATED block has no cmd=\"...\" attribute — cannot verify or rebuild"
      continue
    fi

    # Split on whitespace so a renderer may carry arguments.
    local -a cmd_parts
    read -r -a cmd_parts <<< "$cmd"
    if [[ ! -x "${cmd_parts[0]}" ]]; then
      report "$f:$lineno GENERATED block names cmd \"${cmd_parts[0]}\", which is not an executable file"
      continue
    fi

    embedded="$(awk -v start="$lineno" '
      NR > start && /<!-- END:GENERATED -->/ { exit }
      NR > start { print }
    ' "$f")"

    if ! fresh="$("${cmd_parts[@]}" 2>&1)"; then
      report "$f:$lineno renderer \"$cmd\" exited non-zero"
      continue
    fi

    [[ "$embedded" == "$fresh" ]] && continue

    if (( FIX )); then
      # Stage the replacement through a file rather than `awk -v`, which would
      # interpret backslash escapes in the rendered content.
      freshfile="$(mktemp)"
      printf '%s\n' "$fresh" > "$freshfile"
      tmp="$(mktemp)"
      awk -v start="$lineno" -v repl="$freshfile" '
        NR <= start { print; next }
        !done && /<!-- END:GENERATED -->/ {
          while ((getline line < repl) > 0) print line
          close(repl)
          print
          done = 1
          next
        }
        !done { next }
        { print }
      ' "$f" > "$tmp"
      mv "$tmp" "$f"
      rm -f "$freshfile"
      echo "lint-docs: rebuilt $f block $i via $cmd"
      fixed=1
    else
      report "$f:$lineno GENERATED block is stale — run: scripts/lint-docs.sh --fix"
    fi
  done
}

# ---------------------------------------------------------------- check 3
# Verification age. A doc declares which paths it describes; if any of those
# paths changed after the stamp, the doc's claim to be current is unbacked.
#
#   <!-- DOC:VERIFIES paths="scripts/ claude/shared/" -->
#   > Last verified: 2026-07-30
check_stamp() {
  local f="$1" marker lineno paths stamp code_date p
  marker="$(nonfenced "$f" | grep '<!-- DOC:VERIFIES' | head -1 || true)"
  [[ -z "$marker" ]] && return 0

  lineno="${marker%%:*}"
  paths="$(printf '%s\n' "${marker#*:}" | sed -nE 's/.*paths="([^"]+)".*/\1/p')"
  if [[ -z "$paths" ]]; then
    report "$f:$lineno DOC:VERIFIES has no paths=\"...\" attribute"
    return 0
  fi

  stamp="$(nonfenced "$f" \
    | sed -nE 's/^[0-9]+:>[[:space:]]*Last verified:[[:space:]]*([0-9]{4}-[0-9]{2}-[0-9]{2}).*/\1/p' \
    | head -1)"
  if [[ -z "$stamp" ]]; then
    report "$f:$lineno DOC:VERIFIES declared but no \"> Last verified: YYYY-MM-DD\" line found"
    return 0
  fi

  local -a path_args
  read -r -a path_args <<< "$paths"
  for p in "${path_args[@]}"; do
    [[ -e "$p" ]] || report "$f:$lineno DOC:VERIFIES names path \"$p\", which does not exist"
  done

  # Exclude the doc itself: editing its prose must not invalidate its own stamp.
  code_date="$(git log -1 --format=%cs -- "${path_args[@]}" ":(exclude)$f" 2>/dev/null || true)"
  [[ -z "$code_date" ]] && return 0

  if [[ "$code_date" > "$stamp" ]]; then
    report "$f:$lineno stale — describes [$paths], last changed $code_date, stamp says $stamp. Re-read it, then restamp."
  fi
}

for f in "${files[@]}"; do
  [[ -f "$f" ]] || continue
  is_ignored "$f" && continue
  check_links "$f"
  check_generated "$f"
  check_stamp "$f"
done

if (( FIX )) && (( fixed )); then
  echo "lint-docs: generated blocks rebuilt — review the diff before committing" >&2
fi

exit "$fail"
