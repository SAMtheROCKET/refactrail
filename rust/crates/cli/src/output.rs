//! Report renderers, byte-identical to src/refactrail/report.py and
//! sarif.py (text, JSON, GitHub annotations, SARIF 2.1.0).

use refactrail_engine::Row;

/// json.encoder.encode_basestring_ascii.
pub fn json_string(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            ' '..='~' => out.push(c),
            c => {
                let mut units = [0u16; 2];
                for unit in c.encode_utf16(&mut units) {
                    out.push_str(&format!("\\u{unit:04x}"));
                }
            }
        }
    }
    out.push('"');
    out
}

pub fn text(rows: &[Row], files: usize) -> String {
    use std::fmt::Write as _;
    let mut out = String::with_capacity(rows.len() * 96 + 128);
    for (path, line, column, code, _, message) in rows {
        let _ = writeln!(out, "{path}:{line}:{column}: {code} {message}");
    }
    let errors = rows.iter().filter(|row| row.4 == "error").count();
    if rows.is_empty() {
        let _ = writeln!(out, "All checks passed ({files} file(s)).");
    } else {
        let _ = writeln!(
            out,
            "Found {} finding(s) in {files} file(s): {errors} error(s), {} warning(s).",
            rows.len(),
            rows.len() - errors
        );
    }
    out
}

pub fn json(rows: &[Row]) -> String {
    if rows.is_empty() {
        return "[]\n".into();
    }
    let objects: Vec<String> = rows
        .iter()
        .map(|(path, line, column, code, severity, message)| {
            format!(
                "  {{\n    \"path\": {},\n    \"line\": {line},\n    \"column\": {column},\n    \"code\": {},\n    \"severity\": {},\n    \"message\": {}\n  }}",
                json_string(path),
                json_string(code),
                json_string(severity),
                json_string(message)
            )
        })
        .collect();
    format!("[\n{}\n]\n", objects.join(",\n"))
}

pub fn github(rows: &[Row]) -> String {
    let lines: Vec<String> = rows
        .iter()
        .map(|(path, line, column, code, severity, message)| {
            let message = message.replace('%', "%25").replace('\r', "%0D").replace('\n', "%0A");
            format!("::{severity} file={path},line={line},col={column},title={code}::{code} {message}")
        })
        .collect();
    if lines.is_empty() { String::new() } else { lines.join("\n") + "\n" }
}

/// render_statistics_str: counts per code, most frequent first (ties in
/// order of first appearance, as collections.Counter.most_common).
pub fn statistics(rows: &[Row]) -> String {
    let mut counts: Vec<(&str, usize)> = Vec::new();
    for row in rows {
        match counts.iter_mut().find(|(code, _)| *code == row.3) {
            Some(entry) => entry.1 += 1,
            None => counts.push((&row.3, 1)),
        }
    }
    counts.sort_by(|left, right| right.1.cmp(&left.1));
    counts.iter().map(|(code, count)| format!("{count:5}  {code}\n")).collect()
}

/// pathlib's Path(path).absolute().as_uri().
fn file_uri(path: &str) -> String {
    let absolute = std::path::absolute(path).unwrap_or_else(|_| std::path::PathBuf::from(path));
    let text = absolute.to_string_lossy().replace('\\', "/");
    let quote = |part: &str| -> String {
        part.bytes()
            .map(|byte| {
                if byte.is_ascii_alphanumeric() || b"_.-~/".contains(&byte) {
                    (byte as char).to_string()
                } else {
                    format!("%{byte:02X}")
                }
            })
            .collect()
    };
    if cfg!(windows) {
        // file:///C:/path — the drive keeps its colon.
        match text.split_once(':') {
            Some((drive, rest)) if drive.len() == 1 => format!("file:///{drive}:{}", quote(rest)),
            _ => format!("file:{}", quote(&text)),
        }
    } else {
        format!("file://{}", quote(&text))
    }
}

pub fn sarif(rows: &[Row], version: &str) -> String {
    let failed = rows.iter().any(|row| matches!(row.3.as_str(), "RT000" | "RT001" | "RT002"));
    let results: Vec<String> = rows
        .iter()
        .map(|(path, line, column, code, severity, message)| {
            format!(
                "        {{\n          \"ruleId\": {},\n          \"level\": {},\n          \"message\": {{\n            \"text\": {}\n          }},\n          \"locations\": [\n            {{\n              \"physicalLocation\": {{\n                \"artifactLocation\": {{\n                  \"uri\": {}\n                }},\n                \"region\": {{\n                  \"startLine\": {line},\n                  \"startColumn\": {column}\n                }}\n              }}\n            }}\n          ]\n        }}",
                json_string(code),
                json_string(severity),
                json_string(message),
                json_string(&file_uri(path))
            )
        })
        .collect();
    let results = if results.is_empty() { "[]".to_string() } else { format!("[\n{}\n      ]", results.join(",\n")) };
    format!(
        "{{\n  \"$schema\": \"https://docs.oasis-open.org/sarif/sarif/v2.1.0/os/schemas/sarif-schema-2.1.0.json\",\n  \"version\": \"2.1.0\",\n  \"runs\": [\n    {{\n      \"tool\": {{\n        \"driver\": {{\n          \"name\": \"RefacTrail\",\n          \"version\": {}\n        }}\n      }},\n      \"columnKind\": \"unicodeCodePoints\",\n      \"invocations\": [\n        {{\n          \"executionSuccessful\": {}\n        }}\n      ],\n      \"results\": {results}\n    }}\n  ]\n}}\n",
        json_string(version),
        if failed { "false" } else { "true" }
    )
}
