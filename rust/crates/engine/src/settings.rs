//! Settings passed from Python, and the shared word lists (embedded from
//! the same data files the Python engine reads).

use std::collections::HashSet;
use std::sync::LazyLock;

const STRICT_ONLY_CODES: [&str; 4] = ["RT203", "RT205", "RT206", "RT207"];
const ERROR_CODES: [&str; 6] = ["RT000", "RT001", "RT002", "RT101", "RT502", "RT503"];

/// Mirror of the Python `Settings` dataclass (fields used by rules).
#[derive(Clone, Debug)]
pub struct Settings {
    pub profile: String,
    pub line_length: usize,
    pub function_preferred_lines: usize,
    pub function_max_lines: usize,
    pub main_max_lines: usize,
    pub select: Vec<String>,
    pub ignore: Vec<String>,
}

impl Settings {
    pub fn strict(&self) -> bool {
        self.profile == "strict"
    }

    /// is_code_enabled_bool from models.py.
    pub fn is_enabled(&self, code: &str) -> bool {
        if ["RT000", "RT001", "RT002"].contains(&code) { return true; }
        if STRICT_ONLY_CODES.contains(&code) && !self.strict() {
            return false;
        }
        let selected = self.select.iter().any(|prefix| code.starts_with(prefix.as_str()));
        let ignored = self.ignore.iter().any(|prefix| code.starts_with(prefix.as_str()));
        selected && !ignored
    }
}

/// "error" for error codes, otherwise "warning".
pub fn severity_of(code: &str) -> &'static str {
    if ERROR_CODES.contains(&code) {
        "error"
    } else {
        "warning"
    }
}

fn load_words(text: &'static str) -> HashSet<&'static str> {
    text.lines()
        .filter(|line| !line.trim().is_empty() && !line.starts_with('#'))
        .map(str::trim)
        .collect()
}

pub static VERBS: LazyLock<HashSet<&'static str>> =
    LazyLock::new(|| load_words(include_str!("../../../../src/refactrail/data/verbs.txt")));
pub static GENERIC_NAMES: LazyLock<HashSet<&'static str>> = LazyLock::new(|| {
    load_words(include_str!("../../../../src/refactrail/data/generic_names.txt"))
});
pub static PROTOCOL_HOOKS: LazyLock<HashSet<&'static str>> = LazyLock::new(|| {
    load_words(include_str!("../../../../src/refactrail/data/protocol_hooks.txt"))
});

/// is_protocol_hook_bool from rules/context.py.
pub fn is_protocol_hook(name: &str) -> bool {
    PROTOCOL_HOOKS.iter().any(|hook| {
        name == *hook
            || hook
                .strip_suffix('*')
                .is_some_and(|prefix| name.starts_with(prefix))
    })
}
