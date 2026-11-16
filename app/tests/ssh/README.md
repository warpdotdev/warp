# Testing Warp Functionality over SSH

## Pre-req: Install Docker
1. `brew install --cask docker`
2. Open the Docker desktop app. This is necessary to create the symbolic links that will make the `docker` CLI available.

## Running Warp over SSH
There's a workflow called "Build image and start container for SSH testing" in this repo. After that, you may SSH in by running bash@0.0.0.0 or zsh@0.0.0.0. It'll prompt for a password which is `password` for these VMs.

After you've built the image, you can just launch the container again with the second command in the workflow.

Note that you can only have one docker container in your system that has port 22.

## More advanced use
Sometimes SSH users have issues based on their SSH server configs which is usually `/etc/ssh/sshd_config` (this is also the case in Ubuntu, which the Dockerfile in this repo uses). If you need to edit it, you'll have to reboot the SSH daemon afterwards to have the changes reflected with `sudo service ssh restart`. This will cause the container to be stopped, and you'll have to
restart it in the Docker Desktop app (click the play button) in order to restart it with the new config.
