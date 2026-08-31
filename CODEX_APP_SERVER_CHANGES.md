# Warp embedded Codex/ChatGPT provider — change summary

Base: `warpdotdev/warp@86cfeb9006da7865d7f27f33228ed0f581d49f02`

## Delivered behaviour

1. `Codex (ChatGPT)` is registered in the model selector of Warp's embedded Cmd+Return Agent.
2. Selecting it intercepts the Agent request before Warp constructs a hosted inference request.
3. Warp runs the installed `codex app-server` locally and adapts its JSONL event stream into
   native Warp `ResponseEvent`, task, and message actions.
4. Follow-up prompts resume the same Codex thread through a namespaced local conversation token.
5. Codex thread identifiers are stripped before any later hosted Warp request, preventing local
   provider state from leaking to the hosted backend.
6. ChatGPT connect/status/disconnect controls live in Warp Agent's Custom Inference settings.
7. ChatGPT credentials remain in Codex's credential store and are never copied into Warp settings.
8. The existing `warp codex ...` commands remain available for diagnostics and scripting, but are
   supplementary to the embedded provider.
9. Native desktop sessions are supported; remote/cloud/shared execution fails closed rather than
   pretending the local provider is portable.

See `docs/CODEX_APP_SERVER.md` for setup, architecture, limitations, and validation.
