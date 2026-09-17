// SPDX-License-Identifier: GPL-3.0-or-later

//! The file as group code and value pairs.

use crate::{Error, Result};

/// One group: its code, its value, and the line the code is on.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Pair<'a> {
    pub code: i32,
    pub value: &'a str,
    pub line: usize,
}

/// Splits the text into pairs. Every group code line must have a value
/// line after it.
pub(crate) fn tokenize(text: &str) -> Result<Vec<Pair<'_>>> {
    let mut pairs = Vec::new();
    let mut lines = text.lines().enumerate();
    while let Some((index, code)) = lines.next() {
        let line = index + 1;
        let code = code
            .trim()
            .parse::<i32>()
            .map_err(|_| Error::Syntax { line, reason: "expected a group code" })?;
        let (_, value) = lines
            .next()
            .ok_or(Error::Syntax { line, reason: "a group code needs a value line" })?;
        // Text chunks can contain meaningful indentation and trailing spaces.
        let value = if matches!(code, 1 | 3) { value } else { value.trim() };
        pairs.push(Pair { code, value, line });
    }
    Ok(pairs)
}

/// The index of the next pair with code 0 at or after `from`, or the end.
pub(crate) fn next_entity(pairs: &[Pair<'_>], from: usize) -> usize {
    pairs[from..].iter().position(|p| p.code == 0).map_or(pairs.len(), |n| from + n)
}
