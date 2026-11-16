FROM archlinux:base-devel

ARG USERNAME=build
# We use 1001 as the user UID here because 1000 is the UID for `runneradmin`,
# but the actual jobs are run as `runner` (with UID 1001).
ARG USER_UID=1001
ARG USER_GID=$USER_UID

# Set the packager name in makepkg.conf (so it doesn't say "Unknown Packager")
# when someone queries package info for our packages.
RUN echo 'PACKAGER="Warp Linux Maintainers <linux-maintainers@warp.dev>"' >> /etc/makepkg.conf

# Fetch package lists and install sudo, git, and protobuf (git is needed by
# cargo to resolve git dependencies during license generation, and protobuf
# provides protoc for generating rust bindings of protobuf APIs).
RUN pacman -Sy --needed --noconfirm sudo git protobuf

# Downgrade fakeroot to version 1.34.  Version 1.35 switches from calling
# `close` in a loop to using the new `close_range` syscall, which seems to
# be unavailable in the Ubuntu 20.04 environment that hosts this Docker
# container.
#
# Syscall discussion: https://archlinuxarm.org/forum/viewtopic.php?f=15&t=16943
# Downgrade discussion: https://github.com/docker/for-mac/issues/7331
RUN pacman -U --noconfirm https://archive.archlinux.org/packages/f/fakeroot/fakeroot-1.34-1-x86_64.pkg.tar.zst

# Install cargo-about, which we require for bundling third-party licenses.
RUN pacman -S --needed --noconfirm cargo-about

# Create our build user and give them sudo privileges.
RUN groupadd --gid $USER_GID $USERNAME \
    && useradd --uid $USER_UID --gid $USER_GID -m $USERNAME \
    && echo $USERNAME ALL=\(root\) NOPASSWD:ALL > /etc/sudoers.d/$USERNAME \
    && chmod 0440 /etc/sudoers.d/$USERNAME

# Run as our build user instead of root, as makepkg must be run as a non-root
# user.
USER $USERNAME

COPY entrypoint.sh /entrypoint.sh

ENTRYPOINT ["/entrypoint.sh"]
