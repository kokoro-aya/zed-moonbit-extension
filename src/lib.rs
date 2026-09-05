#![forbid(unsafe_code)]

use zed_extension_api as zed;

struct MoonBitLabExtension;

impl zed::Extension for MoonBitLabExtension {
    fn new() -> Self {
        Self
    }

    fn language_server_command(
        &mut self,
        _language_server_id: &zed::LanguageServerId,
        worktree: &zed::Worktree,
    ) -> zed::Result<zed::Command> {
        let moon = worktree.which("moon").ok_or_else(|| {
            "MoonBit toolchain not found: `moon` is not available in this worktree's PATH."
                .to_string()
        })?;

        Ok(zed::Command {
            command: moon,
            args: vec!["lsp".to_string()],
            env: worktree.shell_env(),
        })
    }
}

zed::register_extension!(MoonBitLabExtension);
