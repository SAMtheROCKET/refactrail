//! Print `ast.dump(ast.parse(file), include_attributes=True)` for each
//! file named on the command line, one line each, after a "=== path"
//! header when several files are given. Set RT_DUMP_VERBOSE to print the
//! error message as a "# ..." line, and RT_DUMP_NO_ATTRIBUTES to leave
//! positions out.

use std::io::Write;

/// RT_PARSE_ONLY: read every file first, then time parsing alone.
fn time_parsing(paths: &[String]) {
    let sources: Vec<String> = paths
        .iter()
        .filter_map(|path| std::fs::read(path).ok().and_then(|bytes| String::from_utf8(bytes).ok()))
        .collect();
    let bytes: usize = sources.iter().map(String::len).sum();
    let started = std::time::Instant::now();
    let mut failures = 0;
    for source in &sources {
        let source = source.strip_prefix('\u{feff}').unwrap_or(source);
        if refactrail_parser::parse(source).is_err() {
            failures += 1;
        }
    }
    let seconds = started.elapsed().as_secs_f64();
    println!(
        "parsed {} files ({:.1} MB) in {:.3} s ({:.1} MB/s), {} failed",
        sources.len(),
        bytes as f64 / 1e6,
        seconds,
        bytes as f64 / 1e6 / seconds,
        failures
    );
}

fn main() {
    let mut paths: Vec<String> = std::env::args().skip(1).collect();
    // "@FILE" reads the paths, one per line, from FILE.
    if let [single] = paths.as_slice() {
        if let Some(list) = single.strip_prefix('@') {
            paths = std::fs::read_to_string(list)
                .unwrap_or_default()
                .lines()
                .filter(|line| !line.is_empty())
                .map(String::from)
                .collect();
        }
    }
    if std::env::var_os("RT_PARSE_ONLY").is_some() {
        time_parsing(&paths);
        return;
    }
    let attributes = std::env::var_os("RT_DUMP_NO_ATTRIBUTES").is_none();
    let verbose = std::env::var_os("RT_DUMP_VERBOSE").is_some();
    let stdout = std::io::stdout();
    let mut out = std::io::BufWriter::new(stdout.lock());
    for path in &paths {
        if paths.len() > 1 {
            let _ = writeln!(out, "=== {path}");
        }
        let text = match std::fs::read(path).map(String::from_utf8) {
            Ok(Ok(text)) => text,
            _ => {
                let _ = writeln!(out, "UNDECODABLE");
                continue;
            }
        };
        let source = text.strip_prefix('\u{feff}').unwrap_or(&text);
        if std::env::var_os("RT_DUMP_NFKC").is_some() {
            for line in text.split('\n') {
                let _ = writeln!(out, "{}", refactrail_parser::nfkc::nfkc(line));
            }
            continue;
        }
        if std::env::var_os("RT_DUMP_SYMTABLE").is_some() {
            let _ = writeln!(out, "{}", refactrail_parser::dump_symtable_source(source));
            continue;
        }
        if std::env::var_os("RT_DUMP_COMPILE").is_some() {
            let _ = match refactrail_parser::compile_check(source) {
                None => writeln!(out, "OK"),
                Some((line, offset, message)) => writeln!(out, "ERROR {line} {offset} {message}"),
            };
            continue;
        }
        match refactrail_parser::parse(source) {
            Ok(tree) => {
                let _ = writeln!(out, "{}", refactrail_parser::dump_module(&tree, attributes));
            }
            Err(error) => {
                if verbose {
                    let _ = writeln!(out, "# {error}");
                }
                let _ = writeln!(out, "ERROR {}", error.line);
            }
        }
    }
}
