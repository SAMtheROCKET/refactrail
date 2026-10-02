//! RefacTrail's Rust engine as a Python module: a thin PyO3 wrapper over
//! the pure-Rust `refactrail-engine`. It must return exactly the findings
//! of the Python reference engine (see docs/RULES.md).

use pyo3::prelude::*;
use rayon::prelude::*;
use refactrail_engine::{analyze, row, Analysis, Row, Settings};

fn read_settings(settings: &Bound<'_, PyAny>) -> PyResult<Settings> {
    Ok(Settings {
        profile: settings.getattr("profile")?.extract()?,
        line_length: settings.getattr("line_length")?.extract()?,
        function_preferred_lines: settings.getattr("function_preferred_lines")?.extract()?,
        function_max_lines: settings.getattr("function_max_lines")?.extract()?,
        main_max_lines: settings.getattr("main_max_lines")?.extract()?,
        select: settings.getattr("select")?.extract()?,
        ignore: settings.getattr("ignore")?.extract()?,
    })
}

/// Findings for one analysis; for a compilation failure CPython supplies
/// the exact message, so both engines report identical findings (nothing
/// is executed).
fn finish(py: Python<'_>, path: &str, raw: &[u8], analysis: Analysis, settings: &Settings) -> PyResult<Vec<Row>> {
    match analysis {
        Analysis::Rows(rows) => Ok(rows),
        Analysis::CompileFailure { line, offset, message } => {
            if !settings.is_enabled("RT001") {
                return Ok(Vec::new());
            }
            let text = refactrail_engine::source::decode_source(raw).unwrap_or("");
            let exact: Option<(usize, usize, String)> =
                py.import("refactrail.validation")?.getattr("compile_error_tuple")?.call1((text, path))?.extract()?;
            Ok(vec![match exact {
                Some((line, column, message)) => row(path, line, column, "RT001", message),
                None => row(path, line, offset, "RT001", format!("Syntax error: {message}")),
            }])
        }
    }
}

/// Check one file's bytes; `settings` is a refactrail.models.Settings.
#[pyfunction]
fn check_source(py: Python<'_>, path: &str, raw: &[u8], settings: &Bound<'_, PyAny>) -> PyResult<Vec<Row>> {
    let settings = read_settings(settings)?;
    let analysis = py.detach(|| refactrail_engine::with_analysis_stack(|| analyze(path, raw, &settings)));
    finish(py, path, raw, analysis, &settings)
}

/// Read and check many files in parallel (`jobs` threads, 0 = one per
/// core); an unreadable file raises OSError.
#[pyfunction]
#[pyo3(signature = (paths, settings, jobs=0))]
fn check_files(py: Python<'_>, paths: Vec<String>, settings: &Bound<'_, PyAny>, jobs: usize) -> PyResult<Vec<Vec<Row>>> {
    let settings = read_settings(settings)?;
    let mut sources = Vec::with_capacity(paths.len());
    for path in &paths {
        let raw = std::fs::read(path)
            .map_err(|error| pyo3::exceptions::PyOSError::new_err(format!("{path}: {error}")))?;
        sources.push((path.clone(), raw));
    }
    let analyses = py.detach(|| {
        let run = || -> Vec<Analysis> { sources.par_iter().map(|(path, raw)| analyze(path, raw, &settings)).collect() };
        rayon::ThreadPoolBuilder::new()
            .num_threads(jobs)
            .stack_size(refactrail_engine::ANALYSIS_STACK_BYTES)
            .build()
            .map(|pool| pool.install(run))
            .map_err(|error| error.to_string())
    });
    let analyses = analyses.map_err(pyo3::exceptions::PyOSError::new_err)?;
    sources
        .iter()
        .zip(analyses)
        .map(|((path, raw), analysis)| finish(py, path, raw, analysis, &settings))
        .collect()
}

/// Lint (path, bytes) snapshots in parallel (`jobs` threads, 0 = one per
/// core). A file that does not compile gives None, so the caller can take
/// CPython's exact refusal from the Python engine.
#[pyfunction]
#[pyo3(signature = (snapshots, select, ignore, jobs=0))]
fn lint_files(
    py: Python<'_>,
    snapshots: Vec<(String, Bound<'_, pyo3::types::PyBytes>)>,
    select: Vec<String>,
    ignore: Vec<String>,
    jobs: usize,
) -> PyResult<Vec<Option<Vec<Row>>>> {
    let settings = refactrail_engine::lint::lint_settings(&select, &ignore);
    let sources: Vec<(String, Vec<u8>)> =
        snapshots.into_iter().map(|(path, raw)| (path, raw.as_bytes().to_vec())).collect();
    let results = py.detach(|| {
        let run = || -> Vec<Option<Vec<Row>>> {
            sources
                .par_iter()
                .map(|(path, raw)| match refactrail_engine::lint::analyze_lint(path, raw, &settings) {
                    refactrail_engine::lint::LintOutcome::Rows(rows) => Some(rows),
                    _ => None,
                })
                .collect()
        };
        rayon::ThreadPoolBuilder::new()
            .num_threads(jobs)
            .stack_size(refactrail_engine::ANALYSIS_STACK_BYTES)
            .build()
            .map(|pool| pool.install(run))
            .map_err(|error| error.to_string())
    });
    results.map_err(pyo3::exceptions::PyOSError::new_err)
}

/// Format decoded source (or notebook JSON when `notebook`), or None when
/// it cannot be formatted (the caller then reports the Python engine's
/// exact refusal).
#[pyfunction]
#[pyo3(signature = (text, path, width=None, hug=false, notebook=false))]
fn format_text(py: Python<'_>, text: &str, path: &str, width: Option<usize>, hug: bool, notebook: bool) -> Option<String> {
    py.detach(|| {
        refactrail_engine::with_analysis_stack(|| {
            if notebook {
                refactrail_engine::notebook::format_notebook(text, path, width, hug).ok()
            } else {
                refactrail_engine::format::format_source(text, width, hug).ok()
            }
        })
    })
}

#[pymodule]
fn refactrail_core(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_function(wrap_pyfunction!(check_source, module)?)?;
    module.add_function(wrap_pyfunction!(check_files, module)?)?;
    module.add_function(wrap_pyfunction!(lint_files, module)?)?;
    module.add_function(wrap_pyfunction!(format_text, module)?)?;
    module.add("__version__", env!("CARGO_PKG_VERSION"))?;
    module.add("RULE_CONTRACT_VERSION", 3)?;
    Ok(())
}
