# Language registry

This crate is the lightweight source of truth for file-to-language identity in
Warp. It deliberately has no dependencies on parsers, the editor, or platform
APIs so native and `wasm32` consumers can use it.

## Provenance and conflict policy

The initial table is a behavior-preserving migration of Warp's mappings at
commit `abea51cd1e102b363935f1b25ef03d335bc7b36f`:

- `crates/languages/src/lib.rs` supplies the grammar associations.
- `crates/lsp/src/config.rs` supplies the built-in LSP associations and
  language identifiers.

No new file association is imported in this initial migration. For future
updates, the upstream comparison sources are pinned to:

- VS Code `ae0e51e8241f55c818f43e816fda2fcfd2a53ea3`
- Neovim `d06bac614c2bc09db62e439b2e42ac7101653333`

An existing Warp LSP identifier has highest precedence. For a selector without
one, VS Code is consulted next and Neovim last; therefore VS Code wins a
cross-source disagreement deterministically. For a selector that still needs an
identifier, the generator stops instead of guessing if one source assigns it
multiple identities. Upstream data can fill identity and `language_id`, but it
never adds a selector or changes a grammar association from the Warp baseline.
Dynamic Neovim detectors and VS Code filename patterns are intentionally not
flattened into a single value; first-line and general pattern detection remain
later work. A source update must pin new revisions in this file; it must not
silently change the registry from a moving upstream branch.

At runtime, exact filenames take precedence over the two legacy Dockerfile
prefixes, which take precedence over extensions. Duplicate selectors within a
precedence tier are rejected by tests. Entries may share an identity when the
existing consumers support different selectors. For example, `.py3` has a
Python grammar but no built-in LSP mapping, while `.C` has a built-in LSP
mapping but no editor grammar mapping.

That precedence applies to general/editor resolution. Built-in LSP resolution
uses the same tiers but skips an entry that has no `built_in_lsp` facet before
trying the next tier. Consequently `Dockerfile.rs` keeps the Dockerfile grammar
while retaining the historical Rust built-in LSP dispatch from its extension.

## Updating parity fixtures

The golden cases are intentionally explicit and independent of the production
table:

- `crates/languages/src/lib_tests.rs` lists every pre-migration grammar selector.
- `crates/lsp/src/config_tests.rs` lists every pre-migration LSP selector and
  every built-in LSP language identifier.

`fixtures/warp_baseline.tsv` is the reviewed selector/grammar and built-in LSP
baseline. Its `built_in_lsp` column is the exact historical
`LanguageId::from_path` allowlist, independent of the broader `language_id`
facet. `script/generate_seed.py` combines only those selectors with the two
pinned upstream repositories and writes both `fixtures/registry_seed.tsv` and
`src/generated.rs`. The generator reads each pin directly with `git ls-tree`
and `git show`; the repositories' checked-out HEAD, dirty files, untracked
files, and ignored files cannot affect output:

```sh
python3 crates/language_registry/script/generate_seed.py \
  --vscode-root /path/to/vscode \
  --neovim-root /path/to/neovim
```

When an association is intentionally changed, update the baseline and the
corresponding golden row, regenerate both outputs, explain the behavior change
in the pull request, and run:

```sh
cargo nextest run -p language_registry -p languages -p lsp
cargo check -p language_registry --target wasm32-unknown-unknown
cargo check -p lsp -p warpui --target wasm32-unknown-unknown
```

First-line and general filename-pattern detection remain reserved for a later
change. The resolver accepts a first line now so consumers will not need an API
change when that detection is introduced.
