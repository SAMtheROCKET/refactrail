//! RefacTrail's rule engine in pure Rust. It must return exactly the
//! findings of the Python reference engine (see docs/RULES.md). Used by
//! the PyO3 module (`refactrail-core`) and the native command line.

mod bindings_count;
mod correctness;
pub mod lexical;
pub mod lint;
pub mod notebook;
mod facts;
pub mod format;
pub mod format_wrap;
pub mod index;
mod rules;
#[doc(hidden)]
pub use rules::{rule_timings, start_rule_timing};
mod scopes;
pub mod settings;
pub mod source;
mod walk;

pub use settings::{severity_of, Settings};

/// Stack for threads that analyse source: trees of deeply nested (but
/// valid) code are walked recursively, and default thread stacks (1-2 MB,
/// 1 MB on the Windows main thread) are too small for them.
pub const ANALYSIS_STACK_BYTES: usize = 64 << 20;

pub use refactrail_parser::Version;

/// Run `work` on a thread with ANALYSIS_STACK_BYTES of stack.
pub fn with_analysis_stack<T: Send>(work: impl FnOnce() -> T + Send) -> T {
    std::thread::scope(|scope| {
        let handle = std::thread::Builder::new()
            .stack_size(ANALYSIS_STACK_BYTES)
            .spawn_scoped(scope, work)
            .expect("an analysis thread can be started");
        handle.join().unwrap_or_else(|panic| std::panic::resume_unwind(panic))
    })
}

/// (path, line, column, code, severity, message), as Python's Finding.
pub type Row = (String, usize, usize, String, String, String);

pub fn row(path: &str, line: usize, column: usize, code: &str, message: String) -> Row {
    (path.to_string(), line, column, code.to_string(), severity_of(code).to_string(), message)
}

/// The result of analysing one file: its findings, or the compilation
/// failure (as CPython's SyntaxError line, offset and message) that
/// replaces them.
pub enum Analysis {
    Rows(Vec<Row>),
    CompileFailure { line: usize, offset: usize, message: String },
}

/// Parse a file once, run the compile checks and then every rule on the
/// same tree (check_text_list from engine.py).
pub fn analyze(path: &str, raw: &[u8], settings: &Settings) -> Analysis {
    let Some(text) = source::decode_source(raw) else {
        if !settings.is_enabled("RT002") {
            return Analysis::Rows(Vec::new());
        }
        return Analysis::Rows(vec![row(path, 1, 1, "RT002", "File is not UTF-8 or declares another encoding.".to_string())]);
    };
    if text.contains('\0') {
        return Analysis::CompileFailure { line: 1, offset: 1, message: "source code string cannot contain null bytes".into() };
    }
    let (tree, comments) = match refactrail_parser::parse_with_comments(text) {
        Ok(parsed) => parsed,
        Err(error) => {
            let (line, offset, message) = refactrail_parser::syntax_error_tuple(text, error);
            return Analysis::CompileFailure { line: line as usize, offset: offset as usize, message };
        }
    };
    if let Some(error) = refactrail_parser::compile::check(&tree) {
        return Analysis::CompileFailure { line: error.line as usize, offset: error.offset as usize, message: error.message };
    }
    let source_file = source::SourceFile::new(text, &comments);
    let findings =
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| rules::run_rules(&source_file, &tree, settings)));
    let rows: Vec<Row> = match findings {
        Ok(mut findings) => {
            // The rows of one file share their path (and a code fixes its
            // severity), so this is the order of sorting the rows.
            findings.sort_unstable_by(|left, right| {
                (left.line, left.column, left.code, &left.message).cmp(&(right.line, right.column, right.code, &right.message))
            });
            findings.into_iter().map(|finding| row(path, finding.line, finding.column, finding.code, finding.message)).collect()
        }
        Err(_) => vec![row(path, 1, 1, "RT000", "Internal error in the Rust engine. Please report it.".to_string())],
    };
    Analysis::Rows(rows)
}

/// A file's findings without any CPython involvement; a compilation
/// failure becomes RT001 with our own message (CPython's message on ~99%
/// of broken files).
pub fn check_file(path: &str, raw: &[u8], settings: &Settings) -> Vec<Row> {
    match analyze(path, raw, settings) {
        Analysis::Rows(rows) => rows,
        Analysis::CompileFailure { line, offset, message } => {
            if !settings.is_enabled("RT001") {
                return Vec::new();
            }
            vec![row(path, line, offset, "RT001", format!("Syntax error: {message}"))]
        }
    }
}
