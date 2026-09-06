# MoonBit Zed Extension — Cycle 3 Iteration 0 result

Status: graduated  
Version: `0.2.9`  
Branch: `codex/zed-mbt-extension`  
Independent acceptance: passed on 2026-09-06

## Result

Cycle 3 Iteration 0 established a production-shaped MoonBit extension from an
empty component directory. The resulting extension provides the complete
editor loop targeted by the iteration:

- pinned Tree-sitter highlighting, brackets, indentation, Outline, and
  runnable discovery for `.mbt` and `.mbti`;
- native `moon lsp` completion, hover, definition, references, rename,
  formatting, and incremental diagnostics;
- a validated launch plan with explicit command, argument, environment, and
  provenance semantics;
- independent library and application modules plus a canonical parent
  `moon.work` workspace;
- workspace-safe check, test, run, and formatting tasks;
- main and test gutter runnables;
- repeatable strict-query and native-LSP evidence kept outside the production
  WASM dependency graph.

The decisive engineering fact is that direct-module and parent-workspace Zed
sessions are semantically equivalent for the tested modules. Identically named
packages and symbols retain module-local definition/reference identity, and a
Zed worktree above both module roots does not introduce false diagnostics.

## Evidence chain

The graduation claim rests on all six validation authorities defined by the
iteration workflow:

1. Both fixtures independently pass format checking, warning-denying checks,
   tests, and golden-output runs; their parent `moon.work` passes workspace
   format/check/test.
2. Rust unit tests, Clippy with warnings denied, and the strict pinned-grammar
   audit pass.
3. `evidence/lsp-all.json` records capabilities, direct/parent diagnostics and
   navigation isolation, and unsaved diagnostic freshness against
   `/Users/irony/.moon/bin/moon`.
4. The release `wasm32-wasip2` extension component builds successfully.
5. `zed-evidence.md` records real Zed 1.18.1 installation, launch provenance,
   semantic UI behavior, tasks, runnables, scratch rename/formatting, and
   direct/parent equivalence.
6. The user independently replayed the acceptance checklist and reported that
   every test passed.

This ordering matters: protocol observations did not stand in for the editor,
and the agent-controlled editor pass did not stand in for independent use.

## Architectural reading

The extension reached this feature surface without parsing MoonBit semantics,
resolving packages, routing multiple language servers, or inventing project
identity. The full editor experience is produced by a clean authority chain:

```text
moon.mod / moon.work
        -> MoonBit compiler and native LSP
        -> provenance-preserving extension launch
        -> Zed worktree, buffers, tasks, and UI
```

This supports the iteration's thin-client thesis. The extension is substantial
at the editor boundary while remaining deliberately small at the language and
project-semantics boundaries.

## Non-claims

Graduation does not claim Zed Registry publication, cross-platform validation,
VS Code Code Lens parity, debugger or backend selection, `.mbt.md`, dedicated
manifest language modes, package distribution, automatic toolchain management,
or MoonBit 1.0 compatibility. Those remain outside Cy3 It0 rather than hidden
partial features.
