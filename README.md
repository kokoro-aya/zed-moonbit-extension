# MoonBit Lab for Zed

MoonBit Lab is a local-development Zed language extension for the native
MoonBit toolchain. Cycle 3 Iteration 0 targets MoonBit 0.10.6 and Zed 1.18.1.

The extension remains a thin client: MoonBit owns language semantics and its
module/package/workspace model, while the extension supplies Zed registration,
launch transport, syntax queries, and declarative tasks.

Version `0.3.1` is the post-review candidate after the accepted Cycle 3 editor
surface. Version 0.3.0 is recognized as that usability milestone but was
intentionally skipped as a manifest release. This repository does not yet claim
Zed Registry publication or MoonBit 1.0 compatibility.

## Development installation

MoonBit Lab currently installs as a Zed development extension rather than from
the Registry:

1. Install a current Zed release and the MoonBit toolchain for the host OS.
2. Verify that `moon --version` works in the project environment visible to
   Zed. Restart Zed after changing `PATH`.
3. Clone this repository, run **zed: install dev extension** from the command
   palette, and select the directory containing extension.toml.
4. Open a directory containing `moon.mod`, or a parent workspace containing
   `moon.work`, then open an `.mbt` file.

Zed and MoonBit both provide macOS, Linux, and Windows distributions. The
extension's production component is WASI, uses structured task arguments, and
does not select a Unix-only shell or script. The macOS editor path is
independently tested;
Linux and Windows remain portability targets until their own editor runs are
recorded. Registry installation is also still a non-claim.

If `moon` is intentionally absent from Zed's project `PATH`, configure an
absolute language-server command with the standard Zed setting:

```json
{
  "lsp": {
    "moonbit": {
      "binary": {
        "path": "/absolute/path/to/moon",
        "arguments": ["lsp"]
      }
    }
  }
}
```

That setting controls the LSP only; tasks retain the project-environment
contract described below.

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
