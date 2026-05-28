#!/usr/bin/env bash
#
# Resolve declarative Cargo build profiles from app/Cargo.toml.
#
# The profile path is a dot-separated path under
# `package.metadata.warp.build_profiles`, for example:
#   linux.oss.app
#   macos.dev.cli
#   windows.preview.app

resolve_cargo_bundle_features() {
  local workspace_root_dir="$1"
  local profile_path="$2"

  "$workspace_root_dir/script/check_cargo_build_profiles" --profile "$profile_path"
}
