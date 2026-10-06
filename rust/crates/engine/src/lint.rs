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
    let needs_tokens = ["E701", "E702", "E703", "F541"].iter().any(|code| settings.is_enabled(code));
    let parsed = if needs_tokens {
        refactrail_parser::parse_with_tokens(text)
    } else {
        refactrail_parser::parse_with_comments(text).map(|(tree, comments)| (tree, comments, Vec::new()))
    };
    let (tree, comments, tokens) = match parsed {
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
    let mut report_with_parent = |code: &'static str, position: (usize, usize), message: String, parent: usize| {
        if !settings.is_enabled(code) {
            return;
        }
        if crate::source::is_suppressed(&source.noqa, position.0, code)
            || (parent > 0 && crate::source::is_suppressed(&source.noqa, parent, code))
        {
            return;
        }
        findings.push((position.0, position.1, code, message));
    };
    for (code, position, message, parent) in compat_findings(path, text, &tree, &tokens, &source, &comments, &symbols, settings) {
        report_with_parent(code, position, message, parent);
    }
    let mut report = |code: &'static str, position: (usize, usize), message: String| report_with_parent(code, position, message, 0);
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

/// The pycodestyle- and Pyflakes-compatible findings that are selected.
#[allow(clippy::too_many_arguments)]
fn compat_findings(
    path: &str,
    text: &str,
    tree: &refactrail_parser::ast::Module,
    tokens: &[refactrail_parser::LexedToken],
    source: &SourceFile,
    comments: &[(usize, usize)],
    symbols: &refactrail_parser::symtable::SymbolTable,
    settings: &Settings,
) -> Vec<crate::compat_pycodestyle::CompatFinding> {
    use crate::walk::Visitor;
    let mut out = Vec::new();
    let any_enabled = |codes: &[&str]| codes.iter().any(|code| settings.is_enabled(code));
    if any_enabled(&["E701", "E702", "E703"]) && !tokens.is_empty() {
        crate::compat_pycodestyle::check_statement_tokens(tokens, text, tree, source, &mut out);
    }
    if any_enabled(&[
        "F501", "F502", "F503", "F504", "F505", "F506", "F507", "F508", "F509", "F521", "F522", "F523", "F524", "F525", "F541", "F601", "F602",
        "F631", "F632", "F633", "F634", "F722", "F901",
    ]) {
        out.extend(crate::compat_pyflakes::check_module(tree, source, tokens));
    }
    if any_enabled(&crate::compat_scope::SCOPE_CODES) {
        out.extend(crate::compat_scope::check_module(tree, source, path));
    }
    if settings.is_enabled("E402") {
        crate::compat_pycodestyle::check_import_position(tree, source, &mut out);
    }
    if any_enabled(&["E401", "E711", "E712", "E713", "E714", "E721", "E722", "E731", "E741", "E742", "E743"]) {
        let lexical = if settings.is_enabled("E721") && crate::compat_pycodestyle::has_type_comparison_candidate(tree) {
            lexical::analyze_with(tree, source, comments, symbols, true)
        } else {
            lexical::LexicalReport { limitations: Vec::new(), reads: Vec::new(), imports: Vec::new(), scopes: Vec::new(), diagnostics_supported: false }
        };
        let mut checks = crate::compat_pycodestyle::NodeChecks::new(source, &lexical, tree);
        for statement in &tree.body {
            checks.visit_stmt(statement);
        }
        out.extend(checks.out);
    }
    out
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
