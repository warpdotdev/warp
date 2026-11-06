# Skill reference parser cases

`cases.yaml` is the parity contract between the Go parser
(`logic/factoryfile/skill_reference.go`) and the Rust parser in the `warp`
client: both run every case, and every error kind appears in at least one.
Edit this directory here first, then copy it to the client verbatim. The
parsing rules themselves live in PRODUCT.md (P3–P9, P18) and the parsers.

Each case has:

- `name`: unique, and names the test.
- `input`: the reference file's contents.
- `expected`: `{error: <kind>}` for the first problem in the file, or
  `{reference: {source, include, exclude}}`, where `source` is
  `{code_forge: null | "<FORGE>", owner, repo, ref: null | {kind: "sha" | "named", value}}`,
  each entry is `{kind: "name" | "path" | "name_glob" | "path_glob", value}`,
  and `exclude` is `[]` when absent.
