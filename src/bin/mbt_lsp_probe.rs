#![forbid(unsafe_code)]

#[path = "../lsp_probe.rs"]
mod lsp_probe;

use std::env;
use std::path::PathBuf;

use lsp_probe::{
    run_capabilities, run_freshness, run_project_roots, OutputFormat, ProbeOptions, Suite,
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
        Suite::All => return Err("all suite is incomplete until every scenario exists".to_string()),
    };

    match options.format {
        OutputFormat::Json => println!(
            "{}",
            serde_json::to_string_pretty(&evidence)
                .map_err(|error| format!("serialize probe evidence: {error}"))?
        ),
        OutputFormat::Text => print_text(&evidence),
    }
    Ok(())
}

fn parse_args(args: impl Iterator<Item = String>) -> Result<ProbeOptions, String> {
    let mut suite = None;
    let mut moon = None;
    let mut format = OutputFormat::Text;
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
            "-h" | "--help" => {
                println!(
                    "mbt_lsp_probe --suite capabilities|project-roots|freshness|all \\\n+  --moon <absolute-path> --format text|json"
                );
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
    })
}

fn print_text(evidence: &serde_json::Value) {
    println!("MoonBit LSP probe");
    println!("suite: {}", evidence["suite"]);
    println!("binary: {}", evidence["binary"]["path"]);
    println!("version: {}", evidence["binary"]["version"]);
    println!("cwd: {}", evidence["session"]["cwd"]);
    println!("root URI: {}", evidence["session"]["root_uri"]);
    println!(
        "initialize capabilities: {}",
        evidence["result"]["capabilities"]
    );
    println!("stderr: {}", evidence["session"]["stderr"]["text"]);
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
}
