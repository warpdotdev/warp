---
name: oz-notify-user
description: Send a progress notification to the user who triggered this Oz task (e.g., via Slack or Linear).
---
When you want to send a progress update to the user (for example, after completing a significant milestone), run:

```sh
"$OZ_CLI" harness-support notify-user --message '<message>'
```

Replace `<message>` with a short, informative status update describing what you've accomplished or what you're working on next.
