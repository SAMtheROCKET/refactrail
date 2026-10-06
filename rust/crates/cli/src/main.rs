//! `refactrail-native check`: RefacTrail's style check without Python.
//!
//! Same options, configuration, discovery, findings and output as
//! `refactrail check --engine rust --no-cache`, except that a file which
//! does not compile is reported with RefacTrail's own SyntaxError message
//! (CPython's exact message on ~99% of such files) instead of asking
//! CPython. Exit status: 0 clean (or --exit-zero), 1 findings, 2 errors.

mod commands;
mod config;
mod diff;
mod format_cmd;
mod index_cmd;
mod json;
mod output;

use rayon::prelude::*;
use refactrail_engine::{check_file, Row};
use std::io::Write;
use std::path::{Path, PathBuf};

/// mimalloc: the parser allocates many small nodes.
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

pub const VERSION: &str = "0.4.0a0";
const USAGE: &str = "usage: refactrail-native check [paths ...] [--profile {standard,strict}] [--line-length N]
                                [--select CODES] [--ignore CODES]
                                [--output-format {text,json,github,sarif}] [--statistics]
                                [--exit-zero] [--no-cache] [--jobs N]";

/// The RT and RC codes with their titles, as `refactrail rules` prints
/// them (kept identical by tests/test_rules.py).
const RULES_TEXT: &str = include_str!("../data/rules.txt");

struct Options {
    paths: Vec<String>,
    overrides: config::Overrides,
    output: String,
    statistics: bool,
    exit_zero: bool,
    jobs: usize,
}

/// A worker pool (`jobs` threads, 0 = one per core) with analysis stacks.
pub fn pool(jobs: usize) -> Result<rayon::ThreadPool, String> {
    rayon::ThreadPoolBuilder::new()
        .num_threads(jobs)
        .stack_size(refactrail_engine::ANALYSIS_STACK_BYTES)
        .build()
        .map_err(|error| error.to_string())
}

pub fn split_codes(text: &str) -> Vec<String> {
    text.split(',').map(|code| code.trim().to_uppercase()).filter(|code| !code.is_empty()).collect()
}

fn parse_options(arguments: &[String]) -> Result<Options, String> {
    let mut options = Options {
        paths: Vec::new(),
        overrides: config::Overrides::default(),
        output: "text".into(),
        statistics: false,
        exit_zero: false,
        jobs: 0,
    };
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
            "--profile" => {
                let profile = value()?;
                if profile != "standard" && profile != "strict" {
                    return Err(format!("argument --profile: invalid choice: '{profile}' (choose from 'standard', 'strict')"));
                }
                options.overrides.profile = Some(profile);
            }
            "--line-length" => {
                let text = value()?;
                options.overrides.line_length =
                    Some(text.parse().map_err(|_| format!("argument --line-length: invalid int value: '{text}'"))?);
            }
            "--select" => options.overrides.select = Some(split_codes(&value()?)),
            "--ignore" => options.overrides.ignore = Some(split_codes(&value()?)),
            "--output-format" => {
                let format = value()?;
                if !["text", "json", "github", "sarif"].contains(&format.as_str()) {
                    return Err(format!("argument --output-format: invalid choice: '{format}'"));
                }
                options.output = format;
            }
            "--jobs" => {
                let text = value()?;
                let jobs: i64 = text.parse().map_err(|_| format!("argument --jobs: invalid int value: '{text}'"))?;
                if jobs < 0 {
                    return Err("jobs must be nonnegative".into());
                }
                options.jobs = jobs as usize;
            }
            "--statistics" => options.statistics = true,
            "--exit-zero" => options.exit_zero = true,
            "--no-cache" => {}
            "--engine" => {
                value()?;
            }
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

/// str(pathlib.Path(text)): "." and empty components dropped, no trailing
/// separator, "/" kept for the root.
pub fn python_path(text: &str) -> String {
    let separator = std::path::MAIN_SEPARATOR;
    let text = if cfg!(windows) { text.replace('/', "\\") } else { text.to_string() };
    let absolute = text.starts_with(separator);
    let parts: Vec<&str> = text.split(separator).filter(|part| !part.is_empty() && *part != ".").collect();
    let joined = parts.join(&separator.to_string());
    match (absolute, joined.is_empty()) {
        (true, _) => format!("{separator}{joined}"),
        (false, true) => ".".into(),
        (false, false) => joined,
    }
}

fn join(folder: &str, name: &str) -> String {
    if folder == "." {
        name.to_string()
    } else if folder.ends_with(std::path::MAIN_SEPARATOR) {
        format!("{folder}{name}")
    } else {
        format!("{folder}{}{name}", std::path::MAIN_SEPARATOR)
    }
}

fn is_link(path: &Path) -> bool {
    std::fs::symlink_metadata(path).map(|meta| meta.file_type().is_symlink()).unwrap_or(false)
}

/// discover_python_files_list from discovery.py.
fn discover(paths: &[String], exclude: &[String], suffixes: &[&str]) -> Result<Vec<String>, String> {
    let mut found = std::collections::BTreeSet::new();
    for path_text in paths {
        let path = PathBuf::from(path_text);
        if is_link(&path) {
            return Err(format!("Linked input is not supported: {}", python_path(path_text)));
        }
        if path.is_file() {
            found.insert(python_path(path_text));
            continue;
        }
        if !path.is_dir() {
            return Err(format!("No such file or folder: {path_text}"));
        }
        let mut stack = vec![python_path(path_text)];
        while let Some(folder) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&folder) else { continue };
            let mut folders = Vec::new();
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                let full = join(&folder, &name);
                let full_path = Path::new(&full);
                if full_path.is_dir() {
                    let skipped = exclude.contains(&name)
                        || name.ends_with(".egg-info")
                        || is_link(full_path)
                        || full_path.join("pyvenv.cfg").is_file();
                    if !skipped {
                        folders.push(full);
                    }
                } else if suffixes.iter().any(|suffix| name.ends_with(suffix)) && !is_link(full_path) {
                    found.insert(full);
                }
            }
            folders.sort();
            stack.extend(folders.into_iter().rev());
        }
    }
    Ok(found.into_iter().collect())
}

fn run(arguments: &[String]) -> Result<i32, String> {
    let timing = std::env::var_os("RT_TIMING").is_some();
    let started = std::time::Instant::now();
    let mark = |label: &str| {
        if timing {
            eprintln!("{label:12} {:7.1} ms", started.elapsed().as_secs_f64() * 1e3);
        }
    };
    let options = parse_options(arguments)?;
    let settings = config::load(Path::new(&options.paths[0]), options.overrides)?;
    mark("config");
    let files = discover(&options.paths, &settings.exclude, &[".py"])?;
    mark("discover");
    // Each worker reads and checks its files; a read error is reported for
    // the first such file in order, as when reading everything first.
    let check = || -> Result<Vec<Row>, String> {
        let results: Vec<Result<Vec<Row>, String>> = files
            .par_iter()
            .map(|path| match std::fs::read(path) {
                Ok(raw) => Ok(check_file(path, &raw, &settings.engine)),
                Err(error) => Err(format!("{path}: {error}")),
            })
            .collect();
        let mut rows = Vec::with_capacity(results.iter().map(|result| result.as_ref().map_or(0, Vec::len)).sum());
        for result in results {
            rows.extend(result?);
        }
        rows.sort();
        Ok(rows)
    };
    let rows = pool(options.jobs)?.install(check)?;
    mark("check");
    let report = match options.output.as_str() {
        "json" => output::json(&rows),
        "github" => output::github(&rows),
        "sarif" => output::sarif(&rows, VERSION),
        _ => output::text(&rows, files.len()),
    };
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    let _ = out.write_all(report.as_bytes());
    if options.statistics {
        let _ = std::io::stderr().write_all(output::statistics(&rows).as_bytes());
    }
    mark("output");
    Ok(if options.exit_zero || rows.is_empty() { 0 } else { 1 })
}

/// Take `--python-version X.Y` (anywhere) or REFACTRAIL_PYTHON_VERSION and
/// configure the grammar; the default is the newest supported version.
fn configure_python_version(arguments: &mut Vec<String>) -> Result<(), String> {
    let mut chosen = std::env::var("REFACTRAIL_PYTHON_VERSION").ok();
    let mut index = 0;
    while index < arguments.len() {
        if let Some(value) = arguments[index].strip_prefix("--python-version=") {
            chosen = Some(value.to_string());
            arguments.remove(index);
        } else if arguments[index] == "--python-version" {
            arguments.remove(index);
            if index >= arguments.len() {
                return Err("argument --python-version: expected one argument".into());
            }
            chosen = Some(arguments.remove(index));
        } else {
            index += 1;
        }
    }
    let version = match chosen {
        None => refactrail_engine::Version::LATEST,
        Some(text) => refactrail_engine::Version::parse(&text)
            .ok_or_else(|| format!("argument --python-version: invalid choice: '{text}' (choose from '3.12', '3.13', '3.14')"))?,
    };
    refactrail_engine::Version::configure(version);
    Ok(())
}

fn main() {
    let mut arguments: Vec<String> = std::env::args().skip(1).collect();
    if let Err(message) = configure_python_version(&mut arguments) {
        eprintln!("refactrail: error: {message}");
        std::process::exit(2);
    }
    match arguments.first().map(String::as_str) {
        Some("--version") => {
            println!("{VERSION}");
            return;
        }
        Some("rules") => {
            print!("{RULES_TEXT}");
            return;
        }
        Some("check") | Some("lint") | Some("scope") | Some("format") | Some("index") => {}
        _ => {
            eprintln!("{USAGE}
{}
{}
{}
{}", commands::LINT_USAGE, commands::SCOPE_USAGE, format_cmd::FORMAT_USAGE, index_cmd::INDEX_USAGE);
            std::process::exit(2);
        }
    }
    let result = refactrail_engine::with_analysis_stack(|| match arguments[0].as_str() {
        "lint" => commands::run_lint(&arguments[1..]),
        "scope" => commands::run_scope(&arguments[1..]),
        "format" => format_cmd::run_format(&arguments[1..]),
        "index" => index_cmd::run_index(&arguments[1..]),
        _ => run(&arguments[1..]),
    });
    match result {
        Ok(code) => std::process::exit(code),
        Err(message) => {
            eprintln!("refactrail: error: {message}");
            std::process::exit(2);
        }
    }
}
