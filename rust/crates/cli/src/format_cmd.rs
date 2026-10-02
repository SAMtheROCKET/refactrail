//! `refactrail-native format`: the same proposals, previews, checks and
//! writes as `refactrail format`, without Python, for .py, .pyi and
//! .ipynb files.

use crate::diff::{split_lines_keep, unified_diff};
use crate::json::{dumps, sha256_hex, Json};
use crate::{config, discover};
use refactrail_engine::format::{format_source, FormatError};
use refactrail_engine::lint::syntax_error_text;
use std::io::Write;
use std::path::Path;

pub const FORMAT_USAGE: &str = "usage: refactrail-native format [paths ...] [--line-length LINE_LENGTH] [--notebooks]
                                 [--bracket-style {own-line,hug}] [--write | --check | --diff]
                                 [--output-format {text,json}]";

struct FormatOptions {
    paths: Vec<String>,
    width: Option<usize>,
    notebooks: bool,
    hug: bool,
    write: bool,
    check: bool,
    json: bool,
}

/// One validated proposal (FormatPlan).
struct Plan {
    path: String,
    original: Vec<u8>,
    output: Vec<u8>,
    digest: String,
}

fn parse_format_options(arguments: &[String]) -> Result<FormatOptions, String> {
    let mut options = FormatOptions { paths: Vec::new(), width: None, notebooks: false, hug: false, write: false, check: false, json: false };
    let mut mode: Option<&str> = None;
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
            "--line-length" => {
                let text = value()?;
                let width: i64 = text.parse().map_err(|_| format!("argument --line-length: invalid int value: '{text}'"))?;
                options.width = Some(if width < 0 { usize::MAX } else { width as usize });
            }
            "--notebooks" => options.notebooks = true,
            "--bracket-style" => {
                let style = value()?;
                if style != "own-line" && style != "hug" {
                    return Err(format!("argument --bracket-style: invalid choice: '{style}' (choose from 'own-line', 'hug')"));
                }
                options.hug = style == "hug";
            }
            "--write" | "--check" | "--diff" => {
                if let Some(previous) = mode.filter(|previous| *previous != flag) {
                    return Err(format!("argument {flag}: not allowed with argument {previous}"));
                }
                mode = Some(flag);
                options.write = flag == "--write";
                options.check = flag == "--check";
            }
            "--output-format" => {
                let format = value()?;
                if format != "text" && format != "json" {
                    return Err(format!("argument --output-format: invalid choice: '{format}' (choose from 'text', 'json')"));
                }
                options.json = format == "json";
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

/// discover_general_files_list: .py and .pyi (and .ipynb on request).
pub fn discover_general(paths: &[String], notebooks: bool) -> Result<Vec<String>, String> {
    let settings = config::load(Path::new(&paths[0]), config::Overrides::default())?;
    let suffixes: &[&str] = if notebooks { &[".py", ".pyi", ".ipynb"] } else { &[".py", ".pyi"] };
    let files = discover(paths, &settings.exclude, suffixes)?;
    if files.is_empty() {
        return Err("No Python source files were selected".into());
    }
    let supported = |path: &String| {
        let extension = Path::new(path).extension().and_then(|extension| extension.to_str()).unwrap_or("");
        matches!(extension, "py" | "pyi" | "ipynb")
    };
    if !files.iter().all(supported) {
        return Err("Only .py, .pyi and Python .ipynb files are supported".into());
    }
    Ok(files)
}

fn is_linked(path: &Path) -> bool {
    std::fs::symlink_metadata(path).map(|meta| meta.file_type().is_symlink()).unwrap_or(false)
}

/// plan_format: read, format, validate; never writes.
fn plan_format(path: &str, width: Option<usize>, hug: bool) -> Result<Plan, String> {
    let file = Path::new(path);
    if is_linked(file) {
        return Err(format!("Linked source is not supported: {}", crate::python_path(path)));
    }
    let raw = std::fs::read(file).map_err(|error| format!("{path}: {error}"))?;
    let Some(text) = refactrail_engine::source::decode_source(&raw) else {
        return Err("Source is not UTF-8 or declares another encoding".into());
    };
    let formatted = if file.extension().is_some_and(|extension| extension == "ipynb") {
        refactrail_engine::notebook::format_notebook(text, path, width, hug).map_err(|message| format!("Cannot format {path}: {message}"))?
    } else {
        format_source(text, width, hug).map_err(|error| match error {
            FormatError::Syntax { line, message } => format!("Cannot format {path}: {}", syntax_error_text(path, line, &message)),
            FormatError::Value(message) => format!("Cannot format {path}: {message}"),
        })?
    };
    let mut output = if raw.starts_with(b"\xef\xbb\xbf") { b"\xef\xbb\xbf".to_vec() } else { Vec::new() };
    output.extend_from_slice(formatted.as_bytes());
    Ok(Plan { path: path.to_string(), digest: sha256_hex(&raw), original: raw, output })
}

/// render_format_diff_str.
fn plan_diff(plan: &Plan) -> String {
    let decode = |bytes: &[u8]| -> String {
        let text = String::from_utf8_lossy(bytes).into_owned();
        text.strip_prefix('\u{feff}').map(str::to_string).unwrap_or(text)
    };
    let (original, output) = (decode(&plan.original), decode(&plan.output));
    let (before, after) = (split_lines_keep(&original), split_lines_keep(&output));
    unified_diff(&before, &after, &format!("a/{}", plan.path), &format!("b/{}", plan.path))
}

/// verify_original_none: the file is unchanged and not a link.
fn verify_original(path: &str, digest: &str) -> Result<(), String> {
    let file = Path::new(path);
    let current = std::fs::read(file).map_err(|error| format!("{path}: {error}"))?;
    if is_linked(file) || sha256_hex(&current) != digest {
        return Err(format!("Source changed or is linked: {}", crate::python_path(path)));
    }
    Ok(())
}

/// write_atomically_none: temporary file, fsync, permissions, replace.
fn write_atomically(plan: &Plan) -> Result<(), String> {
    let file = Path::new(&plan.path);
    let folder = file.parent().filter(|parent| !parent.as_os_str().is_empty()).unwrap_or(Path::new("."));
    let permissions = std::fs::metadata(file).map_err(|error| error.to_string())?.permissions();
    let mut attempt = 0u32;
    let (temporary, mut stream) = loop {
        let candidate = folder.join(format!(".refactrail-{}-{attempt}", std::process::id()));
        match std::fs::OpenOptions::new().write(true).create_new(true).open(&candidate) {
            Ok(stream) => break (candidate, stream),
            Err(_) if attempt < 100 => attempt += 1,
            Err(error) => return Err(error.to_string()),
        }
    };
    let result = (|| -> Result<(), String> {
        stream.write_all(&plan.output).map_err(|error| error.to_string())?;
        stream.sync_all().map_err(|error| error.to_string())?;
        drop(stream);
        std::fs::set_permissions(&temporary, permissions).map_err(|error| error.to_string())?;
        verify_original(&plan.path, &plan.digest)?;
        std::fs::rename(&temporary, file).map_err(|error| error.to_string())
    })();
    let _ = std::fs::remove_file(&temporary);
    result
}

/// `format`: exit 1 when a preview or check finds changes.
pub fn run_format(arguments: &[String]) -> Result<i32, String> {
    let options = parse_format_options(arguments)?;
    let files = discover_general(&options.paths, options.notebooks)?;
    let mut plans = Vec::with_capacity(files.len());
    for path in &files {
        plans.push(plan_format(path, options.width, options.hug)?);
    }
    if options.write {
        for plan in &plans {
            verify_original(&plan.path, &plan.digest)?;
        }
        for plan in plans.iter().filter(|plan| plan.original != plan.output) {
            write_atomically(plan)?;
        }
    }
    let changed: Vec<&Plan> = plans.iter().filter(|plan| plan.original != plan.output).collect();
    let report = if options.json {
        let files = plans
            .iter()
            .map(|plan| {
                Json::Object(vec![
                    ("path".into(), Json::Str(plan.path.clone())),
                    ("source_sha256".into(), Json::Str(plan.digest.clone())),
                    ("changed".into(), Json::Bool(plan.original != plan.output)),
                    ("diff".into(), Json::Str(plan_diff(plan))),
                ])
            })
            .collect();
        dumps(&Json::Object(vec![
            ("schema_version".into(), Json::Str("refactrail-format-1".into())),
            ("engine".into(), Json::Str("refactrail-rust".into())),
            ("written".into(), Json::Bool(options.write)),
            ("behavior_verified".into(), Json::Bool(false)),
            ("files".into(), Json::List(files)),
        ])) + "\n"
    } else {
        let mut text = String::new();
        if !options.write && !options.check {
            for plan in &changed {
                text.push_str(&plan_diff(plan));
            }
        }
        let action = if options.write { "formatted" } else { "would be formatted" };
        text.push_str(&format!("{} file(s) {action}.\n", changed.len()));
        text
    };
    print!("{report}");
    Ok(i32::from(!changed.is_empty() && !options.write))
}
