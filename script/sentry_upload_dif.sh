#!/bin/bash

# These env vars must be set:
# SENTRY_PROJECT
# SENTRY_AUTH_TOKEN
# SENTRY_ORG
# DEBUG_FILE_OR_FOLDER_PATH

set -x

MAX_ATTEMPTS=3
INITIAL_RETRY_DELAY_SECONDS=5
MAX_RETRY_DELAY_SECONDS=10

if ! command -v sentry-cli >/dev/null; then
  echo "::error title=Error uploading Sentry debug files::sentry-cli not installed, download from https://github.com/getsentry/sentry-cli/releases"
  exit 1
fi

if [ ! -e "$DEBUG_FILE_OR_FOLDER_PATH" ]; then
  echo "::error title=Error uploading Sentry debug files::DEBUG_FILE_OR_FOLDER_PATH '$DEBUG_FILE_OR_FOLDER_PATH' does not exist"
  exit 1
fi

attempt=1
retry_delay_seconds=$INITIAL_RETRY_DELAY_SECONDS
while true; do
  if sentry-cli upload-dif "$DEBUG_FILE_OR_FOLDER_PATH"; then
    echo "Sentry debug file upload succeeded on attempt $attempt/$MAX_ATTEMPTS."
    exit 0
  fi

  if [ "$attempt" -ge "$MAX_ATTEMPTS" ]; then
    echo "::error title=Error uploading Sentry debug files::sentry-cli upload-dif failed after $MAX_ATTEMPTS attempts"
    exit 1
  fi
  echo "::warning title=Sentry debug file upload failed::Attempt $attempt/$MAX_ATTEMPTS failed; retrying in ${retry_delay_seconds}s"
  sleep "$retry_delay_seconds"
  attempt=$((attempt + 1))
  retry_delay_seconds=$((retry_delay_seconds * 2))
  if [ "$retry_delay_seconds" -gt "$MAX_RETRY_DELAY_SECONDS" ]; then
    retry_delay_seconds=$MAX_RETRY_DELAY_SECONDS
  fi
done
