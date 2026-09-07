# MoonBit Lab for Zed

MoonBit Lab is a local-development Zed language extension for the native
MoonBit toolchain. Cycle 3 Iteration 0 targets MoonBit 0.10.6 and Zed 1.18.1.

The extension remains a thin client: MoonBit owns language semantics and its
module/package/workspace model, while the extension supplies Zed registration,
launch transport, syntax queries, and declarative tasks.

Version `0.2.9` is the first Cycle 3 iteration. This repository does not yet
claim Zed Registry publication or MoonBit 1.0 compatibility.

## Toolchain and task roots

The LSP and tasks have deliberately separate toolchain contracts. The standard
`lsp.moonbit.binary` setting selects the language-server command and
environment. Declarative tasks run `moon` from Zed's project environment and
therefore use its login-shell `PATH`; configure those two environments
consistently when a project requires one exact MoonBit toolchain.

“Current project” tasks start in the current file's directory and let Moon
discover the nearest `moon.mod` or `moon.work`. “Worktree root” tasks start at
Zed's root and are intended for a root-level `moon.work` or `moon.mod`; they
truthfully fail when that root is not a MoonBit project. Test runnables pass the
captured declaration name to `moon test --filter`, so selecting one runnable
does not silently execute every test in the file.
