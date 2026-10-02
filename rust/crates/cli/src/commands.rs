//! `refactrail-native lint` and `refactrail-native scope`: the same
//! options, findings and documents as `refactrail lint --no-cache` and
//! `refactrail scope`, without Python.

use crate::json::{dumps, sha256_hex, Json};
use crate::{output, split_codes};
use rayon::prelude::*;
use refactrail_engine::lexical::LexicalReport;
use refactrail_engine::lint::{lint_file, lint_settings, syntax_error_text};
use refactrail_engine::Row;
use std::path::Path;

pub const LINT_USAGE: &str = "usage: refactrail-native lint [paths ...] [--select SELECT] [--ignore IGNORE] [--jobs JOBS]
                               [--no-cache] [--output-format {text,json,sarif}]";
pub const SCOPE_USAGE: &str = "usage: refactrail-native scope path";

struct LintOptions {
    paths: Vec<String>,
    select: Vec<String>,
    ignore: Vec<String>,
    jobs: usize,
    output: String,
}

fn parse_lint_options(arguments: &[String]) -> Result<LintOptions, String> {
    let mut options =
        LintOptions { paths: Vec::new(), select: split_codes("RC"), ignore: Vec::new(), jobs: 1, output: "text".into() };
    let mut index = 0;
    while index < arguments.len() {
        let argument = arguments[index].as_str();
        let (flag, inline) = match argument.split_once('=') {
            Some((flag, value)) if flag.starts_with("--") => (flag, Some(value.to_string())),
            _ => (argument, None),
        };
        let mut value = || -> Result<String, String> {
            if let Some(value) = inline.clone() {
                return Ok(value);
            }
            index += 1;
            arguments.get(index).cloned().ok_or_else(|| format!("argument {flag}: expected one argument"))
        };
        match flag {
            "--select" => options.select = split_codes(&value()?),
            "--ignore" => options.ignore = split_codes(&value()?),
            "--jobs" => {
                let text = value()?;
                let jobs: i64 = text.parse().map_err(|_| format!("argument --jobs: invalid int value: '{text}'"))?;
                if jobs < 0 {
                    return Err("jobs must be a nonnegative whole number".into());
                }
                options.jobs = jobs as usize;
            }
            "--output-format" => {
                let format = value()?;
                if !["text", "json", "sarif"].contains(&format.as_str()) {
                    return Err(format!("argument --output-format: invalid choice: '{format}'"));
                }
                options.output = format;
            }
            "--no-cache" => {}
            _ if argument.starts_with("--") => return Err(format!("unrecognized arguments: {argument}")),
            _ => options.paths.push(argument.to_string()),
        }
        index += 1;
    }
    if options.paths.is_empty() {
        options.paths.push(".".into());
    }
    Ok(options)
}

/// `lint`: exit 1 when there are findings.
pub fn run_lint(arguments: &[String]) -> Result<i32, String> {
    let options = parse_lint_options(arguments)?;
    let files = crate::format_cmd::discover_general(&options.paths, false)?;
    if files.iter().any(|path| path.ends_with(".ipynb")) {
        return Err("Notebook linting needs kernel execution context; use format for notebooks or lint exported Python".into());
    }
    let lint = lint_settings(&options.select, &options.ignore);
    let check = || -> Result<Vec<Row>, String> {
        let results: Vec<Result<Vec<Row>, String>> = files
            .par_iter()
            .map(|path| match std::fs::read(path) {
                Ok(raw) => Ok(lint_file(path, &raw, &lint)),
                Err(error) => Err(format!("{path}: {error}")),
            })
            .collect();
        let mut rows = Vec::new();
        for result in results {
            rows.extend(result?);
        }
        rows.sort();
        Ok(rows)
    };
    let rows = crate::pool(options.jobs)?.install(check)?;
    let report = match options.output.as_str() {
        "json" => output::json(&rows),
        "sarif" => output::sarif(&rows, crate::VERSION),
        _ => output::text(&rows, files.len()),
    };
    print!("{report}");
    Ok(i32::from(!rows.is_empty()))
}

fn lexical_fields(report: &LexicalReport) -> Vec<(String, Json)> {
    let reads = report
        .reads
        .iter()
        .map(|read| {
            Json::Object(vec![
                ("name".into(), Json::Str(read.name.clone())),
                ("line".into(), Json::Int(read.line as i64)),
                ("column".into(), Json::Int(read.column as i64)),
                ("scope".into(), Json::Int(read.scope as i64)),
                ("resolution".into(), Json::Str(read.resolution.into())),
                ("annotation".into(), Json::Bool(read.annotation)),
            ])
        })
        .collect();
    let imports = report
        .imports
        .iter()
        .map(|import| {
            Json::Object(vec![
                ("name".into(), Json::Str(import.name.clone())),
                ("line".into(), Json::Int(import.line as i64)),
                ("column".into(), Json::Int(import.column as i64)),
                ("used".into(), Json::Bool(import.used)),
                ("exempt".into(), Json::Bool(import.exempt)),
            ])
        })
        .collect();
    let scopes = report
        .scopes
        .iter()
        .map(|scope| {
            Json::Object(vec![
                ("id".into(), Json::Int(scope.id as i64)),
                ("name".into(), Json::Str(scope.name.clone())),
                ("line".into(), Json::Int(i64::from(scope.line))),
                ("kind".into(), Json::Str(scope.kind.into())),
                ("symbols".into(), Json::List(scope.symbols.iter().map(|name| Json::Str(name.clone())).collect())),
            ])
        })
        .collect();
    vec![
        ("coverage".into(), Json::Str("partial".into())),
        ("initialization_verified".into(), Json::Bool(false)),
        ("limitations".into(), Json::List(report.limitations.iter().map(|text| Json::Str(text.clone())).collect())),
        ("reads".into(), Json::List(reads)),
        ("imports".into(), Json::List(imports)),
        ("scopes".into(), Json::List(scopes)),
        ("diagnostics_supported".into(), Json::Bool(report.diagnostics_supported)),
    ]
}

/// The scope report document for one file (read_scope_report_dict).
pub fn scope_document(path: &str) -> Result<String, String> {
    let file = Path::new(path);
    let linked = std::fs::symlink_metadata(file).map(|meta| meta.file_type().is_symlink()).unwrap_or(false);
    if linked {
        return Err("Linked scope inputs are unsupported".into());
    }
    let raw = std::fs::read(file).map_err(|error| format!("{path}: {error}"))?;
    let Some(text) = refactrail_engine::source::decode_source(&raw) else {
        return Err("Scope input must be UTF-8 Python source".into());
    };
    if text.contains('\0') {
        return Err("source code string cannot contain null bytes".into());
    }
    let (tree, comments) = refactrail_parser::parse_with_comments(text).map_err(|error| {
        let (line, _, message) = refactrail_parser::syntax_error_tuple(text, error);
        syntax_error_text(path, line as usize, &message)
    })?;
    let symbols = refactrail_parser::compile::check_with_symbols(&tree)
        .map_err(|error| syntax_error_text(path, error.line as usize, &error.message))?;
    let source = refactrail_engine::source::SourceFile::new(text, &comments);
    let report = refactrail_engine::lexical::analyze(&tree, &source, &comments, &symbols);
    let mut fields = vec![
        ("schema_version".into(), Json::Str("refactrail-scope-1".into())),
        ("path".into(), Json::Str(path.into())),
        ("source_sha256".into(), Json::Str(sha256_hex(&raw))),
    ];
    fields.extend(lexical_fields(&report));
    Ok(dumps(&Json::Object(fields)) + "\n")
}

/// `scope PATH`: one JSON document.
pub fn run_scope(arguments: &[String]) -> Result<i32, String> {
    let [path] = arguments else {
        return Err(if arguments.is_empty() {
            "the following arguments are required: path".into()
        } else {
            format!("unrecognized arguments: {}", arguments[1..].join(" "))
        });
    };
    print!("{}", scope_document(path)?);
    Ok(0)
}
