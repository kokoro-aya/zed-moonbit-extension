#![forbid(unsafe_code)]

#[path = "../lsp_probe.rs"]
mod lsp_probe;

use std::env;
use std::fs;
use std::path::PathBuf;

use lsp_probe::{
    run_all, run_capabilities, run_freshness, run_project_roots, OutputFormat, ProbeOptions,
    ServerRequestPolicy, Suite,
};

fn main() {
    if let Err(error) = run() {
        eprintln!("mbt_lsp_probe: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let options = parse_args(env::args().skip(1))?;
    let evidence = match options.suite {
        Suite::Capabilities => run_capabilities(&options)?,
        Suite::ProjectRoots => run_project_roots(&options)?,
        Suite::Freshness => run_freshness(&options)?,
        Suite::All => run_all(&options)?,
    };

    let rendered = match options.format {
        OutputFormat::Json => serde_json::to_string_pretty(&evidence)
            .map_err(|error| format!("serialize probe evidence: {error}"))?,
        OutputFormat::Text => render_text(&evidence),
    };
    if let Some(output) = &options.output {
        fs::write(output, format!("{rendered}\n"))
            .map_err(|error| format!("write evidence {}: {error}", output.display()))?;
    } else {
        println!("{rendered}");
    }
    Ok(())
}

fn parse_args(args: impl Iterator<Item = String>) -> Result<ProbeOptions, String> {
    let mut suite = None;
    let mut moon = None;
    let mut format = OutputFormat::Text;
    let mut output = None;
    let mut server_request_policy = ServerRequestPolicy::Strict;
    let mut args = args.peekable();

    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--suite" => {
                let value = args
                    .next()
                    .ok_or_else(|| "--suite requires a value".to_string())?;
                suite = Some(Suite::parse(&value)?);
            }
            "--moon" => {
                moon =
                    Some(PathBuf::from(args.next().ok_or_else(|| {
                        "--moon requires an absolute path".to_string()
                    })?));
            }
            "--format" => {
                let value = args
                    .next()
                    .ok_or_else(|| "--format requires text or json".to_string())?;
                format = OutputFormat::parse(&value)?;
            }
            "--output" => {
                output =
                    Some(PathBuf::from(args.next().ok_or_else(|| {
                        "--output requires a file path".to_string()
                    })?));
            }
            "--server-requests" => {
                let value = args.next().ok_or_else(|| {
                    "--server-requests requires strict or exploratory".to_string()
                })?;
                server_request_policy = ServerRequestPolicy::parse(&value)?;
            }
            "-h" | "--help" => {
                println!(concat!(
                    "mbt_lsp_probe --suite capabilities|project-roots|freshness|all ",
                    "--moon <absolute-path> --format text|json [--output <path>] ",
                    "[--server-requests strict|exploratory]"
                ));
                std::process::exit(0);
            }
            _ => return Err(format!("unknown argument: {argument}")),
        }
    }

    let moon = moon.ok_or_else(|| "--moon is required".to_string())?;
    if !moon.is_absolute() {
        return Err("--moon must be an absolute path".to_string());
    }

    Ok(ProbeOptions {
        suite: suite.ok_or_else(|| "--suite is required".to_string())?,
        moon,
        format,
        output,
        server_request_policy,
    })
}

fn render_text(evidence: &serde_json::Value) -> String {
    let mut lines = vec![
        "MoonBit LSP probe".to_string(),
        format!("suite: {}", evidence["suite"]),
        format!("binary: {}", evidence["binary"]["path"]),
        format!("version: {}", evidence["binary"]["version"]),
    ];
    if evidence["suite"] == "all" {
        lines.push(format!(
            "completed suites: {}",
            evidence["results"].as_array().map_or(0, Vec::len)
        ));
    } else {
        lines.push(format!("root URI: {}", evidence["session"]["root_uri"]));
        lines.push(format!("stderr: {}", evidence["session"]["stderr"]["text"]));
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_requires_an_absolute_binary() {
        let error = parse_args(
            ["--suite", "capabilities", "--moon", "moon"]
                .into_iter()
                .map(str::to_string),
        )
        .expect_err("relative moon path");

        assert!(error.contains("absolute"));
    }

    #[test]
    fn cli_defaults_to_strict_server_requests() {
        let options = parse_args(
            ["--suite", "capabilities", "--moon", "/toolchains/moon"]
                .into_iter()
                .map(str::to_string),
        )
        .expect("strict defaults");

        assert_eq!(options.server_request_policy, ServerRequestPolicy::Strict);
    }

    #[test]
    fn cli_can_select_exploratory_server_requests() {
        let options = parse_args(
            [
                "--suite",
                "capabilities",
                "--moon",
                "/toolchains/moon",
                "--server-requests",
                "exploratory",
            ]
            .into_iter()
            .map(str::to_string),
        )
        .expect("exploratory policy");

        assert_eq!(
            options.server_request_policy,
            ServerRequestPolicy::Exploratory
        );
    }
}
