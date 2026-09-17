// SPDX-License-Identifier: GPL-3.0-or-later

//! A parameter file, read losslessly.
//!
//! The vendor writes a small XML subset: one `ParameterRoot`, nested
//! elements with quoted attributes, no text, no entities beyond the five
//! predefined ones and numeric references. This parser accepts exactly that,
//! remembers where every attribute value sits in the original bytes, and
//! rejects anything else rather than guessing.

use crate::{Error, Result};
use std::collections::BTreeMap;
use std::ops::Range;

/// The attributes of one element, by name.
pub type Attributes = BTreeMap<String, String>;

/// Largest file accepted.
pub const MAX_BYTES: usize = 8 * 1024 * 1024;
const MAX_ELEMENTS: usize = 20_000;
const MAX_ATTRIBUTES: usize = 200_000;
const MAX_DEPTH: usize = 128;

/// Which of the vendor's parameter files a document is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Kind {
    /// The hardware parameters.
    Hardware,
    /// The layer parameters: the cutting recipes.
    Layer,
    /// The manual parameters: motion and operator settings.
    Manual,
    /// A machine backup, which holds everything.
    Backup,
}

impl Kind {
    /// The kind for a vendor file stem or our slug. A backup's stem ends
    /// in `backup`; the vendor puts the machine model before it.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "hardpara" | "hardware" => Some(Self::Hardware),
            "layerpara" | "layer" => Some(Self::Layer),
            "manupara" | "manual" => Some(Self::Manual),
            _ if value.ends_with("backup") => Some(Self::Backup),
            _ => None,
        }
    }

    /// The vendor's file stem.
    #[must_use]
    pub const fn slug(self) -> &'static str {
        match self {
            Self::Hardware => "hardpara",
            Self::Layer => "layerpara",
            Self::Manual => "manupara",
            Self::Backup => "backup",
        }
    }

    /// Which file a parameter group is written to.
    fn holding(group: &str) -> Self {
        if group.starts_with("LayerParam") || group.starts_with("CO2LayerParam") {
            Self::Layer
        } else if ["ManuParam", "ECParam"].contains(&group) {
            Self::Manual
        } else {
            Self::Hardware
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Element {
    path: String,
    attributes: Attributes,
    spans: BTreeMap<String, Range<usize>>,
}

/// One parameter file.
#[derive(Clone, PartialEq, Eq)]
pub struct Document {
    kind: Kind,
    original: Vec<u8>,
    elements: Vec<Element>,
}

impl std::fmt::Debug for Document {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Document")
            .field("kind", &self.kind)
            .field("bytes", &self.original.len())
            .field("elements", &self.elements.len())
            .finish()
    }
}

impl Document {
    /// Parses `bytes` as a file of `kind`.
    pub fn parse(kind: Kind, bytes: &[u8]) -> Result<Self> {
        if bytes.is_empty() || bytes.len() > MAX_BYTES {
            return Err(Error::Limit("size"));
        }
        let text = std::str::from_utf8(bytes)
            .map_err(|e| Error::Malformed { offset: e.valid_up_to(), reason: "not UTF-8" })?;
        let elements = parse_xml(text)?;
        if elements.first().is_none_or(|e| e.path != "/ParameterRoot") {
            return Err(Error::Malformed {
                offset: 0,
                reason: "the root element is not ParameterRoot",
            });
        }
        Ok(Self { kind, original: bytes.to_vec(), elements })
    }

    /// Which file this is.
    #[must_use]
    pub const fn kind(&self) -> Kind {
        self.kind
    }

    /// The bytes as read.
    #[must_use]
    pub fn original(&self) -> &[u8] {
        &self.original
    }

    /// Every path that carries attributes, in file order. A path listed
    /// twice cannot be read or edited.
    #[must_use]
    pub fn paths(&self) -> Vec<&str> {
        self.elements.iter().filter(|e| !e.attributes.is_empty()).map(|e| e.path.as_str()).collect()
    }

    fn element(&self, path: &str) -> Result<&Element> {
        let mut matches = self.elements.iter().filter(|e| e.path == path);
        let element = matches.next().ok_or_else(|| Error::Missing(path.to_owned()))?;
        if matches.next().is_some() {
            return Err(Error::Duplicate(path.to_owned()));
        }
        Ok(element)
    }

    /// The attributes at `path`, such as `/ParameterRoot/PManuParam/MC`.
    pub fn attributes(&self, path: &str) -> Result<&Attributes> {
        self.element(path).map(|e| &e.attributes)
    }

    /// A new document with one attribute value replaced in place. Every
    /// other byte is kept, and the result is parsed again before it is
    /// returned.
    pub fn with_attribute(&self, path: &str, attribute: &str, value: &str) -> Result<Self> {
        let element = self.element(path)?;
        let span = element
            .spans
            .get(attribute)
            .ok_or_else(|| Error::Missing(format!("{path}/@{attribute}")))?;
        if !value.chars().all(xml_char) {
            return Err(Error::Malformed {
                offset: span.start,
                reason: "invalid XML character in the new value",
            });
        }
        let escaped = escape(value);
        let mut bytes = Vec::with_capacity(self.original.len() + escaped.len());
        bytes.extend_from_slice(&self.original[..span.start]);
        bytes.extend_from_slice(escaped.as_bytes());
        bytes.extend_from_slice(&self.original[span.end..]);
        Self::parse(self.kind, &bytes)
    }
}

/// The parameter files of one machine, looked up together: a group is read
/// from the file that owns it, and from the backup when that file is absent.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Bundle {
    documents: BTreeMap<Kind, Document>,
}

impl Bundle {
    /// A bundle of one machine backup.
    #[must_use]
    pub fn from_backup(backup: Document) -> Self {
        let mut bundle = Self::default();
        bundle.insert(backup);
        bundle
    }

    /// Adds or replaces a file.
    pub fn insert(&mut self, document: Document) {
        self.documents.insert(document.kind(), document);
    }

    /// The file of `kind`, if present.
    #[must_use]
    pub fn document(&self, kind: Kind) -> Option<&Document> {
        self.documents.get(&kind)
    }

    /// The attributes of `tag` in parameter `group`, such as `MC` in
    /// `ManuParam`, from the file that owns the group or the backup.
    pub fn group(&self, group: &str, tag: &str) -> Result<&Attributes> {
        let path = format!("/ParameterRoot/P{group}/{tag}");
        for kind in [Kind::holding(group), Kind::Backup] {
            if let Some(document) = self.documents.get(&kind) {
                match document.attributes(&path) {
                    Ok(attributes) => return Ok(attributes),
                    Err(Error::Missing(_)) => {}
                    Err(error) => return Err(error),
                }
            }
        }
        Err(Error::Missing(format!("P{group}/{tag}")))
    }

    /// The laser the host was last set to, from the soft parameters: 0 for
    /// fiber, 1 for CO2, `None` when the group is absent.
    pub fn saved_laser_mode(&self) -> Result<Option<u32>> {
        match self.group("SoftParam", "SP") {
            Ok(attributes) => crate::attrs::Group::new(attributes, "SoftParam", "SP")
                .optional_uint("m_iEnableLaserType", 1)
                .map(Some),
            Err(Error::Missing(_)) => Ok(None),
            Err(error) => Err(error),
        }
    }
}

/// `value` as a quoted attribute value: the five entities, and numeric
/// references for line breaks and tabs.
pub(crate) fn escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
        .replace('\n', "&#10;")
        .replace('\r', "&#13;")
        .replace('\t', "&#9;")
}

// The parser --------------------------------------------------------------

fn malformed(offset: usize, reason: &'static str) -> Error {
    Error::Malformed { offset, reason }
}

fn xml_space(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\r' | b'\n')
}

fn xml_char(c: char) -> bool {
    matches!(c as u32, 9 | 10 | 13 | 0x20..=0xd7ff | 0xe000..=0xfffd | 0x1_0000..=0x10_ffff)
}

fn xml_name(name: &str) -> bool {
    let mut bytes = name.bytes();
    bytes.next().is_some_and(|v| v.is_ascii_alphabetic() || v == b'_')
        && bytes.all(|v| v.is_ascii_alphanumeric() || matches!(v, b'_' | b'-' | b'.'))
}

fn syntax(offset: usize, error: impl std::fmt::Display) -> Error {
    Error::Syntax { offset, reason: error.to_string() }
}

/// The slice reader borrows attribute values directly from its input. Derive
/// exact spans from those slices, including empty values and arbitrarily long
/// names or whitespace, without searching for a possibly repeated value.
fn source_span(text: &str, bytes: &[u8]) -> Result<Range<usize>> {
    let start = bytes
        .as_ptr()
        .addr()
        .checked_sub(text.as_ptr().addr())
        .filter(|&start| start <= text.len() && bytes.len() <= text.len() - start)
        .ok_or(malformed(0, "XML value does not refer to the source bytes"))?;
    Ok(start..start + bytes.len())
}

fn attributes(
    text: &str,
    source: quick_xml::events::attributes::Attributes<'_>,
    count: &mut usize,
) -> Result<(Attributes, BTreeMap<String, Range<usize>>)> {
    let mut values = Attributes::new();
    let mut spans = BTreeMap::new();
    for attribute in source {
        let attribute = attribute.map_err(|e| syntax(0, e))?;
        *count += 1;
        if *count > MAX_ATTRIBUTES {
            return Err(Error::Limit("attribute count"));
        }
        let name_span = source_span(text, attribute.key.as_ref())?;
        let name = &text[name_span.clone()];
        if name_span.start == 0 || !xml_space(text.as_bytes()[name_span.start - 1]) {
            return Err(malformed(name_span.start, "attributes must be separated by whitespace"));
        }
        if !xml_name(name) {
            return Err(malformed(name_span.start, "invalid attribute name"));
        }
        let span = source_span(text, &attribute.value)?;
        let raw = &text[span.clone()];
        if raw.contains('<') {
            return Err(malformed(span.start, "unescaped less-than in attribute"));
        }
        // Retain the vendor reader's literal whitespace, which XML attribute
        // normalization would change to spaces, while checking every entity.
        values.insert(name.to_owned(), decode_entities(raw, span.start)?);
        spans.insert(name.to_owned(), span);
    }
    Ok((values, spans))
}

fn declaration(text: &str, content: &[u8]) -> Result<()> {
    let span = source_span(text, content)?;
    let bom = if text.starts_with('\u{feff}') { 3 } else { 0 };
    if span.start != bom + 2 {
        return Err(Error::Forbidden("processing instruction"));
    }
    let (attributes, _) = attributes(
        text,
        quick_xml::events::attributes::Attributes::new(&text[span.clone()], 3),
        &mut 0,
    )?;
    if attributes.get("version").map(String::as_str) != Some("1.0")
        || attributes.get("encoding").is_some_and(|v| !v.eq_ignore_ascii_case("utf-8"))
        || attributes.get("standalone").is_some_and(|v| !matches!(v.as_str(), "yes" | "no"))
        || !attributes.keys().all(|k| matches!(k.as_str(), "version" | "encoding" | "standalone"))
    {
        return Err(malformed(span.start, "unsupported XML declaration"));
    }
    Ok(())
}

/// General XML tokenization, attribute syntax, comments and matching end tags
/// belong to the streaming parser. This loop only enforces the vendor subset,
/// bounds retained data, and records paths and original attribute spans.
fn parse_xml(text: &str) -> Result<Vec<Element>> {
    use quick_xml::events::Event;
    if !text.chars().all(xml_char) {
        return Err(malformed(0, "invalid XML character"));
    }
    let mut reader = quick_xml::Reader::from_str(text);
    reader.config_mut().check_comments = true;
    let mut elements = Vec::new();
    let mut open = Vec::new();
    let mut count = 0;
    loop {
        let event = reader.read_event().map_err(|e| {
            syntax(usize::try_from(reader.error_position()).unwrap_or(text.len()), e)
        })?;
        let empty = matches!(event, Event::Empty(_));
        match event {
            Event::Start(start) | Event::Empty(start) => {
                if elements.len() >= MAX_ELEMENTS {
                    return Err(Error::Limit("element count"));
                }
                if open.len() >= MAX_DEPTH {
                    return Err(Error::Limit("element depth"));
                }
                let name_span = source_span(text, start.name().as_ref())?;
                let name = &text[name_span.clone()];
                if !xml_name(name) {
                    return Err(malformed(name_span.start, "invalid element name"));
                }
                if open.is_empty() && !elements.is_empty() {
                    return Err(malformed(name_span.start, "multiple root elements"));
                }
                let (attributes, spans) = attributes(text, start.attributes(), &mut count)?;
                open.push(name);
                elements.push(Element { path: format!("/{}", open.join("/")), attributes, spans });
                if empty {
                    open.pop();
                }
            }
            Event::End(_) => {
                open.pop();
            }
            Event::Text(value) if value.iter().copied().all(xml_space) => {}
            Event::Comment(_) => {}
            Event::Decl(value) => declaration(text, &value)?,
            Event::PI(_) => return Err(Error::Forbidden("processing instruction")),
            Event::DocType(_) => return Err(Error::Forbidden("DOCTYPE")),
            Event::CData(_) => return Err(Error::Forbidden("CDATA")),
            Event::Text(_) | Event::GeneralRef(_) => {
                return Err(malformed(
                    usize::try_from(reader.buffer_position()).unwrap_or(text.len()),
                    "element text is not supported",
                ));
            }
            Event::Eof => break,
        }
    }
    if elements.is_empty() || !open.is_empty() {
        return Err(malformed(text.len(), "missing or unclosed root"));
    }
    Ok(elements)
}

fn decode_entities(text: &str, offset: usize) -> Result<String> {
    let mut output = String::new();
    let mut rest = text;
    while let Some(index) = rest.find('&') {
        output.push_str(&rest[..index]);
        let entity = &rest[index..];
        let end = entity.find(';').ok_or(malformed(offset, "unterminated entity"))?;
        let name = &entity[1..end];
        let c = match name {
            "amp" => '&',
            "lt" => '<',
            "gt" => '>',
            "quot" => '"',
            "apos" => '\'',
            _ => {
                let n = if let Some(v) = name.strip_prefix("#x") {
                    u32::from_str_radix(v, 16).ok()
                } else if let Some(v) = name.strip_prefix('#') {
                    if v.bytes().all(|b| b.is_ascii_digit()) { v.parse::<u32>().ok() } else { None }
                } else {
                    None
                };
                n.and_then(char::from_u32)
                    .filter(|c| xml_char(*c))
                    .ok_or(malformed(offset, "unsupported XML entity"))?
            }
        };
        output.push(c);
        rest = &entity[end + 1..];
    }
    output.push_str(rest);
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fmt::Write;

    /// A byte order mark and declaration are accepted, values decode their
    /// entities, and an edit changes only the one value, escaped, leaving
    /// the rest of the bytes as they were.
    #[test]
    fn edits_are_local_and_lossless() {
        let xml = b"\xef\xbb\xbf<?xml version=\"1.0\" encoding=\"UTF-8\"?><ParameterRoot><X v='a&gt;b' untouched=\"0.0000001\"/></ParameterRoot>";
        let doc = Document::parse(Kind::Backup, xml).unwrap();
        assert_eq!(doc.attributes("/ParameterRoot/X").unwrap()["v"], "a>b");
        let edited = doc.with_attribute("/ParameterRoot/X", "v", "x<&'\"\n").unwrap();
        assert_eq!(edited.attributes("/ParameterRoot/X").unwrap()["v"], "x<&'\"\n");
        let text = std::str::from_utf8(edited.original()).unwrap();
        assert!(text.contains("untouched=\"0.0000001\""));
        assert!(text.contains("v='x&lt;&amp;&apos;&quot;&#10;'"));
        assert!(matches!(doc.with_attribute("/ParameterRoot/Y", "v", "1"), Err(Error::Missing(_))));
    }

    /// Two roots, unseparated attributes, a raw less-than, a bad comment,
    /// a null reference and a space after the bracket are all refused.
    #[test]
    fn malformed_files_are_refused() {
        for xml in [
            "<ParameterRoot/><ParameterRoot/>",
            "<ParameterRoot a='1'b='2'/>",
            "<ParameterRoot><X v='<bad'/></ParameterRoot>",
            "<ParameterRoot><!--bad--comment--></ParameterRoot>",
            "<ParameterRoot a='&#0;'/>",
            "< ParameterRoot/>",
            "<ParameterRoot>text</ParameterRoot>",
            "<Other/>",
        ] {
            assert!(Document::parse(Kind::Backup, xml.as_bytes()).is_err(), "{xml}");
        }
        assert_eq!(
            Document::parse(Kind::Hardware, b"<!DOCTYPE x><ParameterRoot></ParameterRoot>"),
            Err(Error::Forbidden("DOCTYPE"))
        );
    }

    #[test]
    fn literal_attribute_whitespace_and_long_spans_remain_lossless() {
        let name = "a".repeat(65_537);
        let spaces = " ".repeat(300);
        let xml = format!(
            "<ParameterRoot><X {name}{spaces}={spaces}'before\t\n\rafter&#9;' other=\"unchanged\"/></ParameterRoot>"
        );
        let doc = Document::parse(Kind::Backup, xml.as_bytes()).unwrap();
        assert_eq!(doc.attributes("/ParameterRoot/X").unwrap()[&name], "before\t\n\rafter\t");
        let edited = doc.with_attribute("/ParameterRoot/X", &name, "new & value").unwrap();
        assert_eq!(
            edited.original(),
            xml.replace("before\t\n\rafter&#9;", "new &amp; value").as_bytes()
        );
    }

    #[test]
    fn only_the_vendor_xml_subset_is_accepted() {
        for xml in [
            "<?xml version='1.1'?><ParameterRoot/>",
            "<?xml version='1.0' encoding='UTF-16'?><ParameterRoot/>",
            "<?xml version='1.0' standalone='maybe'?><ParameterRoot/>",
            "<ParameterRoot><?instruction value?></ParameterRoot>",
            "<ParameterRoot><![CDATA[ ]]></ParameterRoot>",
            "<ParameterRoot>&#32;</ParameterRoot>",
            "<!DOCTYPE ParameterRoot [<!ENTITY a 'x'>]><ParameterRoot/>",
            "<ParameterRoot xmlns:x='urn:test'><x:X/></ParameterRoot>",
            "<ParameterRoot><é/></ParameterRoot>",
            "<ParameterRoot a='&unknown;'/>",
            "<ParameterRoot a='1' a='2'/>",
        ] {
            assert!(Document::parse(Kind::Backup, xml.as_bytes()).is_err(), "{xml}");
        }
        let xml = "<?xml\nversion='1.0' encoding='utf-8' standalone='yes'?><ParameterRoot><!-- <!DOCTYPE x> <?pi?> <![CDATA[ ignored ]]> --><X a='é &quot;&apos;&lt;&gt;&amp;&#10;&#x1F600;'/></ParameterRoot>";
        let doc = Document::parse(Kind::Backup, xml.as_bytes()).unwrap();
        assert_eq!(doc.original(), xml.as_bytes());
        assert_eq!(doc.attributes("/ParameterRoot/X").unwrap()["a"], "é \"'<>&\n😀");
    }

    #[test]
    fn resource_limits_bound_the_streaming_reader() {
        let parse = |body: &str| Document::parse(Kind::Backup, body.as_bytes());
        assert_eq!(parse(&" ".repeat(MAX_BYTES + 1)), Err(Error::Limit("size")));
        let xml = format!(
            "<ParameterRoot>{}{}</ParameterRoot>",
            "<X>".repeat(MAX_DEPTH),
            "</X>".repeat(MAX_DEPTH)
        );
        assert_eq!(parse(&xml), Err(Error::Limit("element depth")));
        let xml = format!("<ParameterRoot>{}</ParameterRoot>", "<X/>".repeat(MAX_ELEMENTS));
        assert_eq!(parse(&xml), Err(Error::Limit("element count")));
        let mut attributes = String::new();
        for i in 0..100 {
            write!(attributes, " a{i}='1'").unwrap();
        }
        let xml = format!(
            "<ParameterRoot>{}</ParameterRoot>",
            format!("<X{attributes}/>").repeat(MAX_ATTRIBUTES / 100 + 1)
        );
        assert_eq!(parse(&xml), Err(Error::Limit("attribute count")));
        let xml = format!(
            "<ParameterRoot>{}{}</ParameterRoot>",
            "<X>".repeat(MAX_DEPTH - 1),
            "</X>".repeat(MAX_DEPTH - 1)
        );
        assert!(parse(&xml).is_ok());
    }

    /// A path present twice is reported as ambiguous when read, and the
    /// bundle reads a group from its own file before the backup.
    #[test]
    fn duplicates_are_ambiguous_and_bundles_prefer_the_owning_file() {
        let doc = Document::parse(
            Kind::Backup,
            b"<ParameterRoot><PManuParam><MC ManuAcc='1'/><MC ManuAcc='2'/></PManuParam></ParameterRoot>",
        )
        .unwrap();
        assert!(matches!(doc.attributes("/ParameterRoot/PManuParam/MC"), Err(Error::Duplicate(_))));
        let backup = Document::parse(Kind::Backup, b"<ParameterRoot><PManuParam><MC ManuAcc='1'/></PManuParam><PSoftParam><SP m_iEnableLaserType='1'/></PSoftParam></ParameterRoot>").unwrap();
        let manual = Document::parse(
            Kind::Manual,
            b"<ParameterRoot><PManuParam><MC ManuAcc='2'/></PManuParam></ParameterRoot>",
        )
        .unwrap();
        let mut bundle = Bundle::from_backup(backup);
        assert_eq!(bundle.group("ManuParam", "MC").unwrap()["ManuAcc"], "1");
        bundle.insert(manual);
        assert_eq!(bundle.group("ManuParam", "MC").unwrap()["ManuAcc"], "2");
        assert_eq!(bundle.saved_laser_mode().unwrap(), Some(1));
        assert!(matches!(bundle.group("GasParam", "MGP"), Err(Error::Missing(_))));
    }
}
