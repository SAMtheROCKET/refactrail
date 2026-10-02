//! An immutable string that keeps up to 22 bytes inline, so most
//! identifiers and number literals need no heap allocation.

use std::fmt;
use std::ops::Deref;

const INLINE: usize = 22;

#[derive(Clone)]
pub enum SmallStr {
    Inline { length: u8, bytes: [u8; INLINE] },
    Heap(Box<str>),
}

impl SmallStr {
    pub fn as_str(&self) -> &str {
        match self {
            // SAFETY: inline bytes are always copied from a valid &str of
            // exactly `length` bytes.
            SmallStr::Inline { length, bytes } => unsafe { std::str::from_utf8_unchecked(&bytes[..*length as usize]) },
            SmallStr::Heap(text) => text,
        }
    }
}

impl From<&str> for SmallStr {
    #[inline]
    fn from(text: &str) -> SmallStr {
        if text.len() <= INLINE {
            let mut bytes = [0; INLINE];
            bytes[..text.len()].copy_from_slice(text.as_bytes());
            SmallStr::Inline { length: text.len() as u8, bytes }
        } else {
            SmallStr::Heap(text.into())
        }
    }
}

impl From<String> for SmallStr {
    fn from(text: String) -> SmallStr {
        if text.len() <= INLINE {
            SmallStr::from(text.as_str())
        } else {
            SmallStr::Heap(text.into_boxed_str())
        }
    }
}

impl Deref for SmallStr {
    type Target = str;

    #[inline]
    fn deref(&self) -> &str {
        self.as_str()
    }
}

impl std::borrow::Borrow<str> for SmallStr {
    fn borrow(&self) -> &str {
        self.as_str()
    }
}

impl AsRef<str> for SmallStr {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl PartialEq for SmallStr {
    fn eq(&self, other: &SmallStr) -> bool {
        self.as_str() == other.as_str()
    }
}

impl Eq for SmallStr {}

impl PartialEq<str> for SmallStr {
    fn eq(&self, other: &str) -> bool {
        self.as_str() == other
    }
}

impl std::hash::Hash for SmallStr {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.as_str().hash(state);
    }
}

impl fmt::Display for SmallStr {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self.as_str(), formatter)
    }
}

impl fmt::Debug for SmallStr {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self.as_str(), formatter)
    }
}
