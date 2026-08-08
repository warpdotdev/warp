# Vendored cross-repo contracts

**These files are copies. `warpdotdev/warp-server` is canonical — do not edit them here.**

| Path | Canonical location in `warpdotdev/warp-server` |
| --- | --- |
| `factory_plugin_runtime_contract.json` | `logic/ai/ambient_agents/workers/common/testdata/factory_plugin_runtime_contract.json` |
| `agentplugins-conformance/` | `logic/factoryfile/testdata/agentplugins-conformance/` |

`PROVENANCE.json` records the exact warp-server commit these copies were taken from and the
SHA-256 of every vendored file.

## Why copies rather than a shared artifact

The Factory plugin environment contract is produced by warp-server and consumed by this client,
and the Agent Plugins conformance corpus is validated independently by both. When either drifts,
Factory sync accepts something the runtime then rejects — invisible until a user's plugin stops
working.

We have no build-time mechanism for sharing an artifact across the two repositories, so the
mechanism is deliberate, visible duplication: warp-server asserts the producing half against its
originals, this crate asserts the consuming half against these copies, and a divergence becomes a
diff between two committed files rather than a runtime mystery.

## The staleness hole, and what closes it

Duplication alone is not enough, and this is worth stating because we hit it. A vendored copy plus
a test that compares the client against *that copy* only ever proves the client agrees with
itself. It cannot notice that the copy has fallen behind upstream. That is exactly how the scope
segment silently diverged once: the copy predated an upstream change, every test passed, and a
human diffing the two files by hand was the only thing that caught it.

Two things close it, and both are needed:

1. **`PROVENANCE.json` pins the upstream commit and every file hash**, and
   `contract_provenance_matches_the_vendored_files` asserts the hashes. A local edit to a vendored
   file now fails the build instead of quietly forking the contract.
2. **The recorded commit makes staleness checkable in one command** (below). A reviewer can
   confirm the copy is current without reading both repositories.

## Verify the copies are current

From a warp-server checkout:

```sh
WARP_SERVER=~/src/warp-server                 # your checkout
CONTRACT=crates/ai/src/plugins/contract       # from the warp repo root
REF=$(python3 -c "import json;print(json.load(open('$CONTRACT/PROVENANCE.json'))['upstream_commit'])")

# 1. Is the pinned commit still the tip of the upstream branch?
git -C "$WARP_SERVER" log --oneline -1 "$REF"

# 2. Do the copies match that commit byte for byte? No output means current.
git -C "$WARP_SERVER" show "$REF:logic/ai/ambient_agents/workers/common/testdata/factory_plugin_runtime_contract.json" \
  | diff - "$CONTRACT/factory_plugin_runtime_contract.json"
git -C "$WARP_SERVER" archive "$REF" logic/factoryfile/testdata/agentplugins-conformance \
  | tar -xO --strip-components=4 > /dev/null   # confirms the path still exists upstream
for f in $(git -C "$WARP_SERVER" ls-tree -r --name-only "$REF" -- logic/factoryfile/testdata/agentplugins-conformance); do
  rel="agentplugins-conformance/${f#logic/factoryfile/testdata/agentplugins-conformance/}"
  [ "$(basename "$rel")" = "README.md" ] && continue   # see below
  git -C "$WARP_SERVER" show "$REF:$f" | diff -q - "$CONTRACT/$rel" || echo "STALE: $rel"
done
```

## Updating

Change the file in warp-server first. Then re-copy it here verbatim, regenerate `PROVENANCE.json`
with the new upstream commit and hashes, and update this side's behavior — all in the same change.
If the two copies differ, warp-server wins.

## The one deliberate exception

`agentplugins-conformance/README.md` is **not** a verbatim copy and is excluded from
`PROVENANCE.json`. Upstream it opens by calling itself the canonical corpus, which is true there
and false here; copying that sentence in would invite someone to edit the wrong copy, defeating the
whole mechanism. Its replacement carries the same content with corrected provenance framing. Every
other file in this directory is byte-identical to upstream.
