#!/usr/bin/env bash
# validate_specs.sh — Check that all spec directories in specs/ are well-formed.
#
# Usage: ./script/validate_specs.sh
#
# Each spec subdirectory must have either (product.md or PRODUCT.md) and
# (tech.md or TECH.md). Directories without a recognized issue-number prefix
# (e.g. contributor-named directories like "zachlloyd/") are skipped.
# Exits non-zero on any validation failure.

set -euo pipefail

REPO_ROOT="$(git rev-parse --show-toplevel)"
SPECS_DIR="$REPO_ROOT/specs"

# Issue number prefixes used in this repo's specs/ directory
ISSUE_PREFIX_RE='^(APP-|GH[0-9]+|CODE-|QUALITY-|REMOTE-|UNTRIAGED-)'

errors=0

for spec_dir in "$SPECS_DIR"/*/; do
    [[ -d "$spec_dir" ]] || continue
    spec_name=$(basename "$spec_dir")

    # Skip contributor-named directories (no recognized issue-number prefix)
    if [[ ! "$spec_name" =~ $ISSUE_PREFIX_RE ]]; then
        continue
    fi

    # Check for product.md (any case)
    if [[ ! -f "$spec_dir/product.md" && ! -f "$spec_dir/PRODUCT.md" ]]; then
        echo "MISSING: $spec_name/product.md (or PRODUCT.md)"
        errors=1
    fi

    # Check for tech.md (any case)
    if [[ ! -f "$spec_dir/tech.md" && ! -f "$spec_dir/TECH.md" ]]; then
        echo "MISSING: $spec_name/tech.md (or TECH.md)"
        errors=1
    fi
done

if [[ $errors -eq 1 ]]; then
    echo ""
    echo "Spec validation failed. Fix the issues above before committing."
    exit 1
fi

total=$(find "$SPECS_DIR" -mindepth 1 -maxdepth 1 -type d | wc -l)
echo "Spec validation passed ($total entries checked)."
exit 0