#!/bin/bash

# These env vars must be set:
# SENTRY_PROJECT
# SENTRY_AUTH_TOKEN
# SENTRY_ORG
# SENTRY_ENVIRONMENT
# RELEASE_VERSION

# This script is adapted from getsentry/action-release, a JS GitHub Action.  See
# the relevant source code here: https://github.com/getsentry/action-release/blob/master/src/main.ts.

if which sentry-cli >/dev/null; then
    # Create the new release.
    ERROR=$(sentry-cli releases new "$RELEASE_VERSION")
    if [ $? -ne 0 ]; then
        echo "::error title=Error creating Sentry release::$ERROR"
    fi

    # Set the commits for the release automatically.
    ERROR=$(sentry-cli releases set-commits --auto "$RELEASE_VERSION")
    if [ $? -ne 0 ]; then
        echo "::error title=Error setting commits for Sentry release::$ERROR"
    fi

    # Add a deploy for the release.
    ERROR=$(sentry-cli deploys new --release "$RELEASE_VERSION" --env "$SENTRY_ENVIRONMENT")
    if [ $? -ne 0 ]; then
        echo "::error title=Error adding deploy for Sentry release::$ERROR"
    fi

    # Finalize the release.
    ERROR=$(sentry-cli releases finalize "$RELEASE_VERSION")
    if [ $? -ne 0 ]; then
        echo "::error title=Error finalizing Sentry release::$ERROR"
    fi
else
    NOT_INSTALLED="sentry-cli not installed, download from https://github.com/getsentry/sentry-cli/releases"
    echo "::error title=Error creating Sentry release::$NOT_INSTALLED"
fi
