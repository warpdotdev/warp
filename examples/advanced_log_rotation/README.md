# Advanced log rotation — sample artifacts

Companion to the **Optional LLM-driven summarization on log rotation** feature
request and its POC PR.

This directory contains synthetic but realistic samples showing what the
proposed sidecar files look like in practice. They're for review only — not
checked-in production fixtures. Each file is generated to illustrate one of
the two layers:

| File | Layer | Purpose |
|---|---|---|
| [`sample_mcp_server.log`](sample_mcp_server.log) | Source | A ~200-line synthetic MCP-server stderr capture showing a mix of `info`, `warn`, and `error` lines. The input to a rotation cycle. |
| [`sample.rotations.jsonl`](sample.rotations.jsonl) | A — always on | 5 rotation events, each a single JSON line. Always written when rotation fires, regardless of summarizer config. |
| [`sample.summaries.jsonl`](sample.summaries.jsonl) | B — optional | 3 summary records demonstrating the structured findings + multi-step pipeline trace produced by a configured summarizer. Only the last 2 rotation events here would have produced summaries (the first 3 were inside the rotation cap and didn't discard a file). |

## How to read the sidecar files

Both sidecars are newline-delimited JSON (`.jsonl`). One record per line. Tools
like `jq` work directly:

```bash
# All rotations in the last hour
jq 'select(.timestamp > "2026-05-14T17:00:00Z")' sample.rotations.jsonl

# Summaries that mentioned cache misses
jq 'select(.findings[] | contains("cache miss"))' sample.summaries.jsonl

# Total bytes ever rotated for one server
jq -s 'map(.bytes_rotated) | add' sample.rotations.jsonl
```

## Layer A — rotation events

Every rotation writes one record to `<active-log-path>.rotations.jsonl`. The
record has no external dependency — it's pure metadata.

```jsonc
{
  "timestamp": "2026-05-14T18:32:09Z",  // UTC, ISO 8601
  "active_log": "/.../mcp/<uuid>.log",  // the active file that was rotated
  "bytes_rotated": 10485760,             // size of the file at rotation time
  "discarded_path": "/.../mcp/<uuid>.log.5"  // null until the cap is hit
}
```

## Layer B — summary records

Each summary record is written when a configured `RotationSummarizer` returns
`Ok(Some(_))`. Empty responses (`Ok(None)`) and errors are silently skipped;
the rotation event is still written either way.

```jsonc
{
  "timestamp": "2026-05-14T18:32:14Z",
  "source_path": "/.../mcp/<uuid>.log.5",  // file that was about to be discarded
  "bytes_summarized": 10485760,
  "model": "qwen2.5-coder:7b@ollama-local",  // implementation-defined identifier
  "pipeline": [
    { "step": "extract_events", "duration_ms": 412 },
    { "step": "classify", "duration_ms": 287 },
    { "step": "summarize", "duration_ms": 893 }
  ],
  "summary": "Server emitted 142 warnings about cache misses on /api/products…",
  "findings": [
    "Cache miss rate spiked from baseline 3% to 41% between 16:30 and 16:50",
    "Single transport disconnect at 16:42 (recovered)",
    "No errors above WARN level"
  ]
}
```

## Design contract reminders

- **Layer A is always on** when rotation is configured. Cheap, no model dep.
- **Layer B is fully opt-in.** No model call unless the user wires a summarizer.
- **Summarizer failures never block rotation.** A model that times out or
  returns an error degrades to "no summary record" — the rotation completes
  and the event log still captures what happened.
- **Sidecars live next to the active log.** They rotate alongside the user's
  existing log retention policy: when the parent directory is purged on
  app restart (the existing namespace policy), the sidecars go with it.
