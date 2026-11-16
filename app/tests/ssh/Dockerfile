FROM ubuntu:latest

# Install basic dependencies for testing. git and curl are added so we can install rust/Cargo and
# clone the warp-internal repo to simulate more realistic development scenarios.
RUN apt update && apt install openssh-server sudo zsh fish xxd git curl vim -y

RUN sudo echo `which fish` >> /etc/shells

RUN useradd -rm -d /home/bash -s /bin/bash -g root -G sudo -u 1000 bash
RUN useradd -rm -d /home/zsh -s /bin/zsh -g root -G sudo -u 1001 zsh
RUN useradd -rm -d /home/fish -s `which fish` -g root -G sudo -u 1002 fish

RUN  echo 'bash:password' | chpasswd
RUN  echo 'zsh:password' | chpasswd

RUN service ssh start

EXPOSE 22

CMD ["/usr/sbin/sshd","-D"]
