#!/usr/bin/env node
// Node LTS harness for the wasm32-unknown-unknown + Node prototype (REMOTE-2264).
//
// Imports the generated wasm-bindgen Node loader, invokes the exported
// `run_multi_agent` with an explicit prompt/API key/server URL, supplies the
// host `fetch` transport (Node's global `fetch` + `AbortController` + a web
// stream reader), captures stdout/stderr separately, redacts the API key from
// all output, and exits nonzero on structured failure.
//
// Usage:
//   node script/wasm/node-harness.mjs \
//     --prompt hello \
//     --api-key "$WARP_API_KEY" \
//     --server-root-url "$WARP_SERVER_ROOT_URL" \
//     [--model <model-id>] [--timeout-ms <n>]
//
// `--api-key` defaults to $WARP_API_KEY; `--server-root-url` defaults to
// $WARP_SERVER_ROOT_URL (and otherwise to https://app.warp.dev).
//
// This is a proof-of-concept harness. It never logs the API key.

import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";
import { readFileSync } from "node:fs";
import { performance } from "node:perf_hooks";

const __dirname = dirname(fileURLToPath(import.meta.url));
const REPO_ROOT = join(__dirname, "..", "..");

function parseArgs(argv) {
  const out = {};
  for (let i = 2; i < argv.length; i++) {
    const a = argv[i];
    if (a.startsWith("--")) {
      const key = a.slice(2).replace(/-([a-z])/g, (_, c) => c.toUpperCase());
      const val = argv[i + 1];
      out[key] = val;
      i++;
    }
  }
  return out;
}

const args = parseArgs(process.argv);
const prompt = args.prompt ?? "";
const apiKey = args.apiKey ?? process.env.WARP_API_KEY ?? "";
const serverRootUrl =
  args.serverRootUrl ??
  process.env.WARP_SERVER_ROOT_URL ??
  "https://app.warp.dev";
const model = args.model ?? null;
const timeoutMs = args.timeoutMs ? Number(args.timeoutMs) : null;

// Redaction: a single sink so the API key can never leak.
const REDACT = new RegExp(
  apiKey.replace(/[.*+?^${}()|[\]\\]/g, "\\$&"),
  "g",
);
function redact(s) {
  if (!apiKey) return s;
  return String(s).replace(REDACT, "<redacted>");
}
function stdout(s) {
  process.stdout.write(redact(s) + "\n");
}
function stderr(s) {
  process.stderr.write(redact(s) + "\n");
}

// Locate the generated Node loader. The build script writes it under
// target/wasm32-unknown-unknown/<debug|release>/node/warp_node_proto.js.
function resolveLoader() {
  const base = join(
    REPO_ROOT,
    "target",
    "wasm32-unknown-unknown",
  );
  for (const profile of ["debug", "release"]) {
    const candidate = join(base, profile, "node", "warp_node_proto.js");
    try {
      readFileSync(candidate); // exists?
      return candidate;
    } catch {
      // continue
    }
  }
  stderr(
    `Could not find generated loader under ${base}/{debug,release}/node/warp_node_proto.js. ` +
      `Run ./script/wasm/build-node first.`,
  );
  process.exit(2);
}

// ---- host contract ---------------------------------------------------------
//
// host.fetch(url, init) -> Promise<{ status, statusText, headers, body }>
// where body.read() -> Promise<{ done, value?: Uint8Array }> mirrors a web
// ReadableStream reader. The harness owns the AbortController/timeout.
function makeHost() {
  return {
    async fetch(url, init) {
      const controller = new AbortController();
      let timer = null;
      if (init && init.timeoutMs) {
        timer = setTimeout(() => controller.abort(), init.timeoutMs);
      }
      const headers = init?.headers ?? {};
      try {
        const res = await fetch(url, {
          method: init?.method ?? "POST",
          headers,
          body: init?.body,
          signal: controller.signal,
          // Node's fetch follows redirects by default; keep that for the
          // server's auth/redirect flow if any.
        });
        const headerObj = {};
        res.headers.forEach((v, k) => {
          headerObj[k] = v;
        });
        const reader = res.body?.getReader?.();
        const body = {
          read() {
            if (!reader) {
              return Promise.resolve({ done: true, value: undefined });
            }
            return reader.read();
          },
        };
        return {
          status: res.status,
          statusText: res.statusText,
          headers: headerObj,
          body,
        };
      } finally {
        if (timer) clearTimeout(timer);
      }
    },
  };
}

async function main() {
  if (!apiKey) {
    stderr("error: missing api_key (pass --api-key or set WARP_API_KEY)");
    process.exit(2);
  }
  if (!prompt) {
    stderr("error: missing prompt (pass --prompt)");
    process.exit(2);
  }

  const loaderPath = resolveLoader();
  const mod = await import(loaderPath);
  const runMultiAgent = mod.run_multi_agent;
  if (typeof runMultiAgent !== "function") {
    stderr(`error: loader ${loaderPath} did not export run_multi_agent`);
    process.exit(2);
  }

  const config = {
    prompt,
    api_key: apiKey,
    server_root_url: serverRootUrl,
    timeout_ms: timeoutMs,
  };
  if (model) config.model = model;

  const t0 = performance.now();
  let resultStr;
  try {
    resultStr = await runMultiAgent(config, makeHost());
  } catch (err) {
    stderr(`harness: run_multi_agent threw: ${redact(String(err?.stack ?? err))}`);
    process.exit(1);
  }
  const elapsed = performance.now() - t0;

  let result;
  try {
    result = JSON.parse(resultStr);
  } catch {
    stderr(`harness: result was not JSON: ${redact(String(resultStr))}`);
    process.exit(1);
  }

  // Structured result to stdout; diagnostics to stderr. Order preserved.
  stdout(JSON.stringify(result, null, 2));
  if (result.timings_ms) {
    stderr(`harness: wall clock ${elapsed.toFixed(1)}ms (wasm total ${result.timings_ms.total_ms?.toFixed(1) ?? "?"}ms, node ${process.version})`);
  }

  process.exit(result.ok ? 0 : 1);
}

main().catch((err) => {
  stderr(`harness: fatal: ${redact(String(err?.stack ?? err))}`);
  process.exit(1);
});
