#!/usr/bin/env node
// A minimal Agent Client Protocol agent for exercising Warp's ACP harness without a model.
// Speaks newline-delimited JSON-RPC 2.0 over stdio and replays a scripted turn:
// thought chunk -> message chunks -> execute tool call (with a permission request) -> plan ->
// client fs write + read round trip -> final message -> `end_turn`.
//
// Usage: node fake_agent.mjs [--delay-ms N] [--fail]
import { createInterface } from "node:readline";
import { tmpdir } from "node:os";
import { join } from "node:path";

const args = process.argv.slice(2);
const delayMs = Number(args[args.indexOf("--delay-ms") + 1]) || 150;
const shouldFail = args.includes("--fail");

let nextId = 1;
const pending = new Map();
let sessionId = null;

function send(message) {
  process.stdout.write(`${JSON.stringify({ jsonrpc: "2.0", ...message })}\n`);
}

function notify(method, params) {
  send({ method, params });
}

function request(method, params) {
  const id = nextId++;
  return new Promise((resolve, reject) => {
    pending.set(id, { resolve, reject });
    send({ id, method, params });
  });
}

function update(update) {
  notify("session/update", { sessionId, update });
}

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

async function runTurn(prompt) {
  const promptText = prompt
    .map((block) => (block.type === "text" ? block.text : `[${block.type}]`))
    .join("\n");

  update({
    sessionUpdate: "agent_thought_chunk",
    content: { type: "text", text: "The user wants a quick demo; I'll run a command and summarize." },
  });
  await sleep(delayMs);

  for (const chunk of ["Hello from the ", "fake ACP agent. ", `You said: "${promptText.slice(0, 60)}".\n`]) {
    update({
      sessionUpdate: "agent_message_chunk",
      messageId: "msg-1",
      content: { type: "text", text: chunk },
    });
    await sleep(delayMs);
  }

  update({
    sessionUpdate: "plan",
    entries: [
      { content: "Inspect the working directory", priority: "high", status: "in_progress" },
      { content: "Report back", priority: "medium", status: "pending" },
    ],
  });

  const toolCallId = "call-1";
  update({
    sessionUpdate: "tool_call",
    toolCallId,
    title: "List files",
    kind: "execute",
    status: "pending",
    rawInput: { command: "ls -1 | head -5" },
  });

  const permission = await request("session/request_permission", {
    sessionId,
    toolCall: { toolCallId },
    options: [
      { optionId: "allow-once", name: "Allow once", kind: "allow_once" },
      { optionId: "allow-always", name: "Always allow", kind: "allow_always" },
      { optionId: "reject-once", name: "Reject", kind: "reject_once" },
    ],
  });
  process.stderr.write(`permission outcome: ${JSON.stringify(permission.outcome)}\n`);

  update({ sessionUpdate: "tool_call_update", toolCallId, status: "in_progress" });
  await sleep(delayMs);
  update({
    sessionUpdate: "tool_call_update",
    toolCallId,
    status: shouldFail ? "failed" : "completed",
    content: [
      {
        type: "content",
        content: { type: "text", text: "README.md\nCargo.toml\napp\ncrates\nscript" },
      },
    ],
  });

  update({
    sessionUpdate: "plan",
    entries: [
      { content: "Inspect the working directory", priority: "high", status: "completed" },
      { content: "Report back", priority: "medium", status: "in_progress" },
    ],
  });
  await sleep(delayMs);

  // Exercise the client's file system methods the way a real agent's read/edit tools would.
  const scratchPath = join(tmpdir(), `fake-acp-agent-${process.pid}.txt`);
  await request("fs/write_text_file", { sessionId, path: scratchPath, content: "line 1\nline 2\nline 3\n" });
  const read = await request("fs/read_text_file", { sessionId, path: scratchPath, line: 2, limit: 1 });
  process.stderr.write(`fs round trip read back: ${JSON.stringify(read.content)}\n`);

  update({
    sessionUpdate: "agent_message_chunk",
    messageId: "msg-2",
    content: { type: "text", text: `Done. The directory looks like a Rust workspace (scratch read back: ${read.content}).` },
  });
  update({ sessionUpdate: "usage_update", used: 1234, size: 200000 });

  return { stopReason: shouldFail ? "refusal" : "end_turn" };
}

async function handleRequest(message) {
  switch (message.method) {
    case "initialize":
      return {
        protocolVersion: 1,
        agentCapabilities: { loadSession: false, promptCapabilities: { image: false } },
        agentInfo: { name: "fake-acp-agent", title: "Fake ACP Agent", version: "0.1.0" },
        authMethods: [],
      };
    case "session/new":
      sessionId = `sess_${Date.now()}`;
      return { sessionId };
    case "session/prompt":
      return runTurn(message.params.prompt ?? []);
    case "session/cancel":
      return null;
    default:
      throw { code: -32601, message: `Method ${message.method} not found` };
  }
}

const reader = createInterface({ input: process.stdin, crlfDelay: Infinity });
reader.on("line", async (line) => {
  if (!line.trim()) return;
  let message;
  try {
    message = JSON.parse(line);
  } catch (error) {
    process.stderr.write(`bad json: ${error}\n`);
    return;
  }
  if (message.method !== undefined && message.id !== undefined) {
    try {
      const result = await handleRequest(message);
      send({ id: message.id, result });
    } catch (error) {
      send({ id: message.id, error: { code: error.code ?? -32603, message: String(error.message ?? error) } });
    }
  } else if (message.id !== undefined) {
    const waiter = pending.get(message.id);
    pending.delete(message.id);
    if (!waiter) return;
    if (message.error) waiter.reject(new Error(message.error.message));
    else waiter.resolve(message.result);
  } else if (message.method === "session/cancel") {
    process.stderr.write("cancel requested\n");
  }
});
reader.on("close", () => process.exit(0));
