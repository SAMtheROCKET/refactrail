//! `lint --fix` / `lint --diff`: apply or preview the safe fixes of the
//! selected findings, file by file, exactly as `refactrail lint --fix`
//! (refactrail/general_cli.py and refactrail/fixing.py) does.

use std::collections::BTreeMap;
use std::path::Path;

use refactrail_engine::compat_fix::{fix_compat_text, FIXABLE_CODES};
use refactrail_engine::settings::Settings;
use refactrail_engine::Row;

use crate::diff::{split_lines_keep, unified_diff};
use crate::format_cmd::{verify_original, write_atomically, Plan};
use crate::json::sha256_hex;

/// The text of a file as fix_file prepares it.
struct FileFix {
    original: String,
    fixed: String,
    applied: Vec<String>,
    notes: Vec<String>,
    ending: &'static str,
    bom: bool,
    digest: String,
    raw: Vec<u8>,
}

/// detect_line_ending_str.
fn line_ending(text: &str) -> &'static str {
    ["\r\n", "\r", "\n"].into_iter().find(|ending| text.contains(ending)).unwrap_or("\n")
}

/// fix_file: compute one file's fixes; None when it is skipped.
fn fix_file(path: &str, settings: &Settings, known: &[Row]) -> Result<Option<FileFix>, String> {
    let file = Path::new(path);
    if std::fs::symlink_metadata(file).map(|meta| meta.file_type().is_symlink()).unwrap_or(false) {
        return Err(format!("Linked source is not writable: {}", crate::python_path(path)));
    }
    let raw = std::fs::read(file).map_err(|error| format!("{path}: {error}"))?;
    let Some(text) = refactrail_engine::source::decode_source(&raw) else { return Ok(None) };
    let ending = line_ending(text);
    let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
    let compiles = !normalized.contains('\0')
        && refactrail_parser::parse_with_comments(&normalized)
            .ok()
            .is_some_and(|(tree, _)| refactrail_parser::compile::check_with_symbols(&tree).is_ok());
    if !compiles {
        return Ok(None);
    }
    let outcome = fix_compat_text(&normalized, path, settings, Some(known));
    Ok(Some(FileFix {
        original: normalized,
        fixed: outcome.text,
        applied: outcome.applied,
        notes: outcome.notes,
        ending,
        bom: raw.starts_with(b"\xef\xbb\xbf"),
        digest: sha256_hex(&raw),
        raw,
    }))
}

/// write_fix_none: the file's own line endings and BOM, verified first.
fn write_fix(path: &str, fix: &FileFix) -> Result<(), String> {
    verify_original(path, &fix.digest)?;
    let mut output = if fix.bom { b"\xef\xbb\xbf".to_vec() } else { Vec::new() };
    output.extend_from_slice(fix.fixed.replace('\n', fix.ending).as_bytes());
    let plan = Plan { path: path.to_string(), digest: fix.digest.clone(), original: fix.raw.clone(), output };
    write_atomically(&plan)
}

/// run_lint_fixes_bool: true when any file has (or had) fixes.
pub fn run_lint_fixes(rows: &[Row], settings: &Settings, diff: bool) -> Result<bool, String> {
    let mut by_path: BTreeMap<&str, Vec<Row>> = BTreeMap::new();
    for row in rows {
        by_path.entry(row.0.as_str()).or_default().push(row.clone());
    }
    let mut changed = 0usize;
    for (path, known) in &by_path {
        if !known.iter().any(|row| FIXABLE_CODES.contains(&row.3.as_str())) {
            continue;
        }
        let Some(fix) = fix_file(path, settings, known)? else { continue };
        for note in &fix.notes {
            eprintln!("{path}: note: {note}");
        }
        if fix.fixed == fix.original {
            continue;
        }
        changed += 1;
        if diff {
            let (before, after) = (split_lines_keep(&fix.original), split_lines_keep(&fix.fixed));
            print!("{}", unified_diff(&before, &after, &format!("a/{path}"), &format!("b/{path}")));
        } else {
            write_fix(path, &fix)?;
            for line in &fix.applied {
                eprintln!("{path}: fixed {line}");
            }
        }
    }
    eprintln!("{changed} file(s) {}.", if diff { "would be fixed" } else { "fixed" });
    Ok(changed > 0)
}
