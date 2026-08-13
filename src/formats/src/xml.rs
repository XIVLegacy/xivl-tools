//! A reader for the XML subsets the client's documents use.
//!
//! Two document families reach this reader and they use different subsets,
//! so the profile is named by the caller rather than sniffed.
//!
//! The SSD documents stay inside a closed grammar: seven element names,
//! fourteen attribute names, double-quoted attribute values, and no entity
//! reference, comment, CDATA section, doctype, or namespace anywhere
//! (`docs/formats/ssd-sheet.md`, "The SSD document subset").
//!
//! The widget documents inside SQEX containers add exactly two constructs
//! and nothing else: comments, in 12 of the 1155 documents, and ampersands
//! in text and attribute values, in 5 of them. Both are measured over the
//! whole install (`docs/formats/sqex.md`, "What the widget documents
//! are"). An ampersand run is carried verbatim and never expanded, so the
//! text a caller gets back is the text the file holds.
//!
//! Each profile accepts exactly what its family uses and refuses the rest
//! with `unsupported-xml-construct`, because a parser that accepts more
//! than the format uses is a parser whose behavior nobody has evidence
//! for.
//!
//! The reader is byte-oriented so every error carries a byte offset into
//! the original input. Names and values are cut at ASCII delimiters, which
//! never fall inside a multi-byte UTF-8 sequence, so the slices between
//! them are valid UTF-8 once the whole input is.

use crate::error::{ErrorKind, FormatError, Result};
use crate::reader::Span;

/// The UTF-8 byte order mark the client writes at the start of every one
/// of these documents.
pub const BYTE_ORDER_MARK: &[u8; 3] = &[0xEF, 0xBB, 0xBF];

/// One attribute, verbatim and in document order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attribute {
    pub name: String,
    pub value: String,
    pub span: Span,
}

/// One element and everything under it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Element {
    pub name: String,
    /// The whole element, opening delimiter through closing delimiter.
    pub span: Span,
    pub attributes: Vec<Attribute>,
    pub children: Vec<Element>,
    /// Character data directly inside this element, concatenated in
    /// document order. Elements with children carry only whitespace here.
    pub text: String,
}

impl Element {
    pub fn attribute(&self, name: &str) -> Option<&Attribute> {
        self.attributes
            .iter()
            .find(|attribute| attribute.name == name)
    }

    pub fn children_named<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Element> {
        self.children.iter().filter(move |child| child.name == name)
    }

    pub fn child(&self, name: &str) -> Option<&Element> {
        self.children.iter().find(|child| child.name == name)
    }

    /// The element text with surrounding whitespace removed.
    pub fn trimmed_text(&self) -> &str {
        self.text.trim()
    }
}

/// The `<?xml ... ?>` declaration, when the document opens with one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Declaration {
    pub version: String,
    pub encoding: Option<String>,
    pub span: Span,
}

/// A whole document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Document {
    pub byte_order_mark: bool,
    pub declaration: Option<Declaration>,
    pub root: Element,
    /// How many comments the document held. A comment is neither element
    /// nor content, so it is counted rather than kept. A count still says
    /// the document had them.
    pub comments: u64,
}

/// Which family's grammar to accept.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Profile {
    /// The SSD documents: no comment and no ampersand anywhere.
    #[default]
    SsdSubset,
    /// The widget documents inside SQEX containers, which add comments and
    /// verbatim ampersand runs.
    SqwtWidget,
}

impl Profile {
    fn allows_comments(self) -> bool {
        matches!(self, Profile::SqwtWidget)
    }

    fn allows_ampersand(self) -> bool {
        matches!(self, Profile::SqwtWidget)
    }
}

// Bounds recursive descent before hostile input reaches the process stack.
const MAX_ELEMENT_DEPTH: u32 = 128;

/// Parse a document from its bytes, in the SSD subset.
pub fn parse_document(data: &[u8]) -> Result<Document> {
    parse_document_with(data, Profile::SsdSubset)
}

/// Parse a document from its bytes, in a named profile.
pub fn parse_document_with(data: &[u8], profile: Profile) -> Result<Document> {
    let byte_order_mark = data.starts_with(BYTE_ORDER_MARK);
    let start = if byte_order_mark {
        BYTE_ORDER_MARK.len()
    } else {
        0
    };

    // One UTF-8 check over the whole input, so every later slice taken at
    // an ASCII delimiter is known to be valid text.
    if let Err(error) = std::str::from_utf8(&data[start..]) {
        return Err(FormatError::new(
            ErrorKind::InvalidUtf8,
            (start + error.valid_up_to()) as u64,
            "document is not valid UTF-8",
        ));
    }

    let mut parser = Parser {
        data,
        position: start,
        profile,
        comments: 0,
    };
    parser.skip_trivia()?;
    let declaration = parser.parse_declaration()?;
    parser.skip_trivia()?;
    let root = parser.parse_element(0)?;
    parser.skip_trivia()?;
    if parser.position < data.len() {
        return Err(parser.error(
            ErrorKind::MalformedXml,
            parser.position,
            "content after the root element",
        ));
    }
    Ok(Document {
        byte_order_mark,
        declaration,
        root,
        comments: parser.comments,
    })
}

struct Parser<'a> {
    data: &'a [u8],
    position: usize,
    profile: Profile,
    comments: u64,
}

impl<'a> Parser<'a> {
    fn error(&self, kind: ErrorKind, offset: usize, detail: impl Into<String>) -> FormatError {
        FormatError::new(kind, offset as u64, detail)
    }

    fn peek(&self) -> Option<u8> {
        self.data.get(self.position).copied()
    }

    fn starts_with(&self, prefix: &[u8]) -> bool {
        self.data[self.position..].starts_with(prefix)
    }

    fn skip_whitespace(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\r' | b'\n')) {
            self.position += 1;
        }
    }

    /// Whitespace, and comments where the profile has them.
    fn skip_trivia(&mut self) -> Result<()> {
        loop {
            self.skip_whitespace();
            if !self.starts_with(b"<!--") {
                return Ok(());
            }
            self.skip_comment()?;
        }
    }

    /// A comment, which the widget documents use and the SSD documents do
    /// not. The text is not kept: a comment is neither element nor content,
    /// and this reader reports structure.
    fn skip_comment(&mut self) -> Result<()> {
        if !self.profile.allows_comments() {
            return Err(self.error(
                ErrorKind::UnsupportedXmlConstruct,
                self.position,
                "comment, CDATA section, or doctype declaration",
            ));
        }
        let start = self.position;
        self.position += b"<!--".len();
        while !self.starts_with(b"-->") {
            if self.peek().is_none() {
                return Err(self.error(
                    ErrorKind::MalformedXml,
                    start,
                    "input ends inside a comment",
                ));
            }
            self.position += 1;
        }
        self.position += b"-->".len();
        self.comments += 1;
        Ok(())
    }

    /// Slice text between two byte positions. Both ends sit on ASCII
    /// delimiters and the whole input was validated, so this cannot fail.
    fn text(&self, start: usize, end: usize) -> String {
        String::from_utf8_lossy(&self.data[start..end]).into_owned()
    }

    fn parse_declaration(&mut self) -> Result<Option<Declaration>> {
        if !self.starts_with(b"<?xml") {
            if self.starts_with(b"<?") {
                return Err(self.error(
                    ErrorKind::UnsupportedXmlConstruct,
                    self.position,
                    "processing instruction",
                ));
            }
            return Ok(None);
        }
        let start = self.position;
        self.position += b"<?xml".len();
        let attributes = self.parse_attributes()?;
        if !self.starts_with(b"?>") {
            return Err(self.error(
                ErrorKind::MalformedXml,
                self.position,
                "xml declaration does not close with '?>'",
            ));
        }
        self.position += 2;

        let version = attributes
            .iter()
            .find(|attribute| attribute.name == "version")
            .map(|attribute| attribute.value.clone())
            .ok_or_else(|| {
                self.error(
                    ErrorKind::MalformedXml,
                    start,
                    "xml declaration has no version",
                )
            })?;
        let encoding = attributes
            .iter()
            .find(|attribute| attribute.name == "encoding")
            .map(|attribute| attribute.value.clone());
        Ok(Some(Declaration {
            version,
            encoding,
            span: Span::new(start as u64, (self.position - start) as u64),
        }))
    }

    fn parse_attributes(&mut self) -> Result<Vec<Attribute>> {
        let mut attributes = Vec::new();
        loop {
            self.skip_whitespace();
            match self.peek() {
                None => {
                    return Err(self.error(
                        ErrorKind::MalformedXml,
                        self.position,
                        "input ends inside a tag",
                    ))
                }
                Some(b'>' | b'/' | b'?') => return Ok(attributes),
                Some(_) => {}
            }

            let name_start = self.position;
            while matches!(self.peek(), Some(byte) if is_name_byte(byte)) {
                self.position += 1;
            }
            if self.position == name_start {
                return Err(self.error(
                    ErrorKind::MalformedXml,
                    self.position,
                    "expected an attribute name",
                ));
            }
            let name = self.text(name_start, self.position);
            self.skip_whitespace();
            if self.peek() != Some(b'=') {
                return Err(self.error(
                    ErrorKind::MalformedXml,
                    self.position,
                    format!("attribute '{name}' has no value"),
                ));
            }
            self.position += 1;
            self.skip_whitespace();
            if self.peek() == Some(b'\'') {
                return Err(self.error(
                    ErrorKind::UnsupportedXmlConstruct,
                    self.position,
                    "single-quoted attribute value",
                ));
            }
            if self.peek() != Some(b'"') {
                return Err(self.error(
                    ErrorKind::MalformedXml,
                    self.position,
                    format!("attribute '{name}' value is not quoted"),
                ));
            }
            self.position += 1;
            let value_start = self.position;
            loop {
                match self.peek() {
                    None => {
                        return Err(self.error(
                            ErrorKind::MalformedXml,
                            self.position,
                            "input ends inside an attribute value",
                        ))
                    }
                    Some(b'"') => break,
                    Some(b'&') if !self.profile.allows_ampersand() => {
                        return Err(self.error(
                            ErrorKind::UnsupportedXmlConstruct,
                            self.position,
                            "entity reference in an attribute value",
                        ))
                    }
                    Some(b'<') => {
                        return Err(self.error(
                            ErrorKind::MalformedXml,
                            self.position,
                            "'<' in an attribute value",
                        ))
                    }
                    Some(_) => self.position += 1,
                }
            }
            let value = self.text(value_start, self.position);
            self.position += 1;
            attributes.push(Attribute {
                name,
                value,
                span: Span::new(name_start as u64, (self.position - name_start) as u64),
            });
        }
    }

    fn parse_element(&mut self, depth: u32) -> Result<Element> {
        if depth >= MAX_ELEMENT_DEPTH {
            return Err(self.error(
                ErrorKind::NestingTooDeep,
                self.position,
                format!("element nesting exceeds the depth limit of {MAX_ELEMENT_DEPTH}"),
            ));
        }
        let start = self.position;
        if self.peek() != Some(b'<') {
            return Err(self.error(
                ErrorKind::MalformedXml,
                self.position,
                "expected an element",
            ));
        }
        if self.starts_with(b"<!") {
            return Err(self.error(
                ErrorKind::UnsupportedXmlConstruct,
                self.position,
                "comment, CDATA section, or doctype declaration",
            ));
        }
        self.position += 1;

        let name_start = self.position;
        while matches!(self.peek(), Some(byte) if is_name_byte(byte)) {
            self.position += 1;
        }
        if self.position == name_start {
            return Err(self.error(
                ErrorKind::MalformedXml,
                self.position,
                "expected an element name",
            ));
        }
        let name = self.text(name_start, self.position);
        if name.contains(':') {
            return Err(self.error(
                ErrorKind::UnsupportedXmlConstruct,
                name_start,
                "namespace-qualified element name",
            ));
        }
        let attributes = self.parse_attributes()?;

        if self.starts_with(b"/>") {
            self.position += 2;
            return Ok(Element {
                name,
                span: Span::new(start as u64, (self.position - start) as u64),
                attributes,
                children: Vec::new(),
                text: String::new(),
            });
        }
        if self.peek() != Some(b'>') {
            return Err(self.error(
                ErrorKind::MalformedXml,
                self.position,
                format!("element '{name}' opening tag does not close"),
            ));
        }
        self.position += 1;

        let mut children = Vec::new();
        let mut text = String::new();
        loop {
            let run_start = self.position;
            loop {
                match self.peek() {
                    None => {
                        return Err(self.error(
                            ErrorKind::MalformedXml,
                            self.position,
                            format!("input ends before '</{name}>'"),
                        ))
                    }
                    Some(b'<') => break,
                    Some(b'&') if !self.profile.allows_ampersand() => {
                        return Err(self.error(
                            ErrorKind::UnsupportedXmlConstruct,
                            self.position,
                            "entity reference in element content",
                        ))
                    }
                    Some(_) => self.position += 1,
                }
            }
            text.push_str(&self.text(run_start, self.position));

            if self.starts_with(b"<!--") {
                self.skip_comment()?;
                continue;
            }
            if self.starts_with(b"</") {
                let close_start = self.position;
                self.position += 2;
                let close_name_start = self.position;
                while matches!(self.peek(), Some(byte) if is_name_byte(byte)) {
                    self.position += 1;
                }
                let close_name = self.text(close_name_start, self.position);
                self.skip_whitespace();
                if self.peek() != Some(b'>') {
                    return Err(self.error(
                        ErrorKind::MalformedXml,
                        self.position,
                        format!("end tag for '{name}' does not close"),
                    ));
                }
                self.position += 1;
                if close_name != name {
                    return Err(self.error(
                        ErrorKind::MalformedXml,
                        close_start,
                        format!("end tag '</{close_name}>' does not match '<{name}>'"),
                    ));
                }
                return Ok(Element {
                    name,
                    span: Span::new(start as u64, (self.position - start) as u64),
                    attributes,
                    children,
                    text,
                });
            }
            children.push(self.parse_element(depth + 1)?);
        }
    }
}

fn is_name_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.' | b':')
}

/// Read an attribute as an unsigned decimal value.
pub fn attribute_u32(element: &Element, name: &str) -> Result<u32> {
    let attribute = element.attribute(name).ok_or_else(|| {
        FormatError::new(
            ErrorKind::MalformedXml,
            element.span.offset,
            format!("element '{}' has no '{name}' attribute", element.name),
        )
    })?;
    parse_u32(&attribute.value, attribute.span.offset, name)
}

/// Read an attribute as a signed decimal value. `cache` is `-1` in most
/// documents and `0` in one, so it cannot be unsigned.
pub fn attribute_i64(element: &Element, name: &str) -> Result<i64> {
    let attribute = element.attribute(name).ok_or_else(|| {
        FormatError::new(
            ErrorKind::MalformedXml,
            element.span.offset,
            format!("element '{}' has no '{name}' attribute", element.name),
        )
    })?;
    attribute.value.trim().parse::<i64>().map_err(|_| {
        FormatError::new(
            ErrorKind::InvalidAttributeValue,
            attribute.span.offset,
            format!("'{name}' is not a decimal integer: '{}'", attribute.value),
        )
    })
}

/// Read decimal text, naming what it was supposed to be.
pub fn parse_u32(text: &str, offset: u64, what: &str) -> Result<u32> {
    text.trim().parse::<u32>().map_err(|_| {
        FormatError::new(
            ErrorKind::InvalidAttributeValue,
            offset,
            format!("{what} is not a 32-bit decimal value: '{text}'"),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn document(text: &str) -> Result<Document> {
        parse_document(text.as_bytes())
    }

    #[test]
    fn reads_the_shape_the_client_writes() {
        let parsed = parse_document(
            b"\xEF\xBB\xBF<?xml version=\"1.0\" encoding=\"utf-8\"?>\r\n\
              <ssd version=\"0.1\">\r\n  <sheet name=\"a\" infofile=\"7\" />\r\n</ssd>",
        )
        .unwrap();
        assert!(parsed.byte_order_mark);
        let declaration = parsed.declaration.unwrap();
        assert_eq!(declaration.version, "1.0");
        assert_eq!(declaration.encoding.as_deref(), Some("utf-8"));
        assert_eq!(parsed.root.name, "ssd");
        assert_eq!(parsed.root.attribute("version").unwrap().value, "0.1");
        let sheet = &parsed.root.children[0];
        assert_eq!(sheet.name, "sheet");
        assert_eq!(attribute_u32(sheet, "infofile").unwrap(), 7);
    }

    #[test]
    fn keeps_text_and_nesting() {
        let parsed = document("<a><b>str</b><b> 12 </b></a>").unwrap();
        let values: Vec<&str> = parsed
            .root
            .children_named("b")
            .map(Element::trimmed_text)
            .collect();
        assert_eq!(values, ["str", "12"]);
    }

    #[test]
    fn refuses_constructs_outside_the_subset() {
        for (text, offset) in [
            ("<a><!-- note --></a>", 3),
            ("<a><![CDATA[x]]></a>", 3),
            ("<!DOCTYPE a><a/>", 0),
            ("<a b='1'/>", 5),
            ("<a b=\"&amp;\"/>", 6),
            ("<a>&amp;</a>", 3),
            ("<a:b/>", 1),
            ("<?php ?><a/>", 0),
        ] {
            let error = document(text).unwrap_err();
            assert_eq!(
                error.kind(),
                ErrorKind::UnsupportedXmlConstruct,
                "{text}: {error}"
            );
            assert_eq!(error.offset(), offset, "{text}");
        }
    }

    #[test]
    fn the_widget_profile_adds_comments_and_ampersands_and_nothing_else() {
        let widget = |text: &str| parse_document_with(text.as_bytes(), Profile::SqwtWidget);

        let parsed = widget("<!-- lead --><a><!-- inner -->text &amp; more &<b/></a>").unwrap();
        assert_eq!(parsed.root.children.len(), 1);
        // The ampersand runs are carried verbatim, not expanded, so the
        // text a caller reads back is the text the file holds.
        assert_eq!(parsed.root.trimmed_text(), "text &amp; more &");

        assert_eq!(
            widget("<a b=\"x &amp; y\"/>")
                .unwrap()
                .root
                .attribute("b")
                .unwrap()
                .value,
            "x &amp; y"
        );

        // Everything the SSD profile refuses beyond those two is still
        // refused here.
        for (text, offset) in [
            ("<a><![CDATA[x]]></a>", 3),
            ("<!DOCTYPE a><a/>", 0),
            ("<a b='1'/>", 5),
            ("<a:b/>", 1),
            ("<?php ?><a/>", 0),
        ] {
            let error = widget(text).unwrap_err();
            assert_eq!(
                error.kind(),
                ErrorKind::UnsupportedXmlConstruct,
                "{text}: {error}"
            );
            assert_eq!(error.offset(), offset, "{text}");
        }

        // And a comment that never closes is malformed, not accepted.
        let error = widget("<a><!-- open </a>").unwrap_err();
        assert_eq!(error.kind(), ErrorKind::MalformedXml);
        assert_eq!(error.offset(), 3);
    }

    #[test]
    fn refuses_malformed_documents() {
        for text in [
            "<a>",
            "<a></b>",
            "<a b/>",
            "<a b=1/>",
            "<a><b></b>",
            "<a/>tail",
            "<a/><b/>",
            "<",
            "",
        ] {
            let error = document(text).unwrap_err();
            assert_eq!(error.kind(), ErrorKind::MalformedXml, "{text}: {error}");
        }
    }

    #[test]
    fn refuses_bytes_that_are_not_utf8() {
        let error = parse_document(b"\xEF\xBB\xBF<a>\xC3\x28</a>").unwrap_err();
        assert_eq!(error.kind(), ErrorKind::InvalidUtf8);
        assert_eq!(error.offset(), 6);
    }

    #[test]
    fn multibyte_text_survives() {
        let parsed = document("<a>\u{30A8}\u{30E9}</a>").unwrap();
        assert_eq!(parsed.root.trimmed_text(), "\u{30A8}\u{30E9}");
    }
}
