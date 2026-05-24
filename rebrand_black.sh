#!/usr/bin/env bash
# Deterministic Warp -> Black first-party rename.
# Renames ONLY first-party crate tokens + module paths. Preserves external
# warpdotdev git-dep package names (warp_multi_agent_api, warp-workflows,
# warp-command-signatures, warp-command-corrections, warp-proto-apis).
#
# Usage:
#   ./rebrand_black.sh idents      # rename crate identifiers + module paths in .rs/.toml
#   ./rebrand_black.sh verify      # report remaining first-party warp tokens
set -euo pipefail
cd "$(dirname "$0")"

# First-party crate token map. ORDER MATTERS: longest / most-specific first so
# e.g. warpui_extras and warpui_core are replaced before warpui, and
# warp_graphql_schema before warp_graphql.
# Format: old<TAB>new
read -r -d '' MAP <<'EOF' || true
warpui_extras	black_ui_extras
warpui_core	black_ui_core
warpui	black_ui
warp_graphql_schema	black_graphql_schema
warp_graphql	black_graphql
warp_server_client	black_server_client
warp_isolation_platform	black_isolation_platform
warp_managed_secrets	black_managed_secrets
warp_web_event_bus	black_web_event_bus
warp_completer	black_completer
warp_features	black_features
warp_logging	black_logging
warp_ripgrep	black_ripgrep
warp_terminal	black_terminal
warp_editor	black_editor
warp_assets	black_assets
warp_files	black_files
warp_core	black_core
warp_util	black_util
warp_cli	black_cli
warp_js	black_js
EOF

# Tokens we must NEVER touch (external git deps). The script's map above does
# not include these, but we guard the app-crate `warp` rename (handled
# separately) and these stay literal.
EXTERNAL='warp_multi_agent_api|warp-workflows|warp_workflows|warp-command-signatures|warp_command_signatures|warp-command-corrections|warp_command_corrections|warp-proto-apis'

# Files in scope: tracked .rs and Cargo.toml, excluding target/.git and this script.
scope_files() {
  git ls-files '*.rs' '*.toml' | grep -v -E '^(target/|\.git/)' || true
}

cmd_idents() {
  local files; files="$(scope_files)"
  local count=0
  while IFS=$'\t' read -r old new; do
    [ -z "$old" ] && continue
    # Word-boundary replace; external tokens are longer/distinct so the
    # ordered longest-first map plus \b avoids clobbering them. We still
    # explicitly skip lines that are external git-dep declarations.
    echo "  $old -> $new"
    # Use perl for reliable \b and in-place edit; skip external-token lines.
    echo "$files" | while read -r f; do
      [ -f "$f" ] || continue
      perl -i -pe "
        next if /(?:$EXTERNAL)/;
        s/\\b\Q$old\E\\b/$new/g;
      " "$f"
    done
    count=$((count+1))
  done <<< "$MAP"
  echo "Applied $count token renames."
}

cmd_verify() {
  echo "=== Remaining FIRST-PARTY warp crate tokens (should be 0) ==="
  local files; files="$(scope_files)"
  local total=0
  while IFS=$'\t' read -r old new; do
    [ -z "$old" ] && continue
    local n
    n=$(echo "$files" | xargs grep -hoE "\\b$old\\b" 2>/dev/null | wc -l | tr -d ' ')
    if [ "$n" -gt 0 ]; then echo "  $old: $n"; total=$((total+n)); fi
  done <<< "$MAP"
  echo "Total remaining first-party crate-token matches: $total"
  echo ""
  echo "=== External tokens preserved (expected NONzero) ==="
  echo "$files" | xargs grep -hoE "$EXTERNAL" 2>/dev/null | sort | uniq -c
}

case "${1:-}" in
  idents) cmd_idents ;;
  verify) cmd_verify ;;
  *) echo "usage: $0 {idents|verify}"; exit 2 ;;
esac
