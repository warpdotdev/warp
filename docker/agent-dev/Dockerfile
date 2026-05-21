FROM ubuntu:24.04

ENV RUSTUP_HOME=/usr/local/rustup
ENV CARGO_HOME=/usr/local/cargo
ENV PATH=/usr/local/cargo/bin:/usr/local/go/bin:$PATH

# This set of packages is adapted from ./script/bootstrap in warp-internal and warp-server.
# It doesn't include *everything* (ex. no wgslfmt or gcloud), but is enough to build and test Warp.
# Note: currently, only x86_64 builds are supported.

# Install system-level packages. These are needed to build on Linux, and/or to run integration tests.
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
    ca-certificates \
    xvfb

# Install rustup. It will download the pinned version in our rust-toolchain.toml as needed.
RUN set -eux; \
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --no-modify-path --profile minimal; \
  chmod -R a+w $RUSTUP_HOME $CARGO_HOME; \
  rustup --version; \
  cargo --version; \
  rustc --version

# Install go and go command-line tools we use.
RUN set -eux; \
  dpkgArch="$(dpkg --print-architecture)"; ARCH="${dpkgArch##*-}"; \
  curl --proto '=https' --tlsv1.2 -fsSLO https://go.dev/dl/go1.26.1.linux-$ARCH.tar.gz; \
  tar -C /usr/local -xzf go1.26.1.linux-$ARCH.tar.gz; \
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
  rm go1.26.1.linux-$ARCH.tar.gz

# Install golangci-lint
RUN curl -sSfL https://raw.githubusercontent.com/golangci/golangci-lint/HEAD/install.sh | sh -s -- -b /usr/local/go/bin v2.6.1 \
    && golangci-lint --version

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
RUN cd ./setup && rustup toolchain install

# Install all dependencies needed to build, run, and test Warp
COPY ./script ./setup/script
RUN cd ./setup && ./script/linux/install_test_deps

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
