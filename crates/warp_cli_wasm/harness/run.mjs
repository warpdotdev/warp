#!/usr/bin/env node
// Node harness for the warp_cli_wasm spike.
//
// Loads the wasm-bindgen-generated Node module and exercises the three
// entrypoints that prove the CLI can run sandboxless in a JS-hosted WASM
// runtime:
//   1. agent_run_from_config  — build an `agent run` command from a JSON config
//   2. agent_run_from_argv    — parse an argv-style array through the real clap parser
//   3. http_get               — make an outbound HTTP request from inside the WASM module
//
// Usage:
//   node harness/run.mjs <path-to-warp_cli_wasm.js> [ping-url]
//
// Defaults to the package produced by `wasm-bindgen --target nodejs` at
// <repo-root>/target/wasm-cli-pkg/warp_cli_wasm.js (see README.md for the exact
// build commands).

import { createRequire } from "node:module";
import path from "node:path";

const require = createRequire(import.meta.url);

// This file lives at crates/warp_cli_wasm/harness/run.mjs, so the repo-root
// target dir is three levels up (harness -> warp_cli_wasm -> crates -> root).
const modulePath =
  process.argv[2] ||
  path.resolve(import.meta.dirname, "../../../target/wasm-cli-pkg/warp_cli_wasm.js");
const pingUrl = process.argv[3] || "https://httpbin.org/get";

console.log(`[harness] loading wasm module: ${modulePath}`);
const wasm = require(modulePath);

function show(label, json) {
  console.log(`\n=== ${label} ===`);
  try {
    const parsed = JSON.parse(json);
    console.log(JSON.stringify(parsed, null, 2));
  } catch {
    console.log(json);
  }
}

// Redact any `--api-key` value before logging argv, so a real WARP_API_KEY is
// never written to terminal/CI output. Handles both `--api-key value` and
// `--api-key=value` forms (the Rust parser accepts both; only the *logged*
// copy is redacted — the raw value is still passed to the wasm module).
function redactArgv(argv) {
  const out = [];
  for (let i = 0; i < argv.length; i++) {
    const arg = argv[i];
    if (arg === "--api-key") {
      out.push(arg);
      if (i + 1 < argv.length) {
        out.push("<redacted>");
        i++;
      }
    } else if (arg.startsWith("--api-key=")) {
      out.push("--api-key=<redacted>");
    } else {
      out.push(arg);
    }
  }
  return out;
}

// 1. agent_run_from_config — equivalent to:
//    oz-dev agent run --prompt hello --api-key="$WARP_API_KEY"
const config = {
  prompt: "hello",
  api_key: process.env.WARP_API_KEY || "test-key-redacted",
  output_format: "json",
  harness: "oz",
};
console.log("[harness] calling agent_run_from_config with:", {
  ...config,
  api_key: config.api_key ? "<redacted>" : null,
});
const result1 = wasm.agent_run_from_config(JSON.stringify(config));
show("agent_run_from_config result", result1);

// 2. agent_run_from_argv — parse a real argv through the clap parser.
const argv = [
  "oz",
  "agent",
  "run",
  "--prompt",
  "hello",
  "--api-key",
  process.env.WARP_API_KEY || "test-key-redacted",
  "--output-format",
  "json",
];
console.log("\n[harness] calling agent_run_from_argv with argv:", redactArgv(argv));
const result2 = wasm.agent_run_from_argv(JSON.stringify(argv));
show("agent_run_from_argv result", result2);

// 3. http_get — outbound HTTP request from inside the WASM module.
console.log(`\n[harness] calling http_get("${pingUrl}") from inside the WASM module...`);
let httpOk = false;
try {
  const result3 = await wasm.http_get(pingUrl);
  console.log("=== http_get result ===");
  console.log(result3);
  httpOk = true;
} catch (err) {
  console.log("=== http_get failed ===");
  console.log(String(err));
}

console.log("\n[harness] done. The warp_cli layer executed inside a Node-hosted WASM module.");

// The harness is the spike's networking verification signal, so a failed
// http_get must surface as a non-zero exit status — otherwise automation can
// report a broken HTTP path as passing.
if (!httpOk) {
  console.error("[harness] http_get failed; exiting with status 1.");
  process.exit(1);
}
