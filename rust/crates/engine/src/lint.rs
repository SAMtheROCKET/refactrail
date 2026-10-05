//! The `lint` operation: general correctness checks RC101-RC107 and the
//! lexical diagnostics RC201/RC202 (mirror of correctness.py,
//! check_correctness_list). Findings must match the Python engine.

use crate::lexical;
use crate::settings::Settings;
use crate::source::{decode_source, SourceFile};
use crate::{row, Row};

/// The result of linting one file.
pub enum LintOutcome {
    Rows(Vec<Row>),
    /// A SyntaxError from compiling the file: line, offset and message.
    SyntaxError { line: usize, offset: usize, message: String },
    /// A ValueError from compiling the file (null bytes).
    ValueError { message: String },
}

/// Settings for lint: rule prefixes only (the standard profile).
pub fn lint_settings(select: &[String], ignore: &[String]) -> Settings {
    Settings {
        profile: "standard".into(),
        line_length: 79,
        function_preferred_lines: 40,
        function_max_lines: 50,
        main_max_lines: 100,
        select: select.to_vec(),
        ignore: ignore.to_vec(),
    }
}

/// Lint one file's bytes.
pub fn analyze_lint(path: &str, raw: &[u8], settings: &Settings) -> LintOutcome {
    let Some(text) = decode_source(raw) else {
        return LintOutcome::Rows(vec![row(path, 1, 1, "RT002", "File is not UTF-8 or declares another encoding.".to_string())]);
    };
    if text.contains('\0') {
        return LintOutcome::ValueError { message: "source code string cannot contain null bytes".into() };
    }
    let (tree, comments) = match refactrail_parser::parse_with_comments(text) {
        Ok(parsed) => parsed,
        Err(error) => {
            let (line, offset, message) = refactrail_parser::syntax_error_tuple(text, error);
            return LintOutcome::SyntaxError { line: line as usize, offset: offset as usize, message };
        }
    };
    let symbols = match refactrail_parser::compile::check_with_symbols(&tree) {
        Ok(symbols) => symbols,
        Err(error) => {
            return LintOutcome::SyntaxError { line: error.line as usize, offset: error.offset as usize, message: error.message };
        }
    };
    let source = SourceFile::new(text, &comments);
    let mut findings: Vec<(usize, usize, &'static str, String)> = Vec::new();
    let mut report = |code: &'static str, position: (usize, usize), message: String| {
        if !settings.is_enabled(code) {
            return;
        }
        if crate::source::is_suppressed(&source.noqa, position.0, code) {
            return;
        }
        findings.push((position.0, position.1, code, message));
    };
    for (code, loc, message) in crate::correctness::check_module(&tree.body) {
        report(code, source.position(loc), message);
    }
    if settings.is_enabled("RC201") || settings.is_enabled("RC202") {
        let lexical = lexical::analyze(&tree, &source, &comments, &symbols);
        for read in &lexical.reads {
            if read.resolution == "unresolved" && !read.annotation {
                report(
                    "RC201",
                    (read.line, read.column),
                    format!("No lexical binding found for '{}'; dynamic availability is unverified.", read.name),
                );
            }
        }
        let file = std::path::Path::new(path);
        let skip_imports = file.file_name().is_some_and(|name| name == "__init__.py")
            || file.extension().is_some_and(|extension| extension == "pyi");
        if !skip_imports {
            for import in &lexical.imports {
                if !import.used && !import.exempt {
                    report(
                        "RC202",
                        (import.line, import.column),
                        format!("Import '{}' has no lexical read; review exports and side effects.", import.name),
                    );
                }
            }
        }
    }
    findings.sort();
    findings.dedup();
    LintOutcome::Rows(findings.into_iter().map(|(line, column, code, message)| row(path, line, column, code, message)).collect())
}

/// Python's str(SyntaxError): "msg (file name, line N)", where the file
/// name is the part after the platform's path separator.
pub fn syntax_error_text(path: &str, line: usize, message: &str) -> String {
    let separator = std::path::MAIN_SEPARATOR;
    let name = path.rsplit(separator).next().unwrap_or(path);
    format!("{message} ({name}, line {line})")
}

/// A file's lint rows without CPython (our own error messages).
pub fn lint_file(path: &str, raw: &[u8], settings: &Settings) -> Vec<Row> {
    match analyze_lint(path, raw, settings) {
        LintOutcome::Rows(rows) => rows,
        LintOutcome::SyntaxError { line, offset, message } => {
            vec![row(path, line.max(1), offset.max(1), "RT001", format!("Syntax error: {}", syntax_error_text(path, line, &message)))]
        }
        LintOutcome::ValueError { message } => vec![row(path, 1, 1, "RT001", format!("Syntax error: {message}"))],
    }
}
