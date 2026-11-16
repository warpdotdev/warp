#!/usr/bin/env python3
"""Extract RELEASE_FLAGS, PREVIEW_FLAGS, and DOGFOOD_FLAGS from warp_features.

Parses crates/warp_features/src/lib.rs to find the const arrays and extracts
the FeatureFlag variant names. Stdlib only, no pip deps.

Usage:
    python3 extract_feature_flags.py --file crates/warp_features/src/lib.rs

Outputs JSON to stdout.
"""

import argparse
import json
import re
import sys


def extract_flag_list(source: str, const_name: str) -> list[str]:
    """Extract FeatureFlag variant names from a const array definition."""
    # Match: pub const CONST_NAME: &[FeatureFlag] = &[ ... ];
    pattern = rf"pub\s+const\s+{re.escape(const_name)}\s*:\s*&\[FeatureFlag\]\s*=\s*&\[(.*?)\];"
    m = re.search(pattern, source, re.DOTALL)
    if not m:
        return []

    block = m.group(1)
    # Extract FeatureFlag::VariantName entries, ignoring #[cfg(...)] attributes
    variants = re.findall(r"FeatureFlag::(\w+)", block)
    return variants


def main() -> None:
    parser = argparse.ArgumentParser(description="Extract feature flag gate lists")
    parser.add_argument(
        "--file",
        required=True,
        help="Path to warp_features lib.rs",
    )
    args = parser.parse_args()

    with open(args.file) as f:
        source = f.read()

    output = {
        "release_flags": extract_flag_list(source, "RELEASE_FLAGS"),
        "preview_flags": extract_flag_list(source, "PREVIEW_FLAGS"),
        "dogfood_flags": extract_flag_list(source, "DOGFOOD_FLAGS"),
    }
    json.dump(output, sys.stdout, indent=2)
    print()


if __name__ == "__main__":
    main()
