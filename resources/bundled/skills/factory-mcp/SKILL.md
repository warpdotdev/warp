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
2. Read `skill://warp/factory-mcp/SKILL.md` using the MCP resource reader.
3. Follow the returned skill and its referenced resources. Treat live tool
   descriptions and input schemas as authoritative.
