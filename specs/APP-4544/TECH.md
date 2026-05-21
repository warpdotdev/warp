# Claude Code Notifications in Remote Tmux Sessions

## Support Expectation

Claude Code notifications are supported in remote Warp SSH sessions when the
Claude Code plugin emits Warp's structured OSC 777 notification protocol into
the tmux pane output stream. This is the same `warp://cli-agent` payload path
used for local sessions; remote/tmux sessions differ only in how those bytes are
delivered to Warp.

In tmux control mode, Warp receives pane output as `%output` messages. The
primary pane is rendered through the normal ANSI processor, and non-primary pane
output is not rendered into the visible terminal grid. Warp still scans
non-primary pane output for OSC 777 notifications so plugin events written
directly to a pane PTY can register/update the CLI agent session.

## Reproduction Shape

The failure mode is reproduced by feeding tmux control-mode output with an OSC
777 notification from a non-primary pane:

```text
%output %1 \033]777;notify;warp://cli-agent;{...}\007
```

Before APP-4544, Warp only parsed OSC notifications from the primary pane, so
the event above was ignored. After APP-4544, non-primary pane output is scanned
with a notification-only ANSI handler; terminal rendering actions are discarded
and only pluggable notifications are forwarded.

## Official Path

Remote/tmux notification integrations should write structured OSC 777
notifications to the target tmux pane's PTY/output stream. Warp does not depend
on the plugin process being local, and does not require a separate side channel
for Claude Code notification events.
