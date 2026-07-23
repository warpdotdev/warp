#!/usr/bin/env node
// Minimal Node harness for the warp_wasm_node AgentDriver path (REMOTE-2264).
// Imports the generated wasm-bindgen Node loader, invokes
// `run_agent_driver_wasm`, and reports the structured result + peak memory.
//
// Usage:
//   node script/wasm/node-harness-driver.mjs \
//     --prompt hello \
//     --api-key "$WARP_API_KEY" \
//     --server-root-url "$WARP_SERVER_ROOT_URL"

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
      out[key] = argv[i + 1];
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

const REDACT = new RegExp(
  apiKey.replace(/[.*+?^${}()|[\]\\]/g, "\\$&"),
  "g",
);
function redact(s) {
  if (!apiKey) return s;
  return String(s).replace(REDACT, "<redacted>");
}
function stdout(s) { process.stdout.write(redact(s) + "\n"); }
function stderr(s) { process.stderr.write(redact(s) + "\n"); }

function resolveLoader() {
  const base = join(REPO_ROOT, "target", "wasm32-unknown-unknown");
  for (const profile of ["debug", "release", "release-wasm"]) {
    const candidate = join(base, profile, "node", "warp_wasm_node.js");
    try {
      readFileSync(candidate);
      return candidate;
    } catch { /* continue */ }
  }
  stderr(`Could not find warp_wasm_node.js. Run wasm-bindgen first.`);
  process.exit(2);
}

async function main() {
  if (!apiKey) { stderr("error: missing api_key"); process.exit(2); }
  if (!prompt) { stderr("error: missing prompt"); process.exit(2); }

  const loaderPath = resolveLoader();

  // Peak RSS sampler.
  let peakRss = process.memoryUsage().rss;
  let peakHeap = process.memoryUsage().heapUsed;
  let peakExternal = process.memoryUsage().external;
  let peakArrayBuffers = process.memoryUsage().arrayBuffers;
  const rssTimer = setInterval(() => {
    const mem = process.memoryUsage();
    if (mem.rss > peakRss) peakRss = mem.rss;
    if (mem.heapUsed > peakHeap) peakHeap = mem.heapUsed;
    if (mem.external > peakExternal) peakExternal = mem.external;
    if (mem.arrayBuffers > peakArrayBuffers) peakArrayBuffers = mem.arrayBuffers;
  }, 10);

  stderr(`Loading wasm module: ${loaderPath}`);
  const mod = await import(loaderPath);
  const runFn = mod.run_agent_driver_wasm;
  if (typeof runFn !== "function") {
    stderr(`error: loader did not export run_agent_driver_wasm`);
    clearInterval(rssTimer);
    process.exit(2);
  }

  const config = { prompt, api_key: apiKey, server_root_url: serverRootUrl };
  const t0 = performance.now();
  let resultStr;
  try {
    resultStr = await runFn(config);
  } catch (err) {
    clearInterval(rssTimer);
    stderr(`harness: run_agent_driver_wasm threw: ${redact(String(err?.stack ?? err))}`);
    reportMem();
    process.exit(1);
  }
  const elapsed = performance.now() - t0;
  clearInterval(rssTimer);

  let result;
  try { result = JSON.parse(resultStr); }
  catch {
    stderr(`harness: result was not JSON: ${redact(String(resultStr))}`);
    reportMem();
    process.exit(1);
  }

  stdout(JSON.stringify(result, null, 2));
  stderr(`harness: wall clock ${elapsed.toFixed(1)}ms (node ${process.version})`);
  reportMem();
  process.exit(result.ok ? 0 : 1);

  function reportMem() {
    const fmt = (b) => `${(b / 1048576).toFixed(1)} MB (${b} bytes)`;
    stderr(`harness: peak memory — rss: ${fmt(peakRss)}, heap: ${fmt(peakHeap)}, external: ${fmt(peakExternal)}, arrayBuffers: ${fmt(peakArrayBuffers)}`);
  }
}

main().catch((err) => {
  stderr(`harness: fatal: ${redact(String(err?.stack ?? err))}`);
  process.exit(1);
});
