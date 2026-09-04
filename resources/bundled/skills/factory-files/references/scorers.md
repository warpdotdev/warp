# File-defined Scorers
 
A Scorer lives at `scorers/<name>/scorer.md`. Its directory provides its name,
YAML frontmatter defines the classification contract, and the non-empty
Markdown body is the rubric evaluated against eligible runs.
 
```markdown
---
description: Checks whether implementation runs include test evidence.
agents:
  - implementer
labels:
  - value: tests_run
    description: The transcript contains a test command and its result.
    score: 1
  - value: tests_partial
    description: Only a relevant subset of tests ran.
    score: 0.5
  - value: tests_skipped
    score: 0
passingScore: 1
samplingRate: 25
model: claude-4-5-haiku
selfImprovement: true
---
Evaluate whether the agent ran the repository's relevant tests before
finishing. Return exactly one declared label.
```
 
## Fields
 
- `agents` — required non-empty list of Agent names declared in this Factory.
  Names are trimmed and must be unique.
- `enabled` — optional boolean, default true. Use `enabled: false` rather than
  a zero sampling rate to pause scoring.
- `output` — optional output form. `classification` is the current known form.
- `labels` — required non-empty list of classifications; the current server
  accepts at most 20. Each label requires a non-empty `value` and a numeric
  `score` from 0 through 1; `description` is optional. Label values are
  trimmed and unique.
- `passingScore` — required numeric threshold from 0 through 1. At least one
  label must score at or above it, and at least one below it.
- `samplingRate` — optional percentage, default 25. Values are rounded to two
  decimal places and must resolve to 0.01–100.
- `model` — required model ID. The server validates availability.
- `selfImprovement` — optional boolean, default false.
 
The Markdown body must not be empty.

The field list above is a summary for authoring; the server's schema is the
contract. If a Scorer already contains a field this page does not mention,
leave it alone and validate against the server rather than deleting it.
