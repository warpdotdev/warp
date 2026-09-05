---
name: factory-mcp
description: Use the Warp Factory MCP to hand work to a software factory and collaborate with it — bundle local work and send it to the cloud, find factory tasks from a Slack thread / Linear ticket / description, and pull a task down to test or iterate locally and hand it back
---

# Factory MCP

The canonical Factory workflow is served by the connected `warp-factory` MCP
server so its instructions stay synchronized with the live tool schemas.

Before invoking a Factory MCP tool:

1. Confirm the `warp-factory` server and its tools are available. If they are
   unavailable, stop and tell the user that Factory MCP must be connected and
   authenticated.
2. If the harness provides an MCP resource reader, read
   `skill://warp/factory-mcp/SKILL.md`, then follow the returned skill and its
   referenced resources.
3. If the harness cannot read MCP resources, use the server instructions and
   live tool descriptions as the reduced-capability guide. Do not claim that
   the canonical workflow was loaded.

Treat live tool descriptions and input schemas as authoritative.
