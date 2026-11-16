/// Minimal stub of the former legacy MCP server manager module.
///
/// The only thing still needed from this module is the legacy secure-storage
/// key constant in `oauth`, which is read during the one-time migration of
/// legacy MCP servers to the templatable model.
pub mod oauth;
