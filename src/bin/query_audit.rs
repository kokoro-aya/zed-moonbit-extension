#![forbid(unsafe_code)]

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use tree_sitter::{Language, Parser, Query};

const QUERY_DIR: &str = "languages/moonbit";
const CASES: [&str; 3] = [
    "tests/cases/syntax.mbt",
    "tests/cases/interface.mbti",
    "tests/cases/runnables.mbt",
];
const EXPECTED_QUERY_FILES: [&str; 5] = [
    "brackets.scm",
    "highlights.scm",
    "indents.scm",
    "outline.scm",
    "runnables.scm",
];

fn main() {
    match audit() {
        Ok(report) => {
            println!("grammar_revision={}", report.grammar_revision);
            println!("query_files={}", report.query_files.len());
            println!("parsed_cases={}", report.parsed_cases.len());
            for path in report.query_files {
                println!("query={}", path.display());
            }
            for path in report.parsed_cases {
                println!("case={}", path.display());
            }
        }
        Err(error) => {
            eprintln!("query audit failed: {error}");
            std::process::exit(1);
        }
    }
}

#[derive(Debug)]
struct AuditReport {
    grammar_revision: String,
    query_files: Vec<PathBuf>,
    parsed_cases: Vec<PathBuf>,
}

fn audit() -> Result<AuditReport, String> {
    let pins = validate_manifest_consistency(
        &read(Path::new("Cargo.toml"))?,
        &read(Path::new("extension.toml"))?,
    )?;
    let language: Language = tree_sitter_moonbit::LANGUAGE.into();
    let query_files = collect_query_files(Path::new(QUERY_DIR))?;
    validate_inventory(&query_files)?;

    for path in &query_files {
        let source = read(path)?;
        let query = Query::new(&language, &source)
            .map_err(|error| format!("{} does not compile: {error}", path.display()))?;
        validate_captures(path, &query)?;
    }

    let mut parser = Parser::new();
    parser
        .set_language(&language)
        .map_err(|error| format!("failed to load pinned MoonBit grammar: {error}"))?;

    let mut parsed_cases = Vec::new();
    for case in CASES {
        let path = PathBuf::from(case);
        let source = read(&path)?;
        let tree = parser
            .parse(&source, None)
            .ok_or_else(|| format!("parser returned no tree for {}", path.display()))?;
        if tree.root_node().has_error() {
            return Err(format!(
                "{} contains a Tree-sitter ERROR node: {}",
                path.display(),
                tree.root_node().to_sexp()
            ));
        }
        parsed_cases.push(path);
    }

    Ok(AuditReport {
        grammar_revision: pins.grammar_revision,
        query_files,
        parsed_cases,
    })
}

#[derive(Debug, PartialEq, Eq)]
struct ManifestPins {
    grammar_revision: String,
}

fn validate_manifest_consistency(
    cargo_source: &str,
    extension_source: &str,
) -> Result<ManifestPins, String> {
    let cargo: toml::Value = toml::from_str(cargo_source)
        .map_err(|error| format!("Cargo.toml is not valid TOML: {error}"))?;
    let extension: toml::Value = toml::from_str(extension_source)
        .map_err(|error| format!("extension.toml is not valid TOML: {error}"))?;

    let cargo_version = manifest_string(&cargo, &["package", "version"], "Cargo.toml")?;
    let extension_version = manifest_string(&extension, &["version"], "extension.toml")?;
    require_equal_pin(
        "extension release version",
        "Cargo.toml package.version",
        cargo_version,
        "extension.toml version",
        extension_version,
    )?;

    let api_version =
        manifest_string(&cargo, &["dependencies", "zed_extension_api"], "Cargo.toml")?;
    let wasm_api_version = manifest_string(&extension, &["lib", "version"], "extension.toml")?;
    require_equal_pin(
        "Zed extension API version",
        "Cargo.toml dependencies.zed_extension_api",
        api_version,
        "extension.toml lib.version",
        wasm_api_version,
    )?;

    let grammar_revision = manifest_string(
        &cargo,
        &["dependencies", "tree-sitter-moonbit", "rev"],
        "Cargo.toml",
    )?;
    let extension_grammar_revision = manifest_string(
        &extension,
        &["grammars", "moonbit", "rev"],
        "extension.toml",
    )?;
    require_equal_pin(
        "MoonBit grammar revision",
        "Cargo.toml dependencies.tree-sitter-moonbit.rev",
        grammar_revision,
        "extension.toml grammars.moonbit.rev",
        extension_grammar_revision,
    )?;

    Ok(ManifestPins {
        grammar_revision: grammar_revision.to_string(),
    })
}

fn manifest_string<'a>(
    manifest: &'a toml::Value,
    path: &[&str],
    manifest_name: &str,
) -> Result<&'a str, String> {
    let mut value = manifest;
    for segment in path {
        value = value
            .get(*segment)
            .ok_or_else(|| format!("{manifest_name} is missing required pin {}", path.join(".")))?;
    }
    value.as_str().ok_or_else(|| {
        format!(
            "{manifest_name} pin {} must be a string, found {value}",
            path.join(".")
        )
    })
}

fn require_equal_pin(
    pin_name: &str,
    left_name: &str,
    left: &str,
    right_name: &str,
    right: &str,
) -> Result<(), String> {
    if left == right {
        Ok(())
    } else {
        Err(format!(
            "{pin_name} mismatch: {left_name}={left:?}, {right_name}={right:?}"
        ))
    }
}

fn collect_query_files(directory: &Path) -> Result<Vec<PathBuf>, String> {
    let mut files = Vec::new();
    for entry in fs::read_dir(directory)
        .map_err(|error| format!("failed to read {}: {error}", directory.display()))?
    {
        let path = entry
            .map_err(|error| format!("failed to read query entry: {error}"))?
            .path();
        if path.extension().and_then(|value| value.to_str()) == Some("scm") {
            files.push(path);
        }
    }
    files.sort();
    Ok(files)
}

fn validate_inventory(files: &[PathBuf]) -> Result<(), String> {
    let actual: BTreeSet<_> = files
        .iter()
        .filter_map(|path| path.file_name().and_then(|value| value.to_str()))
        .collect();
    let expected: BTreeSet<_> = EXPECTED_QUERY_FILES.into_iter().collect();
    if actual != expected {
        return Err(format!(
            "query inventory differs: expected {expected:?}, found {actual:?}"
        ));
    }
    Ok(())
}

fn validate_captures(path: &Path, query: &Query) -> Result<(), String> {
    let family = path
        .file_stem()
        .and_then(|value| value.to_str())
        .ok_or_else(|| format!("invalid query path {}", path.display()))?;
    let invalid: Vec<_> = query
        .capture_names()
        .iter()
        .filter(|capture| !capture_allowed(family, capture))
        .collect();
    if invalid.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "{} uses unsupported Zed captures: {invalid:?}",
            path.display()
        ))
    }
}

fn capture_allowed(family: &str, capture: &str) -> bool {
    match family {
        "highlights" => matches!(
            capture,
            "attribute"
                | "boolean"
                | "comment"
                | "comment.doc"
                | "constant"
                | "constant.builtin"
                | "constructor"
                | "embedded"
                | "function"
                | "keyword"
                | "label"
                | "number"
                | "operator"
                | "property"
                | "punctuation.bracket"
                | "punctuation.delimiter"
                | "punctuation.special"
                | "string"
                | "string.escape"
                | "string.regex"
                | "string.special"
                | "type"
                | "type.builtin"
                | "variable"
                | "variable.parameter"
                | "variable.special"
                | "variant"
        ),
        "brackets" => matches!(capture, "open" | "close"),
        "indents" => matches!(capture, "indent" | "end"),
        "outline" => matches!(capture, "name" | "item" | "context" | "context.extra"),
        "runnables" => capture == "run" || capture.starts_with('_'),
        _ => false,
    }
}

fn read(path: &Path) -> Result<String, String> {
    fs::read_to_string(path).map_err(|error| format!("failed to read {}: {error}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;
    use tree_sitter::{QueryCursor, StreamingIterator};

    #[test]
    fn pinned_grammar_parses_representative_sources() {
        let report = audit().expect("strict grammar audit");
        assert_eq!(report.parsed_cases.len(), 3);
    }

    #[test]
    fn actual_manifests_keep_release_api_and_grammar_pins_equal() {
        validate_manifest_consistency(
            include_str!("../../Cargo.toml"),
            include_str!("../../extension.toml"),
        )
        .expect("manifest pins");
    }

    #[test]
    fn release_version_mismatch_names_both_manifest_fields() {
        let extension = include_str!("../../extension.toml").replacen(
            "version = \"0.2.9\"",
            "version = \"9.9.9\"",
            1,
        );
        let error = validate_manifest_consistency(include_str!("../../Cargo.toml"), &extension)
            .expect_err("release mismatch");

        assert!(error.contains("extension release version mismatch"));
        assert!(error.contains("Cargo.toml package.version"));
        assert!(error.contains("extension.toml version"));
    }

    #[test]
    fn api_version_mismatch_names_both_manifest_fields() {
        let extension = include_str!("../../extension.toml").replacen(
            "version = \"0.7.0\"",
            "version = \"9.9.9\"",
            1,
        );
        let error = validate_manifest_consistency(include_str!("../../Cargo.toml"), &extension)
            .expect_err("API mismatch");

        assert!(error.contains("Zed extension API version mismatch"));
        assert!(error.contains("dependencies.zed_extension_api"));
        assert!(error.contains("lib.version"));
    }

    #[test]
    fn grammar_revision_mismatch_names_both_manifest_fields() {
        let extension = include_str!("../../extension.toml").replacen(
            "5435c307c6cf2ef0d508a99047b06f35a4308444",
            "deadbeef",
            1,
        );
        let error = validate_manifest_consistency(include_str!("../../Cargo.toml"), &extension)
            .expect_err("grammar mismatch");

        assert!(error.contains("MoonBit grammar revision mismatch"));
        assert!(error.contains("dependencies.tree-sitter-moonbit.rev"));
        assert!(error.contains("grammars.moonbit.rev"));
    }

    #[test]
    fn unknown_highlight_capture_is_rejected() {
        assert!(!capture_allowed("highlights", "module"));
        assert!(capture_allowed("highlights", "type.builtin"));
    }

    #[test]
    fn outline_exposes_top_level_declarations() {
        let language: Language = tree_sitter_moonbit::LANGUAGE.into();
        let source = read(Path::new("tests/cases/syntax.mbt")).expect("syntax case");
        let query_source = read(Path::new("languages/moonbit/outline.scm")).expect("outline query");
        let query = Query::new(&language, &query_source).expect("compiled outline");
        let mut parser = Parser::new();
        parser.set_language(&language).expect("MoonBit grammar");
        let tree = parser.parse(&source, None).expect("syntax tree");
        let name_index = query
            .capture_index_for_name("name")
            .expect("outline name capture");
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&query, tree.root_node(), source.as_bytes());
        let mut names = BTreeSet::new();
        while let Some(query_match) = matches.next() {
            for capture in query_match.captures {
                if capture.index == name_index {
                    names.insert(
                        capture
                            .node
                            .utf8_text(source.as_bytes())
                            .expect("UTF-8 capture")
                            .to_string(),
                    );
                }
            }
        }

        assert!(names.contains("Point"));
        assert!(names.contains("Shape"));
        assert!(names.contains("measure"));
        assert!(names.contains("\"measure a point\""));
    }

    #[test]
    fn tasks_use_structured_non_mutating_commands() {
        let tasks: Value = serde_json::from_str(include_str!("../../languages/moonbit/tasks.json"))
            .expect("tasks JSON");
        let tasks = tasks.as_array().expect("task inventory");

        assert_eq!(tasks.len(), 5);
        assert!(tasks.iter().all(|task| task["command"] == "moon"));
        assert!(tasks.iter().all(|task| task["args"].is_array()));
        assert!(tasks.iter().all(|task| task.get("cwd").is_none()));
        let format = tasks
            .iter()
            .find(|task| task["label"] == "MoonBit: check formatting")
            .expect("format-check task");
        assert_eq!(format["args"], serde_json::json!(["fmt", "--check"]));
    }

    #[test]
    fn runnables_are_top_level_main_and_tests_only() {
        let language: Language = tree_sitter_moonbit::LANGUAGE.into();
        let source = read(Path::new("tests/cases/runnables.mbt")).expect("runnable case");
        let query_source =
            read(Path::new("languages/moonbit/runnables.scm")).expect("runnable query");
        let query = Query::new(&language, &query_source).expect("compiled runnables");
        let mut parser = Parser::new();
        parser.set_language(&language).expect("MoonBit grammar");
        let tree = parser.parse(&source, None).expect("runnable syntax tree");
        let run_index = query.capture_index_for_name("run").expect("run capture");
        let mut cursor = QueryCursor::new();
        let mut matches = cursor.matches(&query, tree.root_node(), source.as_bytes());
        let mut captures = Vec::new();
        while let Some(query_match) = matches.next() {
            let tag = query
                .property_settings(query_match.pattern_index)
                .iter()
                .find(|property| property.key.as_ref() == "tag")
                .and_then(|property| property.value.as_deref())
                .expect("runnable tag");
            for capture in query_match.captures {
                if capture.index == run_index {
                    captures.push((
                        capture
                            .node
                            .utf8_text(source.as_bytes())
                            .expect("UTF-8 runnable")
                            .to_string(),
                        capture.node.start_position().row,
                        tag.to_string(),
                    ));
                }
            }
        }

        assert_eq!(captures.len(), 2);
        assert!(captures
            .iter()
            .any(|(text, row, tag)| text == "main" && *row == 14 && tag == "moon-run"));
        assert!(captures
            .iter()
            .any(|(text, row, tag)| text == "test" && *row == 19 && tag == "moon-test"));
    }
}
