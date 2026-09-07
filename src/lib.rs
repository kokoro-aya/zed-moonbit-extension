#![forbid(unsafe_code)]

mod launch;

use zed_extension_api::{self as zed, settings::LspSettings};

use launch::{build_launch_plan, EnvironmentKeySemantics};

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
        let settings = LspSettings::for_worktree("moonbit", worktree)
            .map_err(|error| format!("Invalid MoonBit LSP settings: {error}"))?;
        let binary = settings.binary.as_ref();
        let (platform, _) = zed::current_platform();
        let environment_key_semantics = match platform {
            zed::Os::Mac | zed::Os::Linux => EnvironmentKeySemantics::CaseSensitive,
            zed::Os::Windows => EnvironmentKeySemantics::AsciiCaseInsensitive,
        };

        build_launch_plan(
            binary.and_then(|value| value.path.clone()),
            worktree.which("moon"),
            binary.and_then(|value| value.arguments.clone()),
            worktree.shell_env(),
            binary.and_then(|value| value.env.clone()),
            environment_key_semantics,
        )
        .map(launch::LaunchPlan::into_command)
    }
}

zed::register_extension!(MoonBitLabExtension);
