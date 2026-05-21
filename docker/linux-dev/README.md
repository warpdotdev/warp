# Dockerfile for Linux Development

The Dockerfile in this directory defines a container that has all of the necessary tools installed to quickly get engineers up and running with building Warp on Linux.

This container is based on Debian Sid, Debian's unstable branch.  It ensures that you are running the latest versions of things like `mesa` (an open-source 3D graphics library, providing implementations of OpenGL and Vulkan).

## Prerequisites

You'll need to install:
* Docker (e.g.: Docker Desktop)
* XQuartz (download [here](https://www.xquartz.org/))
  * You'll want to enable iGLX (indirect GL extensions) for proper rendering by running `defaults write org.xquartz.X11 enable_iglx -bool true`; you can do this before you install XQuartz.
  * After installing XQuartz, run it, and enable "Allow connections from network clients" in the Security tab in its settings.  You'll need to quit and relaunch XQuartz after making this change.

## Setup

You'll run all of these commands from the repository's root directory.

First, build the docker container image:

```
CONTAINER_NAME="warp-client-linux-dev"
docker build -t $CONTAINER_NAME docker/linux-dev
```

Next, run the container:

```
# The path to the source code directory that you want to mount in the
# container. This can be the `warp` repository or some parent
# directory of your choice.
LOCAL_PATH="/Users/$USER/src"

# Run the image as a container, bridging port 22 for SSH connections,
# mounting the provided directory into the container as `/src`, mounting
# your SSH key directory into the container (so you don't need to create a
# new GitHub SSH key), and mounting gcloud configuration (and auth information,
# so you can run SSH integration tests).
docker run -dp 127.0.0.1:22:22/tcp -v $LOCAL_PATH:/src -v $HOME/.ssh:/home/dev/.ssh -v $HOME/.config/gcloud:/home/dev/.config/gcloud $CONTAINER_NAME
```

## Usage

Every time you start XQuartz, you'll need to run this once in order for programs running in the container to connect to it:

```
xhost +localhost
```

You should be able to SSH into the container and build and run warp without any additional setup (dev account password is "password"):

```
ssh dev@localhost
cd /src
cargo run --features fast_dev
```

It's possible you'll run into some odd errors while trying to compile Warp; if so, just keep rerunning the cargo command and it should work eventually.
