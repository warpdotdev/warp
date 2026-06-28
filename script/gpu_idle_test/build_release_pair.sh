#!/usr/bin/env bash
#
# Build release BASE (merge-base, no fix) and CANDIDATE (HEAD, with fix) binaries
# for same-commit GPU idle A/B testing.
#
# Uses memory-friendly release flags (codegen-units=256, debug=0) so the warp
# crate fits in ~12GB peak RSS instead of OOMing on 32GB machines.
#
# Usage: build_release_pair.sh [BASE_COMMIT]
#   BASE_COMMIT defaults to the merge-base with origin/master.
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$HERE/../.." && pwd)"
OUT_DIR="${TEST_BIN_DIR:-$REPO_ROOT/target/gpu_idle_test/bin}"
BASE_COMMIT="${1:-$(git -C "$REPO_ROOT" merge-base HEAD origin/master)}"
BRANCH="$(git -C "$REPO_ROOT" rev-parse --abbrev-ref HEAD)"

export CARGO_PROFILE_RELEASE_CODEGEN_UNITS=256
export CARGO_PROFILE_RELEASE_DEBUG=0
BUILD_FLAGS=(--release --bin warp-oss -j "${BUILD_JOBS:-16}")

mkdir -p "$OUT_DIR"

echo "Building CANDIDATE (HEAD) -> $OUT_DIR/warp-oss-rel-candidate"
cd "$REPO_ROOT"
cargo build "${BUILD_FLAGS[@]}"
cp -f target/release/warp-oss "$OUT_DIR/warp-oss-rel-candidate"

echo "Building BASE ($BASE_COMMIT) -> $OUT_DIR/warp-oss-rel-base"
git checkout "$BASE_COMMIT"
cargo build "${BUILD_FLAGS[@]}"
cp -f target/release/warp-oss "$OUT_DIR/warp-oss-rel-base"

if [ "$BRANCH" != "HEAD" ]; then
  git checkout "$BRANCH"
fi

echo "Done."
ls -la "$OUT_DIR/warp-oss-rel-base" "$OUT_DIR/warp-oss-rel-candidate"
