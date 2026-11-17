---
name: oz-report-pr
description: Report a pull request back to the Oz platform after creating one.
---
After creating a pull request, report it to Oz by running:

```sh
"$OZ_CLI" harness-support report-artifact pull-request --url '<url>' --branch '<branch>'
```

Replace `<url>` with the full pull request URL and `<branch>` with the branch name.

If you reported screenshots or video recordings during this task (see the oz-browser-verification skill), run `"$OZ_CLI" harness-support pr-media-markdown` before writing the pull request body and insert the blocks it prints above the attribution line, videos above screenshots. Re-run it and replace the whole managed block whenever you report new captures.
