# Tech Spec: Prevent false positive secret redaction warnings for MCP server configs

**Issue:** [warpdotdev/warp#13300](https://github.com/warpdotdev/warp/issues/13300)  
**Status:** In Review  
**Product Spec:** [`specs/GH13300/product.md`](specs/GH13300/product.md)  

---

## 1. Context & Architecture Analysis

### Current Secret Redaction Architecture
Warp manages secret detection and redaction across several layers:

1. **Regex Engine (`crates/secret_redaction/src/lib.rs`)**:
   - `SECRETS_REGEX`: Global `Mutex<Arc<SecretsRegex>>` compiled with `regex_automata::meta::Regex` and `RegexDFAs`.
   - `DEFAULT_REGEXES_WITH_NAMES`: A static slice of 17 recommended pattern definitions, including:
     - `IPV4_ADDRESS` (`r"\b((25[0-5]|(2[0-4]|1\d|[1-9]|)\d)\.?\b){4}\b"`)
     - `IPV6_ADDRESS` (`r"\b((([0-9A-Fa-f]{1,4}:){1,6}:)|(([0-9A-Fa-f]{1,4}:){7}))([0-9A-Fa-f]{1,4})\b"`)
     - Standard credentials: Google API Key, OpenAI API Key, Anthropic API Key, AWS Access ID, GitHub tokens, Slack app token, Stripe keys, JWT, Warp API Key, etc.
   - `find_secrets_in_text(text: &str) -> Vec<StringRange>`: Iterates matches against `SECRETS_REGEX` and returns `StringRange` entries containing both `char_range` and `byte_range`.

2. **User & Enterprise Customization (`app/src/settings/privacy.rs` & `app/src/settings_view/privacy_page.rs`)**:
   - Users can customize patterns in **Settings > Privacy**:
     - `user_secret_regex_list`: Vector of `CustomSecretRegex { pattern, name }`.
     - Users can toggle/delete individual recommended patterns and add custom regexes.
   - Enterprise workspaces can enforce enterprise patterns via `get_enterprise_secret_redaction_regex_list()`.
   - `set_user_and_enterprise_secret_regexes`: Recompiles `SECRETS_REGEX` additively, prioritizing enterprise patterns followed by deduplicated user patterns.

3. **MCP Server Validation (`app/src/settings_view/mcp_servers/edit_page.rs`)**:
   - When a user enters or pastes an MCP server configuration, `detect_secrets_in_templatable_mcp_server` (lines 534–558) validates the JSON string:
     ```rust
     let contains_secrets = !find_secrets_in_text(&templatable_mcp_server.template.json).is_empty();
     if should_block_save_for_secrets(safe_mode_enabled, enterprise_enforced, contains_secrets) {
         // Display toast and return Err
     }
     ```
   - When a configuration contains e.g. `"url": "http://127.0.0.1:29979/mcp"`, `find_secrets_in_text` returns the range corresponding to `127.0.0.1`.
   - As a result, `contains_secrets` evaluates to `true`, and saving is blocked.

---

## 2. Analysis of Approaches (Addressing Maintainer Inquiries)

Maintainer `@peicodes` noted:
> *"I think the most likely solution here is to have secret redaction ignore IP addresses and not flag them as secrets. I think a spec will be helpful because it should describe how the current system works and what the approach is for getting the redaction to ignore IP addresses. Are there multiple redaction rules or just one? Is it something the user can customize?"*

### Answers to Maintainer Questions:
- **How many IP redaction rules exist?** There are **two** distinct default rules: `IPV4_ADDRESS` and `IPV6_ADDRESS` in `DEFAULT_REGEXES_WITH_NAMES`.
- **Can users customize them?** Yes, both appear in **Settings > Privacy** as toggleable/deletable items in `user_secret_regex_list`, and users can add arbitrary custom regexes.
- **Approaches considered:**

### Approach Comparison

| Approach | Description | Pros | Cons | Recommendation |
| :--- | :--- | :--- | :--- | :--- |
| **Approach 1: Remove IP rules globally from `DEFAULT_REGEXES_WITH_NAMES`** | Delete `IPV4_ADDRESS` and `IPV6_ADDRESS` from default recommended list. | Eliminates IP false positives across all surfaces. | Breaking change for terminal output redaction; breaks sync with server-side `logic/ai/util.go` noted in `secret_redaction/src/lib.rs:377`. Users relying on IP masking in terminal logs lose default coverage. | ❌ Too disruptive |
| **Approach 2: Context-aware filtering at MCP validation boundary** | In MCP validation (`edit_page.rs`), evaluate detected secret ranges and ignore matches that parse as valid `std::net::IpAddr`. | Surgical; zero risk to terminal grid redaction; preserves user settings; robust against formatting. | MCP-specific, though MCP is currently the only place where network endpoint JSON is validated as secrets. | ✅ Preferred / Clean |
| **Approach 3: Crate-level helper in `crates/secret_redaction`** | Expose `find_secrets_in_text_excluding_ips(text: &str) -> Vec<StringRange>` in `crates/secret_redaction` and call it in `edit_page.rs`. | Reusable across any future config validation; unit tested directly in `secret_redaction`; leaves terminal redaction untouched. | Slightly broader API addition. | ⭐️ Best Architecture (Selected) |

---

## 3. Proposed Implementation

We adopt **Approach 3**: implement `find_secrets_in_text_excluding_ips` in `crates/secret_redaction` and update `app/src/settings_view/mcp_servers/edit_page.rs` to use it.

### 3.1 `crates/secret_redaction/src/lib.rs`
Add `find_secrets_in_text_excluding_ips`:
```rust
/// Returns the ranges of detected secrets in the given text, excluding matches that parse as IP addresses.
///
/// Used when validating configuration files (such as MCP server endpoints) where IPv4/IPv6 addresses
/// represent valid network hosts rather than sensitive auth credentials.
pub fn find_secrets_in_text_excluding_ips(text: &str) -> Vec<StringRange> {
    find_secrets_in_text(text)
        .into_iter()
        .filter(|range| {
            let matched_text = text.get(range.byte_range.clone()).unwrap_or("");
            matched_text.parse::<std::net::IpAddr>().is_err()
        })
        .collect()
}
```

### 3.2 `app/src/ai/blocklist/block/secret_redaction.rs`
Re-export the new helper:
```rust
pub use secret_redaction::{
    SECRET_REDACTION_REPLACEMENT_CHARACTER, find_secrets_in_text,
    find_secrets_in_text_excluding_ips, find_secrets_in_text_with_levels,
};
```

### 3.3 `app/src/settings_view/mcp_servers/edit_page.rs`
Update `detect_secrets_in_templatable_mcp_server`:
```rust
use crate::ai::blocklist::secret_redaction::find_secrets_in_text_excluding_ips;

...

fn detect_secrets_in_templatable_mcp_server(
    &self,
    ctx: &mut ViewContext<Self>,
    templatable_mcp_server: &TemplatableMCPServer,
) -> Result<(), String> {
    let safe_mode_enabled = *SafeModeSettings::as_ref(ctx).safe_mode_enabled.value();
    let enterprise_enforced =
        UserWorkspaces::as_ref(ctx).is_enterprise_secret_redaction_enabled();
    let contains_secrets =
        !find_secrets_in_text_excluding_ips(&templatable_mcp_server.template.json).is_empty();

    if should_block_save_for_secrets(safe_mode_enabled, enterprise_enforced, contains_secrets) {
        ...
```

---

## 4. Verification Plan

### Invariant Verification Matrix

| Invariant | Test Scenario | Verified By |
| :--- | :--- | :--- |
| **INV-1** (IPv4 URLs Allowed) | JSON with `http://127.0.0.1:29979/mcp` and `0.0.0.0` has zero secrets detected | `secret_redaction` unit test + `edit_page` test |
| **INV-2** (IPv6 URLs Allowed) | JSON with `http://[::1]:8080/mcp` has zero secrets detected | `secret_redaction` unit test |
| **INV-3** (Real Credentials Blocked) | JSON with `ghp_...` or `sk-...` detects secret and triggers save block | `edit_page_tests` |
| **INV-4** (Mixed IP + Credential) | JSON with both `127.0.0.1` and `sk-...` detects secret and blocks save | `secret_redaction` unit test + `edit_page_tests` |
| **INV-5** (Redaction Disabled) | Safe mode = false, enterprise = false allows save regardless | `edit_page_tests::does_not_block_when_redaction_off_even_if_secrets_present` |

### Automated Commands
```bash
# Unit tests in secret_redaction crate
cargo test -p secret_redaction

# Unit tests in warp app settings_view
cargo test -p warp --lib settings_view::mcp_servers::edit_page

# Linting
cargo clippy -p secret_redaction -j 2 --all-targets --tests -- -D warnings
cargo clippy -p warp -j 2 --all-targets --tests -- -D warnings

# Mutating formatter
./script/format
```

---

## 5. Risks & Mitigations

- **Risk:** Could an attacker use an IP address as a secret?  
  **Mitigation:** IP addresses are public/private network routable addresses, not cryptographic secrets or API keys. Legitimate credentials (tokens, private keys, passwords) do not parse as valid `std::net::IpAddr`.
- **Risk:** Malformed unicode slicing.  
  **Mitigation:** `text.get(range.byte_range)` is used instead of direct indexing, returning empty string safely if bounds are invalid.
