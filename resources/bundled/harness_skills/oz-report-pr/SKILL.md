---
name: oz-report-pr
description: Report a pull request back to the Oz platform after creating one.
---
After creating a pull request, report it to Oz by running:

```sh
"$OZ_CLI" harness-support report-artifact pull-request --url '<url>' --branch '<branch>'
```

Replace `<url>` with the full pull request URL and `<branch>` with the branch name.
