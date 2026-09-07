use std::collections::{BTreeMap, HashMap};

use zed_extension_api::{self as zed, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CommandOrigin {
    Configured,
    WorktreePath,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EnvironmentOrigin {
    WorktreeShell,
    Configured,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EnvironmentKeySemantics {
    CaseSensitive,
    AsciiCaseInsensitive,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct EnvironmentEntry {
    key: String,
    value: String,
    origin: EnvironmentOrigin,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LaunchPlan {
    command: String,
    command_origin: CommandOrigin,
    args: Vec<String>,
    environment: Vec<EnvironmentEntry>,
}

impl LaunchPlan {
    pub(crate) fn into_command(self) -> zed::Command {
        let Self {
            command,
            command_origin,
            args,
            environment,
        } = self;
        let _ = command_origin;
        let env = environment
            .into_iter()
            .map(|entry| {
                let _ = entry.origin;
                (entry.key, entry.value)
            })
            .collect();

        zed::Command { command, args, env }
    }
}

pub(crate) fn build_launch_plan(
    configured_path: Option<String>,
    path_moon: Option<String>,
    configured_args: Option<Vec<String>>,
    shell_env: Vec<(String, String)>,
    configured_env: Option<HashMap<String, String>>,
    environment_key_semantics: EnvironmentKeySemantics,
) -> Result<LaunchPlan> {
    let (command, command_origin) = match configured_path {
        Some(path) if path.trim().is_empty() => {
            return Err("MoonBit LSP binary.path must not be empty.".to_string());
        }
        Some(path) => (path, CommandOrigin::Configured),
        None => (
            path_moon.ok_or_else(|| {
                "MoonBit toolchain not found: `moon` is not available in this worktree's PATH."
                    .to_string()
            })?,
            CommandOrigin::WorktreePath,
        ),
    };

    let args = configured_args.unwrap_or_else(|| vec!["lsp".to_string()]);
    validate_arguments(&args)?;

    Ok(LaunchPlan {
        command,
        command_origin,
        args,
        environment: merge_environment(shell_env, configured_env, environment_key_semantics),
    })
}

fn validate_arguments(args: &[String]) -> Result<()> {
    if args.first().map(String::as_str) != Some("lsp") {
        return Err(
            "MoonBit LSP binary.arguments is the final Zed argument vector and must start with `lsp`."
                .to_string(),
        );
    }

    if args
        .iter()
        .filter(|argument| argument.as_str() == "lsp")
        .count()
        != 1
    {
        return Err(
            "MoonBit LSP binary.arguments must contain the `lsp` subcommand exactly once."
                .to_string(),
        );
    }

    Ok(())
}

fn merge_environment(
    shell_env: Vec<(String, String)>,
    configured_env: Option<HashMap<String, String>>,
    key_semantics: EnvironmentKeySemantics,
) -> Vec<EnvironmentEntry> {
    let mut entries = BTreeMap::new();
    for (key, value) in shell_env {
        entries.insert(
            normalized_environment_key(&key, key_semantics),
            (key, value, EnvironmentOrigin::WorktreeShell),
        );
    }
    for (key, value) in configured_env.unwrap_or_default() {
        entries.insert(
            normalized_environment_key(&key, key_semantics),
            (key, value, EnvironmentOrigin::Configured),
        );
    }

    entries
        .into_iter()
        .map(|(_, (key, value, origin))| EnvironmentEntry { key, value, origin })
        .collect()
}

fn normalized_environment_key(key: &str, semantics: EnvironmentKeySemantics) -> String {
    match semantics {
        EnvironmentKeySemantics::CaseSensitive => key.to_string(),
        EnvironmentKeySemantics::AsciiCaseInsensitive => key.to_ascii_uppercase(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unix_environment() -> EnvironmentKeySemantics {
        EnvironmentKeySemantics::CaseSensitive
    }

    #[test]
    fn defaults_to_worktree_moon_and_lsp_subcommand() {
        let plan = build_launch_plan(
            None,
            Some("/toolchains/moon".to_string()),
            None,
            vec![],
            None,
            unix_environment(),
        )
        .expect("default plan");

        assert_eq!(plan.command, "/toolchains/moon");
        assert_eq!(plan.command_origin, CommandOrigin::WorktreePath);
        assert_eq!(plan.args, ["lsp"]);
    }

    #[test]
    fn configured_command_wins_over_path() {
        let plan = build_launch_plan(
            Some("/configured/moon".to_string()),
            Some("/path/moon".to_string()),
            None,
            vec![],
            None,
            unix_environment(),
        )
        .expect("configured plan");

        assert_eq!(plan.command, "/configured/moon");
        assert_eq!(plan.command_origin, CommandOrigin::Configured);
    }

    #[test]
    fn empty_configured_command_is_not_fallback_permission() {
        let error = build_launch_plan(
            Some("  ".to_string()),
            Some("/path/moon".to_string()),
            None,
            vec![],
            None,
            unix_environment(),
        )
        .expect_err("empty command should fail");

        assert!(error.contains("must not be empty"));
    }

    #[test]
    fn missing_command_reports_the_worktree_lookup() {
        let error = build_launch_plan(None, None, None, vec![], None, unix_environment())
            .expect_err("missing command should fail");

        assert!(error.contains("worktree's PATH"));
    }

    #[test]
    fn configured_arguments_are_preserved_as_the_final_vector() {
        let plan = build_launch_plan(
            None,
            Some("moon".to_string()),
            Some(vec!["lsp".to_string(), "--trace".to_string()]),
            vec![],
            None,
            unix_environment(),
        )
        .expect("configured arguments");

        assert_eq!(plan.args, ["lsp", "--trace"]);
    }

    #[test]
    fn arguments_must_start_with_lsp() {
        let error = build_launch_plan(
            None,
            Some("moon".to_string()),
            Some(vec!["--trace".to_string()]),
            vec![],
            None,
            unix_environment(),
        )
        .expect_err("missing subcommand should fail");

        assert!(error.contains("must start with `lsp`"));
    }

    #[test]
    fn arguments_reject_a_duplicate_lsp_subcommand() {
        let error = build_launch_plan(
            None,
            Some("moon".to_string()),
            Some(vec!["lsp".to_string(), "lsp".to_string()]),
            vec![],
            None,
            unix_environment(),
        )
        .expect_err("duplicate subcommand should fail");

        assert!(error.contains("exactly once"));
    }

    #[test]
    fn configured_environment_overrides_shell_with_origin() {
        let plan = build_launch_plan(
            None,
            Some("moon".to_string()),
            None,
            vec![
                ("PATH".to_string(), "/shell".to_string()),
                ("MOON_HOME".to_string(), "/shell/moon".to_string()),
            ],
            Some(HashMap::from([
                ("MOON_HOME".to_string(), "/configured/moon".to_string()),
                ("MOON_WORK".to_string(), "off".to_string()),
            ])),
            unix_environment(),
        )
        .expect("merged environment");

        assert_eq!(
            plan.environment,
            vec![
                EnvironmentEntry {
                    key: "MOON_HOME".to_string(),
                    value: "/configured/moon".to_string(),
                    origin: EnvironmentOrigin::Configured,
                },
                EnvironmentEntry {
                    key: "MOON_WORK".to_string(),
                    value: "off".to_string(),
                    origin: EnvironmentOrigin::Configured,
                },
                EnvironmentEntry {
                    key: "PATH".to_string(),
                    value: "/shell".to_string(),
                    origin: EnvironmentOrigin::WorktreeShell,
                },
            ]
        );
    }

    #[test]
    fn windows_environment_keys_are_case_insensitive() {
        let plan = build_launch_plan(
            None,
            Some("moon.exe".to_string()),
            None,
            vec![("PATH".to_string(), r"C:\shell".to_string())],
            Some(HashMap::from([(
                "Path".to_string(),
                r"C:\configured".to_string(),
            )])),
            EnvironmentKeySemantics::AsciiCaseInsensitive,
        )
        .expect("Windows environment");

        assert_eq!(
            plan.environment,
            vec![EnvironmentEntry {
                key: "Path".to_string(),
                value: r"C:\configured".to_string(),
                origin: EnvironmentOrigin::Configured,
            }]
        );
    }

    #[test]
    fn unix_environment_keys_remain_case_sensitive() {
        let plan = build_launch_plan(
            None,
            Some("moon".to_string()),
            None,
            vec![("PATH".to_string(), "/shell".to_string())],
            Some(HashMap::from([(
                "Path".to_string(),
                "/configured".to_string(),
            )])),
            EnvironmentKeySemantics::CaseSensitive,
        )
        .expect("Unix environment");

        assert_eq!(plan.environment.len(), 2);
        assert!(plan.environment.iter().any(|entry| {
            entry.key == "PATH" && entry.origin == EnvironmentOrigin::WorktreeShell
        }));
        assert!(plan
            .environment
            .iter()
            .any(|entry| { entry.key == "Path" && entry.origin == EnvironmentOrigin::Configured }));
    }
}
