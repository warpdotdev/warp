#!/usr/bin/env bash
# Doc-drift gate. Copied into a repo's .githooks/pre-commit by
# workstation-setup/scripts/install-doc-hooks.sh. Edit it here, not there —
# an edited copy is caught by the canonical-drift check below.
set -euo pipefail

REPO="$(git rev-parse --show-toplevel)"

# Chain any hook installed before this one (code-review-graph, etc.) so wiring
# the doc gate never silently drops existing pre-commit behaviour.
if [[ -x "$REPO/.git/hooks/pre-commit" ]]; then
  "$REPO/.git/hooks/pre-commit" || exit $?
fi

# Refuse when the vendored toolchain has drifted from canonical. This runs on a
# developer machine, where both copies are reachable — CI cannot make this check,
# which is why the vendored model stays honest.
CANONICAL="${WORKSTATION_SETUP:-$HOME/workstation/nshonda/workstation-setup}"
if [[ -d "$CANONICAL" && "$CANONICAL" != "$REPO" ]]; then
  for f in scripts/lint-docs.sh scripts/docs/gen-repo-map.sh scripts/docs/gen-commands.sh \
           templates/doc-lint-pre-commit.sh; do
    [[ -f "$CANONICAL/$f" && -f "$REPO/$f" ]] || continue
    if ! cmp -s "$CANONICAL/$f" "$REPO/$f"; then
      echo "pre-commit: $f has drifted from canonical." >&2
      echo "  re-run: $CANONICAL/scripts/install-doc-hooks.sh $REPO" >&2
      exit 1
    fi
  done
fi

exec "$REPO/scripts/lint-docs.sh"
