//! `refactrail-native index ROOT [--changed PATH]...`: the same
//! refactrail-index-1 document as `refactrail index`, without Python.

use crate::json::{dumps, sha256_hex, Json};
use crate::{config, discover};
use refactrail_engine::lint::syntax_error_text;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path, PathBuf};

pub const INDEX_USAGE: &str = "usage: refactrail-native index [--changed CHANGED] root";

const LIMITS: [&str; 3] = [
    "Import edges are syntax evidence, not resolved runtime imports.",
    "Dynamic imports, search-path changes and monkey patches are unknown.",
    "Symbols and import impact do not prove call graphs or API compatibility.",
];

struct FileRecord {
    path: String,
    module: String,
    package: bool,
    json: Option<Json>,
    imports: Vec<refactrail_engine::index::ImportRecord>,
}

/// Path.resolve(): absolute, symbolic links resolved where the path
/// exists, `.` and `..` removed lexically beyond that.
fn resolve(path: &Path) -> PathBuf {
    let absolute = if path.is_absolute() { path.to_path_buf() } else { std::env::current_dir().unwrap_or_default().join(path) };
    let mut existing = absolute.clone();
    let mut rest = Vec::new();
    while !existing.exists() {
        match (existing.file_name().map(|name| name.to_os_string()), existing.parent().map(Path::to_path_buf)) {
            (Some(name), Some(parent)) => {
                rest.push(name);
                existing = parent;
            }
            _ => break,
        }
    }
    let mut resolved = std::fs::canonicalize(&existing).map(strip_verbatim).unwrap_or(existing);
    for name in rest.into_iter().rev() {
        resolved.push(name);
    }
    let mut normal = PathBuf::new();
    for component in resolved.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normal.pop();
            }
            other => normal.push(other.as_os_str()),
        }
    }
    normal
}

/// Drop Windows' `\\?\` prefix, which Path.resolve() does not show.
fn strip_verbatim(path: PathBuf) -> PathBuf {
    let text = path.to_string_lossy();
    match text.strip_prefix(r"\\?\") {
        Some(rest) if !rest.starts_with("UNC") => PathBuf::from(rest),
        _ => path,
    }
}

fn posix(path: &Path) -> String {
    path.components().map(|part| part.as_os_str().to_string_lossy().into_owned()).collect::<Vec<_>>().join("/")
}

/// The module name and package flag of a root-relative source path.
fn module_identity(relative: &Path) -> (String, bool) {
    let without_suffix = relative.with_extension("");
    let parts: Vec<String> = without_suffix.components().map(|part| part.as_os_str().to_string_lossy().into_owned()).collect();
    let package = parts.last().is_some_and(|part| part == "__init__");
    let used = if package { &parts[..parts.len() - 1] } else { &parts[..] };
    (used.join("."), package)
}

fn read_file(path: &str, root: &Path) -> Result<FileRecord, String> {
    let raw = std::fs::read(path).map_err(|error| format!("{path}: {error}"))?;
    let Some(text) = refactrail_engine::source::decode_source(&raw) else {
        return Err(format!("Unsupported source encoding: {path}"));
    };
    let parsed = if text.contains('\0') {
        Err("source code string cannot contain null bytes".to_string())
    } else {
        match refactrail_parser::parse(text) {
            Err(error) => {
                let (line, _, message) = refactrail_parser::syntax_error_tuple(text, error);
                Err(syntax_error_text(path, line as usize, &message))
            }
            Ok(tree) => match refactrail_parser::compile::check(&tree) {
                Some(error) => Err(syntax_error_text(path, error.line as usize, &error.message)),
                None => Ok(tree),
            },
        }
    };
    let tree = parsed.map_err(|error| format!("Cannot index {path}: {error}"))?;
    let relative = Path::new(path).strip_prefix(root).unwrap_or(Path::new(path)).to_path_buf();
    let (module, package) = module_identity(&relative);
    let symbols = refactrail_engine::index::symbols(&tree)
        .into_iter()
        .map(|symbol| {
            Json::Object(vec![
                ("name".into(), Json::Str(symbol.name)),
                ("kind".into(), Json::Str(symbol.kind.into())),
                ("line".into(), Json::Int(i64::from(symbol.line))),
                ("end_line".into(), Json::Int(i64::from(symbol.end_line))),
            ])
        })
        .collect();
    let imports = refactrail_engine::index::imports(&tree);
    let import_json = imports
        .iter()
        .map(|import| {
            Json::Object(vec![
                ("module".into(), Json::Str(import.module.clone())),
                ("level".into(), Json::Int(import.level)),
                ("names".into(), Json::List(import.names.iter().map(|name| Json::Str(name.clone())).collect())),
                ("line".into(), Json::Int(i64::from(import.line))),
            ])
        })
        .collect();
    let path_text = posix(&relative);
    let json = Json::Object(vec![
        ("path".into(), Json::Str(path_text.clone())),
        ("module".into(), Json::Str(module.clone())),
        ("package".into(), Json::Bool(package)),
        ("source_sha256".into(), Json::Str(sha256_hex(&raw))),
        ("symbols".into(), Json::List(symbols)),
        ("imports".into(), Json::List(import_json)),
    ]);
    Ok(FileRecord { path: path_text, module, package, json: Some(json), imports })
}

/// resolve_import_candidates_list: possible local module names.
fn candidates(file: &FileRecord, import: &refactrail_engine::index::ImportRecord) -> Vec<String> {
    let mut module = import.module.clone();
    if import.level > 0 {
        let mut package: Vec<&str> = file.module.split('.').collect();
        if !file.package {
            package.pop();
        }
        let level = import.level as usize;
        if level > package.len() {
            return Vec::new();
        }
        let mut parent: Vec<&str> = package[..package.len() - level + 1].to_vec();
        if !module.is_empty() {
            parent.push(&import.module);
        }
        module = parent.join(".");
    }
    let mut names = vec![module.clone()];
    names.extend(import.names.iter().filter(|name| *name != "*").map(|name| format!("{module}.{name}")));
    let mut found = BTreeSet::new();
    for name in names.iter().filter(|name| !name.is_empty()) {
        let parts: Vec<&str> = name.split('.').collect();
        for count in 1..=parts.len() {
            found.insert(parts[..count].join("."));
        }
    }
    found.into_iter().collect()
}

/// `index ROOT [--changed PATH]...`.
pub fn run_index(arguments: &[String]) -> Result<i32, String> {
    let mut root_argument = None;
    let mut changed: Vec<String> = Vec::new();
    let mut index = 0;
    while index < arguments.len() {
        let argument = arguments[index].as_str();
        if let Some(value) = argument.strip_prefix("--changed=") {
            changed.push(value.to_string());
        } else if argument == "--changed" {
            index += 1;
            changed.push(arguments.get(index).cloned().ok_or("argument --changed: expected one argument")?);
        } else if argument.starts_with("--") {
            return Err(format!("unrecognized arguments: {argument}"));
        } else if root_argument.is_none() {
            root_argument = Some(argument.to_string());
        } else {
            return Err(format!("unrecognized arguments: {argument}"));
        }
        index += 1;
    }
    let root_argument = root_argument.ok_or("the following arguments are required: root")?;
    let requested = Path::new(&root_argument);
    if std::fs::symlink_metadata(requested).map(|meta| meta.file_type().is_symlink()).unwrap_or(false) {
        return Err("Linked index roots are unsupported".into());
    }
    let root = resolve(requested);
    if !root.is_dir() {
        return Err("index requires a Python import-root directory".into());
    }
    let root_text = root.to_string_lossy().into_owned();
    let settings = config::load(&root, config::Overrides::default())?;
    let paths = discover(&[root_text.clone()], &settings.exclude, &[".py", ".pyi"])?;
    let mut files = Vec::with_capacity(paths.len());
    for path in &paths {
        files.push(read_file(path, &root)?);
    }
    if files.is_empty() {
        return Err("No Python files to index".into());
    }
    let mut normalized = BTreeSet::new();
    for path in &changed {
        let candidate = resolve(&root.join(path));
        let Ok(relative) = candidate.strip_prefix(&root) else {
            return Err(format!("Changed path escapes index root: {path}"));
        };
        normalized.insert(posix(relative));
    }
    let existing: BTreeSet<&str> = files.iter().map(|file| file.path.as_str()).collect();
    let mut providers: Vec<FileRecord> = Vec::new();
    for path in &normalized {
        let extension = Path::new(path).extension().and_then(|extension| extension.to_str()).unwrap_or("");
        if existing.contains(path.as_str()) || !matches!(extension, "py" | "pyi") {
            continue;
        }
        let (module, package) = module_identity(Path::new(path));
        providers.push(FileRecord { path: path.clone(), module, package, json: None, imports: Vec::new() });
    }
    let all: Vec<&FileRecord> = files.iter().chain(providers.iter()).collect();
    let mut modules: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for file in &all {
        modules.entry(file.module.as_str()).or_default().push(file.path.as_str());
    }
    let mut edges = Vec::new();
    let mut reverse: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for file in &all {
        for import in &file.imports {
            let targets: BTreeSet<&str> =
                candidates(file, import).iter().flat_map(|name| modules.get(name.as_str()).cloned().unwrap_or_default()).collect();
            for target in &targets {
                reverse.entry(target.to_string()).or_default().insert(file.path.clone());
            }
            edges.push(Json::Object(vec![
                ("source".into(), Json::Str(file.path.clone())),
                ("line".into(), Json::Int(i64::from(import.line))),
                ("request".into(), Json::Str(import.module.clone())),
                ("candidates".into(), Json::List(targets.iter().map(|target| Json::Str(target.to_string())).collect())),
                ("resolution".into(), Json::Str(if targets.is_empty() { "unresolved" } else { "local_candidates" }.into())),
            ]));
        }
    }
    let mut affected: BTreeSet<String> = normalized.iter().cloned().collect();
    let mut pending: Vec<String> = normalized.iter().cloned().collect();
    while let Some(path) = pending.pop() {
        for source in reverse.get(&path).into_iter().flatten() {
            if affected.insert(source.clone()) {
                pending.push(source.clone());
            }
        }
    }
    let document = Json::Object(vec![
        ("schema_version".into(), Json::Str("refactrail-index-1".into())),
        ("root".into(), Json::Str(root_text)),
        ("files".into(), Json::List(files.into_iter().filter_map(|file| file.json).collect())),
        ("edges".into(), Json::List(edges)),
        ("impact".into(), Json::List(affected.into_iter().map(Json::Str).collect())),
        ("coverage".into(), Json::Str("partial".into())),
        ("limitations".into(), Json::List(LIMITS.iter().map(|text| Json::Str(text.to_string())).collect())),
        ("behavior_verified".into(), Json::Bool(false)),
    ]);
    print!("{}", dumps(&document) + "\n");
    Ok(0)
}
