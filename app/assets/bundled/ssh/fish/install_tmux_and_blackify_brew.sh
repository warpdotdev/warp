brew install tmux

if test $status -eq 0
    tmux -Lwarp -CC
    exit
end
