# tree_sitter_pascal

Vendored tree-sitter grammar for Pascal / Object Pascal / Delphi.

Every other language in `crates/languages` gets its grammar from the `arborium`
crate, but arborium has no Pascal grammar (checked through 2.18.1), so the
generated parser is checked in here instead.

The published `tree-sitter-pascal` crate is not depended on directly because its
build script compiles `parser.c` with a bare `cc::Build`, which fails on
`wasm32-unknown-unknown` — that target has no libc to satisfy `parser.h`'s
`<stdint.h>`/`<stdlib.h>` includes. Vendoring lets `build.rs` point the compiler at
`arborium-sysroot`, exactly as arborium's own grammar crates do, so Pascal builds
everywhere the other languages do.

## Provenance

- Upstream: <https://github.com/Isopod/tree-sitter-pascal>
- Published as `tree-sitter-pascal` 0.10.2 on crates.io
- Revision: `2f28b717be47cf592241e1b7bec3b2b906f59148`
- License: MIT, see `grammar/LICENSE`

The grammar has no external scanner (`EXTERNAL_TOKEN_COUNT` is 0), so `parser.c` is
the only C file. It targets tree-sitter ABI 14, inside the 13–15 range
`arborium-tree-sitter` accepts.

`grammar/src/node-types.json` is not used by the build. It is kept because it is the
reference for checking that the query files still match the grammar.

## Refreshing the grammar

1. Download the desired `tree-sitter-pascal` release from crates.io.
2. Replace `grammar/src/parser.c`, `grammar/src/tree_sitter/parser.h` and
   `grammar/src/node-types.json`.
3. Update the revision above.
4. Re-check the query files in `crates/languages/grammars/pascal/` against the new
   `node-types.json`. `all_supported_languages_load_successfully` in
   `crates/languages/src/lib_tests.rs` fails if a query names a node type or field
   that no longer exists.

## Known limitations

Two constructs the grammar does not accept, each producing an ERROR node:

- a bare re-raise (`raise;` with no exception)
- Free Pascal's binary literals (`%1010`)

Tree-sitter recovery is local, so the rest of the file still parses and highlights —
the `raise` keyword itself even stays colored; only that one statement's structure is
lost. Worth re-checking on every refresh.
