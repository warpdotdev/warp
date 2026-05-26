# Warp Control CLI validation summary
Validated SHA: `c4bb6fdc670d667e78041a9318eda7c6778a22a8`
Agent: `drive-auth-execution`
Build: passed (`warp-oss` app and standalone `warpctrl` with `warp_control_cli`)
Counts: 18 pass, 0 fail, 12 skip

## Result
All intentional denial checks for this shard passed. Outside-Warp disabled commands returned `local_control_disabled`; corrected outside-Warp enabled commands for authenticated Drive and execution-underlying actions returned `execution_context_not_allowed`; fake inside-Warp proof and fake API-key placeholder attempts also failed closed with `execution_context_not_allowed`.

## Skips
- Authenticated success paths for Drive list/inspect/workflow run and `input run` were skipped because this environment did not provide app-issued verified Warp-terminal proof material or disposable Drive object/workflow IDs. The current command metadata also restricts these actions to `inside_warp`, so outside-Warp API-key-style attempts cannot succeed in this build.
- Ordinals 007-012 are retained as skipped/superseded harness attempts: the first enabled run preloaded private settings into the wrong Linux OSS preferences path. Corrected evidence is in ordinals 015-020.

## Blockers
None for denial validation. Authenticated success validation requires a real app-issued inside-Warp proof path and disposable Drive resources.
