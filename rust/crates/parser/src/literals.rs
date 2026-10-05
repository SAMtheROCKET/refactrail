//! Evaluation of number and string literal tokens, as CPython's parser
//! does when it builds `ast.Constant` values.

use crate::node::Constant;
use crate::unicode;

/// Convert a NUMBER token to its constant.
pub fn number(text: &str) -> Result<Constant, String> {
    let clean: String = text.chars().filter(|&c| c != '_').collect();
    let lower = clean.to_ascii_lowercase();
    if let Some(body) = lower.strip_suffix('j') {
        return parse_float(body).map(Constant::Complex);
    }
    let radix = match lower.get(..2) {
        Some("0x") => 16,
        Some("0o") => 8,
        Some("0b") => 2,
        _ => 10,
    };
    if radix == 10 && (lower.contains('.') || lower.contains('e')) {
        return parse_float(&lower).map(Constant::Float);
    }
    let digits = if radix == 10 { &lower[..] } else { &lower[2..] };
    Ok(Constant::Int(to_decimal(digits, radix).into()))
}

fn parse_float(text: &str) -> Result<f64, String> {
    text.parse::<f64>().map_err(|_| format!("invalid float literal {text}"))
}

/// Digits in `radix` to a decimal string, for ints of any size.
fn to_decimal(digits: &str, radix: u32) -> String {
    // Little-endian limbs of base 10^9.
    let mut limbs: Vec<u64> = vec![0];
    for c in digits.chars() {
        let mut carry = c.to_digit(radix).unwrap_or(0) as u64;
        for limb in limbs.iter_mut() {
            let value = *limb * radix as u64 + carry;
            *limb = value % 1_000_000_000;
            carry = value / 1_000_000_000;
        }
        while carry > 0 {
            limbs.push(carry % 1_000_000_000);
            carry /= 1_000_000_000;
        }
    }
    let mut text = limbs.last().copied().unwrap_or(0).to_string();
    for limb in limbs.iter().rev().skip(1) {
        text.push_str(&format!("{limb:09}"));
    }
    text
}

/// A string token split into its prefix flags and body.
pub struct StringParts<'a> {
    pub raw: bool,
    pub bytes: bool,
    pub unicode_kind: bool,
    pub body: &'a str,
}

/// Split a STRING token into prefix flags and the text between quotes.
pub fn split_string(text: &str) -> StringParts<'_> {
    let quote_at = text.find(['\'', '"']).unwrap_or(0);
    let prefix = text[..quote_at].to_ascii_lowercase();
    let rest = &text[quote_at..];
    let quote_len = if rest.len() >= 6 && (rest.starts_with("'''") || rest.starts_with("\"\"\"")) { 3 } else { 1 };
    let body = &rest[quote_len..rest.len().saturating_sub(quote_len).max(quote_len)];
    StringParts {
        raw: prefix.contains('r'),
        bytes: prefix.contains('b'),
        // CPython marks only a lowercase `u` prefix (`U'x'` has no kind).
        unicode_kind: text[..quote_at].contains('u'),
        body,
    }
}

/// CPython reads source with universal newlines.
pub fn normalize_newlines(text: &str) -> std::borrow::Cow<'_, str> {
    if text.contains('\r') {
        std::borrow::Cow::Owned(text.replace("\r\n", "\n").replace('\r', "\n"))
    } else {
        std::borrow::Cow::Borrowed(text)
    }
}

/// Decode the body of a str literal (or f-string text) to code points.
pub fn decode_str(body: &str, raw: bool) -> Result<Vec<u32>, String> {
    decode_str_version(body, raw, refactrail_lexer::Version::default())
}

/// `decode_str` with a given Python version's Unicode names.
pub fn decode_str_version(body: &str, raw: bool, version: refactrail_lexer::Version) -> Result<Vec<u32>, String> {
    let body = normalize_newlines(body);
    if raw || !body.contains('\\') {
        if body.is_ascii() {
            return Ok(body.bytes().map(u32::from).collect());
        }
        let mut out = Vec::with_capacity(body.len());
        out.extend(body.chars().map(|c| c as u32));
        return Ok(out);
    }
    let chars: Vec<char> = body.chars().collect();
    // Positions are only needed for an error message.
    let decode_error = |start: usize, stop: usize, reason: &str| -> String {
        let positions = escape_positions(&chars);
        format!(
            "(unicode error) 'unicodeescape' codec can't decode bytes in position {}-{}: {reason}",
            positions[start],
            positions[stop] - 1
        )
    };
    let mut out = Vec::with_capacity(chars.len());
    let mut index = 0;
    while index < chars.len() {
        let c = chars[index];
        let escape_start = index;
        index += 1;
        if c != '\\' {
            out.push(c as u32);
            continue;
        }
        let Some(&next) = chars.get(index) else {
            out.push('\\' as u32);
            break;
        };
        index += 1;
        match next {
            '\n' => {}
            '\\' | '\'' | '"' => out.push(next as u32),
            'a' => out.push(7),
            'b' => out.push(8),
            'f' => out.push(12),
            'n' => out.push(10),
            'r' => out.push(13),
            't' => out.push(9),
            'v' => out.push(11),
            '0'..='7' => {
                let mut value = next.to_digit(8).unwrap_or(0);
                for _ in 0..2 {
                    match chars.get(index).and_then(|c| c.to_digit(8)) {
                        Some(digit) => {
                            value = value * 8 + digit;
                            index += 1;
                        }
                        None => break,
                    }
                }
                out.push(value);
            }
            'x' | 'u' | 'U' => {
                let (count, reason) = match next {
                    'x' => (2, "truncated \\xXX escape"),
                    'u' => (4, "truncated \\uXXXX escape"),
                    _ => (8, "truncated \\UXXXXXXXX escape"),
                };
                let digits = chars[index..].iter().take(count).take_while(|c| c.is_ascii_hexdigit()).count();
                if digits != count {
                    return Err(decode_error(escape_start, index + digits, reason));
                }
                let hex: String = chars[index..index + count].iter().collect();
                let value = u32::from_str_radix(&hex, 16).unwrap_or(0);
                index += count;
                if value > 0x10FFFF {
                    return Err(decode_error(escape_start, index, "illegal Unicode character"));
                }
                out.push(value);
            }
            'N' => {
                let malformed = "malformed \\N character escape";
                if chars.get(index) != Some(&'{') {
                    return Err(decode_error(escape_start, index, malformed));
                }
                let Some(close) = chars[index..].iter().position(|&c| c == '}') else {
                    return Err(decode_error(escape_start, chars.len(), malformed));
                };
                if close == 1 {
                    return Err(decode_error(escape_start, index + 1, malformed));
                }
                let name: String = chars[index + 1..index + close].iter().collect();
                let Some(point) = unicode::lookup_version(&name, version) else {
                    return Err(decode_error(escape_start, index + close + 1, "unknown Unicode character name"));
                };
                index += close + 1;
                out.push(point);
            }
            _ => {
                out.push('\\' as u32);
                out.push(next as u32);
            }
        }
    }
    Ok(out)
}

/// Positions of each character in the buffer CPython's escape decoder
/// sees: non-ASCII characters become 10-byte `\\U` escapes and a backslash
/// before one becomes `\\u005c`. Has one extra entry for the end.
fn escape_positions(chars: &[char]) -> Vec<usize> {
    let mut positions = Vec::with_capacity(chars.len() + 1);
    let mut position = 0;
    for (index, &c) in chars.iter().enumerate() {
        positions.push(position);
        position += if c.is_ascii() { 1 } else { 10 };
        if c == '\\' && chars.get(index + 1).map_or(true, |next| !next.is_ascii()) {
            position += 5;
        }
    }
    positions.push(position);
    positions
}

/// Decode the body of a bytes literal.
pub fn decode_bytes(body: &str, raw: bool) -> Result<Vec<u8>, String> {
    if !body.is_ascii() {
        return Err("bytes can only contain ASCII literal characters".into());
    }
    let body = normalize_newlines(body);
    let bytes = body.as_bytes();
    if raw {
        return Ok(bytes.to_vec());
    }
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        let byte = bytes[index];
        index += 1;
        if byte != b'\\' {
            out.push(byte);
            continue;
        }
        let Some(&next) = bytes.get(index) else {
            out.push(b'\\');
            break;
        };
        index += 1;
        match next {
            b'\n' => {}
            b'\\' | b'\'' | b'"' => out.push(next),
            b'a' => out.push(7),
            b'b' => out.push(8),
            b'f' => out.push(12),
            b'n' => out.push(10),
            b'r' => out.push(13),
            b't' => out.push(9),
            b'v' => out.push(11),
            b'0'..=b'7' => {
                let mut value = (next - b'0') as u32;
                for _ in 0..2 {
                    match bytes.get(index) {
                        Some(&digit @ b'0'..=b'7') => {
                            value = value * 8 + (digit - b'0') as u32;
                            index += 1;
                        }
                        _ => break,
                    }
                }
                out.push((value & 0xff) as u8);
            }
            b'x' => {
                let hex = bytes.get(index..index + 2).unwrap_or(b"");
                if hex.len() != 2 || !hex.iter().all(|b| b.is_ascii_hexdigit()) {
                    return Err(format!("(value error) invalid \\x escape at position {}", index - 2));
                }
                let text = std::str::from_utf8(hex).unwrap_or("0");
                out.push(u8::from_str_radix(text, 16).unwrap_or(0));
                index += 2;
            }
            _ => {
                out.push(b'\\');
                out.push(next);
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn big_ints_and_floats() {
        assert_eq!(number("0xFFFF_FFFF_FFFF_FFFF_FF").unwrap(), Constant::Int("4722366482869645213695".into()));
        assert_eq!(number("0o17").unwrap(), Constant::Int("15".into()));
        assert_eq!(number("000").unwrap(), Constant::Int("0".into()));
        assert_eq!(number("1_0.5e1").unwrap(), Constant::Float(105.0));
        assert_eq!(number("3J").unwrap(), Constant::Complex(3.0));
    }

    #[test]
    fn escapes() {
        let text: String = decode_str("a\\x41\\101\\N{DEGREE SIGN}\\q\\\nb", false)
            .unwrap().into_iter().map(|p| char::from_u32(p).unwrap()).collect();
        assert_eq!(text, "aAA\u{b0}\\qb");
        assert_eq!(decode_bytes("\\777\\x41", false).unwrap(), vec![0xff, 0x41]);
    }
}
