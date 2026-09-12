FROM ubuntu:24.04

# Opt into the agent entrypoint's non-root rollout gate (see warp-agent-docker's README).
ENV WARP_AGENT_NONROOT=1

ENV RUSTUP_HOME=/usr/local/rustup
ENV CARGO_HOME=/usr/local/cargo
ENV TFENV_ROOT=/usr/local/tfenv
ENV PATH=/usr/local/cargo/bin:/usr/local/go/bin:/usr/local/tfenv/bin:$PATH

# This set of packages is adapted from ./script/bootstrap in warp-internal and warp-server.
# It doesn't include *everything* (ex. no wgslfmt), but is enough to build and test Warp.

# Install system-level packages. These are needed to build on Linux, and/or to run integration tests.
#
# postgresql/postgresql-client: `postgresql`'s postinst initializes a default
# cluster (16/main) but does not start it, since /usr/sbin/policy-rc.d blocks
# service startup during container builds. To use a local server, start the
# existing cluster and create your own role/db, e.g.:
#   sudo pg_ctlcluster 16 main start
#   sudo -u postgres createuser -s "$(whoami)" && sudo -u postgres createdb "$(whoami)"
# postgresql-16-pgvector: warp-server's migrations run `CREATE EXTENSION IF
# NOT EXISTS vector` (db/migrations/20241011184444_enable_pgvector.sql), so a
# local cluster needs pgvector to run migrations. The other extensions
# warp-server uses (pg_trgm, pgcrypto) already ship as part of postgresql-16
# itself, so they need no extra package here.
RUN set -eux; \
  apt-get update; \
  apt-get install -y --no-install-recommends \
    pkg-config git build-essential libclang-dev curl unzip zsh fish vim locales sudo gnupg2 \
    protobuf-compiler libprotobuf-dev \
    fontconfig zlib1g libasound2-dev \
    libx11-6 libxcb1 libxi6 libxcursor1 libxkbcommon-x11-0 \
    libwayland-client0 libwayland-egl1 \
    mesa-vulkan-drivers libegl1 \
    libsqlite3-dev libpq-dev libmysqlclient-dev \
    postgresql postgresql-client postgresql-16-pgvector \
    ca-certificates \
    openssh-client netcat-openbsd lsof \
    xvfb

# Install rustup. The pinned Rust toolchain is installed below from rust-toolchain.toml.
#
# The non-root runtime `agent` user (WARP_AGENT_NONROOT=1 above, created at
# container start -- see warp-agent-docker's entrypoint.bash
# `create_or_validate_agent_user`) needs write access to everything rustup
# and cargo produce, without any `sudo`. Rather than a recursive chgrp/chmod
# pass after the fact (expensive: it copies up every file into a new overlay
# layer), pre-create the two root directories group-owned by a group named
# `agent` with the setgid bit, and set `umask 002` for the rest of this RUN
# and every later RUN that writes into them. setgid makes directories created
# underneath inherit both the group and the setgid bit (so this propagates to
# arbitrarily deep subdirectories), and umask 002 makes newly created files
# and directories group-writable. This is also why the entrypoint can grant
# access without a specific UID baked in here: its `ensure_agent_group`
# helper reuses a pre-existing group named `agent` by name instead of
# creating a new one, so the runtime user (created later, with an unknown
# UID) lands in this group automatically.
RUN set -eux; \
  groupadd agent; \
  mkdir -p "$RUSTUP_HOME" "$CARGO_HOME"; \
  chgrp agent "$RUSTUP_HOME" "$CARGO_HOME"; \
  chmod 2775 "$RUSTUP_HOME" "$CARGO_HOME"; \
  umask 002; \
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --no-modify-path --profile minimal; \
  rustup --version; \
  cargo --version; \
  rustc --version

# Install go and go command-line tools we use.
RUN set -eux; \
  dpkgArch="$(dpkg --print-architecture)"; ARCH="${dpkgArch##*-}"; \
  curl --proto '=https' --tlsv1.2 -fsSLO https://go.dev/dl/go1.26.5.linux-$ARCH.tar.gz; \
  tar -C /usr/local -xzf go1.26.5.linux-$ARCH.tar.gz; \
  go version; \
  go env -w "GOPRIVATE=github.com/warpdotdev/*"; \
  CGO_ENABLED=0 go install -tags='no_clickhouse no_libsql no_mssql no_mysql no_sqlite3 no_vertica no_ydb' github.com/pressly/goose/v3/cmd/goose@v3.24.1; \
  echo "Installed goose"; \
  go install github.com/vektra/mockery/v2@v2.53.5; \
  echo "Installed mockery"; \
  go install google.golang.org/protobuf/cmd/protoc-gen-go@v1.36.6; \
  echo "Installed protoc-gen-go"; \
  go install google.golang.org/grpc/cmd/protoc-gen-go-grpc@v1.2.0; \
  echo "Installed protoc-gen-go-grpc"; \
  rm go1.26.5.linux-$ARCH.tar.gz

# Install golangci-lint. It refuses to load a config that targets a newer Go
# release than the one golangci-lint itself was built with, so this pin has to
# keep up with the Go toolchain above.
RUN curl -sSfL https://raw.githubusercontent.com/golangci/golangci-lint/HEAD/install.sh | sh -s -- -b /usr/local/go/bin v2.12.2 \
    && golangci-lint --version

# Preinstall the static tokenizers library so Go cgo builds that link
# -ltokenizers resolve it from the default search path. A checkout that ships its
# own copy still wins, since cgo passes its -L first. Keep the pin in step with
# what consuming builds expect.
RUN set -eux; \
  tokenizers_version=v1.27.0; \
  case "$(dpkg --print-architecture)" in \
    amd64) tokenizers_asset='libtokenizers.linux-x86_64.tar.gz';; \
    arm64) tokenizers_asset='libtokenizers.linux-arm64.tar.gz';; \
    *) echo "no libtokenizers release for $(dpkg --print-architecture)" >&2; exit 1;; \
  esac; \
  tokenizers_tmp="$(mktemp -d)"; \
  curl --proto '=https' --tlsv1.2 -fsSL \
    "https://github.com/daulet/tokenizers/releases/download/$tokenizers_version/$tokenizers_asset" \
    -o "$tokenizers_tmp/libtokenizers.tar.gz"; \
  tar -xzf "$tokenizers_tmp/libtokenizers.tar.gz" -C "$tokenizers_tmp"; \
  install -m 0644 "$tokenizers_tmp/libtokenizers.a" /usr/local/lib/libtokenizers.a; \
  rm -rf "$tokenizers_tmp"

# Install Node
# Adapted from https://github.com/nodejs/docker-node/blob/eda40f3c41d33cb595232cb7000ab31af1ce2f02/25/bookworm/Dockerfile#L8-L17
RUN set -eux; \
    ARCH= ; dpkgArch="$(dpkg --print-architecture)"; \
    case "${dpkgArch##*-}" in \
      amd64) ARCH='x64';; \
      arm64) ARCH='arm64';; \
    esac; \
    curl --proto '=https' --tlsv1.2 -fsSLO --compressed "https://nodejs.org/dist/v25.0.0/node-v25.0.0-linux-$ARCH.tar.xz"; \
    tar -xJf node-v25.0.0-linux-$ARCH.tar.xz -C /usr/local --strip-components=1 --no-same-owner; \
    rm node-v25.0.0-linux-$ARCH.tar.xz; \
    ln -s /usr/local/bin/node /usr/local/bin/nodejs; \
    node --version; \
    npm --version; \
    npm install -g corepack; \
    corepack enable

# Ensure the correct Rust toolchain is installed
COPY ./rust-toolchain.toml ./setup/rust-toolchain.toml
RUN set -eux; \
  umask 002; \
  cd ./setup; \
  rustup toolchain install; \
  rustup default "$(rustup show active-toolchain | cut -d' ' -f1)"

# Install all dependencies needed to build, run, and test Warp
COPY ./script ./setup/script
# cargo-binstall writes $CARGO_HOME/binstall.toml with an explicit 0600 mode
# that ignores umask, unlike everything else install_test_deps creates. Fix
# just that one file in the same layer that creates it rather than adding a
# separate recursive pass over the whole tree.
RUN set -eux; \
  umask 002; \
  cd ./setup && ./script/linux/install_test_deps; \
  if [ -f "$CARGO_HOME/binstall.toml" ]; then chmod g+rw "$CARGO_HOME/binstall.toml"; fi

# Add the Google Cloud SDK repository and install:
# - The gcloud CLI
# - The GKE gcloud auth plugin
# These are mostly used for verifying Terraform changes.
RUN set -eux; \
  install -d -m 0755 /usr/share/keyrings; \
  curl -fsSL https://packages.cloud.google.com/apt/doc/apt-key.gpg \
    | gpg --batch --yes --dearmor -o /usr/share/keyrings/cloud.google.gpg; \
  echo "deb [signed-by=/usr/share/keyrings/cloud.google.gpg] https://packages.cloud.google.com/apt cloud-sdk main" \
    > /etc/apt/sources.list.d/google-cloud-sdk.list; \
  apt-get update; \
  apt-get install -y --no-install-recommends google-cloud-cli google-cloud-cli-gke-gcloud-auth-plugin; \
  rm -rf /var/lib/apt/lists/*; \
  gke-gcloud-auth-plugin --version

# Install Terraform via tfenv. We preinstall the currently-pinned version, but
# if the version pinned in our repo changes, it will get reinstalled on demand.
# We should keep this up-to-date to avoid unnecessary reinstalls.
#
# tfenv itself is pinned to release tag v3.2.2 for build reproducibility.
RUN set -eux; \
  git clone --depth=1 --branch v3.2.2 https://github.com/tfutils/tfenv.git /usr/local/tfenv; \
  tfenv install 1.15.5; \
  tfenv use 1.15.5; \
  terraform version

# Coding Agent CLIs (optional, makes image significantly larger)
ARG INSTALL_CODING_AGENTS=false
RUN if [ "$INSTALL_CODING_AGENTS" = "true" ]; then \
      npm install -g \
        @anthropic-ai/claude-code \
        @openai/codex \
        @google/gemini-cli && \
      curl -fsSL https://cli.github.com/packages/githubcli-archive-keyring.gpg \
        | dd of=/usr/share/keyrings/githubcli-archive-keyring.gpg 2>/dev/null && \
      chmod go+r /usr/share/keyrings/githubcli-archive-keyring.gpg && \
      echo "deb [arch=$(dpkg --print-architecture) signed-by=/usr/share/keyrings/githubcli-archive-keyring.gpg] https://cli.github.com/packages stable main" \
        > /etc/apt/sources.list.d/github-cli.list && \
      apt-get update && apt-get install -y --no-install-recommends gh && \
      rm -rf /var/lib/apt/lists/* && \
      echo "Installed coding agent CLIs:" && \
      claude --version && \
      codex --version && \
      gemini --version && \
      gh --version && \
      echo "All CLIs verified" ; \
    fi

# Guard against a regression in the permission scheme set up above (setgid
# `agent`-group parent directories + `umask 002` in every RUN that writes
# into $RUSTUP_HOME/$CARGO_HOME, REMOTE-2641): fail the build if a non-root
# member of the `agent` group can't actually write the paths that were
# broken before that fix. This uses a throwaway user rather than the real
# `agent` user, which doesn't exist until the entrypoint creates it at
# container start.
#
# This MUST remain the last instruction in this Dockerfile that could
# possibly write into $RUSTUP_HOME/$CARGO_HOME's tree: it's the only thing
# that would catch a future layer that writes into them without going
# through the setgid+umask scheme above. Add any new writer *above* this
# line, never below it.
RUN set -eux; \
  useradd -m -g agent -s /bin/bash toolchain-write-check; \
  pinned_toolchain="$(rustup show active-toolchain | cut -d' ' -f1)"; \
  toolchain_dir="$RUSTUP_HOME/toolchains/$pinned_toolchain"; \
  test -d "$toolchain_dir"; \
  su -s /bin/bash toolchain-write-check -c " \
    set -eux; \
    test -w '$RUSTUP_HOME'; \
    test -w '$toolchain_dir'; \
    test -w '$CARGO_HOME'; \
    test -w '$CARGO_HOME/.crates.toml'; \
    test -w '$CARGO_HOME/.package-cache'; \
    test -w '$CARGO_HOME/registry'; \
    test -w '$CARGO_HOME/git'; \
    test -w '$CARGO_HOME/binstall'; \
    test -w '$CARGO_HOME/binstall.toml'; \
    flock -w 2 '$CARGO_HOME/.package-cache' true \
  "; \
  userdel -r toolchain-write-check
