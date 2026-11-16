# lsp

This crate provides a stdio-only Language Server Protocol (LSP) client transport for Warp. It:

- Spawns and manages a language server process (child process)
- Communicates over stdio using JSON-RPC with proper Content-Length framing


See `examples/rust-lsp/main.rs` for an example implementation.
