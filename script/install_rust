#!/usr/bin/env bash
#
# Script to install Rust for CI.
#
# This script should be platform-agnostic (eg. no unix-only references like /dev/null).

set -e

# Define some control sequences for text formatting.
red="\033[0;31m"
reset="\033[0m"

CI_FLAGS=""
if [ "${GITHUB_ACTIONS}" == "true" ]; then
  CI_FLAGS="-s -- -y"
fi

# Install Rust.
if ! command -v cargo; then
  echo "⬇️  Installing rust..."
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh $CI_FLAGS
  CARGO_ENV_FILE="$HOME/.cargo/env"
  if [[ -f "$CARGO_ENV_FILE" ]]; then
    source "$CARGO_ENV_FILE"
  else
    echo -e "⚠️  ${red}Please start a new terminal session so that cargo is in your PATH.${reset}"
    exit 1
  fi
fi
