#![forbid(unsafe_code)]

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use tree_sitter::{Language, Parser, Query};

const QUERY_DIR: &str = "languages/moonbit";
const CASES: [&str; 2] = ["tests/cases/syntax.mbt", "tests/cases/interface.mbti"];
const EXPECTED_QUERY_FILES: [&str; 1] = ["highlights.scm"];

fn main() {
    let strict = std::env::args()
        .skip(1)
        .any(|argument| argument == "--strict");
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
            if strict {
                std::process::exit(1);
            }
        }
    }
}

#[derive(Debug)]
struct AuditReport {
    grammar_revision: &'static str,
    query_files: Vec<PathBuf>,
    parsed_cases: Vec<PathBuf>,
}

fn audit() -> Result<AuditReport, String> {
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
        grammar_revision: "5435c307c6cf2ef0d508a99047b06f35a4308444",
        query_files,
        parsed_cases,
    })
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

    #[test]
    fn pinned_grammar_parses_representative_sources() {
        let report = audit().expect("strict grammar audit");
        assert_eq!(report.parsed_cases.len(), 2);
    }

    #[test]
    fn unknown_highlight_capture_is_rejected() {
        assert!(!capture_allowed("highlights", "module"));
        assert!(capture_allowed("highlights", "type.builtin"));
    }
}
