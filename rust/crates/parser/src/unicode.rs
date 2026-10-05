//! Unicode data generated from CPython 3.12 (Unicode 15.0) by
//! scripts/gen_unicode_tables.py.

use std::collections::HashMap;
use std::sync::OnceLock;

use refactrail_lexer::Version;

const NONPRINTABLE: &str = include_str!("../data/nonprintable.txt");
const NONPRINTABLE_313: &str = include_str!("../data/nonprintable_313.txt");
const NONPRINTABLE_314: &str = include_str!("../data/nonprintable_314.txt");
/// Python 3.12's names (Unicode 15.0); later versions only add names.
const NAMES: &str = include_str!("../data/names.txt");
const NAMES_ADDED_313: &str = include_str!("../data/names_added_313.txt");
const NAMES_ADDED_314: &str = include_str!("../data/names_added_314.txt");
/// CJK Unified Ideographs Extension I (Unicode 15.1, Python 3.13).
const CJK_EXTENSION_I: (u32, u32) = (0x2EBF0, 0x2EE5D);

/// CJK unified ideograph blocks of Unicode 15.0.
const CJK_RANGES: [(u32, u32); 9] = [
    (0x3400, 0x4DBF),
    (0x4E00, 0x9FFF),
    (0x20000, 0x2A6DF),
    (0x2A700, 0x2B739),
    (0x2B740, 0x2B81D),
    (0x2B820, 0x2CEA1),
    (0x2CEB0, 0x2EBE0),
    (0x30000, 0x3134A),
    (0x31350, 0x323AF),
];

const JAMO_L: [&str; 19] = [
    "G", "GG", "N", "D", "DD", "R", "M", "B", "BB", "S", "SS", "", "J", "JJ", "C", "K", "T", "P", "H",
];
const JAMO_V: [&str; 21] = [
    "A", "AE", "YA", "YAE", "EO", "E", "YEO", "YE", "O", "WA", "WAE", "OE", "YO", "U", "WEO", "WE",
    "WI", "YU", "EU", "YI", "I",
];
const JAMO_T: [&str; 28] = [
    "", "G", "GG", "GS", "N", "NJ", "NH", "D", "L", "LG", "LM", "LB", "LS", "LT", "LP", "LH", "M",
    "B", "BS", "S", "SS", "NG", "J", "C", "K", "T", "P", "H",
];

fn parse_ranges(text: &str) -> Vec<(u32, u32)> {
    text.lines()
        .filter_map(|line| {
            let (start, end) = line.split_once(' ')?;
            Some((u32::from_str_radix(start, 16).ok()?, u32::from_str_radix(end, 16).ok()?))
        })
        .collect()
}

fn nonprintable_ranges(version: Version) -> &'static Vec<(u32, u32)> {
    static RANGES: [OnceLock<Vec<(u32, u32)>>; 3] = [OnceLock::new(), OnceLock::new(), OnceLock::new()];
    let (index, text) = match version {
        Version::Py312 => (0, NONPRINTABLE),
        Version::Py313 => (1, NONPRINTABLE_313),
        Version::Py314 => (2, NONPRINTABLE_314),
    };
    RANGES[index].get_or_init(|| parse_ranges(text))
}

/// `str.isprintable()` for one code point in a given Python version.
pub fn is_printable_version(point: u32, version: Version) -> bool {
    let ranges = nonprintable_ranges(version);
    let index = ranges.partition_point(|&(start, _)| start <= point);
    !(index > 0 && point <= ranges[index - 1].1)
}

fn parse_names(text: &'static str) -> HashMap<&'static str, u32> {
    text.lines()
        .filter_map(|line| {
            let (name, code) = line.split_once(';')?;
            Some((name, u32::from_str_radix(code, 16).ok()?))
        })
        .collect()
}

/// Names of one table: Python 3.12's, or those 3.13 or 3.14 added.
fn names(table: usize) -> &'static HashMap<&'static str, u32> {
    static TABLES: [OnceLock<HashMap<&'static str, u32>>; 3] = [OnceLock::new(), OnceLock::new(), OnceLock::new()];
    TABLES[table].get_or_init(|| parse_names([NAMES, NAMES_ADDED_313, NAMES_ADDED_314][table]))
}

/// Resolve a `\N{...}` name (case-insensitive) as a given Python
/// version's `unicodedata.lookup`.
pub fn lookup_version(name: &str, version: Version) -> Option<u32> {
    let upper = name.to_ascii_uppercase();
    if let Some(hex) = upper.strip_prefix("CJK UNIFIED IDEOGRAPH-") {
        if !(4..=5).contains(&hex.len()) {
            return None;
        }
        let point = u32::from_str_radix(hex, 16).ok()?;
        let extension_i = version >= Version::Py313 && (CJK_EXTENSION_I.0..=CJK_EXTENSION_I.1).contains(&point);
        return (extension_i || CJK_RANGES.iter().any(|&(start, end)| (start..=end).contains(&point))).then_some(point);
    }
    if let Some(rest) = upper.strip_prefix("HANGUL SYLLABLE ") {
        return hangul(rest);
    }
    let tables = match version {
        Version::Py312 => 1,
        Version::Py313 => 2,
        Version::Py314 => 3,
    };
    (0..tables).find_map(|table| names(table).get(upper.as_str()).copied())
}

fn hangul(rest: &str) -> Option<u32> {
    // Longest-match decomposition, as CPython's find_syllable does.
    let take = |text: &str, table: &[&str]| -> Option<(usize, usize)> {
        let mut best: Option<(usize, usize)> = None;
        for (index, jamo) in table.iter().enumerate() {
            if text.starts_with(jamo) && best.map_or(true, |(_, length)| jamo.len() > length) {
                best = Some((index, jamo.len()));
            }
        }
        best
    };
    let (l, l_len) = take(rest, &JAMO_L)?;
    let (v, v_len) = take(&rest[l_len..], &JAMO_V)?;
    let (t, t_len) = take(&rest[l_len + v_len..], &JAMO_T)?;
    if l_len + v_len + t_len != rest.len() {
        return None;
    }
    Some(0xAC00 + ((l * 21 + v) * 28 + t) as u32)
}
