//! Print the token dump of each file named on the command line, in the
//! format of scripts/dump_tokens.py. With several files, each dump is
//! preceded by a "=== path" line.

use std::io::Write;

fn main() {
    let paths: Vec<String> = std::env::args().skip(1).collect();
    let stdout = std::io::stdout();
    let mut out = std::io::BufWriter::new(stdout.lock());
    for path in &paths {
        if paths.len() > 1 {
            let _ = writeln!(out, "=== {path}");
        }
        let text = match std::fs::read(path) {
            Ok(bytes) => match String::from_utf8(bytes) {
                Ok(text) => text,
                Err(_) => {
                    let _ = writeln!(out, "UNDECODABLE");
                    continue;
                }
            },
            Err(error) => {
                let _ = writeln!(out, "UNREADABLE {error}");
                continue;
            }
        };
        let source = text.strip_prefix('\u{feff}').unwrap_or(&text);
        if std::env::var_os("RT_DUMP_VERBOSE").is_some() {
            if let Err(error) = refactrail_lexer::tokenize(source) {
                let _ = writeln!(out, "# {error}");
            }
        }
        let _ = out.write_all(refactrail_lexer::dump(source).as_bytes());
    }
}
