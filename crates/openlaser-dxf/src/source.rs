// SPDX-License-Identifier: GPL-3.0-or-later

//! The file as text. `AutoCAD` 2007 and later write UTF-8; earlier versions
//! write the Windows code page `$DWGCODEPAGE` names, decoded here rather
//! than refused. A binary DXF holds the same groups as bytes, and is
//! rewritten as the group code and value lines of an ASCII one.

use crate::{Error, Result};
use encoding_rs::Encoding;
use std::borrow::Cow;
use std::fmt::Write as _;

/// How a binary DXF starts.
const BINARY: &[u8] = b"AutoCAD Binary DXF\r\n\x1a\0";

/// The file's text, decoded.
pub(crate) fn text(bytes: &[u8]) -> Result<Cow<'_, str>> {
    if bytes.starts_with(BINARY) {
        return binary(bytes).map(Cow::Owned);
    }
    if let Ok(text) = std::str::from_utf8(bytes) {
        return Ok(Cow::Borrowed(text));
    }
    let encoding = code_page(ascii_header(bytes, b"$DWGCODEPAGE").unwrap_or_default());
    Ok(Cow::Owned(encoding.decode_without_bom_handling(bytes).0.into_owned()))
}

/// The encoding a `$DWGCODEPAGE` value names; Western European when it
/// names none this knows, as most files without one are.
fn code_page(name: &str) -> &'static Encoding {
    match name.trim().to_ascii_uppercase().as_str() {
        "ANSI_874" => encoding_rs::WINDOWS_874,
        "ANSI_932" => encoding_rs::SHIFT_JIS,
        "ANSI_936" => encoding_rs::GBK,
        "ANSI_949" => encoding_rs::EUC_KR,
        "ANSI_950" => encoding_rs::BIG5,
        "ANSI_1250" => encoding_rs::WINDOWS_1250,
        "ANSI_1251" => encoding_rs::WINDOWS_1251,
        "ANSI_1253" => encoding_rs::WINDOWS_1253,
        "ANSI_1254" => encoding_rs::WINDOWS_1254,
        "ANSI_1255" => encoding_rs::WINDOWS_1255,
        "ANSI_1256" => encoding_rs::WINDOWS_1256,
        "ANSI_1257" => encoding_rs::WINDOWS_1257,
        "ANSI_1258" => encoding_rs::WINDOWS_1258,
        _ => encoding_rs::WINDOWS_1252,
    }
}

/// A header variable's value in an ASCII file: the line after the group
/// code that follows its name. Header names and values are plain ASCII.
fn ascii_header<'a>(bytes: &'a [u8], name: &[u8]) -> Option<&'a str> {
    let at = bytes.windows(name.len()).position(|w| w == name)? + name.len();
    let mut lines = bytes[at..].split(|b| *b == b'\n').skip(1);
    lines.next()?;
    std::str::from_utf8(lines.next()?).ok().map(str::trim)
}

/// A header variable's text value in a binary file, found as ezdxf finds
/// it: past the name and its terminator, then a one- or two-byte group code.
fn binary_header(bytes: &[u8], name: &[u8]) -> Option<String> {
    let head = &bytes[..bytes.len().min(1024)];
    let mut at = head.windows(name.len()).position(|w| w == name)? + name.len() + 2;
    if *head.get(at)? != b'A' {
        at += 1;
    }
    let end = at + head.get(at..)?.iter().position(|b| *b == 0)?;
    String::from_utf8(head[at..end].to_vec()).ok()
}

/// How the value of a binary group is stored.
enum Value {
    Text,
    Double,
    Int16,
    Int32,
    Int64,
    Byte,
    Chunk,
}

/// The storage of group `code`, by the DXF reference's value type ranges.
fn value(code: u16) -> Value {
    match code {
        10..=59 | 110..=149 | 210..=239 | 460..=469 | 1010..=1059 => Value::Double,
        60..=79 | 170..=179 | 270..=289 | 370..=389 | 400..=409 | 1060..=1070 => Value::Int16,
        90..=99 | 420..=429 | 440..=459 | 1071 => Value::Int32,
        160..=169 => Value::Int64,
        290..=299 => Value::Byte,
        310..=319 | 1004 => Value::Chunk,
        _ => Value::Text,
    }
}

/// A binary DXF rewritten as ASCII group code and value lines.
fn binary(bytes: &[u8]) -> Result<String> {
    let version = binary_header(bytes, b"$ACADVER").unwrap_or_else(|| "AC1009".into());
    // Release 12 writes one-byte group codes, 255 escaping a two-byte one.
    let short = version.as_str() <= "AC1009";
    let encoding = if version.as_str() >= "AC1021" {
        encoding_rs::UTF_8
    } else {
        code_page(&binary_header(bytes, b"$DWGCODEPAGE").unwrap_or_default())
    };
    let mut out = String::with_capacity(bytes.len() * 2);
    let mut at = BINARY.len();
    let mut groups = 0usize;
    let truncated = |groups: usize| Error::Syntax {
        line: groups * 2 + 1,
        reason: "the binary DXF ends in the middle of a group",
    };
    let take = |at: &mut usize, n: usize, groups: usize| -> Result<&[u8]> {
        let slice = bytes.get(*at..*at + n).ok_or_else(|| truncated(groups))?;
        *at += n;
        Ok(slice)
    };
    while at < bytes.len() {
        let code = if short {
            match take(&mut at, 1, groups)?[0] {
                255 => {
                    let b = take(&mut at, 2, groups)?;
                    u16::from_le_bytes([b[0], b[1]])
                }
                c => u16::from(c),
            }
        } else {
            let b = take(&mut at, 2, groups)?;
            u16::from_le_bytes([b[0], b[1]])
        };
        let _ = writeln!(out, "{code}");
        match value(code) {
            Value::Double => {
                let b = take(&mut at, 8, groups)?;
                let v = f64::from_le_bytes(b.try_into().map_err(|_| truncated(groups))?);
                let _ = writeln!(out, "{v:?}");
            }
            Value::Int16 => {
                let b = take(&mut at, 2, groups)?;
                let _ = writeln!(out, "{}", i16::from_le_bytes([b[0], b[1]]));
            }
            Value::Int32 => {
                let b = take(&mut at, 4, groups)?;
                let _ = writeln!(
                    out,
                    "{}",
                    i32::from_le_bytes(b.try_into().map_err(|_| truncated(groups))?)
                );
            }
            Value::Int64 => {
                let b = take(&mut at, 8, groups)?;
                let _ = writeln!(
                    out,
                    "{}",
                    i64::from_le_bytes(b.try_into().map_err(|_| truncated(groups))?)
                );
            }
            Value::Byte => {
                let _ = writeln!(out, "{}", take(&mut at, 1, groups)?[0]);
            }
            Value::Chunk => {
                let length = usize::from(take(&mut at, 1, groups)?[0]);
                for byte in take(&mut at, length, groups)? {
                    let _ = write!(out, "{byte:02X}");
                }
                out.push('\n');
            }
            Value::Text => {
                let end =
                    bytes[at..].iter().position(|b| *b == 0).ok_or_else(|| truncated(groups))?;
                let text = encoding.decode_without_bom_handling(&bytes[at..at + end]).0;
                // A value is one line; a stray line break would split it.
                out.push_str(&text.replace(['\r', '\n'], " "));
                out.push('\n');
                at += end + 1;
            }
        }
        groups += 1;
        if code == 0 && out.ends_with("\nEOF\n") {
            break;
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A binary DXF in the two-byte group code form, as `AutoCAD` 2000 and
    /// later write it.
    fn two_byte(groups: &[(u16, &[u8])]) -> Vec<u8> {
        let mut bytes = BINARY.to_vec();
        for (code, value) in groups {
            bytes.extend_from_slice(&code.to_le_bytes());
            bytes.extend_from_slice(value);
        }
        bytes
    }

    #[test]
    fn binary_groups_become_lines_by_their_value_type() {
        let text = two_byte(&[
            (0, b"SECTION\0"),
            (9, b"$ACADVER\0"),
            (1, b"AC1015\0"),
            (10, &2.5f64.to_le_bytes()),
            (70, &7i16.to_le_bytes()),
            (90, &(-3i32).to_le_bytes()),
            (290, &[1]),
            (310, &[2, 0xAB, 0x01]),
            (0, b"EOF\0"),
        ]);
        let text = self::text(&text).unwrap();
        assert_eq!(
            text,
            "0\nSECTION\n9\n$ACADVER\n1\nAC1015\n10\n2.5\n70\n7\n90\n-3\n290\n1\n310\nAB01\n0\nEOF\n"
        );
        assert!(matches!(self::text(&two_byte(&[(10, &[1, 2])])), Err(Error::Syntax { .. })));
    }

    #[test]
    fn older_files_are_decoded_in_their_code_page() {
        let mut bytes =
            b"0\nSECTION\n2\nHEADER\n9\n$DWGCODEPAGE\n3\nANSI_1251\n0\nENDSEC\n1\n".to_vec();
        bytes.push(0xC4); // Д in Windows-1251
        assert!(text(&bytes).unwrap().ends_with("1\nД"));
        let western = b"1\nBlech \xD83\n".to_vec(); // Ø in Windows-1252, no code page named
        assert_eq!(text(&western).unwrap(), "1\nBlech Ø3\n");
    }
}
