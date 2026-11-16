FROM debian:sid

# Install utilities and libraries needed for development.
RUN apt-get update \
    && apt-get install \
            sudo \
            openssh-server \
            locales \
            zsh \
            curl \
            # Install some editors - nano is nice for simple edits; vim is
            # needed for some of our tests.
            nano vim \
            # Install various core packages for development work on Linux.
            build-essential cmake pkg-config \
            # Make it so that running `python` runs `python3`.
            python-is-python3 \
            # Development headers for various libraries, needed when compiling
            # certain Rust crate dependencies.
            libssl-dev libfreetype-dev libexpat1-dev \
            # Runtime libraries for X11.
            xserver-xorg libxcursor1 \
            # Open-source Vulkan drivers and OpenGL-related utilities from the
            # mesa project.
            mesa-vulkan-drivers mesa-utils \
            # protoc, in order to compile warp-proto-apis crates like the MAA API.
            protobuf-compiler \
            # Don't ask for confirmation.
            -y

# Install the GitHub CLI.
RUN mkdir -p -m 755 /etc/apt/keyrings \
    && curl -fsSL https://cli.github.com/packages/githubcli-archive-keyring.gpg > /etc/apt/keyrings/githubcli-archive-keyring.gpg \
    && chmod go+r /etc/apt/keyrings/githubcli-archive-keyring.gpg \
    && echo "deb [arch=$(dpkg --print-architecture) signed-by=/etc/apt/keyrings/githubcli-archive-keyring.gpg] https://cli.github.com/packages stable main" | tee /etc/apt/sources.list.d/github-cli.list > /dev/null \
    && apt-get update -y \
    && apt-get install gh -y

# Install the gcloud CLI.
RUN mkdir -p -m 755 /etc/apt/keyrings \
    && curl -fsSL https://packages.cloud.google.com/apt/doc/apt-key.gpg > /etc/apt/keyrings/cloud.google.gpg \
    && chmod go+r /etc/apt/keyrings/cloud.google.gpg \
    && echo "deb [arch=$(dpkg --print-architecture) signed-by=/etc/apt/keyrings/cloud.google.gpg] http://packages.cloud.google.com/apt cloud-sdk main" | tee /etc/apt/sources.list.d/google-cloud-sdk.list > /dev/null \
    && apt-get update -y \
    && apt-get install google-cloud-cli -y

# Set up the dev user (password is "password").
RUN useradd -rm -d /home/dev -s /usr/bin/zsh -g root -G sudo -u 1000 dev && echo 'dev:password' | chpasswd

# Install rustup as the dev user.
RUN su - dev -c "curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y"

# Set some useful environment variables for the dev user.
RUN su - dev -c "echo 'export CARGO_TARGET_DIR=/home/dev/cargo-output\nexport DISPLAY=host.docker.internal:0\nexport XDG_RUNTIME_DIR=/run/user/dev' >> /home/dev/.zshenv"

# Generate the basic english, UTF-8 locale.
RUN locale-gen en_US.UTF-8

# Create the necessary runfiles directory for sshd to work.
RUN mkdir -p /run/sshd

# Enable the sshd service, exposing port 22 to the host.
RUN systemctl enable ssh
EXPOSE 22

# Install the Rust toolchain used to build Warp.
COPY rust-toolchain.toml /home/dev/setup/rust-toolchain.toml
RUN su - dev -c "cd /home/dev/setup && rustup toolchain install"

# Install Warp-on-Linux dependencies.
COPY script/linux/install_test_deps /home/dev/setup/linux/install_test_deps
COPY script/linux/install_runtime_deps /home/dev/setup/linux/install_runtime_deps
COPY script/linux/install_build_deps /home/dev/setup/linux/install_build_deps
COPY script/install_cargo_test_deps /home/dev/setup/script/install_cargo_test_deps
COPY script/install_cargo_build_deps /home/dev/setup/script/install_cargo_build_deps
COPY script/install_rust /home/dev/setup/script/install_rust


# Run sshd.
CMD ["/usr/sbin/sshd", "-D"]
