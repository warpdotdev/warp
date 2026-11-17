---
agentType: MAIN
hooks:
  # The api repository's own, unchanged `.cursor` directory.
  - format: cursor
    repo:
      owner: acme
      name: api
    path: .cursor
  # The reviewer's own policy, kept beside its definition.
  - format: cursor
    repo:
      owner: acme
      name: acme-factory
    path: agents/reviewer/hooks
    onAsk: fail_run
---
Review the change you are given.
