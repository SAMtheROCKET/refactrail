//! NFKC normalization of identifiers, as CPython applies it to every
//! non-ASCII name (PEP 3131). Implements the Unicode normalization
//! algorithm (UAX #15) over tables generated from CPython's own
//! `unicodedata` by scripts/gen_nfkc_tables.py.

use crate::nfkc_tables::{COMBINING, COMPOSITIONS, DECOMPOSITIONS, POOL};
use std::borrow::Cow;

const S_BASE: u32 = 0xAC00;
const L_BASE: u32 = 0x1100;
const V_BASE: u32 = 0x1161;
const T_BASE: u32 = 0x11A7;
const L_COUNT: u32 = 19;
const V_COUNT: u32 = 21;
const T_COUNT: u32 = 28;
const N_COUNT: u32 = V_COUNT * T_COUNT;
const S_COUNT: u32 = L_COUNT * N_COUNT;

/// `unicodedata.normalize("NFKC", text)`; ASCII text is returned as is.
pub fn nfkc(text: &str) -> Cow<'_, str> {
    if text.is_ascii() {
        return Cow::Borrowed(text);
    }
    let mut chars = Vec::with_capacity(text.len());
    for char in text.chars() {
        decompose(char as u32, &mut chars);
    }
    reorder(&mut chars);
    compose(&mut chars);
    let normalized: String = chars.iter().filter_map(|&code| char::from_u32(code)).collect();
    if normalized == text {
        Cow::Borrowed(text)
    } else {
        Cow::Owned(normalized)
    }
}

fn decompose(code: u32, out: &mut Vec<u32>) {
    if (S_BASE..S_BASE + S_COUNT).contains(&code) {
        let index = code - S_BASE;
        out.push(L_BASE + index / N_COUNT);
        out.push(V_BASE + (index % N_COUNT) / T_COUNT);
        if index % T_COUNT != 0 {
            out.push(T_BASE + index % T_COUNT);
        }
        return;
    }
    match DECOMPOSITIONS.binary_search_by_key(&code, |row| row.0) {
        Ok(found) => {
            let (_, start, length) = DECOMPOSITIONS[found];
            out.extend_from_slice(&POOL[start as usize..(start + length) as usize]);
        }
        Err(_) => out.push(code),
    }
}

fn combining_class(code: u32) -> u32 {
    if code < 0x300 {
        return 0;
    }
    COMBINING.binary_search_by_key(&code, |row| row.0).map_or(0, |found| COMBINING[found].1)
}

/// Canonical ordering: a stable sort of each run of nonzero classes.
fn reorder(chars: &mut [u32]) {
    let mut start = 0;
    while start < chars.len() {
        if combining_class(chars[start]) == 0 {
            start += 1;
            continue;
        }
        let mut end = start;
        while end < chars.len() && combining_class(chars[end]) != 0 {
            end += 1;
        }
        chars[start..end].sort_by_key(|&code| combining_class(code));
        start = end;
    }
}

fn composite(first: u32, second: u32) -> Option<u32> {
    if (L_BASE..L_BASE + L_COUNT).contains(&first) && (V_BASE..V_BASE + V_COUNT).contains(&second) {
        return Some(S_BASE + ((first - L_BASE) * V_COUNT + (second - V_BASE)) * T_COUNT);
    }
    if (S_BASE..S_BASE + S_COUNT).contains(&first)
        && (first - S_BASE) % T_COUNT == 0
        && (T_BASE + 1..T_BASE + T_COUNT).contains(&second)
    {
        return Some(first + (second - T_BASE));
    }
    COMPOSITIONS.binary_search_by_key(&(first, second), |row| (row.0, row.1)).ok().map(|found| COMPOSITIONS[found].2)
}

/// Canonical composition (the UAX #15 reference algorithm).
fn compose(chars: &mut Vec<u32>) {
    if chars.is_empty() {
        return;
    }
    let mut starter = 0;
    let mut last_class = combining_class(chars[0]);
    if last_class != 0 {
        last_class = 256;
    }
    let mut written = 1;
    for index in 1..chars.len() {
        let code = chars[index];
        let class = combining_class(code);
        if let Some(combined) = composite(chars[starter], code) {
            if last_class < class || last_class == 0 {
                chars[starter] = combined;
                continue;
            }
        }
        if class == 0 {
            starter = written;
        }
        last_class = class;
        chars[written] = code;
        written += 1;
    }
    chars.truncate(written);
}
